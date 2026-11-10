//! Own-vessel configuration, state and manoeuvring response.

use serde::{Deserialize, Serialize};

use crate::ais::OwnAis;
use crate::geo::{LatLon, Ne, norm_180, norm_360};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Propulsion {
    /// Engine-driven; speed follows the target speed.
    #[default]
    Motor,
    /// Sailing; speed follows the polar (or the engine, when motoring).
    Sail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VesselConfig {
    pub name: String,
    pub mmsi: u32,
    pub call_sign: String,
    pub propulsion: Propulsion,
    /// Polar file (sailing vessels), relative to the scenario file. Absent = built-in polar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polar_file: Option<String>,
    /// Fraction of polar speed actually achieved (crew, fouling, sea state…).
    pub polar_efficiency: f64,
    /// Overall length, metres.
    pub length_m: f64,
    pub beam_m: f64,
    pub draft_m: f64,
    /// Maximum rate of turn, degrees per minute.
    pub max_rot_dpm: f64,
    /// Time constant for the rate of turn to reach its target, seconds.
    pub rot_time_constant_s: f64,
    /// Time constant for speed through water to reach its target, seconds.
    pub speed_time_constant_s: f64,
    /// Maximum speed under engine, knots.
    pub max_speed_kn: f64,
    /// Rudder angle at full rate of turn, degrees.
    pub max_rudder_deg: f64,
    /// Sailing heel per knot² of apparent wind (beam reach), degrees.
    pub heel_coefficient: f64,
    pub max_heel_deg: f64,
    /// Leeway = K · heel / STW² (degrees, knots).
    pub leeway_coefficient: f64,
    /// Natural roll period, seconds.
    pub roll_period_s: f64,
    pub idle_rpm: f64,
    pub max_rpm: f64,
    /// Own AIS transponder (`!AIVDO`).
    pub ais: OwnAis,
}

impl Default for VesselConfig {
    fn default() -> Self {
        Self {
            name: "SIMULATOR".into(),
            mmsi: 100_000_001, // no country has MID 100: see `ais::SIM_MID`
            call_sign: "SIM01".into(),
            propulsion: Propulsion::Motor,
            polar_file: None,
            polar_efficiency: 1.0,
            length_m: 12.0,
            beam_m: 4.0,
            draft_m: 2.0,
            max_rot_dpm: 180.0,
            rot_time_constant_s: 2.5,
            speed_time_constant_s: 20.0,
            max_speed_kn: 25.0,
            max_rudder_deg: 35.0,
            heel_coefficient: 0.06,
            max_heel_deg: 30.0,
            leeway_coefficient: 8.0,
            roll_period_s: 4.5,
            idle_rpm: 800.0,
            max_rpm: 3000.0,
            ais: OwnAis::default(),
        }
    }
}

/// Instantaneous physical state of the own vessel (true values, no overrides).
#[derive(Debug, Clone, Copy)]
pub struct VesselState {
    pub position: LatLon,
    pub heading: f64,
    /// Rate of turn, degrees per minute (+ = starboard).
    pub rot_dpm: f64,
    /// Speed through water, knots.
    pub stw_kn: f64,
    /// Rudder angle, degrees (+ = starboard); derived from the commanded rate of turn.
    pub rudder_deg: f64,
    /// Steady heel from wind and turning, degrees (+ = starboard down); waves add on top.
    pub heel_deg: f64,
    /// Leeway, degrees (+ = water track to starboard of heading).
    pub leeway_deg: f64,
}

/// Commanded behaviour of the vessel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Steering {
    /// Hold a true heading.
    Heading { heading: f64 },
    /// Hold a true wind angle (+ = wind on the starboard side).
    WindAngle { twa: f64 },
    /// Follow a route; the autopilot corrects for current, and a sailing vessel tacks on the
    /// laylines when the next waypoint is upwind.
    Route {
        waypoints: Vec<LatLon>,
        #[serde(default)]
        active: usize,
        #[serde(default = "default_true")]
        looped: bool,
        #[serde(default = "default_arrival")]
        arrival_radius_m: f64,
    },
}

