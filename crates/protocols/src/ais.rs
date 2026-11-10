//! AIS message encoding (ITU-R M.1371-5) and NMEA 0183 `!AIVDM` / `!AIVDO` encapsulation
//! (IEC 61162-1): bit packing, 6-bit ASCII text, payload armoring and multi-sentence
//! fragmentation.

use sim_core::ais::{AisMessage, PositionCommon};

use crate::nmea0183::checksum;

/// Payload characters per sentence. 60 keeps `!AIVDM` sentences well under 82 characters.
pub const MAX_PAYLOAD_CHARS: usize = 60;

#[derive(Default)]
struct Bits {
    bits: Vec<bool>,
}

impl Bits {
    fn u(&mut self, value: u64, n: usize) {
        for i in (0..n).rev() {
            self.bits.push((value >> i) & 1 == 1);
        }
    }

    fn i(&mut self, value: i64, n: usize) {
        self.u((value as u64) & ((1u64 << n) - 1), n);
    }

    fn b(&mut self, v: bool) {
        self.bits.push(v);
    }

    /// Text in AIS 6-bit ASCII, padded with `@` to `chars` characters.
    fn text(&mut self, s: &str, chars: usize) {
        let mut it = s.chars().map(sixbit_char);
        for _ in 0..chars {
            self.u(it.next().unwrap_or(0) as u64, 6);
        }
    }

    fn len(&self) -> usize {
        self.bits.len()
    }

    /// Armored payload and the number of fill bits.
    fn armor(&self) -> (String, u8) {
        let fill = (6 - self.bits.len() % 6) % 6;
        let mut out = String::with_capacity(self.bits.len() / 6 + 1);
        let mut padded = self.bits.clone();
        padded.extend(std::iter::repeat_n(false, fill));
        for chunk in padded.chunks(6) {
            let v = chunk.iter().fold(0u8, |acc, &b| (acc << 1) | b as u8);
            out.push(armor_char(v));
        }
        (out, fill as u8)
    }
}

/// Character → 6-bit code. Lower case is upper-cased; unsupported characters become `?`.
fn sixbit_char(c: char) -> u8 {
    let c = c.to_ascii_uppercase();
    match c as u32 {
        64..=95 => (c as u32 - 64) as u8,
        32..=63 => c as u8,
        _ => b'?',
    }
}

fn armor_char(v: u8) -> char {
    let v = if v < 40 { v + 48 } else { v + 56 };
    v as char
}

/// Rate of turn as transmitted: `4.733·√|ROT|` (°/min), signed; −128 = not available.
fn rot_field(rot_dpm: Option<f64>) -> i64 {
    match rot_dpm {
        None => -128,
        Some(r) if r.is_nan() => -128,
        Some(r) => {
            let v = (4.733 * r.abs().sqrt()).round().min(126.0);
            // ±127: turning faster than 5°/30 s with no turn indicator; we always have a value.
            (v * r.signum()) as i64
        }
    }
}

fn sog_field(sog: Option<f64>) -> u64 {
    match sog {
        Some(v) if v.is_finite() && v >= 0.0 => ((v * 10.0).round() as u64).min(1022),
        _ => 1023,
    }
}

fn cog_field(cog: Option<f64>) -> u64 {
    match cog {
        Some(v) if v.is_finite() => ((v.rem_euclid(360.0) * 10.0).round() as u64) % 3600,
        _ => 3600,
    }
}

fn heading_field(h: Option<f64>) -> u64 {
    match h {
        Some(v) if v.is_finite() => (v.rem_euclid(360.0).round() as u64) % 360,
        _ => 511,
    }
}

/// Longitude/latitude in 1/10 000 minute.
fn lon_field(lon: f64) -> i64 {
    if lon.is_finite() { (lon * 600_000.0).round() as i64 } else { 181 * 600_000 }
}

fn lat_field(lat: f64) -> i64 {
    if lat.is_finite() { (lat * 600_000.0).round() as i64 } else { 91 * 600_000 }
}

fn dims(b: &mut Bits, d: [u16; 4]) {
    b.u(d[0].min(511) as u64, 9);
    b.u(d[1].min(511) as u64, 9);
    b.u(d[2].min(63) as u64, 6);
    b.u(d[3].min(63) as u64, 6);
}

