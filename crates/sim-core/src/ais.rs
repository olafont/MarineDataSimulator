//! AIS: other vessels, aids to navigation and base stations, their motion, and the messages
//! they transmit (protocol-agnostic data; the bit-level encoding lives in `protocols::ais`).
//!
//! Message numbers and reporting intervals follow ITU-R M.1371-5.

use serde::{Deserialize, Serialize};

use crate::geo::{LatLon, MPS_PER_KNOT, Ne, norm_180, norm_360};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AisClass {
    /// SOLAS Class A transponder: messages 1 and 5.
    #[default]
    A,
    /// Class B "CS" transponder: messages 18 and 24A/B (optionally 19).
    B,
    /// Shore base station: message 4.
    BaseStation,
    /// Aid to navigation (buoy, beacon, virtual mark): message 21.
    Aton,
    /// Search-and-rescue transmitter: message 1 with status 14, plus message 14 "SART ACTIVE".
    Sart,
}

impl AisClass {
    pub const ALL: &'static [AisClass] =
        &[AisClass::A, AisClass::B, AisClass::BaseStation, AisClass::Aton, AisClass::Sart];

    pub fn label(self) -> &'static str {
        match self {
            AisClass::A => "Class A",
            AisClass::B => "Class B",
            AisClass::BaseStation => "Base station",
            AisClass::Aton => "AtoN",
            AisClass::Sart => "SART",
        }
    }

    /// Whether the station moves (vessels and SARTs).
    pub fn is_mobile(self) -> bool {
        matches!(self, AisClass::A | AisClass::B | AisClass::Sart)
    }

    /// The MMSI of the `n`th target created in the app. Each station type keeps its MMSI format
    /// (00MIDxxxx base station, 99MIDxxxx AtoN, 970xxyyyy SART), but with "100" in place of a
    /// country code (MID): no country has it, so the numbers cannot belong to a real station.
    pub fn default_mmsi(self, n: u32) -> u32 {
        match self {
            AisClass::A | AisClass::B => SIM_MID * 1_000_000 + 100 + n,
            AisClass::BaseStation => SIM_MID * 10_000 + 9000 + n % 1000,
            AisClass::Aton => 990_000_000 + SIM_MID * 10_000 + 9000 + n % 1000,
            AisClass::Sart => 970_000_000 + n % 1_000_000,
        }
    }
}

/// The "country code" in simulated MMSIs. Real ones (MIDs) run from 201 to 775; 100 belongs to
/// no country, so a simulated station is never mistaken for a real one.
pub const SIM_MID: u32 = 100;

/// Navigational status (message 1/2/3).
pub const NAV_STATUS: [&str; 16] = [
    "Under way using engine",
    "At anchor",
    "Not under command",
    "Restricted manoeuvrability",
    "Constrained by draught",
    "Moored",
    "Aground",
    "Engaged in fishing",
    "Under way sailing",
    "Reserved (HSC)",
    "Reserved (WIG)",
    "Power-driven towing astern",
    "Power-driven pushing ahead",
    "Reserved",
    "AIS-SART active",
    "Not defined",
];

/// Common ship and cargo types (message 5/19/24B). Any value 0–99 is accepted.
pub const SHIP_TYPES: &[(u8, &str)] = &[
    (0, "Not available"),
    (30, "Fishing"),
    (31, "Towing"),
    (33, "Dredging"),
    (35, "Military"),
    (36, "Sailing"),
    (37, "Pleasure craft"),
    (40, "High-speed craft"),
    (50, "Pilot vessel"),
    (51, "Search and rescue"),
    (52, "Tug"),
    (55, "Law enforcement"),
    (60, "Passenger"),
    (69, "Passenger"),
    (70, "Cargo"),
    (71, "Cargo, hazardous A"),
    (79, "Cargo"),
    (80, "Tanker"),
    (81, "Tanker, hazardous A"),
    (89, "Tanker"),
    (90, "Other"),
];

pub fn ship_type_label(t: u8) -> &'static str {
    match t {
        20..=29 => "Wing in ground",
        30 => "Fishing",
        31 | 32 => "Towing",
        33 => "Dredging",
        34 => "Diving ops",
        35 => "Military",
        36 => "Sailing",
        37 => "Pleasure craft",
        40..=49 => "High-speed craft",
        50 => "Pilot vessel",
        51 => "Search and rescue",
        52 => "Tug",
        53 => "Port tender",
        55 => "Law enforcement",
        58 => "Medical transport",
        60..=69 => "Passenger",
        70..=79 => "Cargo",
        80..=89 => "Tanker",
        90..=99 => "Other",
        _ => "Not available",
    }
}

