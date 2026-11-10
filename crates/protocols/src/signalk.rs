//! Signal K: delta messages (JSON, SI units) for own-ship measurements and AIS stations, plus
//! the server hello and discovery documents a Signal K client expects.
//!
//! Own ship is `vessels.urn:mrn:imo:mmsi:<mmsi>`; AIS vessels get their own `vessels.` context,
//! aids to navigation `atons.` and base stations `shore.basestations.`.

use chrono::DateTime;
use serde_json::{Value, json};
use sim_core::ais::{ATON_TYPES, AisMessage, ship_type_label};
use sim_core::geo::norm_360;
use sim_sensors::{Emission, Measurement, SensorKind};

/// Signal K specification version these messages follow.
pub const VERSION: &str = "1.7.0";
const MS_PER_KN: f64 = 1852.0 / 3600.0;
const KELVIN: f64 = 273.15;

/// Navigation states, indexed by AIS navigational status.
const STATES: [&str; 16] = [
    "motoring",
    "anchored",
    "not under command",
    "restricted manouverability",
    "constrained by draft",
    "moored",
    "aground",
    "fishing",
    "sailing",
    "hazardous material high speed",
    "hazardous material wing in ground",
    "reserved for future use",
    "reserved for future use",
    "reserved for future use",
    "ais-sart",
    "default",
];

pub fn vessel_context(mmsi: u32) -> String {
    format!("vessels.urn:mrn:imo:mmsi:{mmsi}")
}

fn iso(t: f64) -> String {
    DateTime::from_timestamp_millis((t * 1000.0).round() as i64)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Angle in radians, −π…π.
fn rad180(deg: f64) -> f64 {
    let d = norm_360(deg);
    (if d > 180.0 { d - 360.0 } else { d }).to_radians()
}

fn rad360(deg: f64) -> f64 {
    norm_360(deg).to_radians()
}

/// Rounds to a sensible number of decimals so deltas stay short.
fn r(v: f64, decimals: i32) -> Value {
    let p = 10f64.powi(decimals);
    json!((v * p).round() / p)
}

fn delta(context: &str, source: &str, time: f64, values: Vec<(&str, Value)>) -> String {
    let values: Vec<Value> = values.into_iter().map(|(path, value)| json!({ "path": path, "value": value })).collect();
    json!({
        "context": context,
        "updates": [{ "$source": source, "timestamp": iso(time), "values": values }],
    })
    .to_string()
}

/// Short name of a sensor, used in `$source` and as the output-filter tag.
pub fn sensor_key(kind: SensorKind) -> &'static str {
    match kind {
        SensorKind::Gps => "gps",
        SensorKind::Compass => "compass",
        SensorKind::SpeedLog => "speed_log",
        SensorKind::DepthSounder => "depth_sounder",
        SensorKind::Wind => "wind",
        SensorKind::WaterTemp => "water_temp",
        SensorKind::Weather => "weather",
        SensorKind::Rudder => "rudder",
        SensorKind::Attitude => "attitude",
        SensorKind::Engine => "engine",
        SensorKind::Ais => "ais",
    }
}