fn header(b: &mut Bits, msg_type: u8, mmsi: u32) {
    b.u(msg_type as u64, 6);
    b.u(0, 2); // repeat indicator
    b.u(mmsi as u64, 30);
}

fn position(b: &mut Bits, c: &PositionCommon) {
    b.b(c.accuracy);
    b.i(lon_field(c.lon), 28);
    b.i(lat_field(c.lat), 27);
    b.u(cog_field(c.cog), 12);
    b.u(heading_field(c.heading), 9);
    b.u(c.second.min(63) as u64, 6);
}

/// SOTDMA communication state with a plausible slot timeout and sub-message.
fn sotdma_state(time_unix: f64) -> u64 {
    let t = time_unix.max(0.0) as u64;
    let timeout = t % 8; // cycles 7..0 in a real station; any value is valid
    let sub = match timeout {
        // UTC hour and minute.
        1 => ((t / 3600 % 24) << 9) | ((t / 60 % 60) << 2),
        // Slot number.
        2 | 4 | 6 => (t % 60) * 2250 / 60,
        // Received stations.
        3 | 5 | 7 => 12,
        // Slot offset.
        _ => 2250,
    } & 0x3FFF;
    (timeout << 14) | sub // sync state 0 (UTC direct)
}

/// Bit-level encoding of a message.
fn encode_bits(msg: &AisMessage) -> Bits {
    let mut b = Bits::default();
    match msg {
        AisMessage::PositionA { msg_type, common, nav_status, rot_dpm, time_unix } => {
            header(&mut b, *msg_type, common.mmsi);
            b.u((*nav_status).min(15) as u64, 4);
            b.i(rot_field(*rot_dpm), 8);
            b.u(sog_field(common.sog_kn), 10);
            position(&mut b, common);
            b.u(0, 2); // manoeuvre indicator: not available
            b.u(0, 3); // spare
            b.b(common.raim);
            b.u(sotdma_state(*time_unix), 19);
        }
        AisMessage::BaseStation { mmsi, time_unix, lat, lon, accuracy, epfd, raim } => {
            header(&mut b, 4, *mmsi);
            let dt = chrono::DateTime::from_timestamp(time_unix.floor() as i64, 0).unwrap_or_default();
            use chrono::{Datelike, Timelike};
            b.u(dt.year() as u64, 14);
            b.u(dt.month() as u64, 4);
            b.u(dt.day() as u64, 5);
            b.u(dt.hour() as u64, 5);
            b.u(dt.minute() as u64, 6);
            b.u(dt.second() as u64, 6);
            b.b(*accuracy);
            b.i(lon_field(*lon), 28);
            b.i(lat_field(*lat), 27);
            b.u((*epfd).min(15) as u64, 4);
            b.u(0, 10); // spare
            b.b(*raim);
            b.u(sotdma_state(*time_unix), 19);
        }
        AisMessage::StaticA {
            mmsi,
            imo,
            call_sign,
            name,
            ship_type,
            dims: d,
            epfd,
            eta,
            draught_m,
            destination,
            dte_unavailable,
        } => {
            header(&mut b, 5, *mmsi);
            b.u(2, 2); // AIS version: ITU-R M.1371-5
            b.u(*imo as u64 & 0x3FFF_FFFF, 30);
            b.text(call_sign, 7);
            b.text(name, 20);
            b.u(*ship_type as u64, 8);
            dims(&mut b, *d);
            b.u((*epfd).min(15) as u64, 4);
            b.u(eta.0 as u64, 4);
            b.u(eta.1 as u64, 5);
            b.u(eta.2 as u64, 5);
            b.u(eta.3 as u64, 6);
            b.u(((draught_m * 10.0).round().clamp(0.0, 255.0)) as u64, 8);
            b.text(destination, 20);
            b.b(*dte_unavailable);
            b.u(0, 1); // spare
        }
        AisMessage::SafetyText { mmsi, text } => {
            header(&mut b, 14, *mmsi);
            b.u(0, 2); // spare
            // Up to 161 characters (5 slots).
            let n = text.chars().count().min(161);
            b.text(text, n);
        }
        AisMessage::PositionB { common, flags } => {
            header(&mut b, 18, common.mmsi);
            b.u(0, 8); // regional reserved
            b.u(sog_field(common.sog_kn), 10);
            position(&mut b, common);
            b.u(0, 2); // regional reserved
            b.b(flags.cs_unit);
            b.b(flags.display);
            b.b(flags.dsc);
            b.b(flags.band);
            b.b(flags.msg22);
            b.b(flags.assigned);
            b.b(common.raim);
            // Communication state selector 1 (ITDMA) with the CS default value for CS units.
            b.u(if flags.cs_unit { 0b1100000000000000110 } else { sotdma_state(common.second as f64) }, 20);
        }
        AisMessage::ExtendedB { common, name, ship_type, dims: d, epfd, dte_unavailable, assigned } => {
            header(&mut b, 19, common.mmsi);
            b.u(0, 8);
            b.u(sog_field(common.sog_kn), 10);
            position(&mut b, common);
            b.u(0, 4); // regional reserved
            b.text(name, 20);
            b.u(*ship_type as u64, 8);
            dims(&mut b, *d);
            b.u((*epfd).min(15) as u64, 4);
            b.b(common.raim);
            b.b(*dte_unavailable);
            b.b(*assigned);
            b.u(0, 4); // spare
        }
        AisMessage::Aton {
            mmsi,
            aton_type,
            name,
            lat,
            lon,
            accuracy,
            dims: d,
            epfd,
            second,
            off_position,
            raim,
            virtual_aton,
            assigned,
        } => {
            header(&mut b, 21, *mmsi);
            b.u((*aton_type).min(31) as u64, 5);
            let name: Vec<char> = name.chars().collect();
            let (head, ext): (String, String) = if name.len() > 20 {
                (name[..20].iter().collect(), name[20..name.len().min(34)].iter().collect())
            } else {
                (name.iter().collect(), String::new())
            };
            b.text(&head, 20);
            b.b(*accuracy);
            b.i(lon_field(*lon), 28);
            b.i(lat_field(*lat), 27);
            dims(&mut b, *d);
            b.u((*epfd).min(15) as u64, 4);
            b.u((*second).min(63) as u64, 6);
            b.b(*off_position);
            b.u(0, 8); // regional reserved
            b.b(*raim);
            b.b(*virtual_aton);
            b.b(*assigned);
            b.u(0, 1); // spare
            if !ext.is_empty() {
                b.text(&ext, ext.chars().count());
                // Pad the extension to a byte boundary.
                let pad = (8 - b.len() % 8) % 8;
                b.u(0, pad);
            }
        }
        AisMessage::StaticB24A { mmsi, name } => {
            header(&mut b, 24, *mmsi);
            b.u(0, 2); // part A
            b.text(name, 20);
        }
        AisMessage::StaticB24B { mmsi, ship_type, vendor_id, call_sign, dims: d } => {
            header(&mut b, 24, *mmsi);
            b.u(1, 2); // part B
            b.u(*ship_type as u64, 8);
            b.text(vendor_id, 7);
            b.text(call_sign, 7);
            dims(&mut b, *d);
            b.u(1, 4); // EPFD: GPS
            b.u(0, 2); // spare
        }
    }
    b
}