/// Electronic position fixing device (EPFD).
pub const EPFD: [&str; 9] = [
    "Undefined",
    "GPS",
    "GLONASS",
    "GPS/GLONASS",
    "Loran-C",
    "Chayka",
    "Integrated navigation",
    "Surveyed",
    "Galileo",
];

/// Aid-to-navigation types (message 21), first entries.
pub const ATON_TYPES: &[(u8, &str)] = &[
    (0, "Not specified"),
    (1, "Reference point"),
    (2, "RACON"),
    (3, "Fixed structure off shore"),
    (5, "Light, without sectors"),
    (9, "Beacon, cardinal N"),
    (13, "Beacon, port hand"),
    (14, "Beacon, starboard hand"),
    (20, "Cardinal mark N"),
    (21, "Cardinal mark E"),
    (22, "Cardinal mark S"),
    (23, "Cardinal mark W"),
    (24, "Port hand mark"),
    (25, "Starboard hand mark"),
    (28, "Isolated danger"),
    (29, "Safe water"),
    (30, "Special mark"),
    (31, "Light vessel / LANBY / rig"),
];

/// Static and voyage data. Antenna offsets in metres from the GNSS antenna.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisStatic {
    pub imo: u32,
    pub ship_type: u8,
    pub to_bow: u16,
    pub to_stern: u16,
    pub to_port: u16,
    pub to_starboard: u16,
    pub draught_m: f64,
    pub destination: String,
    /// Estimated time of arrival, `MM-DD HH:MM` UTC; empty = not available.
    pub eta: String,
    pub epfd: u8,
    /// Data terminal equipment ready (false = available, as transmitted bit 0).
    pub dte_unavailable: bool,
    /// Class B vendor ID (message 24B), up to 7 characters.
    pub vendor_id: String,
}

impl Default for AisStatic {
    fn default() -> Self {
        Self {
            imo: 0,
            ship_type: 0,
            to_bow: 0,
            to_stern: 0,
            to_port: 0,
            to_starboard: 0,
            draught_m: 0.0,
            destination: String::new(),
            eta: String::new(),
            epfd: 1,
            dte_unavailable: false,
            vendor_id: String::new(),
        }
    }
}

impl AisStatic {
    /// Antenna offsets for a hull of `length` × `beam` with the antenna 1/3 from the stern,
    /// on the centreline.
    pub fn with_hull(mut self, length: f64, beam: f64) -> Self {
        let stern = (length / 3.0).round().max(0.0);
        self.to_bow = (length - stern).round().max(0.0) as u16;
        self.to_stern = stern as u16;
        self.to_port = (beam / 2.0).round() as u16;
        self.to_starboard = (beam - (beam / 2.0).round()).round().max(0.0) as u16;
        self
    }

    pub fn length(&self) -> f64 {
        (self.to_bow + self.to_stern) as f64
    }

    pub fn beam(&self) -> f64 {
        (self.to_port + self.to_starboard) as f64
    }

    /// `(month, day, hour, minute)` with AIS "not available" values (0, 0, 24, 60).
    pub fn eta_fields(&self) -> (u8, u8, u8, u8) {
        let na = (0, 0, 24, 60);
        let s = self.eta.trim();
        if s.is_empty() {
            return na;
        }
        let parse = || -> Option<(u8, u8, u8, u8)> {
            let (date, time) = s.split_once(' ')?;
            let (mo, d) = date.split_once('-')?;
            let (h, mi) = time.split_once(':')?;
            Some((mo.parse().ok()?, d.parse().ok()?, h.parse().ok()?, mi.parse().ok()?))
        };
        match parse() {
            Some((mo, d, h, mi)) if mo <= 12 && d <= 31 && h <= 24 && mi <= 60 => (mo, d, h, mi),
            _ => na,
        }
    }
}

/// Flags carried in position reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisFlags {
    /// Position accuracy: true = high (< 10 m, DGNSS).
    pub position_accuracy: bool,
    pub raim: bool,
    /// Class B: true = carrier-sense (CS) unit, false = SOTDMA unit.
    pub cs_unit: bool,
    /// Class B: unit has a display.
    pub display: bool,
    /// Class B: unit has DSC.
    pub dsc: bool,
    /// Class B: can use the whole marine band.
    pub band: bool,
    /// Class B: accepts channel management (message 22).
    pub msg22: bool,
    /// Station operates in assigned mode.
    pub assigned: bool,
}

impl Default for AisFlags {
    fn default() -> Self {
        Self {
            position_accuracy: false,
            raim: false,
            cs_unit: true,
            display: false,
            dsc: true,
            band: true,
            msg22: true,
            assigned: false,
        }
    }
}

