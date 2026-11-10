//! NMEA 2000: PGN payloads from measurements, fast-packet framing, and the text formats that
//! gateways use over TCP, UDP and serial links:
//!
//! - **Yacht Devices RAW** (YDWG-02, YDEN-02): one CAN frame per line,
//!   `hh:mm:ss.sss R 09F80101 3F 9F DC FF FF FF FF FF`.
//! - **Actisense N2K ASCII** (W2K-1, NGX): one whole message per line,
//!   `A173321.107 23FF7 1F513 012F3070002F30709F`.
//! - **canboat / Actisense plain**: one whole message per line,
//!   `2026-06-21T14:00:00.000Z,2,127250,2,255,8,ff,c2,7a,…`.
//!
//! Field layouts, resolutions and "not available" values follow the canboat PGN database
//! (bits are packed least significant first; multi-byte values are little-endian).

use chrono::{DateTime, Datelike, NaiveDate};
use sim_core::ais::AisMessage;
use sim_core::geo::norm_360;
use sim_sensors::{Emission, Measurement, SensorKind};

pub const BROADCAST: u8 = 255;

/// The PGNs this encoder emits, for output filters.
pub const PGNS: &[u32] = &[
    126992, 127245, 127250, 127251, 127252, 127257, 127258, 127488, 128259, 128267, 129025, 129026, 129029,
    130306, 130312, 130313, 130314, // instruments
    129038, 129039, 129040, 129041, 129793, 129794, 129802, 129809, 129810, // AIS
    60928, 126996, // address claim, product information
];
const MS_PER_KN: f64 = 1852.0 / 3600.0;
const KELVIN: f64 = 273.15;

/// One NMEA 2000 message (a whole PGN payload, before fast-packet framing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct N2kMessage {
    pub pgn: u32,
    pub priority: u8,
    pub src: u8,
    pub dst: u8,
    pub data: Vec<u8>,
}

impl N2kMessage {
    /// 29-bit CAN identifier. PDU1 PGNs (PF < 240) carry the destination in the PS byte.
    pub fn can_id(&self) -> u32 {
        let pf = (self.pgn >> 8) & 0xFF;
        let pgn = if pf < 240 { (self.pgn & 0x3FF00) | self.dst as u32 } else { self.pgn };
        ((self.priority as u32 & 7) << 26) | (pgn << 8) | self.src as u32
    }

    /// Messages longer than one CAN frame are sent as fast packets.
    pub fn is_fast_packet(&self) -> bool {
        self.data.len() > 8
    }

    /// CAN frames carrying this message. `seq` is the 3-bit fast-packet sequence counter.
    pub fn frames(&self, seq: u8) -> Vec<Vec<u8>> {
        if !self.is_fast_packet() {
            return vec![self.data.clone()];
        }
        let head = (seq & 7) << 5;
        let mut frames = Vec::with_capacity(1 + (self.data.len() + 1) / 7);
        let mut first = vec![head, self.data.len() as u8];
        first.extend_from_slice(&self.data[..6]);
        frames.push(first);
        for (n, chunk) in self.data[6..].chunks(7).enumerate() {
            let mut f = Vec::with_capacity(8);
            f.push(head | ((n as u8 + 1) & 0x1F));
            f.extend_from_slice(chunk);
            f.resize(8, 0xFF);
            frames.push(f);
        }
        frames
    }
}

/// Gateway text formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFormat {
    YdRaw,
    ActisenseAscii,
    CanboatPlain,
}

/// Source address of each simulated device on the bus.
pub fn source_address(kind: SensorKind) -> u8 {
    match kind {
        SensorKind::Gps => 10,
        SensorKind::Compass => 11,
        SensorKind::SpeedLog => 12,
        SensorKind::DepthSounder => 13,
        SensorKind::Wind => 14,
        SensorKind::WaterTemp => 15,
        SensorKind::Weather => 16,
        SensorKind::Rudder => 17,
        SensorKind::Attitude => 18,
        SensorKind::Engine => 19,
        SensorKind::Ais => 20,
    }
}

/// NMEA 2000 device class and function of each simulated device (ISO NAME fields).
fn device_class_function(kind: SensorKind) -> (u8, u8) {
    match kind {
        SensorKind::Gps => (60, 145),          // Navigation / Ownship Position (GNSS)
        SensorKind::Compass => (40, 160),      // Steering / Heading Sensors
        SensorKind::SpeedLog => (60, 155),     // Navigation / Speed
        SensorKind::DepthSounder => (60, 130), // Navigation / Bottom Depth
        SensorKind::Wind => (85, 130),         // External Environment / Atmospheric
        SensorKind::WaterTemp => (75, 130),    // Sensor Communication Interface / Temperature
        SensorKind::Weather => (85, 130),      // External Environment / Atmospheric
        SensorKind::Rudder => (40, 155),       // Steering / Rudder
        SensorKind::Attitude => (60, 140),     // Navigation / Ownship Attitude
        SensorKind::Engine => (50, 140),       // Propulsion / Engine
        SensorKind::Ais => (60, 195),          // Navigation / AIS
    }
}

