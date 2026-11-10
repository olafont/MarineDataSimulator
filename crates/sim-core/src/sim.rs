//! The simulation: environment dynamics + own vessel + derived observations, advanced in fixed
//! sub-steps.

use std::sync::Arc;

use crate::ais::{AisTarget, AisTargetConfig, OwnAis};
use crate::bathymetry::Bathymetry;
use crate::channels::{Channel, Channels, EnvBase, EnvParam, EnvParams};
use crate::dynamics::{Dynamics, DynamicsState};
use crate::geo::{LatLon, Ne, norm_180, norm_360};
use crate::param::OverrideMode;
use crate::polar::Polar;
use crate::vessel::{Propulsion, Steering, VesselConfig, VesselState, course_to_steer};
use crate::waves::{WaveInput, WaveMotion, WaveState};

/// Largest physics step, seconds. Bigger simulated intervals are split.
pub const MAX_SUBSTEP_S: f64 = 0.1;

/// Everything needed to (re)initialise a simulation.
#[derive(Debug, Clone)]
pub struct InitialState {
    pub start_time_unix: f64,
    /// Seed for the environment's random processes.
    pub seed: u64,
    pub vessel: VesselConfig,
    pub position: LatLon,
    pub heading: f64,
    pub stw_kn: f64,
    pub target_stw_kn: f64,
    pub steering: Steering,
    /// `modeled` = base values; `ovr` = initial environment overrides.
    pub env: EnvParams,
    pub dynamics: Dynamics,
    pub bathymetry: Arc<Bathymetry>,
    pub polar: Arc<Polar>,
    /// Sailing vessels only: motoring.
    pub engine_on: bool,
    /// Other AIS stations.
    pub ais_targets: Vec<AisTargetConfig>,
}

/// Commands accepted by the simulation. They are the only way the outside world changes it.
#[derive(Debug, Clone)]
pub enum Command {
    SetPaused(bool),
    SetTimeScale(f64),
    /// Sets the user's base value; the model adds dynamics on top.
    SetEnvBase(EnvParam, f64),
    SetEnvOverride(EnvParam, Option<(OverrideMode, f64)>),
    SetChannelOverride(Channel, Option<(OverrideMode, f64)>),
    SetTargetSpeed(f64),
    SetSteering(Steering),
    /// Moves the vessel instantly (e.g. dragged on the map).
    Teleport(LatLon),
    SetActiveWaypoint(usize),
    SetDynamics(Box<Dynamics>),
    SetVesselConfig(Box<VesselConfig>),
    SetPolar(Arc<Polar>),
    SetBathymetry(Arc<Bathymetry>),
    SetEngine(bool),
    AddAisTarget(Box<AisTargetConfig>),
    /// Replaces a target's configuration (by id), keeping its motion state.
    UpdateAisTarget(u64, Box<AisTargetConfig>),
    RemoveAisTarget(u64),
    /// Own ship's identity and AIS transponder settings.
    SetOwnAis { name: String, mmsi: u32, call_sign: String, ais: Box<OwnAis> },
    Reset(Box<InitialState>),
}

#[derive(Debug, Clone)]
pub struct Simulation {
    /// Simulated UTC time, seconds since the Unix epoch.
    pub time_unix: f64,
    pub paused: bool,
    pub time_scale: f64,
    pub vessel_cfg: VesselConfig,
    pub vessel: VesselState,
    pub target_stw_kn: f64,
    pub steering: Steering,
    pub env_base: EnvBase,
    pub env: EnvParams,
    pub channels: Channels,
    pub dynamics: Dynamics,
    pub bathymetry: Arc<Bathymetry>,
    pub polar: Arc<Polar>,
    pub engine_on: bool,
    /// Ground velocity, knots (true value).
    pub ground_velocity: Ne,
    /// Wave-induced motion (added to the steady heel for roll).
    pub motion: WaveMotion,
    /// Tide height above chart datum, metres.
    pub tide_height_m: f64,
    /// True while the keel touches the bottom.
    pub grounded: bool,
    pub engine_rpm: f64,
    /// Current tack when beating to a waypoint: +1 = wind on starboard, -1 = port, 0 = none.
    pub tack: f64,
    /// A non-looped route has reached its last waypoint: the vessel stops and drifts.
    pub route_complete: bool,
    /// Other AIS stations.
    pub ais_targets: Vec<AisTarget>,
    next_target_id: u64,
    pressure_offset: f64,
    dyn_state: DynamicsState,
    waves: WaveState,
}