/// Reporting intervals, seconds of simulated time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisIntervals {
    /// Position reports; absent = automatic from class, speed and status (ITU-R M.1371).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_s: Option<f64>,
    /// Static and voyage data (messages 5, 24A/B, 19).
    pub static_s: f64,
    /// Safety-related text (message 14), when a text is set.
    pub safety_text_s: f64,
}

impl Default for AisIntervals {
    fn default() -> Self {
        Self { position_s: None, static_s: 360.0, safety_text_s: 240.0 }
    }
}

/// Automatic position-report interval (ITU-R M.1371-5, Table 1 and Annex 2 §4.3).
pub fn auto_position_interval(class: AisClass, sog_kn: f64, nav_status: u8, rot_dpm: f64) -> f64 {
    // "Changing course": more than ~5° in 30 s.
    let turning = rot_dpm.abs() > 10.0;
    match class {
        AisClass::A => {
            if matches!(nav_status, 1 | 5) && sog_kn <= 3.0 {
                180.0
            } else if sog_kn <= 14.0 {
                if turning { 10.0 / 3.0 } else { 10.0 }
            } else if sog_kn <= 23.0 {
                if turning { 2.0 } else { 6.0 }
            } else {
                2.0
            }
        }
        AisClass::B => {
            if sog_kn <= 2.0 {
                180.0
            } else {
                30.0
            }
        }
        AisClass::BaseStation => 10.0,
        AisClass::Aton => 180.0,
        AisClass::Sart => 60.0,
    }
}

/// How a target moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TargetMotion {
    /// Holds a course and speed over ground.
    Course { cog: f64, sog_kn: f64 },
    /// Follows waypoints at a speed over ground; stops at the end unless looped.
    Route {
        waypoints: Vec<LatLon>,
        sog_kn: f64,
        #[serde(default)]
        looped: bool,
        #[serde(default)]
        active: usize,
    },
    /// Does not move (anchored, moored, base stations, AtoNs). Anchored vessels swing to the wind.
    Stationary,
}

impl TargetMotion {
    pub fn label(&self) -> &'static str {
        match self {
            TargetMotion::Course { .. } => "Course",
            TargetMotion::Route { .. } => "Route",
            TargetMotion::Stationary => "Stationary",
        }
    }

    pub fn speed(&self) -> f64 {
        match self {
            TargetMotion::Course { sog_kn, .. } | TargetMotion::Route { sog_kn, .. } => *sog_kn,
            TargetMotion::Stationary => 0.0,
        }
    }
}

/// A simulated AIS station, as configured by the user (or a scenario file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisTargetConfig {
    pub mmsi: u32,
    pub name: String,
    pub call_sign: String,
    pub class: AisClass,
    pub enabled: bool,
    pub position: LatLon,
    /// Initial heading (Course mode starts on its course; Route on the first leg).
    pub heading: f64,
    pub nav_status: u8,
    pub motion: TargetMotion,
    /// Maximum rate of turn, degrees per minute.
    pub max_rot_dpm: f64,
    #[serde(rename = "static")]
    pub statics: AisStatic,
    pub flags: AisFlags,
    pub intervals: AisIntervals,
    /// Message 14 text; empty = none. SARTs send "SART ACTIVE" when empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub safety_text: String,
    /// Class B: also send message 19 (extended position report) with the static data.
    pub send_msg19: bool,
    /// AtoN only: type (message 21).
    pub aton_type: u8,
    /// AtoN only: virtual (no physical mark).
    pub virtual_aton: bool,
    /// AtoN only: the mark has drifted this far from its charted position, metres.
    pub off_position_m: f64,
}

impl Default for AisTargetConfig {
    fn default() -> Self {
        Self {
            mmsi: 100_000_101,
            name: "TARGET".into(),
            call_sign: String::new(),
            class: AisClass::A,
            enabled: true,
            position: LatLon::new(0.0, 0.0),
            heading: 0.0,
            nav_status: 0,
            motion: TargetMotion::Course { cog: 0.0, sog_kn: 10.0 },
            max_rot_dpm: 60.0,
            statics: AisStatic { ship_type: 70, ..AisStatic::default() }.with_hull(100.0, 18.0),
            flags: AisFlags::default(),
            intervals: AisIntervals::default(),
            safety_text: String::new(),
            send_msg19: false,
            aton_type: 0,
            virtual_aton: false,
            off_position_m: 0.0,
        }
    }
}