/// Armored payload and fill bits for a message.
pub fn payload(msg: &AisMessage) -> (String, u8) {
    encode_bits(msg).armor()
}

/// Number of bits a message encodes to (for tests and diagnostics).
pub fn bit_len(msg: &AisMessage) -> usize {
    encode_bits(msg).len()
}

/// Wraps a message in `!xxVDM` (other stations) or `!xxVDO` (own ship) sentences, without
/// line endings (the transports add `\r\n`).
/// `channel` is `A` or `B`; `seq_id` (0–9) is used only when several sentences are needed.
pub fn sentences(talker: &str, own: bool, channel: char, seq_id: u8, msg: &AisMessage) -> Vec<String> {
    let (payload, fill) = payload(msg);
    let kind = if own { "VDO" } else { "VDM" };
    let chunks: Vec<&str> = if payload.is_empty() {
        vec![""]
    } else {
        // Payload is pure ASCII, so byte chunks are character chunks.
        payload.as_bytes().chunks(MAX_PAYLOAD_CHARS).map(|c| std::str::from_utf8(c).unwrap_or("")).collect()
    };
    let total = chunks.len();
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let seq = if total > 1 { (seq_id % 10).to_string() } else { String::new() };
            let f = if i + 1 == total { fill } else { 0 };
            let body = format!("{talker}{kind},{total},{},{seq},{channel},{chunk},{f}", i + 1);
            format!("!{body}*{:02X}", checksum(&body))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::ais::{AisFlags, AisStatic, static_a, static_b};

    fn common() -> PositionCommon {
        PositionCommon {
            mmsi: 366_000_101,
            lat: 28.4,
            lon: -80.55,
            sog_kn: Some(12.3),
            cog: Some(201.5),
            heading: Some(200.0),
            second: 34,
            accuracy: true,
            raim: false,
        }
    }

    #[test]
    fn message_lengths() {
        let s = AisStatic { ship_type: 70, destination: "PORT CANAVERAL".into(), ..AisStatic::default() };
        let cases: Vec<(AisMessage, usize)> = vec![
            (
                AisMessage::PositionA { msg_type: 1, common: common(), nav_status: 0, rot_dpm: Some(5.0), time_unix: 0.0 },
                168,
            ),
            (
                AisMessage::BaseStation {
                    mmsi: 3_669_001,
                    time_unix: 1.78e9,
                    lat: 28.4,
                    lon: -80.6,
                    accuracy: true,
                    epfd: 7,
                    raim: false,
                },
                168,
            ),
            (static_a(1, "NAME", "CALL", &s), 424),
            (AisMessage::PositionB { common: common(), flags: AisFlags::default() }, 168),
            (static_b(1, "N", "C", &s)[0].clone(), 160),
            (static_b(1, "N", "C", &s)[1].clone(), 168),
        ];
        for (m, n) in cases {
            assert_eq!(bit_len(&m), n, "type {}", m.msg_type());
        }
    }

    #[test]
    fn sixbit_text_and_armor() {
        assert_eq!(sixbit_char('@'), 0);
        assert_eq!(sixbit_char('A'), 1);
        assert_eq!(sixbit_char(' '), 32);
        assert_eq!(sixbit_char('0'), 48);
        assert_eq!(sixbit_char('a'), 1);
        assert_eq!(armor_char(0), '0');
        assert_eq!(armor_char(39), 'W');
        assert_eq!(armor_char(40), '`');
        assert_eq!(armor_char(63), 'w');
    }

    #[test]
    fn multipart_static_report() {
        let s = AisStatic { ship_type: 70, destination: "PORT CANAVERAL".into(), ..AisStatic::default() };
        let lines = sentences("AI", false, 'A', 7, &static_a(366_000_101, "TEST SHIP", "WAB1234", &s));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("!AIVDM,2,1,7,A,"));
        assert!(lines[0].trim_end().ends_with(&format!(",0*{}", &lines[0].trim_end()[lines[0].trim_end().len() - 2..])));
        assert!(lines[1].starts_with("!AIVDM,2,2,7,A,"));
        assert!(lines[1].contains(",2*"), "{}", lines[1]); // 424 bits = 71 chars, 2 fill bits
        for l in &lines {
            assert!(crate::nmea0183::verify(l), "{l}");
            assert!(l.len() + 2 <= 82, "{l}"); // with CR LF
        }
    }

    #[test]
    fn own_ship_uses_vdo() {
        let m = AisMessage::PositionB { common: common(), flags: AisFlags::default() };
        let l = sentences("AI", true, 'B', 0, &m);
        assert_eq!(l.len(), 1);
        assert!(l[0].starts_with("!AIVDO,1,1,,B,"), "{}", l[0]);
        assert_eq!(crate::nmea0183::sentence_id(&l[0]), Some("VDO"));
    }

    #[test]
    fn rot_encoding() {
        assert_eq!(rot_field(None), -128);
        assert_eq!(rot_field(Some(0.0)), 0);
        assert_eq!(rot_field(Some(100.0)), 47);
        assert_eq!(rot_field(Some(-100.0)), -47);
        assert_eq!(rot_field(Some(10_000.0)), 126);
    }
}
