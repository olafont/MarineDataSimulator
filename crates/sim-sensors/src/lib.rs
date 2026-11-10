//! Instrument models: turn the simulation's observed channels into timed, imperfect measurements.
//!
//! Error sources per sensor: update rate, Gaussian noise (`noise_scale`), angular bias
//! (compass/wind-vane misalignment), speed calibration (`speed_scale`), latency, resolution,
//! compass deviation, and masthead heel error. The wind instrument derives true wind from its
//! *measured* apparent wind, the speed log's reading and the compass, as real systems do, so
//! calibration errors propagate realistically.

use std::collections::{HashMap, VecDeque};

use rand::rngs::StdRng;
use rand::{RngExt as _, SeedableRng};
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};
use sim_core::geo::{Ne, norm_180, norm_360};
use sim_core::ais::{AisClass, AisMessage, AisTarget, AisTargetConfig};
use sim_core::{Channel, Channels, LatLon, Simulation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    Gps,
    Compass,
    SpeedLog,
    DepthSounder,
    Wind,
    WaterTemp,
    Weather,
    Rudder,
    Attitude,
    Engine,
    /// AIS transponder: other stations (`!AIVDM`) and own ship (`!AIVDO`).
    Ais,
}

impl SensorKind {
    pub const ALL: &'static [SensorKind] = &[
        SensorKind::Gps,
        SensorKind::Compass,
        SensorKind::SpeedLog,
        SensorKind::DepthSounder,
        SensorKind::Wind,
        SensorKind::WaterTemp,
        SensorKind::Weather,
        SensorKind::Rudder,
        SensorKind::Attitude,
        SensorKind::Engine,
        SensorKind::Ais,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SensorKind::Gps => "GPS",
            SensorKind::Compass => "Compass",
            SensorKind::SpeedLog => "Speed log",
            SensorKind::DepthSounder => "Depth sounder",
            SensorKind::Wind => "Wind",
            SensorKind::WaterTemp => "Water temperature",
            SensorKind::Weather => "Weather station",
            SensorKind::Rudder => "Rudder angle",
            SensorKind::Attitude => "Attitude",
            SensorKind::Engine => "Engine",
            SensorKind::Ais => "AIS transponder",
        }
    }

    /// Conventional NMEA 0183 talker ID.
    pub fn default_talker(self) -> &'static str {
        match self {
            SensorKind::Gps => "GP",
            SensorKind::Compass => "HC",
            SensorKind::SpeedLog => "VW",
            SensorKind::DepthSounder => "SD",
            SensorKind::Wind => "WI",
            SensorKind::WaterTemp => "YX",
            SensorKind::Weather => "WI",
            SensorKind::Rudder => "AG",
            SensorKind::Attitude => "II",
            SensorKind::Engine => "ER",
            SensorKind::Ais => "AI",
        }
    }

    pub fn default_rate_hz(self) -> f64 {
        match self {
            SensorKind::Gps => 1.0,
            SensorKind::Compass => 10.0,
            SensorKind::SpeedLog => 2.0,
            SensorKind::DepthSounder => 1.0,
            SensorKind::Wind => 4.0,
            SensorKind::WaterTemp => 0.5,
            SensorKind::Weather => 0.2,
            SensorKind::Rudder => 5.0,
            SensorKind::Attitude => 10.0,
            SensorKind::Engine => 2.0,
            // Polled every tick; each station keeps its own reporting schedule.
            SensorKind::Ais => 10.0,
        }
    }

    /// Default resolution of the sensor's primary readings (0 = encoder precision only).
    fn default_resolution(self) -> f64 {
        match self {
            SensorKind::Wind => 1.0,
            SensorKind::DepthSounder => 0.1,
            SensorKind::Engine => 10.0,
            _ => 0.0,
        }
    }
}

/// Compass deviation (degrees) as the classic five-coefficient curve:
/// `A + B·sin h + C·cos h + D·sin 2h + E·cos 2h`, with `h` the magnetic heading.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Deviation {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
}