impl AisTargetConfig {
    /// A sensible new station of `class` at `position`.
    pub fn new(class: AisClass, n: u32, position: LatLon) -> Self {
        let base = AisTargetConfig { class, position, mmsi: class.default_mmsi(n), ..Default::default() };
        match class {
            AisClass::A => AisTargetConfig { name: format!("CARGO {n}"), call_sign: format!("WCA{n:04}"), ..base },
            AisClass::B => AisTargetConfig {
                name: format!("YACHT {n}"),
                call_sign: format!("WYB{n:04}"),
                nav_status: 15,
                motion: TargetMotion::Course { cog: 0.0, sog_kn: 6.0 },
                max_rot_dpm: 120.0,
                statics: AisStatic { ship_type: 36, vendor_id: "SIMAIS".into(), ..AisStatic::default() }
                    .with_hull(12.0, 4.0),
                ..base
            },
            AisClass::BaseStation => AisTargetConfig {
                name: format!("BASE {n}"),
                motion: TargetMotion::Stationary,
                statics: AisStatic { epfd: 7, ..AisStatic::default() },
                flags: AisFlags { position_accuracy: true, ..AisFlags::default() },
                ..base
            },
            AisClass::Aton => AisTargetConfig {
                name: format!("BUOY {n}"),
                motion: TargetMotion::Stationary,
                aton_type: 30,
                statics: AisStatic { epfd: 7, ..AisStatic::default() }.with_hull(2.0, 2.0),
                ..base
            },
            AisClass::Sart => AisTargetConfig {
                name: "SART ACTIVE".into(),
                nav_status: 14,
                motion: TargetMotion::Course { cog: 0.0, sog_kn: 0.5 },
                statics: AisStatic::default(),
                ..base
            },
        }
    }
}

/// Instantaneous motion of a target (true values).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TargetState {
    pub position: LatLon,
    pub heading: f64,
    pub cog: f64,
    pub sog_kn: f64,
    pub rot_dpm: f64,
    /// Route finished (non-looped): the target stops.
    pub route_complete: bool,
}

/// A running AIS station: configuration plus state.
#[derive(Debug, Clone, PartialEq)]
pub struct AisTarget {
    /// Stable identifier for commands (the MMSI can be edited).
    pub id: u64,
    pub cfg: AisTargetConfig,
    pub state: TargetState,
}

const TARGET_SPEED_TC_S: f64 = 30.0;

impl AisTarget {
    pub fn new(id: u64, cfg: AisTargetConfig) -> Self {
        let mut heading = cfg.heading;
        if let TargetMotion::Course { cog, .. } = cfg.motion {
            heading = cog;
        }
        if let TargetMotion::Route { waypoints, active, .. } = &cfg.motion
            && let Some(wp) = waypoints.get(*active).or(waypoints.first())
        {
            heading = cfg.position.bearing_to(wp);
        }
        let state = TargetState {
            position: cfg.position,
            heading: norm_360(heading),
            cog: norm_360(heading),
            sog_kn: if cfg.class.is_mobile() { cfg.motion.speed() } else { 0.0 },
            rot_dpm: 0.0,
            route_complete: false,
        };
        Self { id, cfg, state }
    }

    /// Replaces the configuration, keeping position and motion (and the route progress when
    /// the waypoints are unchanged).
    pub fn reconfigure(&mut self, mut cfg: AisTargetConfig) {
        if let (TargetMotion::Route { waypoints: old, active: old_active, .. }, TargetMotion::Route { waypoints, active, .. }) =
            (&self.cfg.motion, &mut cfg.motion)
            && old == waypoints
        {
            *active = *old_active;
        } else if cfg.motion != self.cfg.motion {
            self.state.route_complete = false;
        }
        if cfg.position != self.cfg.position {
            self.state.position = cfg.position;
        }
        if !cfg.class.is_mobile() {
            self.state.sog_kn = 0.0;
            self.state.rot_dpm = 0.0;
        }
        self.cfg = cfg;
    }

    /// The configuration with the current position, heading and route progress written back
    /// (for saving a scenario).
    pub fn captured(&self) -> AisTargetConfig {
        let mut c = self.cfg.clone();
        c.position = self.state.position;
        c.heading = (self.state.heading * 10.0).round() / 10.0;
        c
    }