/// Keeps the counters NMEA 2000 needs across messages: the sequence ID that ties related PGNs
/// together, and the fast-packet sequence counter of each PGN and source.
#[derive(Debug, Default)]
pub struct Encoder {
    sid: u8,
    fast_seq: std::collections::HashMap<(u32, u8), u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The PGNs for one emission.
    pub fn encode(&mut self, e: &Emission) -> Vec<N2kMessage> {
        self.sid = (self.sid + 1) % 253; // 253–255 are reserved
        encode(e, self.sid)
    }

    /// Text lines for a message in the given format (several for fast packets in YD RAW).
    pub fn lines(&mut self, format: TextFormat, msg: &N2kMessage, time_unix: f64) -> Vec<String> {
        let t = DateTime::from_timestamp_millis((time_unix * 1000.0).round() as i64).unwrap_or_default();
        match format {
            TextFormat::YdRaw => {
                let seq = if msg.is_fast_packet() {
                    let s = self.fast_seq.entry((msg.pgn, msg.src)).or_insert(0);
                    *s = (*s + 1) & 7;
                    *s
                } else {
                    0
                };
                let time = t.format("%H:%M:%S%.3f");
                msg.frames(seq)
                    .iter()
                    .map(|f| format!("{time} R {:08X} {}", msg.can_id(), hex(f, " ")))
                    .collect()
            }
            TextFormat::ActisenseAscii => vec![format!(
                "A{} {:02X}{:02X}{:X} {:05X} {}",
                t.format("%H%M%S%.3f"),
                msg.src,
                msg.dst,
                msg.priority & 7,
                msg.pgn,
                hex(&msg.data, "")
            )],
            TextFormat::CanboatPlain => vec![format!(
                "{},{},{},{},{},{},{}",
                t.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
                msg.priority,
                msg.pgn,
                msg.src,
                msg.dst,
                msg.data.len(),
                msg.data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(",")
            )],
        }
    }
}

fn hex(bytes: &[u8], sep: &str) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(sep)
}

// ---------------------------------------------------------------------------------------------
// Bit packing

/// Little-endian, least-significant-bit-first field writer.
struct Bits {
    data: Vec<u8>,
    len: usize,
}

impl Bits {
    fn new() -> Self {
        Self { data: Vec::with_capacity(32), len: 0 }
    }

    fn put(&mut self, v: u64, bits: usize) -> &mut Self {
        for i in 0..bits {
            if self.len.is_multiple_of(8) {
                self.data.push(0);
            }
            if (v >> i) & 1 == 1 {
                *self.data.last_mut().expect("pushed") |= 1 << (self.len % 8);
            }
            self.len += 1;
        }
        self
    }

    /// Reserved bits are all ones.
    fn reserved(&mut self, bits: usize) -> &mut Self {
        self.put(u64::MAX, bits)
    }

    /// An unsigned quantity in units of `res`; `None` = not available (all ones). The two
    /// highest codes are reserved, so values clamp below them.
    fn unsigned(&mut self, v: Option<f64>, res: f64, bits: usize) -> &mut Self {
        let max = (1u64 << bits) - 1;
        let raw = match v {
            Some(v) if v.is_finite() => ((v / res).round().max(0.0) as u64).min(max.saturating_sub(2)),
            _ => max,
        };
        self.put(raw, bits)
    }

    /// A signed quantity in units of `res`; `None` = not available (largest positive value).
    fn signed(&mut self, v: Option<f64>, res: f64, bits: usize) -> &mut Self {
        let max = (1i64 << (bits - 1)) - 1;
        let raw = match v {
            Some(v) if v.is_finite() => ((v / res).round() as i64).clamp(-max, max - 2),
            _ => max,
        };
        let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        self.put(raw as u64 & mask, bits)
    }

    /// Fixed-length string, padded with `pad`.
    fn str_fix(&mut self, s: &str, bytes: usize, pad: u8) -> &mut Self {
        let mut b: Vec<u8> = s.bytes().filter(u8::is_ascii).take(bytes).collect();
        b.resize(bytes, pad);
        for c in b {
            self.put(c as u64, 8);
        }
        self
    }

    /// Variable-length string: length (including the two header bytes), encoding (1 = ASCII),
    /// characters.
    fn str_lau(&mut self, s: &str) -> &mut Self {
        let b: Vec<u8> = s.bytes().filter(u8::is_ascii).take(200).collect();
        self.put(b.len() as u64 + 2, 8).put(1, 8);
        for c in b {
            self.put(c as u64, 8);
        }
        self
    }

    fn done(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.data)
    }
}

fn rad(deg: f64) -> f64 {
    deg.to_radians()
}

/// Angle 0–2π (unsigned 0.0001 rad fields).
fn rad360(deg: f64) -> f64 {
    norm_360(deg).to_radians()
}

