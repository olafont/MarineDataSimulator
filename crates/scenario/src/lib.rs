//! Scenario files (TOML): initial state, environment and its dynamics, bathymetry, overrides,
//! sensors and outputs. External files (polar, soundings CSV) are resolved relative to the
//! scenario file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sim_core::ais::AisTargetConfig;
use sim_core::param::Override;
use sim_core::{
    Bathymetry, Channel, Dynamics, EnvParam, EnvParams, InitialState, LatLon, Polar, Propulsion, Simulation, Sounding,
    Steering, TideConfig, VesselConfig, WindDynamics,
};
use sim_sensors::{SensorConfig, SensorKind};
use transports::OutputConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub simulation: SimulationSection,
    #[serde(default)]
    pub vessel: VesselConfig,
    pub initial: Initial,
    #[serde(default)]
    pub steering: Steering,
    /// Environment base values by key (see `EnvParam` keys, e.g. `wind_speed`).
    #[serde(default)]
    pub environment: BTreeMap<String, f64>,
    #[serde(default)]
    pub dynamics: DynamicsSection,
    #[serde(default)]
    pub tide: TideSection,
    #[serde(default, skip_serializing_if = "BathymetrySection::is_empty")]
    pub bathymetry: BathymetrySection,
    #[serde(default)]
    pub overrides: Overrides,
    /// Other AIS stations (own ship's transponder is `vessel.ais`).
    #[serde(default, skip_serializing_if = "AisSection::is_empty")]
    pub ais: AisSection,
    #[serde(default = "SensorConfig::defaults")]
    pub sensors: Vec<SensorConfig>,
    #[serde(default = "OutputConfig::default_outputs")]
    pub outputs: Vec<OutputConfig>,

    /// Directory external files are resolved against.
    #[serde(skip)]
    pub base_dir: Option<PathBuf>,
    /// Loaded polar (from `vessel.polar_file`, or built-in).
    #[serde(skip)]
    pub polar: Option<Arc<Polar>>,
    /// Loaded soundings (inline + CSV).
    #[serde(skip)]
    pub soundings: Option<Arc<Bathymetry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SimulationSection {
    /// RFC 3339 UTC start time; absent = wall-clock time at start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    pub time_scale: f64,
    pub paused: bool,
    /// RNG seed for sensor noise and environment dynamics (reproducible runs).
    pub seed: u64,
}