impl Simulation {
    pub fn new(init: InitialState) -> Self {
        let mut env_base = EnvBase::default();
        for &p in EnvParam::ALL {
            env_base.set(p, init.env.param(p).modeled);
        }
        let mut dyn_state = DynamicsState::new(init.seed);
        let waves = WaveState::new(&mut dyn_state.rng);
        let mut sim = Simulation {
            time_unix: init.start_time_unix,
            paused: false,
            time_scale: 1.0,
            vessel_cfg: init.vessel,
            vessel: VesselState {
                position: init.position,
                heading: norm_360(init.heading),
                rot_dpm: 0.0,
                stw_kn: init.stw_kn,
                rudder_deg: 0.0,
                heel_deg: 0.0,
                leeway_deg: 0.0,
            },
            target_stw_kn: init.target_stw_kn,
            steering: init.steering,
            env_base,
            env: init.env,
            channels: Channels::default(),
            dynamics: init.dynamics,
            bathymetry: init.bathymetry,
            polar: init.polar,
            engine_on: init.engine_on,
            ground_velocity: Ne::default(),
            motion: WaveMotion::default(),
            tide_height_m: 0.0,
            grounded: false,
            engine_rpm: 0.0,
            tack: 0.0,
            route_complete: false,
            ais_targets: init.ais_targets.into_iter().enumerate().map(|(i, c)| AisTarget::new(i as u64 + 1, c)).collect(),
            next_target_id: 0,
            pressure_offset: 0.0,
            dyn_state,
            waves,
        };
        sim.next_target_id = sim.ais_targets.len() as u64 + 1;
        sim.update_env(0.0);
        sim.ground_velocity = sim.vessel.water_velocity() + sim.current();
        sim.engine_rpm = sim.compute_rpm();
        sim.update_channels();
        sim
    }

    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::SetPaused(p) => self.paused = p,
            Command::SetTimeScale(s) => self.time_scale = s.clamp(0.0, 1000.0),
            Command::SetEnvBase(p, v) => self.env_base.set(p, v),
            Command::SetEnvOverride(p, o) => apply_override(self.env.param_mut(p), o),
            Command::SetChannelOverride(c, o) => apply_override(self.channels.param_mut(c), o),
            Command::SetTargetSpeed(v) => self.target_stw_kn = v.max(0.0),
            Command::SetSteering(s) => {
                self.tack = 0.0;
                self.route_complete = false;
                self.steering = s;
            }
            Command::Teleport(p) => {
                self.vessel.position = p;
                self.grounded = false;
            }
            Command::SetActiveWaypoint(i) => {
                if let Steering::Route { waypoints, active, .. } = &mut self.steering
                    && i < waypoints.len()
                {
                    *active = i;
                    self.tack = 0.0;
                    self.route_complete = false;
                }
            }
            Command::SetDynamics(d) => self.dynamics = *d,
            Command::SetVesselConfig(c) => self.vessel_cfg = *c,
            Command::SetPolar(p) => self.polar = p,
            Command::SetBathymetry(b) => self.bathymetry = b,
            Command::SetEngine(on) => self.engine_on = on,
            Command::AddAisTarget(c) => {
                self.ais_targets.push(AisTarget::new(self.next_target_id, *c));
                self.next_target_id += 1;
            }
            Command::UpdateAisTarget(id, c) => {
                if let Some(t) = self.ais_targets.iter_mut().find(|t| t.id == id) {
                    t.reconfigure(*c);
                }
            }
            Command::RemoveAisTarget(id) => self.ais_targets.retain(|t| t.id != id),
            Command::SetOwnAis { name, mmsi, call_sign, ais } => {
                self.vessel_cfg.name = name;
                self.vessel_cfg.mmsi = mmsi;
                self.vessel_cfg.call_sign = call_sign;
                self.vessel_cfg.ais = *ais;
            }
            Command::Reset(init) => {
                // Channel overrides survive a reset: they are user intent, not state.
                let channels = self.channels.clone();
                *self = Simulation::new(*init);
                for &c in Channel::ALL {
                    self.channels.param_mut(c).ovr = channels.param(c).ovr;
                }
            }
        }
        // Keep the environment and observations coherent with the new inputs even while paused.
        self.update_env(0.0);
        self.engine_rpm = self.compute_rpm();
        self.update_channels();
    }

    /// Current as a north/east vector in knots (towards the set), including tidal stream.
    pub fn current(&self) -> Ne {
        Ne::from_polar(self.env.get(EnvParam::CurrentSpeed), self.env.get(EnvParam::CurrentSet))
    }

    /// Wind velocity of the air over ground, knots (pointing downwind).
    fn wind_air(&self) -> Ne {
        Ne::from_polar(self.env.get(EnvParam::WindSpeed), self.env.get(EnvParam::WindDirection) + 180.0)
    }

    /// True wind relative to the water (what a sailing boat feels): (speed, from-direction).
    pub fn true_wind_over_water(&self) -> (f64, f64) {
        let w = self.wind_air() - self.current();
        (w.magnitude(), norm_360(w.direction() + 180.0))
    }

    pub fn is_sailing(&self) -> bool {
        self.vessel_cfg.propulsion == Propulsion::Sail && !self.engine_on
    }

    /// Advances simulated time by `dt_real * time_scale` seconds (no-op while paused).
    /// Returns the simulated seconds elapsed.
    pub fn advance(&mut self, dt_real: f64) -> f64 {
        if self.paused || dt_real <= 0.0 {
            return 0.0;
        }
        let total = dt_real * self.time_scale;
        let n = (total / MAX_SUBSTEP_S).ceil().max(1.0) as usize;
        let dt = total / n as f64;
        for _ in 0..n {
            self.substep(dt);
        }
        self.update_channels();
        total
    }

    /// Recomputes modeled environment values from the base values and the dynamics.
    /// `dt` > 0 advances the random processes and the pressure trend.
    fn update_env(&mut self, dt: f64) {
        let d = &self.dynamics;
        let st = &mut self.dyn_state;
        let (gust, shift) = if dt > 0.0 {
            (st.gust.step(dt, d.wind.gust_period_s, &mut st.rng), st.shift.step(dt, d.wind.shift_period_s, &mut st.rng))
        } else {
            (st.gust.x, st.shift.x)
        };
        self.pressure_offset += d.pressure_trend_hpa_h * dt / 3600.0;

        let b = &self.env_base;
        for &p in EnvParam::ALL {
            self.env.set_modeled(p, b.get(p));
        }
        let ws = b.get(EnvParam::WindSpeed);
        self.env.set_modeled(EnvParam::WindSpeed, (ws * (1.0 + d.wind.gust_fraction * gust)).max(0.0));
        self.env.set_modeled(EnvParam::WindDirection, norm_360(b.get(EnvParam::WindDirection) + d.wind.shift_deg * shift));

        let cur = Ne::from_polar(b.get(EnvParam::CurrentSpeed), b.get(EnvParam::CurrentSet))
            + d.tide.stream(self.time_unix);
        self.env.set_modeled(EnvParam::CurrentSpeed, cur.magnitude());
        self.env.set_modeled(
            EnvParam::CurrentSet,
            if cur.magnitude() > 1e-9 { cur.direction() } else { b.get(EnvParam::CurrentSet) },
        );
        self.env.set_modeled(EnvParam::Pressure, b.get(EnvParam::Pressure) + self.pressure_offset);
        self.update_depth();
    }

    fn update_depth(&mut self) {
        let chart = self
            .bathymetry
            .depth_at(self.vessel.position)
            .unwrap_or(self.env_base.get(EnvParam::SeabedDepth));
        self.tide_height_m = self.dynamics.tide.height(self.time_unix);
        self.env.set_modeled(EnvParam::SeabedDepth, chart + self.tide_height_m);
    }

    fn compute_rpm(&self) -> f64 {
        let cfg = &self.vessel_cfg;
        if cfg.propulsion == Propulsion::Motor || self.engine_on {
            let load = (self.target_stw_kn / cfg.max_speed_kn.max(0.1)).clamp(0.0, 1.0);
            cfg.idle_rpm + (cfg.max_rpm - cfg.idle_rpm) * load
        } else {
            0.0
        }
    }

    fn substep(&mut self, dt: f64) {
        self.time_unix += dt;
        self.update_env(dt);

        let current = self.current();
        let (tws_w, twd_w) = self.true_wind_over_water();
        let target_heading = self.target_heading(current, tws_w, twd_w);

        // Speed target from the engine or the polar, reduced in waves (most in head seas).
        let cfg = &self.vessel_cfg;
        let hs = self.env.get(EnvParam::WaveHeight);
        let rel_wave = norm_180(self.env.get(EnvParam::WaveDirection) - self.vessel.heading).to_radians();
        let wave_loss = (0.04 * hs * (1.0 + rel_wave.cos())).clamp(0.0, 0.5);
        let motor = self.target_stw_kn.min(cfg.max_speed_kn);
        let base_speed = if self.route_complete { 0.0 } else { match cfg.propulsion {
            Propulsion::Motor => motor,
            Propulsion::Sail => {
                let twa = norm_180(twd_w - self.vessel.heading);
                let sail = self.polar.speed(twa, tws_w) * cfg.polar_efficiency;
                if self.engine_on { sail.max(motor) } else { sail }
            }
        } };
        let target_stw = base_speed * (1.0 - wave_loss);

        let previous = self.vessel.position;
        let previous_depth = self.env.get(EnvParam::SeabedDepth);
        self.ground_velocity = self.vessel.step(&self.vessel_cfg, dt, target_heading, target_stw, current);

        // Grounding: refuse to move into water shallower than the draft (moving out is allowed).
        self.update_depth();
        let depth = self.env.get(EnvParam::SeabedDepth);
        if depth < self.vessel_cfg.draft_m && depth < previous_depth {
            self.vessel.position = previous;
            // Stopped, but still able to pivot off.
            self.vessel.stw_kn = 0.0;
            self.ground_velocity = Ne::default();
            self.grounded = true;
            self.update_depth();
        } else {
            self.grounded = depth < self.vessel_cfg.draft_m;
        }

        self.update_heel_and_leeway(dt);

        let cfg = &self.vessel_cfg;
        self.motion = self.waves.step(dt, &WaveInput {
            hs_m: hs,
            period_s: self.env.get(EnvParam::WavePeriod),
            from_deg: self.env.get(EnvParam::WaveDirection),
            heading: self.vessel.heading,
            stw_kn: self.vessel.stw_kn,
            roll_period_s: cfg.roll_period_s,
        });
        self.engine_rpm = self.compute_rpm();

        let wind_from = self.env.get(EnvParam::WindDirection);
        for t in &mut self.ais_targets {
            if t.cfg.enabled {
                t.step(dt, wind_from);
            }
        }
    }

    /// The id the next added AIS target will get.
    pub fn next_ais_target_id(&self) -> u64 {
        self.next_target_id
    }

    fn update_heel_and_leeway(&mut self, dt: f64) {
        let cfg = &self.vessel_cfg;
        let v = &mut self.vessel;
        let apparent = Ne::from_polar(self.env.get(EnvParam::WindSpeed), self.env.get(EnvParam::WindDirection) + 180.0)
            - self.ground_velocity;
        let aws = apparent.magnitude();
        let awa = norm_180(apparent.direction() + 180.0 - v.heading).to_radians();
        let wind_heel = match cfg.propulsion {
            // Wind from starboard (awa > 0) heels the boat to port (negative).
            Propulsion::Sail => -(cfg.heel_coefficient * aws * aws * awa.sin()).clamp(-cfg.max_heel_deg, cfg.max_heel_deg),
            Propulsion::Motor => 0.0,
        };
        // Displacement hulls heel outwards in a turn.
        let turn_heel = -0.002 * v.rot_dpm * v.stw_kn;
        let target = (wind_heel + turn_heel).clamp(-cfg.max_heel_deg - 10.0, cfg.max_heel_deg + 10.0);
        v.heel_deg += (target - v.heel_deg) * (dt / 3.0).min(1.0);

        v.leeway_deg = match cfg.propulsion {
            Propulsion::Sail if v.stw_kn > 0.3 => {
                (cfg.leeway_coefficient * v.heel_deg / (v.stw_kn * v.stw_kn)).clamp(-10.0, 10.0)
            }
            _ => 0.0,
        };
    }

    /// Heading the autopilot steers, advancing the route when a waypoint is reached.
    fn target_heading(&mut self, current: Ne, tws_w: f64, twd_w: f64) -> f64 {
        let pos = self.vessel.position;
        let stw = self.vessel.stw_kn;
        let leeway = self.vessel.leeway_deg;
        let mut waypoint_changed = false;
        let desired = match &mut self.steering {
            Steering::Heading { heading } => return *heading,
            Steering::WindAngle { twa } => return norm_360(twd_w - *twa),
            Steering::Route { waypoints, active, looped, arrival_radius_m } => {
                if waypoints.is_empty() || self.route_complete {
                    return self.vessel.heading;
                }
                if *active >= waypoints.len() {
                    *active = 0;
                }
                let wp = waypoints[*active];
                let dist = pos.distance_to(&wp);
                // A sailing vessel close to an upwind waypoint rounds it rather than making
                // short tacks to hit the exact point while the bearing swings.
                let upwind_and_close = self.vessel_cfg.propulsion == Propulsion::Sail
                    && !self.engine_on
                    && dist < 3.0 * *arrival_radius_m
                    && norm_180(twd_w - pos.bearing_to(&wp)).abs() < self.polar.beat_angle(tws_w);
                if dist < *arrival_radius_m || upwind_and_close {
                    if *active + 1 < waypoints.len() {
                        *active += 1;
                        waypoint_changed = true;
                    } else if *looped {
                        *active = 0;
                        waypoint_changed = true;
                    } else {
                        // End of route: stop and drift.
                        self.route_complete = true;
                        return self.vessel.heading;
                    }
                }
                course_to_steer(pos.bearing_to(&waypoints[*active]), stw, current, leeway)
            }
        };
        if waypoint_changed {
            self.tack = 0.0;
        }
        if self.is_sailing() { self.sail_heading(desired, tws_w, twd_w) } else { desired }
    }

    /// Layline sailing: when `desired` is inside the no-go zone, sail close-hauled on the
    /// current tack until the waypoint can be fetched on the other.
    fn sail_heading(&mut self, desired: f64, tws: f64, twd: f64) -> f64 {
        let beat = self.polar.beat_angle(tws);
        let twa = norm_180(twd - desired);
        if twa.abs() >= beat {
            self.tack = if twa >= 0.0 { 1.0 } else { -1.0 };
            return desired;
        }
        if self.tack == 0.0 {
            // Start on the tack the boat is already on, rather than turning through the wind.
            let now = norm_180(twd - self.vessel.heading);
            self.tack = if now >= 0.0 { 1.0 } else { -1.0 };
        }
        norm_360(twd - self.tack * beat)
    }

    /// Recomputes every modeled channel from the physical state.
    pub fn update_channels(&mut self) {
        let v = &self.vessel;
        let env = &self.env;
        let ch = &mut self.channels;

        ch.set_modeled(Channel::Latitude, v.position.lat);
        ch.set_modeled(Channel::Longitude, v.position.lon);
        ch.set_modeled(Channel::HeadingTrue, v.heading);
        ch.set_modeled(Channel::MagVariation, env.get(EnvParam::MagVariation));
        ch.set_modeled(Channel::RateOfTurn, v.rot_dpm);
        ch.set_modeled(Channel::Rudder, v.rudder_deg);
        ch.set_modeled(Channel::Stw, v.stw_kn);
        ch.set_modeled(Channel::Sog, self.ground_velocity.magnitude());
        ch.set_modeled(Channel::Cog, self.ground_velocity.direction());
        ch.set_modeled(Channel::Depth, env.get(EnvParam::SeabedDepth));
        ch.set_modeled(Channel::WaterTemp, env.get(EnvParam::WaterTemp));
        ch.set_modeled(Channel::AirTemp, env.get(EnvParam::AirTemp));
        ch.set_modeled(Channel::Pressure, env.get(EnvParam::Pressure));
        ch.set_modeled(Channel::Humidity, env.get(EnvParam::Humidity));
        ch.set_modeled(Channel::Roll, v.heel_deg + self.motion.roll);
        ch.set_modeled(Channel::Pitch, self.motion.pitch);
        ch.set_modeled(Channel::Heave, self.motion.heave);
        ch.set_modeled(Channel::Leeway, v.leeway_deg);
        ch.set_modeled(Channel::EngineRpm, self.engine_rpm);

        // Wind. The environment wind is over ground; air moves *towards* direction + 180.
        let wind_air = Ne::from_polar(env.get(EnvParam::WindSpeed), env.get(EnvParam::WindDirection) + 180.0);
        let apparent = wind_air - self.ground_velocity; // air relative to the boat
        // Wind relative to the water, as instruments compute it from apparent wind and STW.
        let true_water = apparent + v.water_velocity();
        let from = |w: Ne| norm_360(w.direction() + 180.0);

        ch.set_modeled(Channel::Aws, apparent.magnitude());
        ch.set_modeled(Channel::Awa, norm_180(from(apparent) - v.heading));
        ch.set_modeled(Channel::Tws, true_water.magnitude());
        ch.set_modeled(Channel::Twa, norm_180(from(true_water) - v.heading));
        ch.set_modeled(Channel::Twd, from(true_water));
    }
}