/// Days since 1970-01-01 and seconds since midnight (UTC).
fn date_time(t: f64) -> (f64, f64) {
    let days = (t / 86_400.0).floor();
    (days, t - days * 86_400.0)
}

fn msg(pgn: u32, priority: u8, src: u8, data: Vec<u8>) -> N2kMessage {
    N2kMessage { pgn, priority, src, dst: BROADCAST, data }
}

// ---------------------------------------------------------------------------------------------
// Measurements → PGNs

/// The PGNs for one emission. `sid` ties the PGNs of one measurement cycle together.
pub fn encode(e: &Emission, sid: u8) -> Vec<N2kMessage> {
    let src = source_address(e.sensor);
    let sid = sid as u64;
    match &e.measurement {
        Measurement::Gps { time_unix, lat, lon, sog_kn, cog_deg, mag_var_deg, satellites, hdop, altitude_m } => {
            let (date, time) = date_time(*time_unix);
            vec![
                msg(129025, 2, src, Bits::new().signed(Some(*lat), 1e-7, 32).signed(Some(*lon), 1e-7, 32).done()),
                msg(129026, 2, src, Bits::new()
                    .put(sid, 8).put(0, 2).reserved(6) // COG reference: true
                    .unsigned(Some(rad360(*cog_deg)), 1e-4, 16)
                    .unsigned(Some(sog_kn * MS_PER_KN), 0.01, 16)
                    .reserved(16)
                    .done()),
                msg(129029, 3, src, Bits::new()
                    .put(sid, 8)
                    .unsigned(Some(date), 1.0, 16)
                    .unsigned(Some(time), 1e-4, 32)
                    .put((lat * 1e16).round() as i64 as u64, 64)
                    .put((lon * 1e16).round() as i64 as u64, 64)
                    .put((altitude_m * 1e6).round() as i64 as u64, 64)
                    .put(0, 4) // GPS
                    .put(1, 4) // GNSS fix
                    .put(0, 2).reserved(6) // integrity: no checking
                    .put(*satellites as u64, 8)
                    .signed(Some(*hdop), 0.01, 16)
                    .signed(None, 0.01, 16) // PDOP
                    .signed(Some(0.0), 0.01, 32) // geoidal separation
                    .put(0, 8) // reference stations
                    .done()),
                msg(126992, 3, src, Bits::new()
                    .put(sid, 8).put(0, 4).reserved(4) // source: GPS
                    .unsigned(Some(date), 1.0, 16)
                    .unsigned(Some(time), 1e-4, 32)
                    .done()),
                msg(127258, 7, src, Bits::new()
                    .put(sid, 8).put(1, 4).reserved(4) // source: automatic chart
                    .unsigned(Some(date), 1.0, 16)
                    .signed(Some(rad(*mag_var_deg)), 1e-4, 16)
                    .reserved(16)
                    .done()),
            ]
        }
        Measurement::Heading { heading_true, deviation_deg, mag_var_deg, rot_dpm } => {
            // A magnetic compass: the sensor heading, its deviation and the variation (as HDG).
            let sensor = heading_true - mag_var_deg - deviation_deg;
            vec![
                msg(127250, 2, src, Bits::new()
                    .put(sid, 8)
                    .unsigned(Some(rad360(sensor)), 1e-4, 16)
                    .signed(Some(rad(*deviation_deg)), 1e-4, 16)
                    .signed(Some(rad(*mag_var_deg)), 1e-4, 16)
                    .put(1, 2).reserved(6) // reference: magnetic
                    .done()),
                msg(127251, 2, src, Bits::new()
                    .put(sid, 8)
                    .signed(Some(rad(*rot_dpm) / 60.0), 3.125e-8, 32)
                    .reserved(24)
                    .done()),
            ]
        }
        Measurement::Speed { stw_kn, .. } => vec![msg(128259, 2, src, Bits::new()
            .put(sid, 8)
            .unsigned(Some(stw_kn * MS_PER_KN), 0.01, 16)
            .unsigned(None, 0.01, 16) // speed over ground: from the GPS
            .put(0, 8) // paddle wheel
            .put(0, 4)
            .reserved(12)
            .done())],
        Measurement::Depth { below_transducer_m, transducer_depth_m } => vec![msg(128267, 3, src, Bits::new()
            .put(sid, 8)
            .unsigned(Some(*below_transducer_m), 0.01, 32)
            // Offset: + = transducer to waterline (so depth below surface = depth + offset).
            .signed(Some(*transducer_depth_m), 0.001, 16)
            .unsigned(None, 10.0, 8)
            .done())],
        Measurement::Wind { awa, aws_kn, twa, tws_kn, .. } => vec![
            wind(src, sid, *aws_kn, *awa, 2), // apparent
            wind(src, sid, *tws_kn, *twa, 4), // true, water referenced
        ],
        Measurement::WaterTemp { celsius } => vec![temperature(src, sid, 0, *celsius)],
        Measurement::Rudder { starboard_deg } => vec![msg(127245, 2, src, Bits::new()
            .put(0, 8) // instance
            .put(0, 3).reserved(5) // no direction order
            .signed(None, 1e-4, 16) // angle order
            .signed(Some(rad(*starboard_deg)), 1e-4, 16)
            .reserved(16)
            .done())],
        Measurement::Weather { air_temp_c, pressure_hpa, humidity_pct, twd, tws_kn, .. } => vec![
            temperature(src, sid, 1, *air_temp_c),
            msg(130313, 5, src, Bits::new()
                .put(sid, 8).put(0, 8).put(1, 8) // instance 0, outside
                .signed(Some(*humidity_pct), 0.004, 16)
                .signed(None, 0.004, 16)
                .reserved(8)
                .done()),
            msg(130314, 5, src, Bits::new()
                .put(sid, 8).put(0, 8).put(0, 8) // instance 0, atmospheric
                .signed(Some(pressure_hpa * 100.0), 0.1, 32)
                .reserved(8)
                .done()),
            wind(src, sid, *tws_kn, *twd, 0), // true, ground referenced to north
        ],
        Measurement::Attitude { roll_deg, pitch_deg, heave_m } => vec![
            msg(127257, 3, src, Bits::new()
                .put(sid, 8)
                .signed(None, 1e-4, 16) // yaw
                .signed(Some(rad(*pitch_deg)), 1e-4, 16)
                .signed(Some(rad(*roll_deg)), 1e-4, 16)
                .reserved(8)
                .done()),
            msg(127252, 3, src, Bits::new().put(sid, 8).signed(Some(*heave_m), 0.01, 16).reserved(40).done()),
        ],
        Measurement::Engine { rpm } => vec![msg(127488, 2, src, Bits::new()
            .put(0, 8) // single engine
            .unsigned(Some(*rpm), 0.25, 16)
            .unsigned(None, 100.0, 16) // boost pressure
            .signed(None, 1.0, 8) // tilt/trim
            .reserved(16)
            .done())],
        Measurement::Ais { own, channel, message, .. } => {
            // Transceiver information: own reports were transmitted, others received.
            let ch = u64::from(*channel == 'B');
            let xcvr = if *own { 2 + ch } else { ch };
            ais(src, xcvr, e.time_unix, message).into_iter().collect()
        }
    }
}

