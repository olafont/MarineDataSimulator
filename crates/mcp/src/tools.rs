//! The tools. Each one returns JSON text (or TOML for a scenario); a refused call returns a
//! plain sentence saying what to change.

use std::sync::Arc;

use engine::scenario::{self, Scenario};
use engine::sim_core::ais::{AisClass, AisTargetConfig, SIM_MID, TargetMotion, collision_course, cpa_tcpa};
use engine::sim_core::geo::norm_360;
use engine::sim_core::param::{OverrideMode, Param};
use engine::sim_core::{Channel, Command, EnvParam, LatLon, Propulsion, Steering};
use engine::sim_sensors::{SensorConfig, SensorKind};
use engine::transports::{DataFormat, OutputConfig, OutputKind, Status};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::Host;

const INSTRUCTIONS: &str = "\
Marine Data Simulator: a simulated vessel whose instruments send NMEA 0183, NMEA 2000, Signal K \
and AIS data to other software. Everything here is simulated; nothing touches a real vessel.

Units: speeds in knots, distances in nautical miles (nm) unless a name ends in _m (metres), \
angles in degrees, bearings and headings true (0-360), positions in decimal degrees (north and \
east positive), times UTC. True wind angle is positive with the wind on the starboard side.

Start with get_status. The simulation runs in real time times time_scale (set_simulation). \
Two kinds of values can be changed: the environment (wind, current, depth...) drives the \
physics, so changing it changes how the vessel behaves; instruments are what the vessel \
reports, so overriding one changes only the data sent. A change is in force when its tool \
returns. read_data returns the lines going out, in one format at a time. AIS targets must use made-up identities: leave the MMSI out and one is generated that \
cannot belong to a real ship.";

type ToolResult = Result<String, String>;

#[derive(Clone)]
pub struct SimTools {
    host: Arc<dyn Host>,
    tool_router: ToolRouter<Self>,
}

impl SimTools {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host, tool_router: Self::tool_router() }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SimTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("marine-data-simulator", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

// -- Parameters --------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub struct Point {
    /// Latitude, decimal degrees, north positive.
    pub lat: f64,
    /// Longitude, decimal degrees, east positive.
    pub lon: f64,
}