    /// Advances the target by `dt` seconds. `wind_from` turns anchored vessels head to wind.
    pub fn step(&mut self, dt: f64, wind_from: f64) {
        let s = &mut self.state;
        let cfg = &mut self.cfg;
        if !cfg.class.is_mobile() {
            return;
        }
        let (desired_heading, desired_speed) = match &mut cfg.motion {
            TargetMotion::Course { cog, sog_kn } => (*cog, *sog_kn),
            TargetMotion::Stationary => {
                let h = if cfg.nav_status == 1 { wind_from } else { s.heading };
                (h, 0.0)
            }
            TargetMotion::Route { waypoints, sog_kn, looped, active } => {
                if waypoints.is_empty() || s.route_complete {
                    (s.heading, 0.0)
                } else {
                    if *active >= waypoints.len() {
                        *active = 0;
                    }
                    // Arrive within a few ship lengths or 20 s of travel, whichever is larger.
                    let radius = (2.0 * cfg.statics.length()).max(*sog_kn * MPS_PER_KNOT * 20.0).max(50.0);
                    if s.position.distance_to(&waypoints[*active]) < radius {
                        if *active + 1 < waypoints.len() {
                            *active += 1;
                        } else if *looped {
                            *active = 0;
                        } else {
                            s.route_complete = true;
                        }
                    }
                    if s.route_complete {
                        (s.heading, 0.0)
                    } else {
                        (s.position.bearing_to(&waypoints[*active]), *sog_kn)
                    }
                }
            }
        };

        // Turn towards the desired heading at a limited rate (anchored vessels swing slowly).
        let max_rot = if desired_speed <= 0.0 { cfg.max_rot_dpm.min(10.0) } else { cfg.max_rot_dpm.max(1.0) };
        let err = norm_180(desired_heading - s.heading);
        let wanted = (err * 6.0).clamp(-max_rot, max_rot);
        let a = (dt / 5.0).min(1.0);
        s.rot_dpm += (wanted - s.rot_dpm) * a;
        if err.abs() < 0.05 && desired_speed > 0.0 {
            s.rot_dpm = 0.0;
            s.heading = norm_360(desired_heading);
        } else {
            s.heading = norm_360(s.heading + s.rot_dpm * dt / 60.0);
        }

        let a = (dt / TARGET_SPEED_TC_S).min(1.0);
        s.sog_kn += (desired_speed.max(0.0) - s.sog_kn) * a;
        if s.sog_kn < 0.01 && desired_speed <= 0.0 {
            s.sog_kn = 0.0;
        }
        s.cog = s.heading;
        let v = Ne::from_polar(s.sog_kn, s.heading);
        let m = MPS_PER_KNOT * dt;
        s.position = s.position.offset_ne(v.n * m, v.e * m);
    }

    pub fn velocity(&self) -> Ne {
        Ne::from_polar(self.state.sog_kn, self.state.cog)
    }

    /// Effective position-report interval, seconds.
    pub fn position_interval(&self) -> f64 {
        self.cfg.intervals.position_s.filter(|v| *v > 0.0).unwrap_or_else(|| {
            auto_position_interval(self.cfg.class, self.state.sog_kn, self.cfg.nav_status, self.state.rot_dpm)
        })
    }

    /// Messages due at a position-report time.
    pub fn position_messages(&self, time_unix: f64) -> Vec<AisMessage> {
        let c = &self.cfg;
        let s = &self.state;
        let second = utc_second(time_unix);
        let common = PositionCommon {
            mmsi: c.mmsi,
            lat: s.position.lat,
            lon: s.position.lon,
            sog_kn: Some(s.sog_kn),
            cog: Some(s.cog),
            heading: Some(s.heading),
            second,
            accuracy: c.flags.position_accuracy,
            raim: c.flags.raim,
        };
        match c.class {
            AisClass::A | AisClass::Sart => vec![AisMessage::PositionA {
                msg_type: if c.flags.assigned { 2 } else { 1 },
                common: if c.class == AisClass::Sart {
                    PositionCommon { heading: None, ..common }
                } else {
                    common
                },
                nav_status: if c.class == AisClass::Sart { 14 } else { c.nav_status },
                rot_dpm: if c.class == AisClass::A { Some(s.rot_dpm) } else { None },
                time_unix,
            }],
            AisClass::B => vec![AisMessage::PositionB { common, flags: c.flags }],
            AisClass::BaseStation => vec![AisMessage::BaseStation {
                mmsi: c.mmsi,
                time_unix,
                lat: s.position.lat,
                lon: s.position.lon,
                accuracy: c.flags.position_accuracy,
                epfd: c.statics.epfd,
                raim: c.flags.raim,
            }],
            AisClass::Aton => vec![AisMessage::Aton {
                mmsi: c.mmsi,
                aton_type: c.aton_type,
                name: c.name.clone(),
                lat: s.position.lat,
                lon: s.position.lon,
                accuracy: c.flags.position_accuracy,
                dims: dims(&c.statics),
                epfd: c.statics.epfd,
                second,
                off_position: c.off_position_m > 0.0,
                raim: c.flags.raim,
                virtual_aton: c.virtual_aton,
                assigned: c.flags.assigned,
            }],
        }
    }