fn apply_override(p: &mut crate::param::Param, o: Option<(OverrideMode, f64)>) {
    match o {
        Some((mode, v)) => p.set_override(mode, v),
        None => p.clear_override(),
    }
}

impl Default for InitialState {
    fn default() -> Self {
        let mut env = EnvParams::default();
        env.set_modeled(EnvParam::WindSpeed, 12.0);
        env.set_modeled(EnvParam::WindDirection, 270.0);
        env.set_modeled(EnvParam::SeabedDepth, 25.0);
        env.set_modeled(EnvParam::WaterTemp, 16.0);
        env.set_modeled(EnvParam::AirTemp, 18.0);
        env.set_modeled(EnvParam::Pressure, 1013.25);
        env.set_modeled(EnvParam::Humidity, 70.0);
        env.set_modeled(EnvParam::WavePeriod, 6.0);
        env.set_modeled(EnvParam::WaveDirection, 270.0);
        InitialState {
            start_time_unix: 0.0,
            seed: 1,
            vessel: VesselConfig::default(),
            position: LatLon::new(43.2950, 5.3600),
            heading: 0.0,
            stw_kn: 0.0,
            target_stw_kn: 6.0,
            steering: Steering::Heading { heading: 180.0 },
            env,
            // Tests and the M1 behaviour expect a steady environment by default.
            dynamics: Dynamics {
                wind: crate::dynamics::WindDynamics { gust_fraction: 0.0, shift_deg: 0.0, ..Default::default() },
                ..Default::default()
            },
            bathymetry: Arc::new(Bathymetry::default()),
            polar: Arc::new(Polar::default_cruiser()),
            engine_on: false,
            ais_targets: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bathymetry::Sounding;
    use crate::dynamics::{TideConfig, WindDynamics};

    fn sim() -> Simulation {
        let mut s = Simulation::new(InitialState::default());
        s.vessel.stw_kn = 6.0;
        s.target_stw_kn = 6.0;
        s
    }

    fn sailing() -> Simulation {
        let mut init = InitialState::default();
        init.vessel.propulsion = Propulsion::Sail;
        let mut s = Simulation::new(init);
        s.vessel.stw_kn = 5.0;
        s
    }

    #[test]
    fn head_to_wind_apparent_adds_speeds() {
        let mut s = sim();
        s.apply(Command::SetSteering(Steering::Heading { heading: 270.0 }));
        s.vessel.heading = 270.0;
        s.advance(1.0);
        assert!((s.channels.get(Channel::Aws) - 18.0).abs() < 0.05);
        assert!(s.channels.get(Channel::Awa).abs() < 0.5);
        assert!((s.channels.get(Channel::Tws) - 12.0).abs() < 0.05);
    }

    #[test]
    fn current_changes_cog_not_heading() {
        let mut s = sim();
        s.apply(Command::SetSteering(Steering::Heading { heading: 0.0 }));
        s.vessel.heading = 0.0;
        s.apply(Command::SetEnvBase(EnvParam::CurrentSpeed, 2.0));
        s.apply(Command::SetEnvBase(EnvParam::CurrentSet, 90.0));
        s.advance(1.0);
        let cog = s.channels.get(Channel::Cog);
        assert!((cog - (2.0f64).atan2(6.0).to_degrees()).abs() < 0.5, "cog {cog}");
        assert!(norm_180(s.channels.get(Channel::HeadingTrue)).abs() < 0.5);
    }

    #[test]
    fn route_reaches_waypoints_and_loops() {
        let mut s = sim();
        let a = s.vessel.position.destination(90.0, 400.0);
        let b = a.destination(0.0, 400.0);
        s.apply(Command::SetSteering(Steering::Route { waypoints: vec![a, b], active: 0, looped: true, arrival_radius_m: 30.0 }));
        s.apply(Command::SetTimeScale(10.0));
        let mut visited_b = false;
        for _ in 0..200 {
            s.advance(1.0);
            if s.steering.active_waypoint().map(|w| w.0) == Some(1) {
                visited_b = true;
            }
            if visited_b && s.steering.active_waypoint().map(|w| w.0) == Some(0) {
                return;
            }
        }
        panic!("route did not loop; active = {:?}", s.steering.active_waypoint());
    }

    #[test]
    fn channel_override_does_not_affect_physics() {
        let mut s = sim();
        s.apply(Command::SetChannelOverride(Channel::Depth, Some((OverrideMode::Replace, 2.0))));
        assert_eq!(s.channels.get(Channel::Depth), 2.0);
        assert_eq!(s.channels.param(Channel::Depth).modeled, 25.0);
    }

    #[test]
    fn paused_does_not_move() {
        let mut s = sim();
        s.apply(Command::SetPaused(true));
        let p = s.vessel.position;
        assert_eq!(s.advance(5.0), 0.0);
        assert_eq!(s.vessel.position, p);
    }

    #[test]
    fn gusts_vary_around_the_base_wind() {
        let mut s = sim();
        s.apply(Command::SetDynamics(Box::new(Dynamics {
            wind: WindDynamics { gust_fraction: 0.2, gust_period_s: 5.0, shift_deg: 10.0, shift_period_s: 20.0 },
            ..Default::default()
        })));
        let (mut min, mut max, mut sum, n) = (f64::MAX, f64::MIN, 0.0, 3000);
        for _ in 0..n {
            s.advance(0.1);
            let w = s.env.get(EnvParam::WindSpeed);
            min = min.min(w);
            max = max.max(w);
            sum += w;
        }
        assert!(max - min > 3.0, "wind should fluctuate: {min}..{max}");
        assert!((sum / n as f64 - 12.0).abs() < 1.5, "mean {}", sum / n as f64);
        assert_eq!(s.env_base.get(EnvParam::WindSpeed), 12.0);
    }

    #[test]
    fn tide_adds_height_and_stream() {
        let mut s = sim();
        let t0 = s.time_unix;
        s.apply(Command::SetDynamics(Box::new(Dynamics {
            tide: TideConfig { enabled: true, range_m: 3.0, high_water_unix: t0, max_rate_kn: 1.5, flood_set_deg: 45.0, ..Default::default() },
            ..InitialState::default().dynamics
        })));
        assert!((s.env.get(EnvParam::SeabedDepth) - 28.0).abs() < 1e-6, "chart 25 + HW 3");
        assert!(s.env.get(EnvParam::CurrentSpeed) < 1e-6, "slack at HW");
    }

    #[test]
    fn sailing_speed_follows_polar_and_heels_to_leeward() {
        let mut s = sailing();
        // Wind from 270, heading 0: wind on the port beam (twa = -90).
        s.apply(Command::SetSteering(Steering::Heading { heading: 0.0 }));
        s.vessel.heading = 0.0;
        for _ in 0..1200 {
            s.advance(0.1);
        }
        let expect = s.polar.speed(-90.0, 12.0);
        assert!((s.vessel.stw_kn - expect).abs() < 0.3, "stw {} vs polar {expect}", s.vessel.stw_kn);
        assert!(s.vessel.heel_deg > 5.0, "port wind heels to starboard: {}", s.vessel.heel_deg);
        assert!(s.vessel.leeway_deg > 0.0, "leeway to starboard: {}", s.vessel.leeway_deg);
        assert!(s.channels.get(Channel::EngineRpm) == 0.0);
    }

    #[test]
    fn sailing_boat_cannot_point_into_the_wind_but_tacks_upwind() {
        let mut s = sailing();
        // Waypoint 2 nm dead upwind (wind from 270).
        let wp = s.vessel.position.destination(270.0, 2.0 * 1852.0);
        s.apply(Command::SetSteering(Steering::Route { waypoints: vec![wp], active: 0, looped: false, arrival_radius_m: 100.0 }));
        s.apply(Command::SetTimeScale(20.0));
        let mut tacks = 0;
        let mut last_tack = 0.0;
        let mut min_dist = f64::MAX;
        let mut pinching = 0;
        for _ in 0..600 {
            s.advance(1.0);
            // Passing through the wind while tacking is fine; sailing inside the cone is not.
            if s.channels.get(Channel::Twa).abs() < 30.0 {
                pinching += 1;
            }
            if s.tack != 0.0 && s.tack != last_tack {
                tacks += 1;
                last_tack = s.tack;
            }
            min_dist = min_dist.min(s.vessel.position.distance_to(&wp));
        }
        assert!(tacks >= 2, "expected at least one tack, saw {tacks} tack states");
        assert!(s.route_complete, "upwind waypoint should be rounded");
        assert!(tacks <= 6, "too many tacks ({tacks}): flip-flopping near the mark?");
        assert!(pinching < 30, "{pinching} of 600 samples inside 30° of the wind");
        assert!(min_dist < 300.0, "never reached the upwind waypoint (closest {min_dist:.0} m)");
    }

    #[test]
    fn end_of_route_stops_and_drifts() {
        let mut s = sim();
        let wp = s.vessel.position.destination(180.0, 300.0);
        s.apply(Command::SetSteering(Steering::Route { waypoints: vec![wp], active: 0, looped: false, arrival_radius_m: 50.0 }));
        s.apply(Command::SetEnvBase(EnvParam::CurrentSpeed, 1.0));
        s.apply(Command::SetEnvBase(EnvParam::CurrentSet, 90.0));
        s.apply(Command::SetTimeScale(10.0));
        for _ in 0..60 {
            s.advance(1.0);
        }
        assert!(s.route_complete);
        assert!(s.vessel.stw_kn < 0.5, "stopped: stw {}", s.vessel.stw_kn);
        let sog = s.channels.get(Channel::Sog);
        let cog = s.channels.get(Channel::Cog);
        assert!((sog - 1.0).abs() < 0.5 && norm_180(cog - 90.0).abs() < 30.0, "drifting with the current: {sog} kn {cog}°");
        // A new route resumes.
        s.apply(Command::SetSteering(Steering::Heading { heading: 0.0 }));
        assert!(!s.route_complete);
    }

    #[test]
    fn grounding_stops_the_vessel() {
        let mut init = InitialState::default();
        // Shoal 300 m south of the start.
        let shoal = init.position.destination(180.0, 300.0);
        init.bathymetry = Arc::new(Bathymetry {
            soundings: vec![
                Sounding { lat: init.position.lat, lon: init.position.lon, depth: 20.0 },
                Sounding { lat: shoal.lat, lon: shoal.lon, depth: 0.5 },
            ],
            neighbours: Some(2),
        });
        init.heading = 180.0;
        init.stw_kn = 6.0;
        let mut s = Simulation::new(init);
        s.apply(Command::SetTimeScale(10.0));
        for _ in 0..60 {
            s.advance(1.0);
        }
        assert!(s.grounded, "should have run aground");
        assert!(s.vessel.stw_kn < 0.1);
        assert!(s.env.get(EnvParam::SeabedDepth) >= s.vessel_cfg.draft_m - 0.2);
        // Turning back out to deep water is possible.
        s.apply(Command::SetSteering(Steering::Heading { heading: 0.0 }));
        for _ in 0..60 {
            s.advance(1.0);
        }
        assert!(!s.grounded);
    }

    #[test]
    fn waves_make_the_boat_roll() {
        let mut s = sim();
        s.apply(Command::SetEnvBase(EnvParam::WaveHeight, 2.0));
        s.apply(Command::SetEnvBase(EnvParam::WaveDirection, 90.0)); // beam seas for heading 180
        s.apply(Command::SetSteering(Steering::Heading { heading: 180.0 }));
        s.vessel.heading = 180.0;
        let mut max_roll: f64 = 0.0;
        for _ in 0..300 {
            s.advance(0.1);
            max_roll = max_roll.max(s.channels.get(Channel::Roll).abs());
        }
        assert!(max_roll > 2.0, "max roll {max_roll}");
    }
}
