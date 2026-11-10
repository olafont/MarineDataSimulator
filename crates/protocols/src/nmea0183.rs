//! NMEA 0183 sentence encoding.
//!
//! Sentences are returned without the trailing `\r\n`; transports add line endings.
//! Formats follow NMEA 0183 v2.3 field layouts (mode indicators included), which the widest
//! range of consumers accept.

use chrono::{DateTime, Datelike, Timelike, Utc};
use sim_core::geo::norm_360;
use sim_sensors::{Emission, Measurement};

const KMH_PER_KN: f64 = 1.852;
const MS_PER_KN: f64 = 1852.0 / 3600.0;
const FT_PER_M: f64 = 1.0 / 0.3048;
const FATHOM_PER_M: f64 = 1.0 / 1.8288;
const INHG_PER_HPA: f64 = 0.029_529_983;

/// XOR of every byte between `$`/`!` and `*`.
pub fn checksum(body: &str) -> u8 {
    body.bytes().fold(0u8, |acc, b| acc ^ b)
}

/// Builds `$<talker><id>,<fields>*<checksum>`.
pub fn sentence(talker: &str, id: &str, fields: &[String]) -> String {
    let body = format!("{talker}{id},{}", fields.join(","));
    format!("${body}*{:02X}", checksum(&body))
}

/// The three-letter sentence formatter of a sentence (e.g. `RMC` for `$GPRMC,...`).
pub fn sentence_id(line: &str) -> Option<&str> {
    let body = line.strip_prefix('$').or_else(|| line.strip_prefix('!'))?;
    let head = body.split(',').next()?;
    // Proprietary sentences ($P...) are identified by the whole head.
    if head.starts_with('P') {
        return Some(head);
    }
    head.get(2..5)
}

/// Verifies the checksum of a complete sentence.
pub fn verify(line: &str) -> bool {
    let line = line.trim_end();
    let Some(body) = line.strip_prefix('$').or_else(|| line.strip_prefix('!')) else {
        return false;
    };
    let Some((body, cs)) = body.rsplit_once('*') else { return false };
    u8::from_str_radix(cs, 16).is_ok_and(|cs| cs == checksum(body))
}

fn f(v: f64, decimals: usize) -> String {
    format!("{v:.decimals$}")
}

fn s(v: &str) -> String {
    v.to_string()
}

/// Converts to UTC, rounded to the centisecond NMEA time fields carry. Rounding in integer
/// centiseconds avoids f64 error at Unix-epoch magnitudes (`.78` printing as `.77`).
fn time_of(t: f64) -> DateTime<Utc> {
    let cs = (t * 100.0).round() as i64;
    let nanos = (cs.rem_euclid(100) * 10_000_000) as u32;
    DateTime::from_timestamp(cs.div_euclid(100), nanos).unwrap_or_default()
}

fn hhmmss(t: &DateTime<Utc>) -> String {
    format!(
        "{:02}{:02}{:02}.{:02}",
        t.hour(),
        t.minute(),
        t.second(),
        t.timestamp_subsec_millis() / 10
    )
}

fn ddmmyy(t: &DateTime<Utc>) -> String {
    format!("{:02}{:02}{:02}", t.day(), t.month(), t.year().rem_euclid(100))
}

/// `ddmm.mmmm,N` latitude fields.
pub fn lat_fields(lat: f64) -> [String; 2] {
    let hemi = if lat < 0.0 { "S" } else { "N" };
    let (deg, min) = deg_min(lat.abs());
    [format!("{deg:02}{min:07.4}"), s(hemi)]
}

/// `dddmm.mmmm,E` longitude fields.
pub fn lon_fields(lon: f64) -> [String; 2] {
    let hemi = if lon < 0.0 { "W" } else { "E" };
    let (deg, min) = deg_min(lon.abs());
    [format!("{deg:03}{min:07.4}"), s(hemi)]
}

/// Splits into whole degrees and minutes, carrying when minutes round up to 60.
fn deg_min(v: f64) -> (u32, f64) {
    let mut deg = v.trunc() as u32;
    let mut min = ((v - v.trunc()) * 60.0 * 10_000.0).round() / 10_000.0;
    if min >= 60.0 {
        deg += 1;
        min -= 60.0;
    }
    (deg, min)
}