    /// Static/voyage messages (none for base stations and AtoNs, whose data is in their
    /// periodic report).
    pub fn static_messages(&self) -> Vec<AisMessage> {
        let c = &self.cfg;
        match c.class {
            AisClass::A => vec![static_a(c.mmsi, &c.name, &c.call_sign, &c.statics)],
            AisClass::B => {
                let mut v = static_b(c.mmsi, &c.name, &c.call_sign, &c.statics);
                if c.send_msg19 {
                    v.push(AisMessage::ExtendedB {
                        common: PositionCommon {
                            mmsi: c.mmsi,
                            lat: self.state.position.lat,
                            lon: self.state.position.lon,
                            sog_kn: Some(self.state.sog_kn),
                            cog: Some(self.state.cog),
                            heading: Some(self.state.heading),
                            second: 60,
                            accuracy: c.flags.position_accuracy,
                            raim: c.flags.raim,
                        },
                        name: c.name.clone(),
                        ship_type: c.statics.ship_type,
                        dims: dims(&c.statics),
                        epfd: c.statics.epfd,
                        dte_unavailable: c.statics.dte_unavailable,
                        assigned: c.flags.assigned,
                    });
                }
                v
            }
            AisClass::BaseStation | AisClass::Aton | AisClass::Sart => vec![],
        }
    }

    /// The safety-related text, if this target sends one.
    pub fn safety_text(&self) -> Option<AisMessage> {
        let text = if self.cfg.safety_text.is_empty() && self.cfg.class == AisClass::Sart {
            "SART ACTIVE".to_string()
        } else {
            self.cfg.safety_text.clone()
        };
        (!text.is_empty()).then_some(AisMessage::SafetyText { mmsi: self.cfg.mmsi, text })
    }
}

fn dims(s: &AisStatic) -> [u16; 4] {
    [s.to_bow, s.to_stern, s.to_port, s.to_starboard]
}

pub fn static_a(mmsi: u32, name: &str, call_sign: &str, s: &AisStatic) -> AisMessage {
    AisMessage::StaticA {
        mmsi,
        imo: s.imo,
        call_sign: call_sign.into(),
        name: name.into(),
        ship_type: s.ship_type,
        dims: dims(s),
        epfd: s.epfd,
        eta: s.eta_fields(),
        draught_m: s.draught_m,
        destination: s.destination.clone(),
        dte_unavailable: s.dte_unavailable,
    }
}

pub fn static_b(mmsi: u32, name: &str, call_sign: &str, s: &AisStatic) -> Vec<AisMessage> {
    vec![
        AisMessage::StaticB24A { mmsi, name: name.into() },
        AisMessage::StaticB24B {
            mmsi,
            ship_type: s.ship_type,
            vendor_id: s.vendor_id.clone(),
            call_sign: call_sign.into(),
            dims: dims(s),
        },
    ]
}

/// UTC second of the timestamp (the "time stamp" field of position reports).
pub fn utc_second(time_unix: f64) -> u8 {
    (time_unix.floor().rem_euclid(60.0)) as u8
}

/// Own ship's AIS transponder (reported as `!AIVDO`). Name, MMSI and call sign come from the
/// vessel configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OwnAis {
    pub enabled: bool,
    /// `a` or `b`.
    pub class: AisClass,
    pub nav_status: u8,
    #[serde(rename = "static")]
    pub statics: AisStatic,
    pub flags: AisFlags,
    pub intervals: AisIntervals,
}

impl Default for OwnAis {
    fn default() -> Self {
        Self {
            enabled: true,
            class: AisClass::B,
            nav_status: 15,
            statics: AisStatic { ship_type: 36, vendor_id: "SIMAIS".into(), ..AisStatic::default() }.with_hull(12.0, 4.0),
            flags: AisFlags::default(),
            intervals: AisIntervals::default(),
        }
    }
}

/// Fields shared by messages 1/2/3, 18 and 19. Angles in degrees, speed in knots;
/// `None` = "not available".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionCommon {
    pub mmsi: u32,
    pub lat: f64,
    pub lon: f64,
    pub sog_kn: Option<f64>,
    pub cog: Option<f64>,
    pub heading: Option<f64>,
    /// UTC second 0–59; 60 = not available.
    pub second: u8,
    pub accuracy: bool,
    pub raim: bool,
}