impl Point {
    fn checked(self) -> Result<LatLon, String> {
        if self.lat.is_finite() && self.lon.is_finite() && self.lat.abs() <= 90.0 && self.lon.abs() <= 180.0 {
            Ok(LatLon::new(self.lat, self.lon))
        } else {
            Err(format!("position {}, {} is not on Earth: lat must be -90..90 and lon -180..180", self.lat, self.lon))
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    #[default]
    All,
    Environment,
    Instruments,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GetValuesParams {
    /// Which values: "all" (default), "environment" or "instruments".
    #[serde(default)]
    pub group: Group,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSimulationParams {
    /// true pauses simulated time, false resumes it.
    pub paused: Option<bool>,
    /// Simulated seconds per real second, 0.1 to 120 (1 = real time).
    pub time_scale: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SteerMode {
    Heading,
    WindAngle,
    Route,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SteerParams {
    /// "heading" holds a true heading, "wind_angle" holds a true wind angle, "route" follows
    /// waypoints (a sailing vessel tacks when the next one is upwind).
    pub mode: SteerMode,
    /// For "heading": degrees true, 0-360. Default: the current heading.
    pub heading: Option<f64>,
    /// For "wind_angle": true wind angle, -180..180, positive = wind on starboard. Default: the
    /// current angle.
    pub twa: Option<f64>,
    /// For "route": the waypoints, in order. Default: the waypoints of the current route.
    pub waypoints: Option<Vec<Point>>,
    /// For "route": go back to the first waypoint after the last (default: unchanged, or true).
    pub looped: Option<bool>,
    /// For "route": a waypoint counts as reached within this distance, metres.
    pub arrival_radius_m: Option<f64>,
    /// For "route": index (from 0) of the waypoint to head for now.
    pub active: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PropulsionParams {
    /// Sailing vessels only: true motors, false sails.
    pub engine_on: Option<bool>,
    /// Speed through water to hold under engine, knots.
    pub target_speed_kn: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetEnvironmentParams {
    /// Environment key from get_values, e.g. "wind_speed", "wind_direction", "current_speed",
    /// "current_set", "seabed_depth", "wave_height".
    pub key: String,
    /// The new base value, in the key's unit. Gusts, tide and soundings still add to it.
    pub value: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OverrideGroup {
    Environment,
    Instrument,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OverrideAction {
    /// Use `value` instead of the modeled value.
    Replace,
    /// Add `value` to the modeled value.
    Offset,
    /// Keep the current value.
    Freeze,
    /// Remove the override.
    Clear,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetOverrideParams {
    /// "environment" changes what drives the physics; "instrument" changes only what is reported.
    pub group: OverrideGroup,
    /// Key from get_values, e.g. "depth", "sog", "heading_true" (instrument) or "wind_speed"
    /// (environment).
    pub key: String,
    /// "replace", "offset", "freeze" or "clear".
    pub mode: OverrideAction,
    /// Needed for "replace" and "offset", in the key's unit.
    pub value: Option<f64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListTargetsParams {
    /// Also return each target's full configuration (static data, flags, intervals).
    #[serde(default)]
    pub details: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTargetParams {
    /// "a" (default), "b", "base_station", "aton" or "sart".
    pub class: Option<String>,
    /// Where to put it. Default: `range_nm` from own ship on `bearing`.
    pub position: Option<Point>,
    /// True bearing from own ship, degrees (default 45). Used when `position` is absent.
    pub bearing: Option<f64>,
    /// Distance from own ship, nm (default 2). Used when `position` is absent.
    pub range_nm: Option<f64>,
    /// Any other settings, with the keys of a scenario file's `[[ais.targets]]`, e.g.
    /// {"name": "TEST SHIP", "motion": {"mode": "course", "cog": 270, "sog_kn": 12},
    /// "nav_status": 0, "static": {"ship_type": 70, "destination": "TEST"}}. motion.mode is
    /// "course", "route" (waypoints, sog_kn, looped) or "stationary". Leave `mmsi` out to get a
    /// made-up one.
    pub config: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CollisionParams {
    /// Minutes until the target meets own ship (default 10).
    pub minutes: Option<f64>,
    /// Where it comes from, degrees relative to own ship's heading (default 60 = starboard bow).
    pub relative_bearing: Option<f64>,
    /// The target's speed over ground, knots (default 12).
    pub speed_kn: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTargetParams {
    /// The target's id, from list_ais_targets.
    pub id: u64,
    /// The settings to change, same keys as add_ais_target's `config` (plus "position",
    /// "enabled", "class"); everything else stays.
    pub config: Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveTargetsParams {
    /// Ids from list_ais_targets.
    pub ids: Option<Vec<u64>>,
    /// true removes every target.
    pub all: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoadScenarioParams {
    /// File name of a built-in scenario, from list_scenarios (e.g. "marseille-sailing.toml").
    pub builtin: Option<String>,
    /// Or a whole scenario as TOML text (get_scenario returns one to start from). It may not
    /// name external files (vessel.polar_file, bathymetry.csv) nor file or serial outputs.
    pub toml: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSensorParams {
    /// "gps", "compass", "speed_log", "depth_sounder", "wind", "water_temp", "weather", "rudder",
    /// "attitude", "engine" or "ais".
    pub kind: String,
    /// The settings to change, with the keys list_sensors shows, e.g. {"enabled": false},
    /// {"rate_hz": 5, "noise_scale": 0} or {"angle_bias_deg": 2.5}.
    pub config: Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddOutputParams {
    /// Same keys as a scenario file's `[[outputs]]`: "name", "type" and its address, then
    /// optionally "format" ("nmea0183" by default, "yd_raw", "actisense_ascii", "canboat_plain",
    /// "signalk"), "filter" (sentence or PGN list) and "enabled". Types: "udp" (target
    /// "host:port", broadcast), "tcp_server" (bind "0.0.0.0:port"), "tcp_client" (target),
    /// "web_socket" (bind). Example: {"name": "Plotter", "type": "udp", "target":
    /// "127.0.0.1:2000"}.
    pub config: Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveOutputParams {
    /// The output's id, from list_outputs.
    pub id: u64,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ReadDataParams {
    /// "nmea0183" (default), "yd_raw", "actisense_ascii", "canboat_plain" or "signalk". The
    /// buffer holds one format at a time: asking for another one switches it (and the app's
    /// Monitor) and starts afresh.
    pub format: Option<String>,
    /// Keep only these sentences (e.g. ["RMC", "VDM"]), PGNs (["129025"]) or Signal K sensors.
    pub tags: Option<Vec<String>>,
    /// Keep only lines that contain this text.
    pub contains: Option<String>,
    /// Keep only lines newer than this `next_seq` from an earlier call.
    pub after_seq: Option<u64>,
    /// At most this many lines, the newest ones (default 50, at most 500).
    pub limit: Option<usize>,
}

// -- Helpers -----------------------------------------------------------------------------------

fn reply(v: Value) -> ToolResult {
    serde_json::to_string(&v).map_err(|e| e.to_string())
}

/// Rounds for display: enough digits for the quantity, no float noise.
fn r(v: f64, decimals: i32) -> f64 {
    let k = 10f64.powi(decimals);
    (v * k).round() / k
}

fn point(p: LatLon) -> Value {
    json!({ "lat": r(p.lat, 6), "lon": r(p.lon, 6) })
}

fn time_utc(unix: f64) -> String {
    chrono::DateTime::from_timestamp(unix.floor() as i64, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn finite(name: &str, v: f64) -> Result<f64, String> {
    if v.is_finite() { Ok(v) } else { Err(format!("{name} must be a number")) }
}

fn in_range(name: &str, v: f64, min: f64, max: f64) -> Result<f64, String> {
    if v.is_finite() && (min..=max).contains(&v) { Ok(v) } else { Err(format!("{name} must be between {min} and {max}")) }
}

/// Lays `patch` over `base`: objects merge key by key, anything else replaces. An object that
/// changes its "mode" (the motion of a target) replaces the old one whole.
fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) if !p.contains_key("mode") || p.get("mode") == b.get("mode") => {
            for (k, v) in p {
                merge(b.entry(k).or_insert(Value::Null), v);
            }
        }
        (b, p) => *b = p,
    }
}

/// `current` with `patch` laid over it.
fn patched<T: serde::Serialize + serde::de::DeserializeOwned>(current: &T, patch: Map<String, Value>) -> Result<T, String> {
    let mut v = serde_json::to_value(current).map_err(|e| e.to_string())?;
    merge(&mut v, Value::Object(patch));
    serde_json::from_value(v).map_err(|e| format!("invalid settings: {e}"))
}

fn from_name<T: serde::de::DeserializeOwned>(what: &str, name: &str, expected: &str) -> Result<T, String> {
    serde_json::from_value(Value::String(name.to_string())).map_err(|_| format!("unknown {what} \"{name}\"; expected {expected}"))
}

const FORMATS: &str = "nmea0183, yd_raw, actisense_ascii, canboat_plain or signalk";

fn format_key(f: DataFormat) -> Value {
    serde_json::to_value(f).unwrap_or(Value::Null)
}

fn param_json(key: &str, label: &str, unit: &str, p: &Param) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("key".into(), key.into());
    m.insert("label".into(), label.into());
    m.insert("unit".into(), unit.into());
    m.insert("value".into(), r(p.value(), 4).into());
    if let Some(o) = p.ovr {
        m.insert("modeled".into(), r(p.modeled, 4).into());
        m.insert("override".into(), json!({ "mode": o.mode, "value": r(o.value, 4) }));
    }
    m
}

fn target_config_checked(cfg: &AisTargetConfig) -> Result<(), String> {
    Point { lat: cfg.position.lat, lon: cfg.position.lon }.checked()?;
    if let TargetMotion::Route { waypoints, .. } = &cfg.motion {
        for w in waypoints {
            Point { lat: w.lat, lon: w.lon }.checked()?;
        }
    }
    Ok(())
}

/// A note when a ship's MMSI could be a real one.
fn mmsi_note(cfg: &AisTargetConfig) -> Option<String> {
    let made_up = match cfg.class {
        AisClass::A | AisClass::B => cfg.mmsi / 1_000_000 == SIM_MID,
        _ => true,
    };
    (!made_up).then(|| {
        format!(
            "MMSI {} may belong to a real vessel. Simulated ships should use 100xxxxxx (leave mmsi out to get one); never send a real identity to AIS services.",
            cfg.mmsi
        )
    })
}

// -- Tools -------------------------------------------------------------------------------------

#[tool_router]
impl SimTools {
    fn engine(&self) -> &engine::Engine {
        self.host.engine()
    }

    /// Waits until the engine has applied the commands just sent and published the result, so
    /// the next call (from this tool or the agent) sees them. Two ticks: one may be under way.
    fn settle(&self) {
        self.wait_ticks(2);
    }

    fn wait_ticks(&self, n: u64) {
        let target = self.engine().snapshot().tick + n;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while self.engine().snapshot().tick < target && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[tool(description = "Summary of the simulation: scenario, simulated time, pause and time scale, the vessel's position, heading, speeds and depth, wind, propulsion, steering (mode and route), and whether it is aground. Start here.", annotations(read_only_hint = true))]
    fn get_status(&self) -> ToolResult {
        let e = self.engine();
        let snap = e.snapshot();
        let sim = &snap.sim;
        let ch = |c: Channel| sim.channels.get(c);
        let mut v = json!({
            "scenario": e.scenario().name,
            "time_utc": time_utc(sim.time_unix),
            "paused": sim.paused,
            "time_scale": sim.time_scale,
            "vessel": {
                "name": sim.vessel_cfg.name,
                "mmsi": sim.vessel_cfg.mmsi,
                "position": point(sim.vessel.position),
                "heading_true": r(sim.vessel.heading, 1),
                "course_over_ground": r(ch(Channel::Cog), 1),
                "speed_over_ground_kn": r(ch(Channel::Sog), 2),
                "speed_through_water_kn": r(sim.vessel.stw_kn, 2),
                "depth_m": r(ch(Channel::Depth), 1),
                "draft_m": sim.vessel_cfg.draft_m,
                "aground": sim.grounded,
            },
            "wind": {
                "true_speed_kn": r(ch(Channel::Tws), 1),
                "true_direction": r(ch(Channel::Twd), 0),
                "true_angle": r(ch(Channel::Twa), 0),
                "apparent_speed_kn": r(ch(Channel::Aws), 1),
                "apparent_angle": r(ch(Channel::Awa), 0),
            },
            "propulsion": {
                "type": sim.vessel_cfg.propulsion,
                "engine_on": sim.engine_on || sim.vessel_cfg.propulsion == Propulsion::Motor,
                "engine_rpm": r(sim.engine_rpm, 0),
                "target_speed_kn": sim.target_stw_kn,
            },
            "steering": sim.steering,
            "route_complete": sim.route_complete,
            "tide_height_m": r(sim.tide_height_m, 2),
            "ais_targets": sim.ais_targets.len(),
            "lines_per_second": r(snap.sentences_per_sec, 1),
        });
        if let Some(rep) = &snap.replay {
            v["replay"] = json!({ "log": rep.name, "playing": rep.playing, "simulated_data_muted": rep.mute_simulation });
        }
        reply(v)
    }

    #[tool(description = "Every environment input (wind, current, depth, temperatures, pressure, waves: they drive the physics) and every instrument value (position, heading, speeds, wind angles, depth...: what the vessel reports), with key, unit, current value and any override. The keys are the ones set_environment and set_override take.", annotations(read_only_hint = true))]
    fn get_values(&self, Parameters(p): Parameters<GetValuesParams>) -> ToolResult {
        let snap = self.engine().snapshot();
        let sim = &snap.sim;
        let mut v = Map::new();
        if !matches!(p.group, Group::Instruments) {
            let env: Vec<Value> = EnvParam::ALL
                .iter()
                .map(|&e| {
                    let m = e.meta();
                    let mut o = param_json(m.key, m.label, m.unit, sim.env.param(e));
                    o.insert("base".into(), r(sim.env_base.get(e), 4).into());
                    Value::Object(o)
                })
                .collect();
            v.insert("environment".into(), env.into());
        }
        if !matches!(p.group, Group::Environment) {
            let ins: Vec<Value> = Channel::ALL
                .iter()
                .map(|&c| {
                    let m = c.meta();
                    let mut o = param_json(m.key, m.label, m.unit, sim.channels.param(c));
                    if matches!(c, Channel::Latitude | Channel::Longitude) {
                        o.insert("value".into(), r(sim.channels.get(c), 6).into());
                    }
                    Value::Object(o)
                })
                .collect();
            v.insert("instruments".into(), ins.into());
        }
        reply(Value::Object(v))
    }

    #[tool(description = "Pause or resume simulated time, and set how fast it runs (time_scale 0.1 to 120; 1 = real time).")]
    fn set_simulation(&self, Parameters(p): Parameters<SetSimulationParams>) -> ToolResult {
        if p.paused.is_none() && p.time_scale.is_none() {
            return Err("give paused, time_scale or both".into());
        }
        if let Some(ts) = p.time_scale {
            self.engine().send(Command::SetTimeScale(in_range("time_scale", ts, 0.1, 120.0)?));
        }
        if let Some(paused) = p.paused {
            self.engine().send(Command::SetPaused(paused));
        }
        let sim = &self.engine().snapshot().sim;
        self.settle();
        reply(json!({ "paused": p.paused.unwrap_or(sim.paused), "time_scale": p.time_scale.unwrap_or(sim.time_scale) }))
    }

    #[tool(description = "Steer the vessel: hold a true heading, hold a true wind angle, or follow a route of waypoints. The vessel turns at its own rate; check progress with get_status.")]
    fn steer(&self, Parameters(p): Parameters<SteerParams>) -> ToolResult {
        let snap = self.engine().snapshot();
        let sim = &snap.sim;
        let steering = match p.mode {
            SteerMode::Heading => {
                let h = match p.heading {
                    Some(h) => norm_360(finite("heading", h)?),
                    None => sim.vessel.heading.round(),
                };
                Steering::Heading { heading: h }
            }
            SteerMode::WindAngle => {
                let twa = match p.twa {
                    Some(a) => in_range("twa", a, -180.0, 180.0)?,
                    None => sim.channels.get(Channel::Twa).round(),
                };
                Steering::WindAngle { twa }
            }
            SteerMode::Route => {
                let (mut waypoints, mut active, mut looped, mut radius) = match &sim.steering {
                    Steering::Route { waypoints, active, looped, arrival_radius_m } => {
                        (waypoints.clone(), *active, *looped, *arrival_radius_m)
                    }
                    _ => (vec![], 0, true, 50.0),
                };
                if let Some(w) = p.waypoints {
                    waypoints = w.into_iter().map(Point::checked).collect::<Result<_, _>>()?;
                    active = 0;
                }
                if waypoints.is_empty() {
                    return Err("a route needs waypoints: give `waypoints`".into());
                }
                if let Some(a) = p.active {
                    if a >= waypoints.len() {
                        return Err(format!("active must be below the number of waypoints ({})", waypoints.len()));
                    }
                    active = a;
                }
                if let Some(l) = p.looped {
                    looped = l;
                }
                if let Some(m) = p.arrival_radius_m {
                    radius = in_range("arrival_radius_m", m, 5.0, 5000.0)?;
                }
                Steering::Route { waypoints, active, looped, arrival_radius_m: radius }
            }
        };
        let out = serde_json::to_value(&steering).map_err(|e| e.to_string())?;
        self.engine().send(Command::SetSteering(steering));
        self.settle();
        reply(json!({ "steering": out }))
    }

    #[tool(description = "Engine and speed: start or stop the engine of a sailing vessel, and set the speed through water to hold under engine. A motor vessel always runs its engine.")]
    fn set_propulsion(&self, Parameters(p): Parameters<PropulsionParams>) -> ToolResult {
        if p.engine_on.is_none() && p.target_speed_kn.is_none() {
            return Err("give engine_on, target_speed_kn or both".into());
        }
        let cfg = &self.engine().snapshot().sim.vessel_cfg;
        if let Some(kn) = p.target_speed_kn {
            self.engine().send(Command::SetTargetSpeed(in_range("target_speed_kn", kn, 0.0, cfg.max_speed_kn)?));
        }
        if let Some(on) = p.engine_on {
            if cfg.propulsion == Propulsion::Motor {
                return Err("this is a motor vessel: its engine is always on (set target_speed_kn to 0 to stop)".into());
            }
            self.engine().send(Command::SetEngine(on));
        }
        self.settle();
        reply(json!({ "ok": true }))
    }

    #[tool(description = "Put the vessel at a position at once (like dragging it on the chart). Heading, speed and steering stay as they are.")]
    fn move_vessel(&self, Parameters(p): Parameters<Point>) -> ToolResult {
        let pos = p.checked()?;
        self.engine().send(Command::Teleport(pos));
        self.host.vessel_moved();
        self.settle();
        reply(json!({ "position": point(pos) }))
    }

    #[tool(description = "Set the base value of an environment input (e.g. wind_speed 25, wind_direction 270, current_speed 2, seabed_depth 8). This changes how the vessel behaves. The simulator still adds gusts, wind shifts, tide and soundings on top; to pin an exact value use set_override instead.")]
    fn set_environment(&self, Parameters(p): Parameters<SetEnvironmentParams>) -> ToolResult {
        let Some(param) = EnvParam::from_key(&p.key) else {
            let keys: Vec<&str> = EnvParam::ALL.iter().map(|e| e.meta().key).collect();
            return Err(format!("unknown environment key \"{}\"; expected one of: {}", p.key, keys.join(", ")));
        };
        let m = param.meta();
        let value = in_range(m.key, p.value, m.min, m.max)?;
        self.engine().send(Command::SetEnvBase(param, value));
        self.settle();
        reply(json!({ "key": m.key, "base": value, "unit": m.unit }))
    }

    #[tool(description = "Override a value: replace it, offset it, freeze it, or clear the override. group \"instrument\" changes only what the vessel reports (e.g. make the depth sounder read 1.5 m, or freeze the GPS position) without touching the physics; group \"environment\" pins an input of the physics (e.g. exactly 30 kn of wind, no gusts).")]
    fn set_override(&self, Parameters(p): Parameters<SetOverrideParams>) -> ToolResult {
        let ovr = match p.mode {
            OverrideAction::Clear => None,
            OverrideAction::Freeze => Some((OverrideMode::Freeze, 0.0)),
            OverrideAction::Replace | OverrideAction::Offset => {
                let v = finite("value", p.value.ok_or("replace and offset need `value`")?)?;
                Some((if matches!(p.mode, OverrideAction::Replace) { OverrideMode::Replace } else { OverrideMode::Offset }, v))
            }
        };
        match p.group {
            OverrideGroup::Environment => {
                let Some(param) = EnvParam::from_key(&p.key) else {
                    let keys: Vec<&str> = EnvParam::ALL.iter().map(|e| e.meta().key).collect();
                    return Err(format!("unknown environment key \"{}\"; expected one of: {}", p.key, keys.join(", ")));
                };
                self.engine().send(Command::SetEnvOverride(param, ovr));
            }
            OverrideGroup::Instrument => {
                let Some(ch) = Channel::from_key(&p.key) else {
                    let keys: Vec<&str> = Channel::ALL.iter().map(|c| c.meta().key).collect();
                    return Err(format!("unknown instrument key \"{}\"; expected one of: {}", p.key, keys.join(", ")));
                };
                self.engine().send(Command::SetChannelOverride(ch, ovr));
            }
        }
        self.settle();
        reply(json!({ "key": p.key, "override": ovr.map(|(mode, value)| json!({ "mode": mode, "value": value })) }))
    }

    #[tool(description = "The other AIS stations (ships, base stations, aids to navigation, SARTs): id, identity, position, course and speed, plus range and bearing from own ship and the closest point of approach (cpa_nm, tcpa_min; negative tcpa = already past).", annotations(read_only_hint = true))]
    fn list_ais_targets(&self, Parameters(p): Parameters<ListTargetsParams>) -> ToolResult {
        let snap = self.engine().snapshot();
        let sim = &snap.sim;
        let own = sim.vessel.position;
        let targets: Vec<Value> = sim
            .ais_targets
            .iter()
            .map(|t| {
                let (cpa, tcpa) = cpa_tcpa(own, sim.ground_velocity, t.state.position, t.velocity());
                let mut v = json!({
                    "id": t.id,
                    "mmsi": t.cfg.mmsi,
                    "name": t.cfg.name,
                    "class": t.cfg.class,
                    "enabled": t.cfg.enabled,
                    "position": point(t.state.position),
                    "course_over_ground": r(t.state.cog, 1),
                    "speed_over_ground_kn": r(t.state.sog_kn, 1),
                    "heading_true": r(t.state.heading, 1),
                    "nav_status": t.cfg.nav_status,
                    "range_nm": r(own.distance_to(&t.state.position) / 1852.0, 2),
                    "bearing_true": r(own.bearing_to(&t.state.position), 0),
                    "cpa_nm": r(cpa, 2),
                    "tcpa_min": r(tcpa, 1),
                });
                if p.details {
                    v["config"] = serde_json::to_value(t.captured()).unwrap_or(Value::Null);
                }
                v
            })
            .collect();
        reply(json!({ "own_ship": point(own), "targets": targets }))
    }

    #[tool(description = "Add an AIS station: a ship (class a or b), a base station, an aid to navigation or a SART. Give a position, or a bearing and range from own ship. A made-up name, MMSI and call sign are generated unless `config` sets them. Returns the new target's id.")]
    fn add_ais_target(&self, Parameters(p): Parameters<AddTargetParams>) -> ToolResult {
        let snap = self.engine().snapshot();
        let sim = &snap.sim;
        let class: AisClass = match &p.class {
            Some(c) => from_name("class", c, "a, b, base_station, aton or sart")?,
            None => AisClass::A,
        };
        let position = match p.position {
            Some(pt) => pt.checked()?,
            None => {
                let range = in_range("range_nm", p.range_nm.unwrap_or(2.0), 0.0, 500.0)?;
                sim.vessel.position.destination(norm_360(finite("bearing", p.bearing.unwrap_or(45.0))?), range * 1852.0)
            }
        };
        let id = sim.next_ais_target_id();
        let mut cfg = AisTargetConfig::new(class, id as u32, position);
        if let Some(patch) = p.config {
            cfg = patched(&cfg, patch)?;
        }
        target_config_checked(&cfg)?;
        let mut out = json!({ "id": id, "mmsi": cfg.mmsi, "name": cfg.name, "position": point(cfg.position) });
        if let Some(note) = mmsi_note(&cfg) {
            out["warning"] = note.into();
        }
        self.engine().send(Command::AddAisTarget(Box::new(cfg)));
        self.settle();
        reply(out)
    }

    #[tool(description = "Add a Class A ship on a collision course: it will meet own ship's predicted position after the given minutes, coming from a bearing relative to own ship's heading. For testing CPA/TCPA alarms. Returns the new target's id.")]
    fn add_collision_target(&self, Parameters(p): Parameters<CollisionParams>) -> ToolResult {
        let snap = self.engine().snapshot();
        let sim = &snap.sim;
        let minutes = in_range("minutes", p.minutes.unwrap_or(10.0), 1.0, 120.0)?;
        let speed = in_range("speed_kn", p.speed_kn.unwrap_or(12.0), 1.0, 40.0)?;
        let from = norm_360(sim.vessel.heading + finite("relative_bearing", p.relative_bearing.unwrap_or(60.0))?);
        let (start, cog) = collision_course(sim.vessel.position, sim.ground_velocity, minutes, from, speed);
        let id = sim.next_ais_target_id();
        let mut cfg = AisTargetConfig::new(AisClass::A, id as u32, start);
        cfg.name = format!("COLLISION {id}");
        cfg.motion = TargetMotion::Course { cog: cog.round(), sog_kn: speed };
        cfg.heading = cog;
        let out = json!({ "id": id, "mmsi": cfg.mmsi, "name": cfg.name, "position": point(start), "course_over_ground": cog.round(), "speed_over_ground_kn": speed, "meets_own_ship_in_min": minutes });
        self.engine().send(Command::AddAisTarget(Box::new(cfg)));
        self.settle();
        reply(out)
    }

    #[tool(description = "Change an AIS target: only the settings given in `config` change (e.g. {\"motion\": {\"mode\": \"course\", \"cog\": 90, \"sog_kn\": 5}}, {\"position\": {\"lat\": 43.2, \"lon\": 5.3}}, {\"nav_status\": 1}, {\"enabled\": false}).")]
    fn update_ais_target(&self, Parameters(p): Parameters<UpdateTargetParams>) -> ToolResult {
        let snap = self.engine().snapshot();
        let Some(t) = snap.sim.ais_targets.iter().find(|t| t.id == p.id) else {
            return Err(format!("no AIS target with id {} (see list_ais_targets)", p.id));
        };
        let cfg: AisTargetConfig = patched(&t.captured(), p.config)?;
        target_config_checked(&cfg)?;
        let mut out = json!({ "id": p.id, "config": serde_json::to_value(&cfg).map_err(|e| e.to_string())? });
        if let Some(note) = mmsi_note(&cfg) {
            out["warning"] = note.into();
        }
        self.engine().send(Command::UpdateAisTarget(p.id, Box::new(cfg)));
        self.settle();
        reply(out)
    }

    #[tool(description = "Remove AIS targets: the given ids, or all of them.")]
    fn remove_ais_targets(&self, Parameters(p): Parameters<RemoveTargetsParams>) -> ToolResult {
        let snap = self.engine().snapshot();
        let existing: Vec<u64> = snap.sim.ais_targets.iter().map(|t| t.id).collect();
        let ids: Vec<u64> = match (p.all, p.ids) {
            (Some(true), _) => existing,
            (_, Some(ids)) => {
                if let Some(missing) = ids.iter().find(|id| !existing.contains(id)) {
                    return Err(format!("no AIS target with id {missing} (see list_ais_targets)"));
                }
                ids
            }
            _ => return Err("give `ids`, or `all`: true".into()),
        };
        for id in &ids {
            self.engine().send(Command::RemoveAisTarget(*id));
        }
        self.settle();
        reply(json!({ "removed": ids }))
    }

    #[tool(description = "The built-in scenarios (file name and title), for load_scenario.", annotations(read_only_hint = true))]
    fn list_scenarios(&self) -> ToolResult {
        let list: Vec<Value> = scenario::BUILTIN
            .iter()
            .map(|(file, _)| json!({ "file": file, "name": scenario::builtin(file).map(|s| s.name).unwrap_or_default() }))
            .collect();
        reply(json!({ "current": self.engine().scenario().name, "builtin": list }))
    }

    #[tool(description = "Load a scenario: a built-in one by file name, or a whole scenario given as TOML text. This replaces the vessel, environment, AIS targets, sensors and the scenario's outputs, and restarts the simulation from the scenario's initial state.")]
    fn load_scenario(&self, Parameters(p): Parameters<LoadScenarioParams>) -> ToolResult {
        let sc = match (p.builtin, p.toml) {
            (Some(name), None) => scenario::builtin(&name).map_err(|_| {
                let names: Vec<&str> = scenario::BUILTIN.iter().map(|(n, _)| *n).collect();
                format!("no built-in scenario \"{name}\"; expected one of: {}", names.join(", "))
            })?,
            (None, Some(text)) => {
                // Check before `from_toml`, which reads the files a scenario names.
                let raw: Scenario = toml::from_str(&text).map_err(|e| format!("invalid scenario: {e}"))?;
                if raw.vessel.polar_file.is_some() || raw.bathymetry.csv.is_some() {
                    return Err("this scenario names external files (vessel.polar_file or bathymetry.csv); the MCP server does not read files. Remove them, or open the scenario in the app.".into());
                }
                if let Some(o) = raw.outputs.iter().find(|o| matches!(o.kind, OutputKind::File { .. } | OutputKind::Serial { .. })) {
                    return Err(format!("output \"{}\" is a file or serial output; the MCP server only starts network outputs. Remove it, or open the scenario in the app.", o.name));
                }
                Scenario::from_toml(&text).map_err(|e| format!("invalid scenario: {e:#}"))?
            }
            _ => return Err("give either `builtin` or `toml`".into()),
        };
        let name = sc.name.clone();
        self.host.load_scenario(sc);
        self.settle();
        reply(json!({ "loaded": name }))
    }

    #[tool(description = "The current state as a scenario in TOML: vessel, position, steering, environment, tide, overrides, AIS targets, sensors and outputs. Edit it and pass it to load_scenario, or save it as a .toml file for the app.", annotations(read_only_hint = true))]
    fn get_scenario(&self) -> ToolResult {
        self.engine().capture_scenario().to_toml().map_err(|e| format!("{e:#}"))
    }

    #[tool(description = "Go back to the initial state of the loaded scenario (the app's Reset).")]
    fn reset_scenario(&self) -> ToolResult {
        let sc = self.engine().scenario();
        let name = sc.name.clone();
        self.host.load_scenario(sc);
        self.settle();
        reply(json!({ "reset": name }))
    }

    #[tool(description = "The simulated instruments (GPS, compass, speed log, depth sounder, wind, AIS transponder...) with their settings: enabled, rate, talker, noise, bias, calibration, latency, resolution.", annotations(read_only_hint = true))]
    fn list_sensors(&self) -> ToolResult {
        let snap = self.engine().snapshot();
        reply(json!({ "sensors": snap.sensors }))
    }

    #[tool(description = "Change one instrument's settings: switch it off to simulate a failure, change its rate, or add errors (noise_scale: 0 = perfect, 1 = typical; angle_bias_deg; speed_scale; latency_s; resolution). Only the settings given change.")]
    fn set_sensor(&self, Parameters(p): Parameters<SetSensorParams>) -> ToolResult {
        let kind: SensorKind = from_name(
            "sensor",
            &p.kind,
            "gps, compass, speed_log, depth_sounder, wind, water_temp, weather, rudder, attitude, engine or ais",
        )?;
        let snap = self.engine().snapshot();
        let current = snap.sensors.iter().find(|s| s.kind == kind).cloned().unwrap_or_else(|| SensorConfig::new(kind));
        let mut patch = p.config;
        patch.remove("kind");
        let cfg: SensorConfig = patched(&current, patch)?;
        in_range("rate_hz", cfg.rate_hz, 0.01, 50.0)?;
        in_range("latency_s", cfg.latency_s, 0.0, 10.0)?;
        let out = serde_json::to_value(&cfg).map_err(|e| e.to_string())?;
        self.engine().set_sensor(cfg);
        self.settle();
        reply(out)
    }

    #[tool(description = "The outputs that carry the data to other software (UDP, TCP, WebSocket, serial, file): id, address, format, filter, status, lines sent, errors and connected clients.", annotations(read_only_hint = true))]
    fn list_outputs(&self) -> ToolResult {
        let list: Vec<Value> = self
            .engine()
            .outputs()
            .list()
            .into_iter()
            .map(|o| {
                let (status, problem) = match &o.status {
                    Status::Disabled => ("disabled", None),
                    Status::Starting => ("starting", None),
                    Status::Running => ("running", None),
                    Status::Warning(w) => ("running", Some(w.clone())),
                    Status::Failed(f) => ("failed", Some(f.clone())),
                };
                let mut v = json!({
                    "id": o.id,
                    "description": o.config.kind.describe(),
                    "format": format_key(o.config.format),
                    "status": status,
                    "lines_sent": o.sent,
                    "bytes_sent": o.bytes,
                    "errors": o.errors,
                    "clients": o.clients,
                    "config": o.config,
                });
                if let Some(p) = problem {
                    v["problem"] = p.into();
                }
                v
            })
            .collect();
        reply(json!({ "outputs": list }))
    }

    #[tool(description = "Add a network output (UDP, TCP server, TCP client or WebSocket server) so that other software receives the data. It lasts until the next scenario is loaded. Check its status with list_outputs: a port already in use shows as failed.")]
    fn add_output(&self, Parameters(p): Parameters<AddOutputParams>) -> ToolResult {
        let cfg: OutputConfig = serde_json::from_value(Value::Object(p.config)).map_err(|e| format!("invalid output: {e}"))?;
        if matches!(cfg.kind, OutputKind::File { .. } | OutputKind::Serial { .. }) {
            return Err("the MCP server only adds network outputs (udp, tcp_server, tcp_client, web_socket); add file and serial outputs in the app".into());
        }
        let description = cfg.kind.describe();
        let id = self.engine().outputs().add(cfg);
        reply(json!({ "id": id, "description": description }))
    }

    #[tool(description = "Stop and remove an output.")]
    fn remove_output(&self, Parameters(p): Parameters<RemoveOutputParams>) -> ToolResult {
        if !self.engine().outputs().list().iter().any(|o| o.id == p.id) {
            return Err(format!("no output with id {} (see list_outputs)", p.id));
        }
        self.engine().outputs().remove(p.id);
        reply(json!({ "removed": p.id }))
    }

    #[tool(description = "The most recent lines sent to the outputs: NMEA 0183 sentences (including !AIVDM AIS), NMEA 2000 frames or Signal K deltas. Filter by sentence or PGN (`tags`) or by text. Pass the returned next_seq as after_seq to get only newer lines next time. About the last 2000 lines are kept.", annotations(read_only_hint = true))]
    fn read_data(&self, Parameters(p): Parameters<ReadDataParams>) -> ToolResult {
        let format: DataFormat = match &p.format {
            Some(f) => from_name("format", f, FORMATS)?,
            None => DataFormat::Nmea0183,
        };
        let limit = p.limit.unwrap_or(50).clamp(1, 500);
        // The buffer holds one format at a time (the one the app's Monitor shows): switch it.
        let switched = {
            let mut mon = self.engine().monitor().lock().map_err(|_| "monitor unavailable")?;
            let switched = mon.view != format;
            if switched {
                mon.view = format;
                mon.lines.clear();
            }
            switched
        };
        if switched {
            // Give the sensors time to report in the new format (most do within a second).
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
            while std::time::Instant::now() < deadline {
                self.wait_ticks(2);
                if !self.engine().monitor().lock().map_err(|_| "monitor unavailable")?.lines.is_empty() {
                    break;
                }
            }
        }
        let mon = self.engine().monitor().lock().map_err(|_| "monitor unavailable")?;
        let mut lines: Vec<&str> = mon
            .lines
            .iter()
            .rev()
            .filter(|l| l.format == format)
            .filter(|l| p.after_seq.is_none_or(|s| l.seq >= s))
            .filter(|l| p.tags.as_ref().is_none_or(|tags| tags.iter().any(|t| t.eq_ignore_ascii_case(&l.tag))))
            .filter(|l| p.contains.as_ref().is_none_or(|c| l.text.contains(c.as_str())))
            .take(limit)
            .map(|l| l.text.as_str())
            .collect();
        lines.reverse();
        let mut v = json!({ "format": format_key(format), "next_seq": mon.next_seq, "lines": lines });
        if switched {
            v["note"] = format!(
                "the data buffer held another format and now holds {} (the app's Monitor shows it too): only the lines since the switch are here, call again for more",
                format.label()
            )
            .into();
        }
        reply(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineHost;
    use engine::Engine;
    use std::time::Duration;

    /// Tools on an engine with the default scenario and no outputs (so no ports are opened).
    fn tools() -> SimTools {
        let mut sc = scenario::default_scenario();
        sc.outputs.clear();
        SimTools::new(Arc::new(EngineHost(Arc::new(Engine::start(sc, None).unwrap()))))
    }

    /// Lets the simulation thread apply the commands and publish a snapshot.
    fn settle() {
        std::thread::sleep(Duration::from_millis(350));
    }

    fn parse(r: ToolResult) -> Value {
        serde_json::from_str(&r.unwrap()).unwrap()
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn status_and_values() {
        let t = tools();
        let s = parse(t.get_status());
        assert_eq!(s["scenario"], "Cape Canaveral sailing loop");
        assert_eq!(s["steering"]["mode"], "route");
        assert!(s["vessel"]["position"]["lat"].as_f64().unwrap() > 28.0);
        let v = parse(t.get_values(Parameters(GetValuesParams::default())));
        assert_eq!(v["environment"].as_array().unwrap().len(), EnvParam::COUNT);
        assert_eq!(v["instruments"].as_array().unwrap().len(), Channel::COUNT);
    }

    #[test]
    fn steering_environment_and_overrides() {
        let t = tools();
        parse(t.steer(Parameters(SteerParams {
            mode: SteerMode::Heading,
            heading: Some(450.0),
            twa: None,
            waypoints: None,
            looped: None,
            arrival_radius_m: None,
            active: None,
        })));
        parse(t.set_environment(Parameters(SetEnvironmentParams { key: "wind_speed".into(), value: 25.0 })));
        parse(t.set_override(Parameters(SetOverrideParams {
            group: OverrideGroup::Instrument,
            key: "depth".into(),
            mode: OverrideAction::Replace,
            value: Some(1.5),
        })));
        parse(t.set_simulation(Parameters(SetSimulationParams { paused: Some(true), time_scale: Some(10.0) })));
        settle();
        let snap = t.engine().snapshot();
        assert_eq!(snap.sim.steering, Steering::Heading { heading: 90.0 });
        assert_eq!(snap.sim.env_base.get(EnvParam::WindSpeed), 25.0);
        assert_eq!(snap.sim.channels.get(Channel::Depth), 1.5);
        assert!(snap.sim.paused);
        assert_eq!(snap.sim.time_scale, 10.0);

        // Refusals say what to change.
        let e = t.set_environment(Parameters(SetEnvironmentParams { key: "wnd".into(), value: 1.0 })).unwrap_err();
        assert!(e.contains("wind_speed"), "{e}");
        let e = t
            .set_override(Parameters(SetOverrideParams {
                group: OverrideGroup::Instrument,
                key: "depth".into(),
                mode: OverrideAction::Offset,
                value: None,
            }))
            .unwrap_err();
        assert!(e.contains("value"), "{e}");
        assert!(t.move_vessel(Parameters(Point { lat: 95.0, lon: 0.0 })).is_err());
    }

    #[test]
    fn ais_targets() {
        let t = tools();
        let before = t.engine().snapshot().sim.ais_targets.len();
        let added = parse(t.add_ais_target(Parameters(AddTargetParams {
            class: Some("b".into()),
            position: None,
            bearing: Some(90.0),
            range_nm: Some(3.0),
            config: Some(obj(json!({ "name": "TEST YACHT", "motion": { "mode": "course", "cog": 270.0, "sog_kn": 7.0 } }))),
        })));
        let id = added["id"].as_u64().unwrap();
        assert_eq!(added["mmsi"].as_u64().unwrap() / 1_000_000, u64::from(SIM_MID));
        assert!(added.get("warning").is_none());
        settle();
        let list = parse(t.list_ais_targets(Parameters(ListTargetsParams { details: true })));
        let targets = list["targets"].as_array().unwrap();
        assert_eq!(targets.len(), before + 1);
        let mine = targets.iter().find(|x| x["id"] == id).expect("the new target has the returned id");
        assert_eq!(mine["name"], "TEST YACHT");
        assert_eq!(mine["class"], "b");
        assert!((mine["range_nm"].as_f64().unwrap() - 3.0).abs() < 0.1);
        assert_eq!(mine["config"]["motion"]["sog_kn"], 7.0);

        // A patch changes only what it names; a new motion mode replaces the old motion.
        parse(t.update_ais_target(Parameters(UpdateTargetParams { id, config: obj(json!({ "nav_status": 1, "motion": { "mode": "stationary" } })) })));
        settle();
        let snap = t.engine().snapshot();
        let cfg = &snap.sim.ais_targets.iter().find(|x| x.id == id).unwrap().cfg;
        assert_eq!(cfg.nav_status, 1);
        assert_eq!(cfg.motion, TargetMotion::Stationary);
        assert_eq!(cfg.name, "TEST YACHT");

        // A real-looking MMSI is accepted with a warning.
        let w = parse(t.update_ais_target(Parameters(UpdateTargetParams { id, config: obj(json!({ "mmsi": 227_000_001 })) })));
        assert!(w["warning"].as_str().unwrap().contains("real vessel"));

        let c = parse(t.add_collision_target(Parameters(CollisionParams { minutes: Some(5.0), relative_bearing: None, speed_kn: None })));
        settle();
        let list = parse(t.list_ais_targets(Parameters(ListTargetsParams::default())));
        let col = list["targets"].as_array().unwrap().iter().find(|x| x["id"] == c["id"]).unwrap().clone();
        assert!(col["cpa_nm"].as_f64().unwrap() < 0.3, "collision target CPA {col}");

        assert!(t.remove_ais_targets(Parameters(RemoveTargetsParams { ids: Some(vec![9999]), all: None })).is_err());
        parse(t.remove_ais_targets(Parameters(RemoveTargetsParams { ids: None, all: Some(true) })));
        settle();
        assert!(t.engine().snapshot().sim.ais_targets.is_empty());
    }

    #[test]
    fn scenarios() {
        let t = tools();
        let list = parse(t.list_scenarios());
        assert_eq!(list["builtin"].as_array().unwrap().len(), scenario::BUILTIN.len());
        // The current state round-trips through TOML.
        let mut sc = t.engine().capture_scenario();
        sc.outputs.clear();
        sc.name = "Edited by an agent".into();
        let loaded = parse(t.load_scenario(Parameters(LoadScenarioParams { builtin: None, toml: Some(sc.to_toml().unwrap()) })));
        assert_eq!(loaded["loaded"], "Edited by an agent");
        settle();
        assert_eq!(parse(t.get_status())["scenario"], "Edited by an agent");
        assert!(t.get_scenario().unwrap().contains("Edited by an agent"));

        // No files through MCP.
        let mut with_file = sc.clone();
        with_file.vessel.polar_file = Some("/etc/passwd".into());
        let e = t.load_scenario(Parameters(LoadScenarioParams { builtin: None, toml: Some(with_file.to_toml().unwrap()) })).unwrap_err();
        assert!(e.contains("does not read files"), "{e}");
        let mut with_output = sc.clone();
        with_output.outputs.push(OutputConfig {
            name: "log".into(),
            enabled: true,
            kind: OutputKind::File { path: "/tmp/x.nmea".into(), tag_blocks: true },
            format: DataFormat::Nmea0183,
            filter: vec![],
        });
        let e = t.load_scenario(Parameters(LoadScenarioParams { builtin: None, toml: Some(with_output.to_toml().unwrap()) })).unwrap_err();
        assert!(e.contains("network outputs"), "{e}");
        assert!(t.load_scenario(Parameters(LoadScenarioParams { builtin: Some("nope.toml".into()), toml: None })).is_err());
    }

    #[test]
    fn sensors_outputs_and_data() {
        let t = tools();
        let s = parse(t.set_sensor(Parameters(SetSensorParams { kind: "depth_sounder".into(), config: obj(json!({ "enabled": false })) })));
        assert_eq!(s["enabled"], false);
        settle();
        let sensors = parse(t.list_sensors());
        let depth = sensors["sensors"].as_array().unwrap().iter().find(|x| x["kind"] == "depth_sounder").unwrap().clone();
        assert_eq!(depth["enabled"], false);
        assert!(t.set_sensor(Parameters(SetSensorParams { kind: "radar".into(), config: Map::new() })).is_err());

        // File outputs are refused; a UDP one is added and removed.
        let e = t.add_output(Parameters(AddOutputParams { config: obj(json!({ "name": "f", "type": "file", "path": "/tmp/x" })) })).unwrap_err();
        assert!(e.contains("network outputs"), "{e}");
        let o = parse(t.add_output(Parameters(AddOutputParams { config: obj(json!({ "name": "test", "type": "udp", "target": "127.0.0.1:49999" })) })));
        std::thread::sleep(Duration::from_millis(1500));
        let list = parse(t.list_outputs());
        assert_eq!(list["outputs"].as_array().unwrap().len(), 1);
        let data = parse(t.read_data(Parameters(ReadDataParams { tags: Some(vec!["rmc".into()]), limit: Some(5), ..Default::default() })));
        let lines = data["lines"].as_array().unwrap();
        assert!(!lines.is_empty() && lines.iter().all(|l| l.as_str().unwrap().contains("RMC")), "{data}");
        // With the sounder off, no depth sentence goes out any more.
        let seq = data["next_seq"].as_u64().unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let later = parse(t.read_data(Parameters(ReadDataParams { tags: Some(vec!["DBT".into(), "DPT".into()]), after_seq: Some(seq), ..Default::default() })));
        assert!(later["lines"].as_array().unwrap().is_empty(), "{later}");
        parse(t.remove_output(Parameters(RemoveOutputParams { id: o["id"].as_u64().unwrap() })));
        assert!(parse(t.list_outputs())["outputs"].as_array().unwrap().is_empty());
    }
}