fn wind(src: u8, sid: u64, speed_kn: f64, angle_deg: f64, reference: u64) -> N2kMessage {
    msg(130306, 2, src, Bits::new()
        .put(sid, 8)
        .unsigned(Some(speed_kn * MS_PER_KN), 0.01, 16)
        .unsigned(Some(rad360(angle_deg)), 1e-4, 16)
        .put(reference, 3)
        .reserved(21)
        .done())
}

fn temperature(src: u8, sid: u64, source: u64, celsius: f64) -> N2kMessage {
    msg(130312, 5, src, Bits::new()
        .put(sid, 8)
        .put(0, 8) // instance
        .put(source, 8)
        .unsigned(Some(celsius + KELVIN), 0.01, 16)
        .unsigned(None, 0.01, 16)
        .reserved(8)
        .done())
}

// ---------------------------------------------------------------------------------------------
// AIS → PGNs 129038 and friends

fn ais_header(b: &mut Bits, msg_id: u8, mmsi: u32) {
    b.put(msg_id as u64, 6).put(0, 2).put(mmsi as u64, 32);
}

fn lon_lat(b: &mut Bits, lat: f64, lon: f64) {
    b.signed(Some(lon), 1e-7, 32).signed(Some(lat), 1e-7, 32);
}

/// Length, beam and antenna position from the AIS dimensions (to bow, stern, port, starboard).
fn dimensions(b: &mut Bits, dims: [u16; 4]) {
    let [bow, stern, port, stbd] = dims.map(f64::from);
    let known = |v: f64| (v > 0.0).then_some(v);
    b.unsigned(known(bow + stern), 0.1, 16)
        .unsigned(known(port + stbd), 0.1, 16)
        .unsigned(known(stbd), 0.1, 16)
        .unsigned(known(bow), 0.1, 16);
}

/// ETA (month, day, hour, minute) as days since 1970 and seconds since midnight; the year is
/// the next occurrence after `now`.
fn eta(now: f64, (month, day, hour, minute): (u8, u8, u8, u8)) -> (Option<f64>, Option<f64>) {
    let now_date = DateTime::from_timestamp(now as i64, 0).unwrap_or_default().date_naive();
    let date = (month > 0 && day > 0)
        .then(|| {
            let d = NaiveDate::from_ymd_opt(now_date.year(), month as u32, day as u32)?;
            let d = if d < now_date { NaiveDate::from_ymd_opt(now_date.year() + 1, month as u32, day as u32)? } else { d };
            Some((d - NaiveDate::from_ymd_opt(1970, 1, 1)?).num_days() as f64)
        })
        .flatten();
    let time = (hour < 24 && minute < 60).then(|| f64::from(hour) * 3600.0 + f64::from(minute) * 60.0);
    (date, time)
}