/// An AIS message, protocol-agnostic.
#[derive(Debug, Clone, PartialEq)]
pub enum AisMessage {
    /// Messages 1, 2, 3.
    PositionA { msg_type: u8, common: PositionCommon, nav_status: u8, rot_dpm: Option<f64>, time_unix: f64 },
    /// Message 4.
    BaseStation { mmsi: u32, time_unix: f64, lat: f64, lon: f64, accuracy: bool, epfd: u8, raim: bool },
    /// Message 5.
    StaticA {
        mmsi: u32,
        imo: u32,
        call_sign: String,
        name: String,
        ship_type: u8,
        /// To bow, stern, port, starboard (m).
        dims: [u16; 4],
        epfd: u8,
        /// Month, day, hour, minute.
        eta: (u8, u8, u8, u8),
        draught_m: f64,
        destination: String,
        dte_unavailable: bool,
    },
    /// Message 14.
    SafetyText { mmsi: u32, text: String },
    /// Message 18.
    PositionB { common: PositionCommon, flags: AisFlags },
    /// Message 19.
    ExtendedB {
        common: PositionCommon,
        name: String,
        ship_type: u8,
        dims: [u16; 4],
        epfd: u8,
        dte_unavailable: bool,
        assigned: bool,
    },
    /// Message 21.
    Aton {
        mmsi: u32,
        aton_type: u8,
        name: String,
        lat: f64,
        lon: f64,
        accuracy: bool,
        dims: [u16; 4],
        epfd: u8,
        second: u8,
        off_position: bool,
        raim: bool,
        virtual_aton: bool,
        assigned: bool,
    },
    /// Message 24 part A.
    StaticB24A { mmsi: u32, name: String },
    /// Message 24 part B.
    StaticB24B { mmsi: u32, ship_type: u8, vendor_id: String, call_sign: String, dims: [u16; 4] },
}

impl AisMessage {
    pub fn msg_type(&self) -> u8 {
        match self {
            AisMessage::PositionA { msg_type, .. } => *msg_type,
            AisMessage::BaseStation { .. } => 4,
            AisMessage::StaticA { .. } => 5,
            AisMessage::SafetyText { .. } => 14,
            AisMessage::PositionB { .. } => 18,
            AisMessage::ExtendedB { .. } => 19,
            AisMessage::Aton { .. } => 21,
            AisMessage::StaticB24A { .. } | AisMessage::StaticB24B { .. } => 24,
        }
    }

    pub fn mmsi(&self) -> u32 {
        match self {
            AisMessage::PositionA { common, .. }
            | AisMessage::PositionB { common, .. }
            | AisMessage::ExtendedB { common, .. } => common.mmsi,
            AisMessage::BaseStation { mmsi, .. }
            | AisMessage::StaticA { mmsi, .. }
            | AisMessage::SafetyText { mmsi, .. }
            | AisMessage::Aton { mmsi, .. }
            | AisMessage::StaticB24A { mmsi, .. }
            | AisMessage::StaticB24B { mmsi, .. } => *mmsi,
        }
    }
}

/// Closest point of approach between two constant-velocity tracks.
/// Returns `(cpa_nm, tcpa_min)`; TCPA is negative when the closest point is in the past.
pub fn cpa_tcpa(own: LatLon, own_vel: Ne, target: LatLon, target_vel: Ne) -> (f64, f64) {
    // Local flat frame around own ship, nautical miles.
    let d = own.distance_to(&target) / 1852.0;
    let b = own.bearing_to(&target);
    let p = Ne::from_polar(d, b);
    let v = target_vel - own_vel; // knots = nm/h
    let v2 = v.n * v.n + v.e * v.e;
    if v2 < 1e-9 {
        return (d, 0.0);
    }
    let t_h = -(p.n * v.n + p.e * v.e) / v2;
    let c = Ne { n: p.n + v.n * t_h, e: p.e + v.e * t_h };
    (c.magnitude(), t_h * 60.0)
}