/// Variation magnitude and E/W letter.
fn var_fields(var: f64) -> [String; 2] {
    [f(var.abs(), 1), s(if var < 0.0 { "W" } else { "E" })]
}

fn magnetic(true_deg: f64, var: f64) -> f64 {
    norm_360(true_deg - var)
}

/// Magnus-formula dew point, °C.
fn dew_point(t_c: f64, rh_pct: f64) -> f64 {
    let (a, b) = (17.62, 243.12);
    let g = (rh_pct.max(1.0) / 100.0).ln() + a * t_c / (b + t_c);
    b * g / (a - g)
}

/// All sentence formatters this encoder can produce, for filter UIs.
pub const SENTENCES: &[&str] = &[
    "RMC", "GGA", "GLL", "VTG", "ZDA", "DTM", "HDG", "HDT", "ROT", "VHW", "DBT", "DPT", "MWV", "MWD",
    "MTW", "MDA", "XDR", "RSA", "RPM",
];

/// Encodes one instrument emission into NMEA 0183 sentences.
pub fn encode(e: &Emission) -> Vec<String> {
    let t = time_of(e.time_unix);
    let tk = e.talker.as_str();
    match &e.measurement {
        Measurement::Gps { lat, lon, sog_kn, cog_deg, mag_var_deg, satellites, hdop, altitude_m, .. } => {
            let [la, ns] = lat_fields(*lat);
            let [lo, ew] = lon_fields(*lon);
            let [v, vew] = var_fields(*mag_var_deg);
            let time = hhmmss(&t);
            vec![
                sentence(tk, "RMC", &[
                    time.clone(), s("A"), la.clone(), ns.clone(), lo.clone(), ew.clone(),
                    f(*sog_kn, 1), f(*cog_deg, 1), ddmmyy(&t), v, vew, s("A"),
                ]),
                sentence(tk, "GGA", &[
                    time.clone(), la.clone(), ns.clone(), lo.clone(), ew.clone(), s("1"),
                    format!("{satellites:02}"), f(*hdop, 1), f(*altitude_m, 1), s("M"), s("0.0"), s("M"),
                    s(""), s(""),
                ]),
                sentence(tk, "GLL", &[la, ns, lo, ew, time.clone(), s("A"), s("A")]),
                sentence(tk, "VTG", &[
                    f(*cog_deg, 1), s("T"), f(magnetic(*cog_deg, *mag_var_deg), 1), s("M"),
                    f(*sog_kn, 1), s("N"), f(sog_kn * KMH_PER_KN, 1), s("K"), s("A"),
                ]),
                sentence(tk, "ZDA", &[
                    time, format!("{:02}", t.day()), format!("{:02}", t.month()),
                    format!("{:04}", t.year()), s("00"), s("00"),
                ]),
                // Local datum = reference datum = WGS84, no offsets.
                sentence(tk, "DTM", &[
                    s("W84"), s(""), s("0.0"), s("N"), s("0.0"), s("E"), s("0.0"), s("W84"),
                ]),
            ]
        }
        Measurement::Heading { heading_true, deviation_deg, mag_var_deg, rot_dpm } => {
            let [v, vew] = var_fields(*mag_var_deg);
            // HDG carries the raw sensor heading; true = sensor + deviation + variation (E +).
            let sensor = norm_360(heading_true - mag_var_deg - deviation_deg);
            vec![
                sentence(tk, "HDG", &[
                    f(sensor, 1), f(deviation_deg.abs(), 1), s(if *deviation_deg < 0.0 { "W" } else { "E" }), v, vew,
                ]),
                sentence(tk, "HDT", &[f(*heading_true, 1), s("T")]),
                sentence(tk, "ROT", &[f(*rot_dpm, 1), s("A")]),
            ]
        }
        Measurement::Speed { stw_kn, heading_true, mag_var_deg } => vec![sentence(tk, "VHW", &[
            f(*heading_true, 1), s("T"), f(magnetic(*heading_true, *mag_var_deg), 1), s("M"),
            f(*stw_kn, 2), s("N"), f(stw_kn * KMH_PER_KN, 2), s("K"),
        ])],
        Measurement::Depth { below_transducer_m, transducer_depth_m } => vec![
            sentence(tk, "DBT", &[
                f(below_transducer_m * FT_PER_M, 1), s("f"), f(*below_transducer_m, 1), s("M"),
                f(below_transducer_m * FATHOM_PER_M, 1), s("F"),
            ]),
            // Positive offset: distance from transducer to the waterline.
            sentence(tk, "DPT", &[f(*below_transducer_m, 1), f(*transducer_depth_m, 1), s("")]),
        ],
        Measurement::Wind { awa, aws_kn, twa, tws_kn, twd, mag_var_deg } => vec![
            sentence(tk, "MWV", &[f(norm_360(*awa), 1), s("R"), f(*aws_kn, 1), s("N"), s("A")]),
            sentence(tk, "MWV", &[f(norm_360(*twa), 1), s("T"), f(*tws_kn, 1), s("N"), s("A")]),
            mwd(tk, *twd, *mag_var_deg, *tws_kn),
        ],
        // Single rudder: starboard field valid, port field empty/invalid.
        Measurement::Rudder { starboard_deg } => {
            vec![sentence(tk, "RSA", &[f(*starboard_deg, 1), s("A"), s(""), s("V")])]
        }
        // Transducer measurements: pitch (+bow up) and roll (+starboard down), degrees.
        Measurement::Attitude { roll_deg, pitch_deg, .. } => vec![sentence(tk, "XDR", &[
            s("A"), f(*pitch_deg, 1), s("D"), s("PTCH"), s("A"), f(*roll_deg, 1), s("D"), s("ROLL"),
        ])],
        // Source E(ngine), number 1, RPM, pitch (n/a), status.
        Measurement::Ais { own, channel, seq_id, message } => crate::ais::sentences(tk, *own, *channel, *seq_id, message),
        Measurement::Engine { rpm } => vec![sentence(tk, "RPM", &[s("E"), s("1"), f(*rpm, 0), s(""), s("A")])],
        Measurement::WaterTemp { celsius } => vec![sentence(tk, "MTW", &[f(*celsius, 1), s("C")])],
        Measurement::Weather {
            air_temp_c, pressure_hpa, humidity_pct, water_temp_c, twd, tws_kn, mag_var_deg,
        } => vec![
            sentence(tk, "MDA", &[
                f(pressure_hpa * INHG_PER_HPA, 2), s("I"), f(pressure_hpa / 1000.0, 4), s("B"),
                f(*air_temp_c, 1), s("C"), f(*water_temp_c, 1), s("C"),
                f(*humidity_pct, 1), s(""), f(dew_point(*air_temp_c, *humidity_pct), 1), s("C"),
                f(*twd, 1), s("T"), f(magnetic(*twd, *mag_var_deg), 1), s("M"),
                f(*tws_kn, 1), s("N"), f(tws_kn * MS_PER_KN, 1), s("M"),
            ]),
            sentence(tk, "XDR", &[
                s("C"), f(*air_temp_c, 1), s("C"), s("AIRTEMP"),
                s("P"), f(pressure_hpa / 1000.0, 5), s("B"), s("BARO"),
                s("H"), f(*humidity_pct, 1), s("P"), s("HUMIDITY"),
            ]),
        ],
    }
}