/// The delta for one emission, or `None` when it has nothing for Signal K (own-ship AIS
/// reports, which repeat the sensors' data; safety texts).
pub fn encode(e: &Emission, self_context: &str) -> Option<String> {
    let source = format!("marine-sim.{}", sensor_key(e.sensor));
    let t = e.time_unix;
    let values: Vec<(&str, Value)> = match &e.measurement {
        Measurement::Gps { time_unix, lat, lon, sog_kn, cog_deg, mag_var_deg, satellites, hdop, altitude_m } => vec![
            ("navigation.position", json!({ "latitude": r(*lat, 7), "longitude": r(*lon, 7) })),
            ("navigation.courseOverGroundTrue", r(rad360(*cog_deg), 4)),
            ("navigation.speedOverGround", r(sog_kn * MS_PER_KN, 3)),
            ("navigation.magneticVariation", r(rad180(*mag_var_deg), 4)),
            ("navigation.datetime", json!(iso(*time_unix))),
            ("navigation.gnss.satellites", json!(satellites)),
            ("navigation.gnss.horizontalDilution", r(*hdop, 2)),
            ("navigation.gnss.antennaAltitude", r(*altitude_m, 2)),
            ("navigation.gnss.methodQuality", json!("GNSS Fix")),
        ],
        Measurement::Heading { heading_true, deviation_deg, mag_var_deg, rot_dpm } => vec![
            ("navigation.headingTrue", r(rad360(*heading_true), 4)),
            ("navigation.headingMagnetic", r(rad360(heading_true - mag_var_deg), 4)),
            ("navigation.headingCompass", r(rad360(heading_true - mag_var_deg - deviation_deg), 4)),
            ("navigation.magneticDeviation", r(rad180(*deviation_deg), 4)),
            ("navigation.rateOfTurn", r(rot_dpm.to_radians() / 60.0, 5)),
        ],
        Measurement::Speed { stw_kn, .. } => vec![("navigation.speedThroughWater", r(stw_kn * MS_PER_KN, 3))],
        Measurement::Depth { below_transducer_m, transducer_depth_m } => vec![
            ("environment.depth.belowTransducer", r(*below_transducer_m, 2)),
            ("environment.depth.belowSurface", r(below_transducer_m + transducer_depth_m, 2)),
            ("environment.depth.surfaceToTransducer", r(*transducer_depth_m, 2)),
        ],
        Measurement::Wind { awa, aws_kn, twa, tws_kn, twd, .. } => vec![
            ("environment.wind.angleApparent", r(rad180(*awa), 4)),
            ("environment.wind.speedApparent", r(aws_kn * MS_PER_KN, 2)),
            ("environment.wind.angleTrueWater", r(rad180(*twa), 4)),
            ("environment.wind.speedTrue", r(tws_kn * MS_PER_KN, 2)),
            ("environment.wind.directionTrue", r(rad360(*twd), 4)),
        ],
        Measurement::WaterTemp { celsius } => vec![("environment.water.temperature", r(celsius + KELVIN, 2))],
        Measurement::Rudder { starboard_deg } => vec![("steering.rudderAngle", r(starboard_deg.to_radians(), 4))],
        Measurement::Weather { air_temp_c, pressure_hpa, humidity_pct, .. } => vec![
            ("environment.outside.temperature", r(air_temp_c + KELVIN, 2)),
            ("environment.outside.pressure", r(pressure_hpa * 100.0, 0)),
            ("environment.outside.relativeHumidity", r(humidity_pct / 100.0, 3)),
        ],
        Measurement::Attitude { roll_deg, pitch_deg, heave_m } => vec![
            ("navigation.attitude", json!({ "roll": r(roll_deg.to_radians(), 4), "pitch": r(pitch_deg.to_radians(), 4) })),
            ("environment.heave", r(*heave_m, 2)),
        ],
        Measurement::Engine { rpm } => vec![("propulsion.main.revolutions", r(rpm / 60.0, 2))],
        Measurement::Ais { own: true, .. } => return None,
        Measurement::Ais { message, .. } => return ais(message, &source, t),
    };
    Some(delta(self_context, &source, t, values))
}

fn ship_type(t: u8) -> Value {
    json!({ "id": t, "name": ship_type_label(t) })
}