impl Deviation {
    pub fn at(&self, magnetic_heading: f64) -> f64 {
        let h = magnetic_heading.to_radians();
        self.a + self.b * h.sin() + self.c * h.cos() + self.d * (2.0 * h).sin() + self.e * (2.0 * h).cos()
    }

    pub fn is_zero(&self) -> bool {
        *self == Deviation::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SensorConfig {
    pub kind: SensorKind,
    pub enabled: bool,
    pub rate_hz: f64,
    pub talker: String,
    /// 0 = perfect instrument, 1 = typical real-world noise.
    pub noise_scale: f64,
    /// Compass: heading error. Wind: vane misalignment (added to the apparent wind angle).
    pub angle_bias_deg: f64,
    /// Speed log: STW calibration factor. Wind: AWS calibration factor.
    pub speed_scale: f64,
    /// Delay between the physical value and its report, seconds (max 10).
    pub latency_s: f64,
    /// Quantum of the primary readings (0 = encoder precision only).
    pub resolution: f64,
    /// Depth sounder only: transducer depth below the waterline, metres.
    pub transducer_depth_m: f64,
    /// GPS only: antenna height above mean sea level, metres.
    pub antenna_altitude_m: f64,
    /// Compass only: deviation curve (reported in HDG; HDT is corrected).
    #[serde(skip_serializing_if = "Deviation::is_zero")]
    pub deviation: Deviation,
}

impl SensorConfig {
    pub fn new(kind: SensorKind) -> Self {
        Self {
            kind,
            enabled: true,
            rate_hz: kind.default_rate_hz(),
            talker: kind.default_talker().into(),
            noise_scale: 1.0,
            angle_bias_deg: 0.0,
            speed_scale: 1.0,
            latency_s: 0.0,
            resolution: kind.default_resolution(),
            transducer_depth_m: 0.5,
            antenna_altitude_m: 2.0,
            deviation: Deviation::default(),
        }
    }

    pub fn defaults() -> Vec<SensorConfig> {
        SensorKind::ALL.iter().map(|&k| SensorConfig::new(k)).collect()
    }
}

impl Default for SensorConfig {
    fn default() -> Self {
        SensorConfig::new(SensorKind::Gps)
    }
}

/// One instrument reading. Angles in degrees, speeds in knots, depths in metres.
#[derive(Debug, Clone, PartialEq)]
pub enum Measurement {
    Gps {
        time_unix: f64,
        lat: f64,
        lon: f64,
        sog_kn: f64,
        cog_deg: f64,
        mag_var_deg: f64,
        satellites: u8,
        hdop: f64,
        altitude_m: f64,
    },
    Heading {
        heading_true: f64,
        /// Compass deviation at this heading (E positive).
        deviation_deg: f64,
        mag_var_deg: f64,
        rot_dpm: f64,
    },
    Speed {
        stw_kn: f64,
        heading_true: f64,
        mag_var_deg: f64,
    },
    Depth {
        below_transducer_m: f64,
        transducer_depth_m: f64,
    },
    Wind {
        awa: f64,
        aws_kn: f64,
        twa: f64,
        tws_kn: f64,
        twd: f64,
        mag_var_deg: f64,
    },
    WaterTemp {
        celsius: f64,
    },
    Rudder {
        starboard_deg: f64,
    },
    Weather {
        air_temp_c: f64,
        pressure_hpa: f64,
        humidity_pct: f64,
        water_temp_c: f64,
        twd: f64,
        tws_kn: f64,
        mag_var_deg: f64,
    },
    Attitude {
        /// + = starboard side down.
        roll_deg: f64,
        /// + = bow up.
        pitch_deg: f64,
        heave_m: f64,
    },
    Engine {
        rpm: f64,
    },
    /// One AIS message. `channel` is the VHF channel (`A`/`B`); `seq_id` tags multi-sentence
    /// messages.
    Ais {
        own: bool,
        channel: char,
        seq_id: u8,
        message: AisMessage,
    },
}

/// A measurement together with who produced it and when (simulated UTC).
#[derive(Debug, Clone, PartialEq)]
pub struct Emission {
    pub sensor: SensorKind,
    pub talker: String,
    pub time_unix: f64,
    pub measurement: Measurement,
}

struct Slot {
    cfg: SensorConfig,
    next_due: f64,
}

const MAX_LATENCY_S: f64 = 10.0;

pub struct SensorSuite {
    slots: Vec<Slot>,
    rng: StdRng,
    /// Recent channel values, for latency.
    history: VecDeque<(f64, Channels)>,
    ais: AisScheduler,
}

/// Next transmission times of one AIS station.
#[derive(Debug, Clone, Copy)]
struct AisDue {
    position: f64,
    statics: f64,
    text: f64,
}

/// Per-station AIS reporting schedule (the own ship uses key `u64::MAX`).
#[derive(Debug, Default)]
struct AisScheduler {
    due: HashMap<u64, AisDue>,
    channel_b: bool,
    seq_id: u8,
    last_time: f64,
}

const OWN_SHIP_KEY: u64 = u64::MAX;

fn quantize(v: f64, q: f64) -> f64 {
    if q > 0.0 { (v / q).round() * q } else { v }
}

impl SensorSuite {
    pub fn new(configs: Vec<SensorConfig>, seed: u64) -> Self {
        let slots = configs.into_iter().map(|cfg| Slot { cfg, next_due: f64::NEG_INFINITY }).collect();
        Self { slots, rng: StdRng::seed_from_u64(seed), history: VecDeque::new(), ais: AisScheduler::default() }
    }

    pub fn configs(&self) -> Vec<SensorConfig> {
        self.slots.iter().map(|s| s.cfg.clone()).collect()
    }

    /// Replaces a sensor's configuration (matched by kind), or adds it.
    pub fn set_config(&mut self, cfg: SensorConfig) {
        match self.slots.iter_mut().find(|s| s.cfg.kind == cfg.kind) {
            Some(s) => {
                s.cfg = cfg;
                s.next_due = f64::NEG_INFINITY;
            }
            None => self.slots.push(Slot { cfg, next_due: f64::NEG_INFINITY }),
        }
    }

    fn config_of(&self, kind: SensorKind) -> Option<&SensorConfig> {
        self.slots.iter().map(|s| &s.cfg).find(|c| c.kind == kind)
    }

    /// Channel values as they were `delay` seconds ago (oldest available if not that old).
    fn delayed(&self, now: f64, delay: f64) -> &Channels {
        let target = now - delay.clamp(0.0, MAX_LATENCY_S);
        self.history
            .iter()
            .rev()
            .find(|(t, _)| *t <= target + 1e-9)
            .or(self.history.front())
            .map(|(_, c)| c)
            .expect("history is never empty during poll")
    }

    /// Emits every reading that is due at the simulation's current time.
    pub fn poll(&mut self, sim: &Simulation) -> Vec<Emission> {
        let now = sim.time_unix;
        // Time going backwards (scenario reset) invalidates the history.
        if self.history.back().is_some_and(|(t, _)| *t > now) {
            self.history.clear();
        }
        if self.history.back().is_none_or(|(t, _)| *t < now) {
            self.history.push_back((now, sim.channels.clone()));
        }
        while self.history.len() > 2 && self.history[1].0 < now - MAX_LATENCY_S {
            self.history.pop_front();
        }

        let mut out = Vec::new();
        for i in 0..self.slots.len() {
            let slot = &self.slots[i];
            if slot.cfg.kind == SensorKind::Ais {
                if slot.cfg.enabled {
                    let cfg = slot.cfg.clone();
                    out.extend(self.poll_ais(&cfg, sim));
                }
                continue;
            }
            if !slot.cfg.enabled || slot.cfg.rate_hz <= 0.0 || now < slot.next_due {
                continue;
            }
            let period = 1.0 / slot.cfg.rate_hz;
            let cfg = slot.cfg.clone();
            let next = slot.next_due + period;
            // Catch up after pauses or time jumps instead of bursting.
            self.slots[i].next_due = if next <= now { now + period } else { next };
            let ch = self.delayed(now, cfg.latency_s).clone();
            out.push(Emission {
                sensor: cfg.kind,
                talker: cfg.talker.clone(),
                time_unix: now,
                measurement: self.measure(&cfg, &ch, now),
            });
        }
        out
    }

    /// AIS messages due now: each station follows its own intervals. Positions carry the
    /// station's GNSS noise (`noise_scale`); the own ship reports its observed channels.
    fn poll_ais(&mut self, cfg: &SensorConfig, sim: &Simulation) -> Vec<Emission> {
        let now = sim.time_unix;
        if now < self.ais.last_time {
            self.ais.due.clear(); // time went backwards (reset)
        }
        self.ais.last_time = now;

        let mut stations: Vec<(u64, bool, AisTarget)> = sim
            .ais_targets
            .iter()
            .filter(|t| t.cfg.enabled)
            .map(|t| (t.id, false, t.clone()))
            .collect();
        if sim.vessel_cfg.ais.enabled {
            stations.push((OWN_SHIP_KEY, true, own_ship_station(sim)));
        }
        let live: Vec<u64> = stations.iter().map(|s| s.0).collect();
        self.ais.due.retain(|k, _| live.contains(k));

        let k = cfg.noise_scale.max(0.0);
        let mut out = Vec::new();
        for (key, own, mut station) in stations {
            let interval = station.position_interval();
            let due = *self.ais.due.entry(key).or_insert_with(|| {
                // Stagger first reports so a scenario start is not one burst.
                let phase = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64;
                AisDue { position: now + phase * interval.min(10.0), statics: now + 1.0 + phase * 5.0, text: now + 2.0 + phase * 10.0 }
            });
            let mut messages = Vec::new();
            let mut next = due;
            if now >= due.position {
                if station.cfg.class.is_mobile() && !own {
                    let s = &mut station.state;
                    s.position = s.position.offset_ne(gaussian(&mut self.rng, 2.0 * k), gaussian(&mut self.rng, 2.0 * k));
                    if s.sog_kn > 0.0 {
                        s.sog_kn = (s.sog_kn + gaussian(&mut self.rng, 0.05 * k)).max(0.0);
                        s.cog = norm_360(s.cog + gaussian(&mut self.rng, (0.5 + 2.0 / s.sog_kn.max(0.1)).min(30.0) * k));
                    }
                }
                messages.extend(station.position_messages(now));
                next.position = schedule(due.position, interval, now);
            }
            if now >= due.statics {
                messages.extend(station.static_messages());
                next.statics = schedule(due.statics, station.cfg.intervals.static_s.max(1.0), now);
            }
            if now >= due.text {
                messages.extend(station.safety_text());
                next.text = schedule(due.text, station.cfg.intervals.safety_text_s.max(1.0), now);
            }
            self.ais.due.insert(key, next);
            for message in messages {
                self.ais.channel_b = !self.ais.channel_b;
                self.ais.seq_id = (self.ais.seq_id + 1) % 10;
                out.push(Emission {
                    sensor: SensorKind::Ais,
                    talker: cfg.talker.clone(),
                    time_unix: now,
                    measurement: Measurement::Ais {
                        own,
                        channel: if self.ais.channel_b { 'B' } else { 'A' },
                        seq_id: self.ais.seq_id,
                        message,
                    },
                });
            }
        }
        out
    }

    fn noise(&mut self, sigma: f64) -> f64 {
        gaussian(&mut self.rng, sigma)
    }

    fn measure(&mut self, cfg: &SensorConfig, ch: &Channels, now: f64) -> Measurement {
        let k = cfg.noise_scale.max(0.0);
        let q = cfg.resolution;
        let var = ch.get(Channel::MagVariation);
        match cfg.kind {
            SensorKind::Gps => {
                let p = sim_core::LatLon::new(ch.get(Channel::Latitude), ch.get(Channel::Longitude))
                    .offset_ne(self.noise(2.0 * k), self.noise(2.0 * k));
                let sog = (ch.get(Channel::Sog) + self.noise(0.05 * k)).max(0.0);
                // COG gets noisier as the vessel slows down.
                let cog_sigma = (0.5 + 2.0 / sog.max(0.1)).min(30.0) * k;
                Measurement::Gps {
                    time_unix: now - cfg.latency_s.clamp(0.0, MAX_LATENCY_S),
                    lat: p.lat,
                    lon: p.lon,
                    sog_kn: sog,
                    cog_deg: norm_360(ch.get(Channel::Cog) + self.noise(cog_sigma)),
                    mag_var_deg: var,
                    satellites: if k > 0.0 { self.rng.random_range(8..=12) } else { 10 },
                    hdop: 0.9 + self.noise(0.1 * k).abs(),
                    altitude_m: cfg.antenna_altitude_m + self.noise(3.0 * k),
                }
            }
            SensorKind::Compass => {
                let heading = self.compass_heading(cfg, ch);
                Measurement::Heading {
                    heading_true: quantize(heading, q),
                    deviation_deg: cfg.deviation.at(norm_360(heading - var)),
                    mag_var_deg: var,
                    rot_dpm: ch.get(Channel::RateOfTurn) + self.noise(2.0 * k),
                }
            }
            SensorKind::SpeedLog => {
                let compass = self.config_of(SensorKind::Compass).cloned().unwrap_or_else(|| SensorConfig::new(SensorKind::Compass));
                Measurement::Speed {
                    stw_kn: quantize((ch.get(Channel::Stw) * cfg.speed_scale + self.noise(0.05 * k)).max(0.0), q),
                    heading_true: self.compass_heading(&compass, ch),
                    mag_var_deg: var,
                }
            }
            SensorKind::DepthSounder => Measurement::Depth {
                below_transducer_m: quantize(
                    (ch.get(Channel::Depth) - cfg.transducer_depth_m + self.noise(0.05 * k)).max(0.0),
                    q,
                ),
                transducer_depth_m: cfg.transducer_depth_m,
            },
            SensorKind::Wind => self.wind(cfg, ch),
            SensorKind::WaterTemp => Measurement::WaterTemp {
                celsius: quantize(ch.get(Channel::WaterTemp) + self.noise(0.05 * k), q),
            },
            SensorKind::Weather => Measurement::Weather {
                air_temp_c: quantize(ch.get(Channel::AirTemp) + self.noise(0.1 * k), q),
                pressure_hpa: ch.get(Channel::Pressure) + self.noise(0.1 * k),
                humidity_pct: (ch.get(Channel::Humidity) + self.noise(0.5 * k)).clamp(0.0, 100.0),
                water_temp_c: ch.get(Channel::WaterTemp) + self.noise(0.05 * k),
                twd: norm_360(ch.get(Channel::Twd) + self.noise(2.0 * k)),
                tws_kn: (ch.get(Channel::Tws) + self.noise(0.3 * k)).max(0.0),
                mag_var_deg: var,
            },
            SensorKind::Rudder => Measurement::Rudder {
                starboard_deg: quantize(ch.get(Channel::Rudder) + cfg.angle_bias_deg + self.noise(0.2 * k), q),
            },
            SensorKind::Attitude => Measurement::Attitude {
                roll_deg: quantize(ch.get(Channel::Roll) + cfg.angle_bias_deg + self.noise(0.1 * k), q),
                pitch_deg: quantize(ch.get(Channel::Pitch) + self.noise(0.1 * k), q),
                heave_m: ch.get(Channel::Heave) + self.noise(0.02 * k),
            },
            SensorKind::Ais => unreachable!("AIS is polled by poll_ais"),
            SensorKind::Engine => {
                let rpm = ch.get(Channel::EngineRpm);
                Measurement::Engine {
                    rpm: if rpm > 0.0 { quantize((rpm + self.noise(10.0 * k)).max(0.0), q) } else { 0.0 },
                }
            }
        }
    }

    /// True heading as the compass reports it (bias + noise; deviation is corrected).
    fn compass_heading(&mut self, cfg: &SensorConfig, ch: &Channels) -> f64 {
        norm_360(ch.get(Channel::HeadingTrue) + cfg.angle_bias_deg + self.noise(0.3 * cfg.noise_scale.max(0.0)))
    }

    /// Masthead unit + processor: apparent wind is measured (with misalignment, calibration,
    /// heel error and noise); true wind is computed from it with the speed log and compass.
    fn wind(&mut self, cfg: &SensorConfig, ch: &Channels) -> Measurement {
        let k = cfg.noise_scale.max(0.0);
        let q = cfg.resolution;
        let heel = ch.get(Channel::Roll).to_radians();
        // A heeled vane sees the cross-wind component reduced by cos(heel).
        let awa_true = ch.get(Channel::Awa).to_radians();
        let awa_heeled = (awa_true.sin() * heel.cos()).atan2(awa_true.cos()).to_degrees();
        let awa = quantize(norm_180(awa_heeled + cfg.angle_bias_deg + self.noise(2.0 * k)), q);
        let aws = quantize((ch.get(Channel::Aws) * cfg.speed_scale + self.noise(0.3 * k)).max(0.0), q.min(0.1));

        let log = self.config_of(SensorKind::SpeedLog).cloned().unwrap_or_else(|| SensorConfig::new(SensorKind::SpeedLog));
        let compass = self.config_of(SensorKind::Compass).cloned().unwrap_or_else(|| SensorConfig::new(SensorKind::Compass));
        let stw = ch.get(Channel::Stw) * log.speed_scale;
        let heading = self.compass_heading(&compass, ch);
        // Boat frame: x forward, y starboard. Apparent wind blows *from* awa.
        let a = Ne::from_polar(aws, awa + 180.0); // air velocity relative to boat, boat frame
        let t = a + Ne::from_polar(stw, 0.0); // add boat's own motion
        let twa = norm_180(t.direction() + 180.0);
        Measurement::Wind {
            awa,
            aws_kn: aws,
            twa: quantize(twa, q),
            tws_kn: t.magnitude(),
            twd: quantize(norm_360(heading + twa), q),
            mag_var_deg: ch.get(Channel::MagVariation),
        }
    }
}

fn gaussian(rng: &mut StdRng, sigma: f64) -> f64 {
    if sigma <= 0.0 {
        return 0.0;
    }
    Normal::new(0.0, sigma).map(|n| n.sample(rng)).unwrap_or(0.0)
}

/// Next due time: one interval after the previous one, or one interval from now after a pause
/// or time jump (no bursts).
fn schedule(previous: f64, interval: f64, now: f64) -> f64 {
    let next = previous + interval;
    if next <= now { now + interval } else { next }
}

/// The own ship as an AIS station, from its observed (overridable) channels.
fn own_ship_station(sim: &Simulation) -> AisTarget {
    let v = &sim.vessel_cfg;
    let ais = &v.ais;
    let ch = &sim.channels;
    let class = if ais.class == AisClass::A { AisClass::A } else { AisClass::B };
    let cfg = AisTargetConfig {
        mmsi: v.mmsi,
        name: v.name.clone(),
        call_sign: v.call_sign.clone(),
        class,
        enabled: true,
        position: LatLon::new(ch.get(Channel::Latitude), ch.get(Channel::Longitude)),
        heading: ch.get(Channel::HeadingTrue),
        nav_status: ais.nav_status,
        statics: ais.statics.clone(),
        flags: ais.flags,
        intervals: ais.intervals,
        ..AisTargetConfig::default()
    };
    let mut t = AisTarget::new(OWN_SHIP_KEY, cfg);
    t.state.heading = ch.get(Channel::HeadingTrue);
    t.state.cog = ch.get(Channel::Cog);
    t.state.sog_kn = ch.get(Channel::Sog);
    t.state.rot_dpm = ch.get(Channel::RateOfTurn);
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::{InitialState, OverrideMode, Simulation};

    fn quiet(kind: SensorKind) -> SensorConfig {
        let mut c = SensorConfig::new(kind);
        c.noise_scale = 0.0;
        c.resolution = 0.0;
        c
    }

    #[test]
    fn rates_are_respected() {
        let mut sim = Simulation::new(InitialState::default());
        let mut suite = SensorSuite::new(SensorConfig::defaults(), 1);
        let mut gps = 0;
        let mut compass = 0;
        for _ in 0..100 {
            for e in suite.poll(&sim) {
                match e.sensor {
                    SensorKind::Gps => gps += 1,
                    SensorKind::Compass => compass += 1,
                    _ => {}
                }
            }
            sim.advance(0.1);
        }
        assert!((10..=11).contains(&gps), "gps {gps}");
        assert!((99..=101).contains(&compass), "compass {compass}");
    }

    #[test]
    fn zero_noise_reports_truth() {
        let sim = Simulation::new(InitialState::default());
        let mut cfg = quiet(SensorKind::DepthSounder);
        cfg.transducer_depth_m = 1.0;
        let mut suite = SensorSuite::new(vec![cfg], 1);
        let e = suite.poll(&sim);
        assert_eq!(e[0].measurement, Measurement::Depth { below_transducer_m: 24.0, transducer_depth_m: 1.0 });
    }

    #[test]
    fn perfect_wind_instrument_matches_the_model() {
        let mut sim = Simulation::new(InitialState::default());
        sim.vessel.stw_kn = 6.0;
        sim.target_stw_kn = 6.0;
        for _ in 0..50 {
            sim.advance(0.1);
        }
        let mut suite = SensorSuite::new(vec![quiet(SensorKind::Wind), quiet(SensorKind::SpeedLog), quiet(SensorKind::Compass)], 1);
        let e = suite.poll(&sim);
        let Measurement::Wind { twa, tws_kn, twd, .. } = e[0].measurement else { panic!() };
        assert!((twa - sim.channels.get(Channel::Twa)).abs() < 0.5, "{twa}");
        assert!((tws_kn - sim.channels.get(Channel::Tws)).abs() < 0.1);
        assert!(norm_180(twd - sim.channels.get(Channel::Twd)).abs() < 0.5);
    }

    #[test]
    fn speed_log_calibration_error_propagates_to_true_wind() {
        // Head to wind: TWS = AWS - STW. A log reading 10 % high lowers the computed TWS.
        let mut sim = Simulation::new(InitialState::default());
        sim.apply(sim_core::Command::SetSteering(sim_core::Steering::Heading { heading: 270.0 }));
        sim.vessel.heading = 270.0;
        sim.vessel.stw_kn = 6.0;
        sim.advance(0.1);
        let mut log = quiet(SensorKind::SpeedLog);
        log.speed_scale = 1.1;
        let mut suite = SensorSuite::new(vec![quiet(SensorKind::Wind), log, quiet(SensorKind::Compass)], 1);
        let Measurement::Wind { tws_kn, .. } = suite.poll(&sim)[0].measurement else { panic!() };
        let truth = sim.channels.get(Channel::Tws);
        assert!((truth - tws_kn - 0.6).abs() < 0.05, "truth {truth} measured {tws_kn}");
    }

    #[test]
    fn latency_delays_readings() {
        let mut sim = Simulation::new(InitialState::default());
        let mut cfg = quiet(SensorKind::DepthSounder);
        cfg.latency_s = 2.0;
        cfg.rate_hz = 10.0;
        cfg.transducer_depth_m = 0.0;
        let mut suite = SensorSuite::new(vec![cfg], 1);
        let mut readings = vec![];
        for i in 0..60 {
            if i == 20 {
                sim.apply(sim_core::Command::SetChannelOverride(Channel::Depth, Some((OverrideMode::Replace, 3.0))));
            }
            for e in suite.poll(&sim) {
                if let Measurement::Depth { below_transducer_m, .. } = e.measurement {
                    readings.push((i, below_transducer_m));
                }
            }
            sim.advance(0.1);
        }
        let first_change = readings.iter().find(|(_, d)| *d == 3.0).unwrap().0;
        assert!((39..=41).contains(&first_change), "change seen at step {first_change}");
    }

    #[test]
    fn ais_stations_follow_their_intervals() {
        use sim_core::ais::{AisClass, AisTargetConfig, TargetMotion};
        let mut init = InitialState::default();
        let pos = init.position;
        init.stw_kn = 6.0;
        let mut a = AisTargetConfig::new(AisClass::A, 1, pos.destination(90.0, 3000.0));
        a.motion = TargetMotion::Course { cog: 0.0, sog_kn: 10.0 };
        let mut aton = AisTargetConfig::new(AisClass::Aton, 2, pos.destination(0.0, 1000.0));
        aton.intervals.position_s = Some(20.0);
        init.ais_targets = vec![a, aton];
        let mut sim = Simulation::new(init);
        let mut suite = SensorSuite::new(vec![quiet(SensorKind::Ais)], 1);
        let mut counts: HashMap<(bool, u8), usize> = HashMap::new();
        let mut channels = Vec::new();
        for _ in 0..600 {
            for e in suite.poll(&sim) {
                let Measurement::Ais { own, channel, message, .. } = e.measurement else { panic!() };
                *counts.entry((own, message.msg_type())).or_default() += 1;
                channels.push(channel);
            }
            sim.advance(0.1);
        }
        // 60 s: Class A at 10 kn every 10 s, AtoN every 20 s, own Class B at ~6 kn every 30 s.
        assert!((6..=7).contains(&counts[&(false, 1)]), "{counts:?}");
        assert!((3..=4).contains(&counts[&(false, 21)]), "{counts:?}");
        assert!((2..=3).contains(&counts[&(true, 18)]), "{counts:?}");
        // Static data once at start (6 min interval): 5 for the target, 24A+24B for own ship.
        assert_eq!(counts[&(false, 5)], 1);
        assert_eq!(counts[&(true, 24)], 2);
        assert!(channels.contains(&'A') && channels.contains(&'B'));
    }

    #[test]
    fn own_ship_ais_reports_observed_channels() {
        let mut sim = Simulation::new(InitialState::default());
        sim.apply(sim_core::Command::SetChannelOverride(Channel::Sog, Some((OverrideMode::Replace, 12.5))));
        let mut suite = SensorSuite::new(vec![quiet(SensorKind::Ais)], 1);
        let mut sog = None;
        for _ in 0..100 {
            for e in suite.poll(&sim) {
                if let Measurement::Ais { own: true, message: sim_core::ais::AisMessage::PositionB { common, .. }, .. } = e.measurement {
                    sog = common.sog_kn;
                }
            }
            sim.advance(0.1);
        }
        assert_eq!(sog, Some(12.5));
    }

    #[test]
    fn deviation_curve() {
        let d = Deviation { a: 1.0, b: 3.0, c: 0.0, d: 0.0, e: 0.0 };
        assert!((d.at(90.0) - 4.0).abs() < 1e-9);
        assert!((d.at(270.0) + 2.0).abs() < 1e-9);
    }
}