fn default_true() -> bool {
    true
}
fn default_arrival() -> f64 {
    50.0
}

impl Default for Steering {
    fn default() -> Self {
        Steering::Heading { heading: 0.0 }
    }
}

impl Steering {
    pub fn active_waypoint(&self) -> Option<(usize, LatLon)> {
        match self {
            Steering::Route { waypoints, active, .. } => waypoints.get(*active).map(|w| (*active, *w)),
            _ => None,
        }
    }
}

/// Heading that makes the ground track point along `bearing` given the current (and leeway),
/// or `bearing` itself when the vessel is too slow to compensate.
pub fn course_to_steer(bearing: f64, stw_kn: f64, current: Ne, leeway_deg: f64) -> f64 {
    if stw_kn < 0.1 {
        return bearing;
    }
    let c = current.magnitude();
    let set = current.direction();
    let s = c * (set - bearing).to_radians().sin() / stw_kn;
    if s.abs() >= 1.0 {
        return bearing;
    }
    norm_360(bearing - s.asin().to_degrees() - leeway_deg)
}

impl VesselState {
    /// Advances heading and speed by `dt` seconds towards their targets and moves the vessel
    /// along its water track plus `current` (knots, north/east). Returns the ground velocity.
    pub fn step(&mut self, cfg: &VesselConfig, dt: f64, target_heading: f64, target_stw: f64, current: Ne) -> Ne {
        // Yaw: proportional controller on heading error, limited, with first-order lag.
        let err = norm_180(target_heading - self.heading);
        let wanted_rot = (err * 20.0).clamp(-cfg.max_rot_dpm, cfg.max_rot_dpm);
        let a = (dt / cfg.rot_time_constant_s.max(0.01)).min(1.0);
        self.rot_dpm += (wanted_rot - self.rot_dpm) * a;
        self.rudder_deg = wanted_rot / cfg.max_rot_dpm.max(1.0) * cfg.max_rudder_deg;
        self.heading = norm_360(self.heading + self.rot_dpm * dt / 60.0);

        // Speed through water: first-order response.
        let a = (dt / cfg.speed_time_constant_s.max(0.01)).min(1.0);
        self.stw_kn += (target_stw.max(0.0) - self.stw_kn) * a;

        let ground = self.water_velocity() + current;
        let mps = crate::geo::MPS_PER_KNOT * dt;
        self.position = self.position.offset_ne(ground.n * mps, ground.e * mps);
        ground
    }

    /// Velocity through the water (along heading + leeway), knots.
    pub fn water_velocity(&self) -> Ne {
        Ne::from_polar(self.stw_kn, self.heading + self.leeway_deg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn course_to_steer_cancels_cross_current() {
        let h = course_to_steer(0.0, 6.0, Ne::from_polar(2.0, 90.0), 0.0);
        let ground = Ne::from_polar(6.0, h) + Ne::from_polar(2.0, 90.0);
        assert!(ground.e.abs() < 1e-9);
        assert!(ground.n > 0.0);
        assert!(h > 300.0);
    }

    #[test]
    fn course_to_steer_cancels_leeway() {
        // Leeway of 5° to port: steer 5° to starboard of the bearing.
        let h = course_to_steer(0.0, 6.0, Ne::default(), -5.0);
        assert!((h - 5.0).abs() < 1e-9);
    }

    #[test]
    fn converges_on_heading_and_speed() {
        let cfg = VesselConfig::default();
        let mut s = VesselState {
            position: LatLon::new(0.0, 0.0),
            heading: 0.0,
            rot_dpm: 0.0,
            stw_kn: 0.0,
            rudder_deg: 0.0,
            heel_deg: 0.0,
            leeway_deg: 0.0,
        };
        for _ in 0..3000 {
            s.step(&cfg, 0.1, 90.0, 8.0, Ne::default());
        }
        assert!(norm_180(s.heading - 90.0).abs() < 0.5, "heading {}", s.heading);
        assert!((s.stw_kn - 8.0).abs() < 0.1);
        assert!(s.position.lon > 0.0);
    }
}