fn ais(src: u8, xcvr: u64, now: f64, m: &AisMessage) -> Option<N2kMessage> {
    let mut b = Bits::new();
    let pgn = match m {
        AisMessage::PositionA { msg_type, common: c, nav_status, rot_dpm, .. } => {
            ais_header(&mut b, *msg_type, c.mmsi);
            lon_lat(&mut b, c.lat, c.lon);
            b.put(c.accuracy as u64, 1).put(c.raim as u64, 1).put(c.second.min(63) as u64, 6)
                .unsigned(c.cog.map(rad360), 1e-4, 16)
                .unsigned(c.sog_kn.map(|s| s * MS_PER_KN), 0.01, 16)
                .put(0, 19) // communication state
                .put(xcvr, 5)
                .unsigned(c.heading.map(rad360), 1e-4, 16)
                .signed(rot_dpm.map(|r| rad(r) / 60.0), 3.125e-5, 16)
                .put(*nav_status as u64, 4)
                .put(0, 2) // special manoeuvre: not available
                .reserved(2)
                .put(0, 3)
                .reserved(5)
                .put(0xFF, 8); // sequence ID
            129038
        }
        AisMessage::PositionB { common: c, flags } => {
            ais_header(&mut b, 18, c.mmsi);
            lon_lat(&mut b, c.lat, c.lon);
            b.put(c.accuracy as u64, 1).put(c.raim as u64, 1).put(c.second.min(63) as u64, 6)
                .unsigned(c.cog.map(rad360), 1e-4, 16)
                .unsigned(c.sog_kn.map(|s| s * MS_PER_KN), 0.01, 16)
                .put(0, 19)
                .put(xcvr, 5)
                .unsigned(c.heading.map(rad360), 1e-4, 16)
                .put(0, 8)
                .put(0, 2)
                .put(flags.cs_unit as u64, 1)
                .put(flags.display as u64, 1)
                .put(flags.dsc as u64, 1)
                .put(flags.band as u64, 1)
                .put(flags.msg22 as u64, 1)
                .put(flags.assigned as u64, 1)
                .put(flags.cs_unit as u64, 1) // CS units use ITDMA communication state
                .reserved(15);
            129039
        }
        AisMessage::ExtendedB { common: c, name, ship_type, dims, epfd, dte_unavailable, assigned } => {
            ais_header(&mut b, 19, c.mmsi);
            lon_lat(&mut b, c.lat, c.lon);
            b.put(c.accuracy as u64, 1).put(c.raim as u64, 1).put(c.second.min(63) as u64, 6)
                .unsigned(c.cog.map(rad360), 1e-4, 16)
                .unsigned(c.sog_kn.map(|s| s * MS_PER_KN), 0.01, 16)
                .put(0, 8)
                .put(0, 4)
                .reserved(4)
                .put(*ship_type as u64, 8)
                .unsigned(c.heading.map(rad360), 1e-4, 16)
                .reserved(4)
                .put(*epfd as u64 & 0xF, 4);
            dimensions(&mut b, *dims);
            b.str_fix(name, 20, b'@')
                .put(*dte_unavailable as u64, 1)
                .put(*assigned as u64, 1)
                .put(0, 4)
                .put(xcvr, 5)
                .reserved(5);
            129040
        }
        AisMessage::StaticA { mmsi, imo, call_sign, name, ship_type, dims, epfd, eta: e, draught_m, destination, dte_unavailable } => {
            let (eta_date, eta_time) = eta(now, *e);
            ais_header(&mut b, 5, *mmsi);
            b.put(*imo as u64, 32).str_fix(call_sign, 7, b'@').str_fix(name, 20, b'@').put(*ship_type as u64, 8);
            dimensions(&mut b, *dims);
            b.unsigned(eta_date, 1.0, 16)
                .unsigned(eta_time, 1e-4, 32)
                .unsigned((*draught_m > 0.0).then_some(*draught_m), 0.01, 16)
                .str_fix(destination, 20, b'@')
                .put(0, 2) // AIS version: ITU-R M.1371-1
                .put(*epfd as u64 & 0xF, 4)
                .put(*dte_unavailable as u64, 1)
                .reserved(1)
                .put(xcvr, 5)
                .reserved(3);
            129794
        }
        AisMessage::BaseStation { mmsi, time_unix, lat, lon, accuracy, epfd, raim } => {
            let (date, time) = date_time(time_unix.floor());
            ais_header(&mut b, 4, *mmsi);
            lon_lat(&mut b, *lat, *lon);
            b.put(*accuracy as u64, 1).put(*raim as u64, 1).reserved(6)
                .unsigned(Some(time), 1e-4, 32)
                .put(0, 19)
                .put(xcvr, 5)
                .unsigned(Some(date), 1.0, 16)
                .reserved(4)
                .put(*epfd as u64 & 0xF, 4);
            129793
        }
        AisMessage::Aton { mmsi, aton_type, name, lat, lon, accuracy, dims, epfd, second, off_position, raim, virtual_aton, assigned } => {
            ais_header(&mut b, 21, *mmsi);
            lon_lat(&mut b, *lat, *lon);
            b.put(*accuracy as u64, 1).put(*raim as u64, 1).put((*second).min(63) as u64, 6);
            dimensions(&mut b, *dims);
            b.put(*aton_type as u64, 5)
                .put(*off_position as u64, 1)
                .put(*virtual_aton as u64, 1)
                .put(*assigned as u64, 1)
                .put(0, 1)
                .put(*epfd as u64 & 0xF, 4)
                .reserved(3)
                .put(0, 8) // AtoN status
                .put(xcvr, 5)
                .reserved(3)
                .str_lau(name.trim_end());
            129041
        }
        AisMessage::SafetyText { mmsi, text } => {
            ais_header(&mut b, 14, *mmsi);
            b.reserved(1).put(xcvr, 5).put(0, 2).str_lau(text);
            129802
        }
        AisMessage::StaticB24A { mmsi, name } => {
            ais_header(&mut b, 24, *mmsi);
            b.str_fix(name, 20, b'@').put(xcvr, 5).reserved(3).put(0xFF, 8);
            129809
        }
        AisMessage::StaticB24B { mmsi, ship_type, vendor_id, call_sign, dims } => {
            ais_header(&mut b, 24, *mmsi);
            b.put(*ship_type as u64, 8).str_fix(vendor_id, 7, b'@').str_fix(call_sign, 7, b'@');
            dimensions(&mut b, *dims);
            b.put(0, 32) // mothership MMSI
                .reserved(2)
                .put(0, 2)
                .put(0, 4) // position device: undefined
                .put(xcvr, 5)
                .reserved(3)
                .put(0xFF, 8);
            129810
        }
    };
    let priority = match pgn {
        129794 | 129809 | 129810 => 6,
        129793 => 7,
        129802 => 5,
        _ => 4,
    };
    Some(msg(pgn, priority, src, b.done()))
}