/// Start position and course for a target that will hit `own` (moving at `own_vel`) in
/// `minutes`, approaching from true bearing `from_bearing` (as seen from the collision point)
/// at `sog_kn`.
pub fn collision_course(own: LatLon, own_vel: Ne, minutes: f64, from_bearing: f64, sog_kn: f64) -> (LatLon, f64) {
    let t_h = minutes / 60.0;
    let meet = own.offset_ne(own_vel.n * t_h * 1852.0, own_vel.e * t_h * 1852.0);
    let start = meet.destination(from_bearing, sog_kn * t_h * 1852.0);
    (start, norm_360(from_bearing + 180.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_intervals() {
        assert_eq!(auto_position_interval(AisClass::A, 10.0, 0, 0.0), 10.0);
        assert_eq!(auto_position_interval(AisClass::A, 10.0, 0, 30.0), 10.0 / 3.0);
        assert_eq!(auto_position_interval(AisClass::A, 18.0, 0, 0.0), 6.0);
        assert_eq!(auto_position_interval(AisClass::A, 30.0, 0, 0.0), 2.0);
        assert_eq!(auto_position_interval(AisClass::A, 0.1, 1, 0.0), 180.0);
        assert_eq!(auto_position_interval(AisClass::B, 5.0, 0, 0.0), 30.0);
        assert_eq!(auto_position_interval(AisClass::B, 1.0, 0, 0.0), 180.0);
    }

    #[test]
    fn cpa_of_head_on_and_crossing() {
        let own = LatLon::new(0.0, 0.0);
        // Target 2 nm north coming south at 10 kn, own ship still: CPA 0 in 12 min.
        let t = own.destination(0.0, 2.0 * 1852.0);
        let (cpa, tcpa) = cpa_tcpa(own, Ne::default(), t, Ne::from_polar(10.0, 180.0));
        assert!(cpa < 0.01, "{cpa}");
        assert!((tcpa - 12.0).abs() < 0.1, "{tcpa}");
        // Same but passing 1 nm east.
        let t = own.offset_ne(2.0 * 1852.0, 1852.0);
        let (cpa, _) = cpa_tcpa(own, Ne::default(), t, Ne::from_polar(10.0, 180.0));
        assert!((cpa - 1.0).abs() < 0.01, "{cpa}");
    }

    #[test]
    fn collision_course_meets_own_ship() {
        let own = LatLon::new(28.4, -80.55);
        let own_vel = Ne::from_polar(6.0, 200.0);
        let (start, cog) = collision_course(own, own_vel, 10.0, 270.0, 12.0);
        let (cpa, tcpa) = cpa_tcpa(own, own_vel, start, Ne::from_polar(12.0, cog));
        assert!(cpa < 0.02, "cpa {cpa}");
        assert!((tcpa - 10.0).abs() < 0.2, "tcpa {tcpa}");
    }

    #[test]
    fn route_target_follows_waypoints_and_stops() {
        let start = LatLon::new(0.0, 0.0);
        let wps = vec![start.destination(90.0, 2000.0), start.destination(0.0, 2000.0)];
        let cfg = AisTargetConfig {
            position: start,
            motion: TargetMotion::Route { waypoints: wps.clone(), sog_kn: 12.0, looped: false, active: 0 },
            ..AisTargetConfig::new(AisClass::A, 1, start)
        };
        let mut t = AisTarget::new(1, cfg);
        assert!((t.state.heading - 90.0).abs() < 0.1);
        for _ in 0..20_000 {
            t.step(0.1, 0.0);
        }
        assert!(t.state.route_complete);
        assert!(t.state.sog_kn < 0.1);
        assert!(t.state.position.distance_to(&wps[1]) < 400.0, "{}", t.state.position.distance_to(&wps[1]));
    }

    #[test]
    fn course_target_moves_at_speed() {
        let start = LatLon::new(0.0, 0.0);
        let cfg = AisTargetConfig {
            motion: TargetMotion::Course { cog: 45.0, sog_kn: 10.0 },
            ..AisTargetConfig::new(AisClass::A, 1, start)
        };
        let mut t = AisTarget::new(1, cfg);
        for _ in 0..3600 {
            t.step(1.0, 0.0);
        }
        let d = start.distance_to(&t.state.position) / 1852.0;
        assert!((d - 10.0).abs() < 0.05, "{d}");
        assert!((start.bearing_to(&t.state.position) - 45.0).abs() < 0.2);
    }

    #[test]
    fn anchored_vessel_swings_to_wind() {
        let start = LatLon::new(0.0, 0.0);
        let cfg = AisTargetConfig { nav_status: 1, motion: TargetMotion::Stationary, ..AisTargetConfig::new(AisClass::A, 1, start) };
        let mut t = AisTarget::new(1, cfg);
        // Swings at up to 10°/min: 120° takes a bit more than 12 minutes.
        for _ in 0..9000 {
            t.step(0.1, 120.0);
        }
        assert!(norm_180(t.state.heading - 120.0).abs() < 2.0, "{}", t.state.heading);
        assert_eq!(t.state.position, start);
    }

    #[test]
    fn eta_parsing() {
        let s = AisStatic { eta: "06-21 14:30".into(), ..AisStatic::default() };
        assert_eq!(s.eta_fields(), (6, 21, 14, 30));
        let s = AisStatic { eta: "garbage".into(), ..AisStatic::default() };
        assert_eq!(s.eta_fields(), (0, 0, 24, 60));
    }
}