fn mwd(tk: &str, twd: f64, var: f64, tws: f64) -> String {
    sentence(tk, "MWD", &[
        f(twd, 1), s("T"), f(magnetic(twd, var), 1), s("M"),
        f(tws, 1), s("N"), f(tws * MS_PER_KN, 1), s("M"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_sensors::SensorKind;

    fn emission(m: Measurement) -> Emission {
        // 2026-11-08 12:34:56.78 UTC
        Emission { sensor: SensorKind::Gps, talker: "GP".into(), time_unix: 1_794_141_296.78, measurement: m }
    }

    fn gps() -> Measurement {
        Measurement::Gps {
            time_unix: 0.0,
            lat: 43.295_123,
            lon: -5.360_456,
            sog_kn: 6.24,
            cog_deg: 181.36,
            mag_var_deg: -1.5,
            satellites: 9,
            hdop: 0.9,
            altitude_m: 2.0,
        }
    }

    #[test]
    fn known_checksum() {
        assert!(verify("$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47"));
        assert!(!verify("$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*48"));
    }

    #[test]
    fn coordinates() {
        assert_eq!(lat_fields(43.295_123), ["4317.7074".to_string(), "N".into()]);
        assert_eq!(lon_fields(-5.360_456), ["00521.6274".to_string(), "W".into()]);
        // 59.99999' must carry into the degrees, not print as 60.0000.
        assert_eq!(lat_fields(10.999_999_9)[0], "1100.0000");
    }

    #[test]
    fn rmc_golden() {
        let out = encode(&emission(gps()));
        assert_eq!(
            out[0],
            "$GPRMC,123456.78,A,4317.7074,N,00521.6274,W,6.2,181.4,081126,1.5,W,A*39"
        );
        assert!(out.iter().all(|l| verify(l)), "{out:?}");
    }

    /// Round-trips through an independent parser (the `nmea` crate).
    #[test]
    fn independent_parser_accepts_output() {
        let all = [
            gps(),
            Measurement::Heading { heading_true: 359.95, deviation_deg: -1.0, mag_var_deg: 2.0, rot_dpm: -12.3 },
            Measurement::Attitude { roll_deg: -12.5, pitch_deg: 3.2, heave_m: 0.4 },
            Measurement::Engine { rpm: 1850.0 },
            Measurement::Speed { stw_kn: 5.5, heading_true: 10.0, mag_var_deg: 2.0 },
            Measurement::Depth { below_transducer_m: 12.34, transducer_depth_m: 0.5 },
            Measurement::Wind { awa: -35.0, aws_kn: 15.0, twa: -50.0, tws_kn: 11.0, twd: 225.0, mag_var_deg: 0.0 },
            Measurement::WaterTemp { celsius: 15.2 },
            Measurement::Weather {
                air_temp_c: 18.0, pressure_hpa: 1013.25, humidity_pct: 70.0, water_temp_c: 15.0,
                twd: 225.0, tws_kn: 11.0, mag_var_deg: 0.0,
            },
        ];
        let mut parsed = 0;
        for m in all {
            for line in encode(&emission(m)) {
                assert!(verify(&line), "{line}");
                let id = sentence_id(&line).unwrap();
                // Sentences the `nmea` crate implements must parse cleanly.
                if ["RMC", "GGA", "GLL", "VTG", "ZDA", "HDT", "VHW", "DPT", "MWV", "MTW", "MDA"].contains(&id) {
                    nmea::parse_str(&line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
                    parsed += 1;
                }
            }
        }
        assert_eq!(parsed, 12);
    }

    #[test]
    fn hdg_reports_sensor_heading_with_deviation() {
        let out = encode(&emission(Measurement::Heading {
            heading_true: 100.0, deviation_deg: -3.0, mag_var_deg: 2.0, rot_dpm: 0.0,
        }));
        // sensor = 100 - 2 - (-3) = 101; true = 101 + (-3) + 2 = 100.
        assert!(out[0].starts_with("$GPHDG,101.0,3.0,W,2.0,E*"), "{}", out[0]);
        assert!(out[1].starts_with("$GPHDT,100.0,T*"));
    }

    #[test]
    fn attitude_and_engine() {
        let out = encode(&emission(Measurement::Attitude { roll_deg: -12.5, pitch_deg: 3.2, heave_m: 0.0 }));
        assert!(out[0].starts_with("$GPXDR,A,3.2,D,PTCH,A,-12.5,D,ROLL*"), "{}", out[0]);
        let out = encode(&emission(Measurement::Engine { rpm: 1850.4 }));
        assert!(out[0].starts_with("$GPRPM,E,1,1850,,A*"), "{}", out[0]);
    }

    #[test]
    fn wind_angles_are_0_to_360() {
        let out = encode(&emission(Measurement::Wind {
            awa: -35.0, aws_kn: 15.0, twa: -50.0, tws_kn: 11.0, twd: 225.0, mag_var_deg: 0.0,
        }));
        assert!(out[0].starts_with("$GPMWV,325.0,R,15.0,N,A*"));
        assert!(out[1].starts_with("$GPMWV,310.0,T,11.0,N,A*"));
    }
}