// ---------------------------------------------------------------------------------------------
// Network management

/// ISO Address Claim (60928) and Product Information (126996) for a simulated device, which
/// tools use to list the devices on the bus.
pub fn device_info(kind: SensorKind, label: &str, version: &str) -> [N2kMessage; 2] {
    let src = source_address(kind);
    let (class, function) = device_class_function(kind);
    let claim = msg(60928, 6, src, Bits::new()
        .put(0x1_0000 + src as u64, 21) // unique number
        .put(2046, 11) // manufacturer code (unassigned)
        .put(0, 3)
        .put(0, 5) // device instance
        .put(function as u64, 8)
        .put(0, 1)
        .put(class as u64, 7)
        .put(0, 4) // system instance
        .put(4, 3) // industry group: marine
        .put(1, 1) // arbitrary address capable
        .done());
    let product = msg(126996, 6, src, Bits::new()
        .put(2100, 16) // NMEA 2000 version 2.100
        .put(1000 + src as u64, 16)
        .str_fix(&format!("Marine Data Simulator {label}"), 32, 0xFF)
        .str_fix(version, 32, 0xFF)
        .str_fix("simulated", 32, 0xFF)
        .str_fix(&format!("MDS-{src:03}"), 32, 0xFF)
        .put(0, 8) // certification level
        .put(1, 8) // load equivalency (×50 mA)
        .done());
    [claim, product]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::ais::{AisFlags, PositionCommon};

    fn emission(sensor: SensorKind, m: Measurement) -> Emission {
        Emission { sensor, talker: "II".into(), time_unix: 1_782_052_215.25, measurement: m }
    }

    /// Reads `bits` bits at `offset` (LSB first), as canboat does.
    fn field(data: &[u8], offset: usize, bits: usize) -> u64 {
        (0..bits).fold(0, |v, i| {
            let p = offset + i;
            v | ((((data[p / 8] >> (p % 8)) & 1) as u64) << i)
        })
    }

    fn signed(data: &[u8], offset: usize, bits: usize) -> i64 {
        let v = field(data, offset, bits);
        if bits < 64 && v >> (bits - 1) == 1 { v as i64 - (1i64 << bits) } else { v as i64 }
    }

    #[test]
    fn can_id_of_pdu2_and_pdu1() {
        let m = N2kMessage { pgn: 129025, priority: 2, src: 10, dst: 255, data: vec![] };
        assert_eq!(m.can_id(), 0x09F8010A);
        let m = N2kMessage { pgn: 60928, priority: 6, src: 10, dst: 255, data: vec![] };
        assert_eq!(m.can_id(), 0x18EEFF0A);
    }

    #[test]
    fn single_frame_lengths_and_known_layouts() {
        let gps = emission(SensorKind::Gps, Measurement::Gps {
            time_unix: 1_782_052_215.25, lat: 28.4, lon: -80.55, sog_kn: 6.0, cog_deg: 200.0,
            mag_var_deg: -6.5, satellites: 9, hdop: 0.9, altitude_m: 2.0,
        });
        let msgs = encode(&gps, 7);
        let pgns: Vec<u32> = msgs.iter().map(|m| m.pgn).collect();
        assert_eq!(pgns, [129025, 129026, 129029, 126992, 127258]);
        let pos = &msgs[0].data;
        assert_eq!(pos.len(), 8);
        assert_eq!(signed(pos, 0, 32), 284_000_000);
        assert_eq!(signed(pos, 32, 32), -805_500_000);
        let cog = &msgs[1].data;
        assert_eq!(field(cog, 0, 8), 7);
        assert_eq!(field(cog, 16, 16), (200f64.to_radians() / 1e-4).round() as u64);
        assert_eq!(field(cog, 32, 16), (6.0 * MS_PER_KN / 0.01).round() as u64);
        let gnss = &msgs[2].data;
        assert_eq!(gnss.len(), 43);
        assert_eq!(field(gnss, 8, 16), 1_782_052_215 / 86_400);
        assert_eq!(signed(gnss, 56, 64), 284_000_000_000_000_000);
        assert_eq!(field(gnss, 264, 8), 9);
        for m in &msgs {
            if m.pgn != 129029 {
                assert_eq!(m.data.len(), 8, "{}", m.pgn);
            }
        }
    }

    #[test]
    fn every_measurement_encodes() {
        let all = [
            Measurement::Heading { heading_true: 10.0, deviation_deg: -1.0, mag_var_deg: 2.0, rot_dpm: -12.0 },
            Measurement::Speed { stw_kn: 5.5, heading_true: 10.0, mag_var_deg: 2.0 },
            Measurement::Depth { below_transducer_m: 12.34, transducer_depth_m: 0.5 },
            Measurement::Wind { awa: -35.0, aws_kn: 15.0, twa: -50.0, tws_kn: 11.0, twd: 225.0, mag_var_deg: 0.0 },
            Measurement::WaterTemp { celsius: 15.2 },
            Measurement::Rudder { starboard_deg: -5.0 },
            Measurement::Weather {
                air_temp_c: 18.0, pressure_hpa: 1013.25, humidity_pct: 70.0, water_temp_c: 15.0,
                twd: 225.0, tws_kn: 11.0, mag_var_deg: 0.0,
            },
            Measurement::Attitude { roll_deg: -12.5, pitch_deg: 3.2, heave_m: 0.4 },
            Measurement::Engine { rpm: 1850.0 },
        ];
        for m in all {
            for n in encode(&emission(SensorKind::Compass, m.clone()), 1) {
                assert_eq!(n.data.len(), 8, "{} from {m:?}", n.pgn);
            }
        }
        // Spot checks.
        let h = encode(&emission(SensorKind::Compass, Measurement::Heading {
            heading_true: 10.0, deviation_deg: -1.0, mag_var_deg: 2.0, rot_dpm: -12.0,
        }), 1);
        assert_eq!(field(&h[0].data, 8, 16), (9f64.to_radians() / 1e-4).round() as u64);
        assert_eq!(signed(&h[0].data, 24, 16), (-1f64.to_radians() / 1e-4).round() as i64);
        assert_eq!(field(&h[0].data, 56, 2), 1);
        let rpm = encode(&emission(SensorKind::Engine, Measurement::Engine { rpm: 1850.0 }), 1);
        assert_eq!(field(&rpm[0].data, 8, 16), 7400);
        let t = encode(&emission(SensorKind::WaterTemp, Measurement::WaterTemp { celsius: 15.2 }), 1);
        assert_eq!(field(&t[0].data, 24, 16), 28835);
    }

    fn common() -> PositionCommon {
        PositionCommon {
            mmsi: 366_100_001, lat: 28.4, lon: -80.55, sog_kn: Some(12.5), cog: Some(355.0),
            heading: Some(354.0), second: 15, accuracy: true, raim: false,
        }
    }

    #[test]
    fn ais_pgn_lengths_match_canboat() {
        let c = common();
        let cases: Vec<(AisMessage, u32, usize)> = vec![
            (AisMessage::PositionA { msg_type: 1, common: c, nav_status: 0, rot_dpm: Some(10.0), time_unix: 0.0 }, 129038, 28),
            (AisMessage::PositionB { common: c, flags: AisFlags::default() }, 129039, 27),
            (AisMessage::ExtendedB {
                common: c, name: "SPINDRIFT".into(), ship_type: 36, dims: [9, 4, 2, 2], epfd: 1,
                dte_unavailable: false, assigned: false,
            }, 129040, 54),
            (AisMessage::StaticA {
                mmsi: 1, imo: 9_100_001, call_sign: "WSO1001".into(), name: "SEA ODYSSEY".into(), ship_type: 60,
                dims: [250, 50, 18, 18], epfd: 1, eta: (6, 22, 8, 0), draught_m: 8.5,
                destination: "NASSAU".into(), dte_unavailable: false,
            }, 129794, 75),
            (AisMessage::BaseStation { mmsi: 3_669_001, time_unix: 1_782_052_215.0, lat: 28.4, lon: -80.6, accuracy: true, epfd: 7, raim: false }, 129793, 24),
            (AisMessage::StaticB24A { mmsi: 1, name: "SPINDRIFT".into() }, 129809, 27),
            (AisMessage::StaticB24B { mmsi: 1, ship_type: 36, vendor_id: "SIMAIS".into(), call_sign: "WSP1004".into(), dims: [9, 4, 2, 2] }, 129810, 35),
        ];
        for (m, pgn, len) in cases {
            let n = ais(20, 0, 1_782_052_215.0, &m).unwrap();
            assert_eq!((n.pgn, n.data.len()), (pgn, len), "{m:?}");
        }
        let a = ais(20, 0, 0.0, &AisMessage::PositionA { msg_type: 1, common: c, nav_status: 7, rot_dpm: None, time_unix: 0.0 }).unwrap();
        assert_eq!(field(&a.data, 0, 6), 1);
        assert_eq!(field(&a.data, 8, 32), 366_100_001);
        assert_eq!(signed(&a.data, 40, 32), -805_500_000);
        assert_eq!(field(&a.data, 106, 6), 15);
        assert_eq!(field(&a.data, 184, 16), 0x7FFF); // ROT not available
        assert_eq!(field(&a.data, 200, 4), 7);
    }

    #[test]
    fn aton_and_safety_text_use_variable_strings() {
        let n = ais(20, 1, 0.0, &AisMessage::SafetyText { mmsi: 970_000_001, text: "SART ACTIVE".into() }).unwrap();
        assert_eq!(n.data.len(), 6 + 2 + 11);
        assert_eq!(&n.data[8..], b"SART ACTIVE");
        assert_eq!(field(&n.data, 41, 5), 1);
    }

    #[test]
    fn eta_rolls_into_next_year() {
        // 2026-06-21: an ETA of 01-02 is next January.
        let (d, t) = eta(1_782_052_215.0, (1, 2, 8, 30));
        let jan2 = NaiveDate::from_ymd_opt(2027, 1, 2).unwrap() - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        assert_eq!(d, Some(jan2.num_days() as f64));
        assert_eq!(t, Some(8.0 * 3600.0 + 30.0 * 60.0));
        assert_eq!(eta(0.0, (0, 0, 24, 60)), (None, None));
    }

    #[test]
    fn fast_packet_framing() {
        let m = N2kMessage { pgn: 129029, priority: 3, src: 10, dst: 255, data: (0..43).collect() };
        let f = m.frames(3);
        assert_eq!(f.len(), 1 + 37usize.div_ceil(7)); // 6 bytes in the first frame, 7 after
        assert_eq!(f[0][..2], [0x60, 43]);
        assert_eq!(f[0][2..], [0, 1, 2, 3, 4, 5]);
        assert_eq!(f[1][..2], [0x61, 6]);
        assert!(f.iter().all(|f| f.len() == 8));
        assert_eq!(*f.last().unwrap().last().unwrap(), 0xFF);
        // Reassemble.
        let mut data: Vec<u8> = f[0][2..].to_vec();
        for fr in &f[1..] {
            data.extend_from_slice(&fr[1..]);
        }
        data.truncate(43);
        assert_eq!(data, m.data);
    }

    #[test]
    fn text_formats() {
        let mut enc = Encoder::new();
        let m = N2kMessage { pgn: 127250, priority: 2, src: 11, dst: 255, data: vec![1, 2, 3, 4, 5, 6, 7, 0xFC] };
        let t = 1_782_052_215.25; // 2026-06-21 14:30:15.250 UTC
        assert_eq!(enc.lines(TextFormat::YdRaw, &m, t), ["14:30:15.250 R 09F1120B 01 02 03 04 05 06 07 FC"]);
        assert_eq!(enc.lines(TextFormat::ActisenseAscii, &m, t), ["A143015.250 0BFF2 1F112 01020304050607FC"]);
        assert_eq!(
            enc.lines(TextFormat::CanboatPlain, &m, t),
            ["2026-06-21T14:30:15.250Z,2,127250,11,255,8,01,02,03,04,05,06,07,fc"]
        );
        let fast = N2kMessage { pgn: 129029, priority: 3, src: 10, dst: 255, data: vec![0; 43] };
        let a = enc.lines(TextFormat::YdRaw, &fast, t);
        let b = enc.lines(TextFormat::YdRaw, &fast, t);
        assert_eq!(a.len(), 7);
        assert_ne!(a[0][24..26], b[0][24..26], "sequence counter advances: {} / {}", a[0], b[0]);
    }

    #[test]
    fn device_info_layouts() {
        let [claim, product] = device_info(SensorKind::Gps, "GPS", "0.1.0");
        assert_eq!(claim.data.len(), 8);
        assert_eq!(claim.can_id(), 0x18EEFF0A);
        assert_eq!(field(&claim.data, 40, 8), 145);
        assert_eq!(field(&claim.data, 49, 7), 60);
        assert_eq!(field(&claim.data, 60, 3), 4);
        assert_eq!(product.data.len(), 134);
    }
}