/// Length, beam and antenna position from the AIS dimensions (to bow, stern, port, starboard).
fn dimensions(values: &mut Vec<(&'static str, Value)>, [bow, stern, port, stbd]: [u16; 4]) {
    let (bow, stern, port, stbd) = (f64::from(bow), f64::from(stern), f64::from(port), f64::from(stbd));
    if bow + stern > 0.0 {
        values.push(("design.length", json!({ "overall": bow + stern })));
        values.push(("sensors.ais.fromBow", json!(bow)));
    }
    if port + stbd > 0.0 {
        values.push(("design.beam", json!(port + stbd)));
        values.push(("sensors.ais.fromCenter", json!((port - stbd) / 2.0)));
    }
}

fn ais(m: &AisMessage, source: &str, t: f64) -> Option<String> {
    let mmsi = m.mmsi();
    let mut values: Vec<(&'static str, Value)> = vec![("", json!({ "mmsi": mmsi.to_string() }))];
    let context = match m {
        AisMessage::Aton { .. } => format!("atons.urn:mrn:imo:mmsi:{mmsi}"),
        AisMessage::BaseStation { .. } => format!("shore.basestations.urn:mrn:imo:mmsi:{mmsi}"),
        _ => vessel_context(mmsi),
    };
    fn position(values: &mut Vec<(&'static str, Value)>, c: &sim_core::ais::PositionCommon) {
        values.push(("navigation.position", json!({ "latitude": r(c.lat, 7), "longitude": r(c.lon, 7) })));
        if let Some(cog) = c.cog {
            values.push(("navigation.courseOverGroundTrue", r(rad360(cog), 4)));
        }
        if let Some(sog) = c.sog_kn {
            values.push(("navigation.speedOverGround", r(sog * MS_PER_KN, 3)));
        }
        if let Some(h) = c.heading {
            values.push(("navigation.headingTrue", r(rad360(h), 4)));
        }
    }
    match m {
        AisMessage::PositionA { common, nav_status, rot_dpm, .. } => {
            position(&mut values, common);
            if let Some(rot) = rot_dpm {
                values.push(("navigation.rateOfTurn", r(rot.to_radians() / 60.0, 5)));
            }
            values.push(("navigation.state", json!(STATES[*nav_status as usize & 15])));
            values.push(("sensors.ais.class", json!("A")));
        }
        AisMessage::PositionB { common, .. } => {
            position(&mut values, common);
            values.push(("sensors.ais.class", json!("B")));
        }
        AisMessage::ExtendedB { common, name, ship_type: st, dims, .. } => {
            position(&mut values, common);
            values.push(("", json!({ "name": name.trim() })));
            values.push(("design.aisShipType", ship_type(*st)));
            dimensions(&mut values, *dims);
            values.push(("sensors.ais.class", json!("B")));
        }
        AisMessage::StaticA { imo, call_sign, name, ship_type: st, dims, draught_m, destination, .. } => {
            values.push(("", json!({ "name": name.trim() })));
            values.push(("communication.callsignVhf", json!(call_sign.trim())));
            if *imo > 0 {
                values.push(("registrations.imo", json!(format!("IMO {imo}"))));
            }
            values.push(("design.aisShipType", ship_type(*st)));
            dimensions(&mut values, *dims);
            if *draught_m > 0.0 {
                values.push(("design.draft", json!({ "current": draught_m })));
            }
            if !destination.trim().is_empty() {
                values.push(("navigation.destination.commonName", json!(destination.trim())));
            }
            values.push(("sensors.ais.class", json!("A")));
        }
        AisMessage::BaseStation { lat, lon, .. } => {
            values.push(("navigation.position", json!({ "latitude": r(*lat, 7), "longitude": r(*lon, 7) })));
            values.push(("sensors.ais.class", json!("BASE")));
        }
        AisMessage::Aton { aton_type, name, lat, lon, off_position, virtual_aton, dims, .. } => {
            let label = ATON_TYPES.iter().find(|(id, _)| id == aton_type).map_or("", |(_, l)| *l);
            values.push(("", json!({ "name": name.trim() })));
            values.push(("navigation.position", json!({ "latitude": r(*lat, 7), "longitude": r(*lon, 7) })));
            values.push(("atonType", json!({ "id": aton_type, "name": label })));
            values.push(("virtual", json!(virtual_aton)));
            values.push(("offPosition", json!(off_position)));
            dimensions(&mut values, *dims);
            values.push(("sensors.ais.class", json!("ATON")));
        }
        AisMessage::StaticB24A { name, .. } => values.push(("", json!({ "name": name.trim() }))),
        AisMessage::StaticB24B { ship_type: st, call_sign, dims, .. } => {
            values.push(("design.aisShipType", ship_type(*st)));
            values.push(("communication.callsignVhf", json!(call_sign.trim())));
            dimensions(&mut values, *dims);
        }
        AisMessage::SafetyText { .. } => return None,
    }
    Some(delta(&context, source, t, values))
}

/// Own ship's identity and dimensions, sent periodically (Signal K has no "static" message).
pub struct Identity<'a> {
    pub mmsi: u32,
    pub name: &'a str,
    pub call_sign: &'a str,
    pub length_m: f64,
    pub beam_m: f64,
    pub draft_m: f64,
}

pub fn identity(id: &Identity, time: f64) -> String {
    delta(&vessel_context(id.mmsi), "marine-sim.vessel", time, vec![
        ("", json!({ "name": id.name })),
        ("", json!({ "mmsi": id.mmsi.to_string() })),
        ("communication.callsignVhf", json!(id.call_sign)),
        ("design.length", json!({ "overall": id.length_m })),
        ("design.beam", json!(id.beam_m)),
        ("design.draft", json!({ "maximum": id.draft_m })),
    ])
}

/// The first message a Signal K server sends on a stream connection.
pub fn hello(self_context: &str, time: f64) -> String {
    json!({
        "name": "marine-data-simulator",
        "version": VERSION,
        "self": self_context,
        "roles": ["master", "main"],
        "timestamp": iso(time),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::ais::{AisFlags, PositionCommon};

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn value<'a>(d: &'a Value, path: &str) -> &'a Value {
        d["updates"][0]["values"].as_array().unwrap().iter().find(|v| v["path"] == path).map(|v| &v["value"]).unwrap()
    }

    #[test]
    fn own_ship_measurements_in_si_units() {
        let ctx = vessel_context(366_000_001);
        let e = Emission {
            sensor: SensorKind::Wind,
            talker: "WI".into(),
            time_unix: 1_782_052_215.25,
            measurement: Measurement::Wind { awa: 330.0, aws_kn: 10.0, twa: -50.0, tws_kn: 8.0, twd: 225.0, mag_var_deg: 0.0 },
        };
        let d = parse(&encode(&e, &ctx).unwrap());
        assert_eq!(d["context"], "vessels.urn:mrn:imo:mmsi:366000001");
        assert_eq!(d["updates"][0]["timestamp"], "2026-06-21T14:30:15.250Z");
        assert_eq!(d["updates"][0]["$source"], "marine-sim.wind");
        assert_eq!(value(&d, "environment.wind.angleApparent"), &json!(((-30f64).to_radians() * 1e4).round() / 1e4));
        assert_eq!(value(&d, "environment.wind.speedApparent"), &json!(5.14));
        assert_eq!(value(&d, "environment.wind.directionTrue"), &json!(3.927));

        let e = Emission {
            sensor: SensorKind::WaterTemp,
            talker: "YX".into(),
            time_unix: 0.0,
            measurement: Measurement::WaterTemp { celsius: 15.0 },
        };
        assert_eq!(value(&parse(&encode(&e, &ctx).unwrap()), "environment.water.temperature"), &json!(288.15));
    }

    #[test]
    fn ais_stations_get_their_own_contexts() {
        let c = PositionCommon {
            mmsi: 366_100_002, lat: 28.2, lon: -80.43, sog_kn: Some(12.5), cog: Some(355.0),
            heading: None, second: 1, accuracy: false, raim: false,
        };
        let pos = |message| Emission {
            sensor: SensorKind::Ais,
            talker: "AI".into(),
            time_unix: 0.0,
            measurement: Measurement::Ais { own: false, channel: 'A', seq_id: 0, message },
        };
        let d = parse(&encode(&pos(AisMessage::PositionA { msg_type: 1, common: c, nav_status: 7, rot_dpm: None, time_unix: 0.0 }), "self").unwrap());
        assert_eq!(d["context"], "vessels.urn:mrn:imo:mmsi:366100002");
        assert_eq!(value(&d, "navigation.state"), "fishing");
        assert_eq!(value(&d, "navigation.position")["latitude"], 28.2);
        assert!(d["updates"][0]["values"].as_array().unwrap().iter().all(|v| v["path"] != "navigation.headingTrue"));

        let d = parse(&encode(&pos(AisMessage::PositionB { common: c, flags: AisFlags::default() }), "self").unwrap());
        assert_eq!(value(&d, "sensors.ais.class"), "B");

        let aton = AisMessage::Aton {
            mmsi: 993_669_001, aton_type: 29, name: "SIM APPROACH MARK".into(), lat: 28.4, lon: -80.52,
            accuracy: true, dims: [0; 4], epfd: 7, second: 0, off_position: false, raim: false,
            virtual_aton: true, assigned: false,
        };
        let d = parse(&encode(&pos(aton), "self").unwrap());
        assert_eq!(d["context"], "atons.urn:mrn:imo:mmsi:993669001");
        assert_eq!(value(&d, "virtual"), true);

        let own = Emission {
            measurement: Measurement::Ais { own: true, channel: 'A', seq_id: 0, message: AisMessage::PositionB { common: c, flags: AisFlags::default() } },
            ..pos(AisMessage::SafetyText { mmsi: 1, text: String::new() })
        };
        assert!(encode(&own, "self").is_none());
    }

    #[test]
    fn hello() {
        let h = parse(&super::hello("vessels.urn:mrn:imo:mmsi:1", 0.0));
        assert_eq!(h["self"], "vessels.urn:mrn:imo:mmsi:1");
        assert_eq!(h["version"], VERSION);
    }
}