impl Default for SimulationSection {
    fn default() -> Self {
        Self { start_time: None, time_scale: 1.0, paused: false, seed: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Initial {
    pub position: LatLon,
    #[serde(default)]
    pub heading: f64,
    #[serde(default)]
    pub speed: f64,
    /// Target speed through water under engine, knots.
    pub target_speed: f64,
    /// Sailing vessels: start motoring.
    #[serde(default)]
    pub engine_on: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DynamicsSection {
    pub wind: WindDynamics,
    /// Barometric trend, hPa per hour.
    pub pressure_trend_hpa_h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TideSection {
    pub enabled: bool,
    pub range_m: f64,
    pub period_h: f64,
    /// RFC 3339 time of a high water; absent = simulation start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_water: Option<String>,
    pub flood_set_deg: f64,
    pub max_rate_kn: f64,
}

impl Default for TideSection {
    fn default() -> Self {
        let t = TideConfig::default();
        Self {
            enabled: t.enabled,
            range_m: t.range_m,
            period_h: t.period_h,
            high_water: None,
            flood_set_deg: t.flood_set_deg,
            max_rate_kn: t.max_rate_kn,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BathymetrySection {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub soundings: Vec<Sounding>,
    /// CSV of `lat,lon,depth`, relative to the scenario file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csv: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neighbours: Option<usize>,
}

impl BathymetrySection {
    fn is_empty(&self) -> bool {
        self.soundings.is_empty() && self.csv.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AisSection {
    pub targets: Vec<AisTargetConfig>,
}

impl AisSection {
    fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, Override>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, Override>,
}

fn parse_time(t: &str) -> Result<f64> {
    let t = chrono::DateTime::parse_from_rfc3339(t).with_context(|| format!("time `{t}` (expected RFC 3339)"))?;
    Ok(t.timestamp_millis() as f64 / 1000.0)
}

fn format_time(unix: f64) -> String {
    chrono::DateTime::from_timestamp(unix.round() as i64, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

impl Scenario {
    /// Parses a scenario; external files are resolved against `base_dir` (or the working
    /// directory when `None`).
    pub fn from_toml_in(text: &str, base_dir: Option<&Path>) -> Result<Self> {
        let mut s: Scenario = toml::from_str(text).context("parsing scenario")?;
        s.base_dir = base_dir.map(Path::to_path_buf);
        s.validate()?;
        s.resolve()?;
        Ok(s)
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        Self::from_toml_in(text, None)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_toml_in(&text, path.parent()).with_context(|| format!("in {}", path.display()))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("serialising scenario")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_toml()?).with_context(|| format!("writing {}", path.display()))
    }

    pub fn resolve_path(&self, p: &str) -> PathBuf {
        let p = Path::new(p);
        match &self.base_dir {
            Some(dir) if p.is_relative() => dir.join(p),
            _ => p.to_path_buf(),
        }
    }

    /// Loads the polar file and soundings CSV.
    fn resolve(&mut self) -> Result<()> {
        // Scenarios written before the AIS transponder existed list every other sensor.
        if !self.sensors.iter().any(|c| c.kind == SensorKind::Ais) {
            self.sensors.push(SensorConfig::new(SensorKind::Ais));
        }
        self.polar = Some(Arc::new(match &self.vessel.polar_file {
            Some(f) => load_polar(&self.resolve_path(f))?,
            None => Polar::default_cruiser(),
        }));
        let mut soundings = self.bathymetry.soundings.clone();
        if let Some(csv) = &self.bathymetry.csv {
            let path = self.resolve_path(csv);
            let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            soundings.extend(
                Bathymetry::parse_csv(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?,
            );
        }
        self.soundings = Some(Arc::new(Bathymetry { soundings, neighbours: self.bathymetry.neighbours }));
        Ok(())
    }

    /// Checks names that serde cannot (environment and channel keys, times).
    pub fn validate(&self) -> Result<()> {
        for k in self.environment.keys().chain(self.overrides.environment.keys()) {
            if EnvParam::from_key(k).is_none() {
                bail!("unknown environment key `{k}`; expected one of: {}", keys(EnvParam::ALL.iter().map(|p| p.meta().key)));
            }
        }
        for k in self.overrides.channels.keys() {
            if Channel::from_key(k).is_none() {
                bail!("unknown channel `{k}`; expected one of: {}", keys(Channel::ALL.iter().map(|c| c.meta().key)));
            }
        }
        if let Some(t) = &self.simulation.start_time {
            parse_time(t).context("simulation.start_time")?;
        }
        if let Some(t) = &self.tide.high_water {
            parse_time(t).context("tide.high_water")?;
        }
        Ok(())
    }

    /// Start time as Unix seconds, using `now_unix` when the scenario does not fix one.
    pub fn start_time_unix(&self, now_unix: f64) -> f64 {
        self.simulation.start_time.as_deref().and_then(|t| parse_time(t).ok()).unwrap_or(now_unix)
    }

    pub fn dynamics(&self, start_unix: f64) -> Dynamics {
        let t = &self.tide;
        Dynamics {
            wind: self.dynamics.wind.clone(),
            tide: TideConfig {
                enabled: t.enabled,
                range_m: t.range_m,
                period_h: t.period_h,
                high_water_unix: t.high_water.as_deref().and_then(|h| parse_time(h).ok()).unwrap_or(start_unix),
                flood_set_deg: t.flood_set_deg,
                max_rate_kn: t.max_rate_kn,
            },
            pressure_trend_hpa_h: self.dynamics.pressure_trend_hpa_h,
        }
    }

    pub fn initial_state(&self, now_unix: f64) -> InitialState {
        let mut env: EnvParams = InitialState::default().env;
        for (k, v) in &self.environment {
            if let Some(p) = EnvParam::from_key(k) {
                env.set_modeled(p, *v);
            }
        }
        for (k, o) in &self.overrides.environment {
            if let Some(p) = EnvParam::from_key(k) {
                env.param_mut(p).ovr = Some(*o);
            }
        }
        let start = self.start_time_unix(now_unix);
        InitialState {
            start_time_unix: start,
            seed: self.simulation.seed,
            vessel: self.vessel.clone(),
            position: self.initial.position,
            heading: self.initial.heading,
            stw_kn: self.initial.speed,
            target_stw_kn: self.initial.target_speed,
            steering: self.steering.clone(),
            env,
            dynamics: self.dynamics(start),
            bathymetry: self.soundings.clone().unwrap_or_default(),
            polar: self.polar.clone().unwrap_or_else(|| Arc::new(Polar::default_cruiser())),
            engine_on: self.initial.engine_on,
            ais_targets: self.ais.targets.clone(),
        }
    }

    /// Builds a ready-to-run simulation (time scale, pause and channel overrides applied).
    pub fn build_simulation(&self, now_unix: f64) -> Simulation {
        let mut sim = Simulation::new(self.initial_state(now_unix));
        sim.time_scale = self.simulation.time_scale;
        sim.paused = self.simulation.paused;
        for (k, o) in &self.overrides.channels {
            if let Some(c) = Channel::from_key(k) {
                sim.channels.param_mut(c).ovr = Some(*o);
            }
        }
        sim.update_channels();
        sim
    }

    /// Captures a running simulation back into a scenario (for "save as").
    pub fn capture(&self, sim: &Simulation, sensors: Vec<SensorConfig>, outputs: Vec<OutputConfig>) -> Scenario {
        let mut s = self.clone();
        s.vessel = sim.vessel_cfg.clone();
        s.initial = Initial {
            position: sim.vessel.position,
            heading: sim.vessel.heading,
            speed: sim.vessel.stw_kn,
            target_speed: sim.target_stw_kn,
            engine_on: sim.engine_on && sim.vessel_cfg.propulsion == Propulsion::Sail,
        };
        s.steering = sim.steering.clone();
        s.simulation.time_scale = sim.time_scale;
        s.environment = EnvParam::ALL.iter().map(|p| (p.meta().key.to_string(), sim.env_base.get(*p))).collect();
        s.dynamics = DynamicsSection {
            wind: sim.dynamics.wind.clone(),
            pressure_trend_hpa_h: sim.dynamics.pressure_trend_hpa_h,
        };
        let t = &sim.dynamics.tide;
        s.tide = TideSection {
            enabled: t.enabled,
            range_m: t.range_m,
            period_h: t.period_h,
            high_water: Some(format_time(t.high_water_unix)),
            flood_set_deg: t.flood_set_deg,
            max_rate_kn: t.max_rate_kn,
        };
        s.overrides.environment = EnvParam::ALL
            .iter()
            .filter_map(|p| sim.env.param(*p).ovr.map(|o| (p.meta().key.to_string(), o)))
            .collect();
        s.overrides.channels = Channel::ALL
            .iter()
            .filter_map(|c| sim.channels.param(*c).ovr.map(|o| (c.meta().key.to_string(), o)))
            .collect();
        s.ais.targets = sim.ais_targets.iter().map(|t| t.captured()).collect();
        s.sensors = sensors;
        s.outputs = outputs;
        s.polar = Some(sim.polar.clone());
        s
    }
}

pub fn load_polar(path: &Path) -> Result<Polar> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Polar::parse(&text).with_context(|| format!("in {}", path.display()))
}

fn keys<'a>(it: impl Iterator<Item = &'a str>) -> String {
    it.collect::<Vec<_>>().join(", ")
}

/// Built-in scenarios: (file name, contents).
pub const BUILTIN: &[(&str, &str)] = &[
    ("gibraltar-strait-sailing.toml", include_str!("../../../scenarios/gibraltar-strait-sailing.toml")),
    ("cape-canaveral-sailing.toml", include_str!("../../../scenarios/cape-canaveral-sailing.toml")),
    ("marseille-sailing.toml", include_str!("../../../scenarios/marseille-sailing.toml")),
    ("channel-crossing-motor.toml", include_str!("../../../scenarios/channel-crossing-motor.toml")),
    ("singapore-strait-motor.toml", include_str!("../../../scenarios/singapore-strait-motor.toml")),
    ("long-beach-sailing.toml", include_str!("../../../scenarios/long-beach-sailing.toml")),
];

/// The scenario used when none is given.
pub fn default_scenario() -> Scenario {
    builtin(BUILTIN[0].0).expect("built-in scenario is valid")
}

pub fn builtin(name: &str) -> Result<Scenario> {
    let (_, text) = BUILTIN.iter().find(|(n, _)| *n == name).with_context(|| format!("no built-in scenario `{name}`"))?;
    Scenario::from_toml(text)
}

pub const DEFAULT_SCENARIO: &str = BUILTIN[0].1;

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::OverrideMode;

    #[test]
    fn builtin_scenarios_round_trip() {
        for (name, _) in BUILTIN {
            let s = builtin(name).unwrap();
            let again = Scenario::from_toml(&s.to_toml().unwrap()).unwrap();
            assert_eq!(s, again, "{name}");
        }
    }

    /// Built-in scenarios use MMSIs that cannot belong to a real station (MID 100, see
    /// `sim_core::ais::SIM_MID`), so their data is never taken for real traffic.
    #[test]
    fn builtin_mmsis_are_fictitious() {
        use sim_core::ais::{AisClass, SIM_MID};
        for (name, _) in BUILTIN {
            let s = builtin(name).unwrap();
            assert_eq!(s.vessel.mmsi / 1_000_000, SIM_MID, "{name}: own ship {}", s.vessel.mmsi);
            for t in &s.ais.targets {
                let mid = match t.class {
                    AisClass::A | AisClass::B => t.mmsi / 1_000_000,
                    AisClass::BaseStation => t.mmsi / 10_000,              // 00MIDxxxx
                    AisClass::Aton => t.mmsi % 10_000_000 / 10_000,        // 99MIDxxxx
                    AisClass::Sart => SIM_MID,                             // 970xxyyyy: no MID
                };
                assert_eq!(mid, SIM_MID, "{name}: {} has MMSI {}", t.name, t.mmsi);
            }
        }
    }

    #[test]
    fn overrides_apply() {
        let mut s = default_scenario();
        s.overrides.channels.insert("depth".into(), Override { mode: OverrideMode::Replace, value: 1.5 });
        let sim = s.build_simulation(0.0);
        assert_eq!(sim.channels.get(Channel::Depth), 1.5);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let mut s = default_scenario();
        s.environment.insert("wnd_speed".into(), 3.0);
        let err = Scenario::from_toml(&s.to_toml().unwrap()).unwrap_err();
        assert!(format!("{err:#}").contains("wnd_speed"));
    }

    #[test]
    fn output_formats_load_and_save() {
        let text = r#"
            [initial]
            position = { lat = 28.4, lon = -80.55 }
            target_speed = 6.0

            [[outputs]]
            name = "0183"
            type = "tcp_server"
            bind = "0.0.0.0:10113"

            [[outputs]]
            name = "N2K"
            type = "tcp_server"
            bind = "0.0.0.0:1457"
            format = "yd_raw"
            filter = ["129025", "129026"]

            [[outputs]]
            name = "Signal K"
            type = "web_socket"
            bind = "0.0.0.0:3013"
            format = "signalk"
        "#;
        let s = Scenario::from_toml(text).unwrap();
        let formats: Vec<_> = s.outputs.iter().map(|o| o.format).collect();
        use transports::DataFormat::*;
        assert_eq!(formats, [Nmea0183, YdRaw, SignalK]);
        let saved = s.to_toml().unwrap();
        assert!(saved.contains("format = \"yd_raw\"") && !saved.contains("nmea0183"), "{saved}");
        assert_eq!(Scenario::from_toml(&saved).unwrap().outputs, s.outputs);
        assert!(Scenario::from_toml(&text.replace("yd_raw", "ydraw")).is_err());
    }

    #[test]
    fn capture_preserves_state() {
        let s = builtin("channel-crossing-motor.toml").unwrap();
        let mut sim = s.build_simulation(1_800_000_000.0);
        sim.apply(sim_core::Command::SetChannelOverride(Channel::Sog, Some((OverrideMode::Offset, 0.5))));
        sim.advance(30.0);
        let c = s.capture(&sim, s.sensors.clone(), s.outputs.clone());
        let back = Scenario::from_toml(&c.to_toml().unwrap()).unwrap();
        let rebuilt = back.build_simulation(0.0);
        assert_eq!(rebuilt.channels.param(Channel::Sog).ovr, sim.channels.param(Channel::Sog).ovr);
        assert_eq!(rebuilt.dynamics.tide, sim.dynamics.tide, "tide survives (HW time as RFC 3339)");
        assert_eq!(rebuilt.env_base, sim.env_base, "base values saved, not gusty modeled values");
    }

    #[test]
    fn ais_targets_load_and_capture() {
        let s = builtin("cape-canaveral-sailing.toml").unwrap();
        assert_eq!(s.ais.targets.len(), 7);
        assert!(s.sensors.iter().any(|c| c.kind == SensorKind::Ais));
        let mut sim = s.build_simulation(1_800_000_000.0);
        assert_eq!(sim.ais_targets.len(), 7);
        sim.apply(sim_core::Command::SetTimeScale(10.0));
        sim.advance(6.0); // one simulated minute
        let c = s.capture(&sim, s.sensors.clone(), s.outputs.clone());
        let back = Scenario::from_toml(&c.to_toml().unwrap()).unwrap();
        assert_eq!(back.ais.targets.len(), 7);
        let cargo = back.ais.targets.iter().find(|t| t.name == "ATLANTIC TRADER").unwrap();
        let start = s.ais.targets.iter().find(|t| t.name == "ATLANTIC TRADER").unwrap().position;
        let moved = start.distance_to(&cargo.position);
        assert!((moved - 12.5 * 1852.0 / 60.0).abs() < 20.0, "moved {moved} m");
    }

    #[test]
    fn external_files_resolve_relative_to_the_scenario() {
        let dir = std::env::temp_dir().join(format!("marine-sim-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("boat.pol"), "twa/tws;10\n90;7.5\n").unwrap();
        std::fs::write(dir.join("depths.csv"), "lat,lon,depth\n43.0,5.0,12\n").unwrap();
        let mut s = default_scenario();
        s.vessel.polar_file = Some("boat.pol".into());
        s.bathymetry = BathymetrySection { soundings: vec![], csv: Some("depths.csv".into()), neighbours: None };
        let path = dir.join("s.toml");
        s.save(&path).unwrap();
        let loaded = Scenario::load(&path).unwrap();
        assert_eq!(loaded.polar.as_ref().unwrap().speed(90.0, 10.0), 7.5);
        assert_eq!(loaded.soundings.as_ref().unwrap().soundings.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
