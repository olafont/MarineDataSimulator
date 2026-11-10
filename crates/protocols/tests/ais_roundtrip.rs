//! Our AIS sentences decoded by an independent parser (the `ais` crate).

use ais::messages::AisMessage as Decoded;
use ais::{AisFragments, AisParser};
use protocols::ais::sentences;
use sim_core::ais::{
    AisClass, AisFlags, AisMessage, AisStatic, AisTarget, AisTargetConfig, PositionCommon, TargetMotion, static_a,
    static_b,
};
use sim_core::LatLon;

fn decode(lines: &[String]) -> Decoded {
    let mut p = AisParser::new();
    for (i, l) in lines.iter().enumerate() {
        let r = p.parse(l.trim_end().as_bytes(), true).unwrap_or_else(|e| panic!("{l}: {e:?}"));
        match r {
            AisFragments::Complete(s) => {
                assert_eq!(i + 1, lines.len());
                return s.message.expect("decoded message");
            }
            AisFragments::Incomplete(_) => assert!(i + 1 < lines.len()),
        }
    }
    panic!("incomplete: {lines:?}")
}

fn roundtrip(msg: &AisMessage) -> Decoded {
    decode(&sentences("AI", false, 'A', 3, msg))
}

fn close(a: Option<f32>, b: f64, tol: f64) -> bool {
    a.is_some_and(|a| (a as f64 - b).abs() < tol)
}

fn common() -> PositionCommon {
    PositionCommon {
        mmsi: 366_000_101,
        lat: 28.401_234,
        lon: -80.551_234,
        sog_kn: Some(12.3),
        cog: Some(201.5),
        heading: Some(200.0),
        second: 34,
        accuracy: true,
        raim: true,
    }
}

#[test]
fn position_report_class_a() {
    let m = AisMessage::PositionA { msg_type: 1, common: common(), nav_status: 7, rot_dpm: Some(-20.0), time_unix: 1.78e9 };
    let Decoded::PositionReport(r) = roundtrip(&m) else { panic!() };
    assert_eq!(r.message_type, 1);
    assert_eq!(r.mmsi, 366_000_101);
    assert!(close(r.latitude, 28.401_234, 1e-5), "{:?}", r.latitude);
    assert!(close(r.longitude, -80.551_234, 1e-5), "{:?}", r.longitude);
    assert!(close(r.speed_over_ground, 12.3, 1e-3));
    assert!(close(r.course_over_ground, 201.5, 1e-3));
    assert_eq!(r.true_heading, Some(200));
    assert_eq!(r.timestamp, 34);
    assert!(r.raim);
    assert_eq!(format!("{:?}", r.navigation_status), "Some(EngagedInFishing)");
    let rot = format!("{:?}", r.rate_of_turn);
    assert!(rot.contains("-21"), "{rot}"); // 4.733·√20 = 21.2
}

#[test]
fn not_available_values() {
    let c = PositionCommon { sog_kn: None, cog: None, heading: None, second: 60, ..common() };
    let m = AisMessage::PositionA { msg_type: 1, common: c, nav_status: 15, rot_dpm: None, time_unix: 0.0 };
    let Decoded::PositionReport(r) = roundtrip(&m) else { panic!() };
    assert_eq!(r.speed_over_ground, None);
    assert_eq!(r.course_over_ground, None);
    assert_eq!(r.true_heading, None);
    assert_eq!(r.rate_of_turn, None);
}

#[test]
fn southern_and_eastern_hemispheres() {
    let c = PositionCommon { lat: -33.856_7, lon: 151.215_3, ..common() };
    let m = AisMessage::PositionB { common: c, flags: AisFlags::default() };
    let Decoded::StandardClassBPositionReport(r) = roundtrip(&m) else { panic!() };
    assert!(close(r.latitude, -33.856_7, 1e-5));
    assert!(close(r.longitude, 151.215_3, 1e-5));
}

#[test]
fn base_station() {
    // 2026-06-21T14:30:15Z
    let t = 1_782_052_215.0;
    let m = AisMessage::BaseStation { mmsi: 3_669_001, time_unix: t, lat: 28.41, lon: -80.6, accuracy: true, epfd: 7, raim: false };
    let Decoded::BaseStationReport(r) = roundtrip(&m) else { panic!() };
    assert_eq!(r.mmsi, 3_669_001);
    assert_eq!((r.year, r.month, r.day), (Some(2026), Some(6), Some(21)));
    assert_eq!((r.hour, r.minute, r.second), (14, Some(30), Some(15)));
    assert!(close(r.latitude, 28.41, 1e-5));
}

#[test]
fn static_and_voyage() {
    let s = AisStatic {
        imo: 9_123_456,
        ship_type: 70,
        to_bow: 150,
        to_stern: 50,
        to_port: 15,
        to_starboard: 17,
        draught_m: 9.4,
        destination: "PORT CANAVERAL".into(),
        eta: "06-21 14:30".into(),
        ..AisStatic::default()
    };
    let lines = sentences("AI", false, 'B', 4, &static_a(366_000_101, "Atlantic trader", "WAB1234", &s));
    assert_eq!(lines.len(), 2);
    let Decoded::StaticAndVoyageRelatedData(r) = decode(&lines) else { panic!() };
    assert_eq!(r.mmsi, 366_000_101);
    assert_eq!(r.imo_number, 9_123_456);
    assert_eq!(r.callsign.trim(), "WAB1234");
    assert_eq!(r.vessel_name.trim(), "ATLANTIC TRADER");
    assert_eq!(r.destination.trim(), "PORT CANAVERAL");
    assert_eq!((r.dimension_to_bow, r.dimension_to_stern, r.dimension_to_port, r.dimension_to_starboard), (150, 50, 15, 17));
    assert_eq!((r.eta_month_utc, r.eta_day_utc, r.eta_hour_utc, r.eta_minute_utc), (Some(6), Some(21), 14, Some(30)));
    assert!((r.draught - 9.4).abs() < 1e-3);
    assert_eq!(format!("{:?}", r.ship_type), "Some(Cargo)");
}

#[test]
fn class_b_reports() {
    let m = AisMessage::PositionB { common: common(), flags: AisFlags { display: true, ..AisFlags::default() } };
    let Decoded::StandardClassBPositionReport(r) = roundtrip(&m) else { panic!() };
    assert_eq!(r.mmsi, 366_000_101);
    assert!(close(r.speed_over_ground, 12.3, 1e-3));
    assert!(r.has_display);
    assert!(r.has_dsc && r.whole_band);
    assert_eq!(format!("{:?}", r.cs_unit), "CarrierSense");

    let s = AisStatic { ship_type: 36, vendor_id: "SIM".into(), ..AisStatic::default() }.with_hull(12.0, 4.0);
    let v = static_b(366_000_202, "Blue moon", "WYB1", &s);
    let Decoded::StaticDataReport(a) = roundtrip(&v[0]) else { panic!() };
    assert_eq!(format!("{:?}", a.message_part), r#"PartA { vessel_name: "BLUE MOON" }"#);
    let Decoded::StaticDataReport(b) = roundtrip(&v[1]) else { panic!() };
    let part_b = format!("{:?}", b.message_part);
    assert!(part_b.contains("Sailing") && part_b.contains("\"WYB1\"") && part_b.contains("dimension_to_bow: 8"), "{part_b}");
}

#[test]
fn extended_class_b() {
    let m = AisMessage::ExtendedB {
        common: common(),
        name: "BLUE MOON".into(),
        ship_type: 37,
        dims: [8, 4, 2, 2],
        epfd: 1,
        dte_unavailable: false,
        assigned: false,
    };
    let lines = sentences("AI", false, 'A', 1, &m);
    assert_eq!(lines.len(), 1); // 312 bits = 52 characters
    let Decoded::ExtendedClassBPositionReport(r) = decode(&lines) else { panic!() };
    assert_eq!(r.name.trim(), "BLUE MOON");
    assert_eq!(r.dimension_to_bow, 8);
    assert!(close(r.latitude, 28.401_234, 1e-5));
}

#[test]
fn aid_to_navigation_with_name_extension() {
    let m = AisMessage::Aton {
        mmsi: 993_669_001,
        aton_type: 30,
        name: "CANAVERAL APPROACH BUOY 4".into(),
        lat: 28.35,
        lon: -80.45,
        accuracy: true,
        dims: [1, 1, 1, 1],
        epfd: 7,
        second: 12,
        off_position: true,
        raim: false,
        virtual_aton: true,
        assigned: false,
    };
    let Decoded::AidToNavigationReport(r) = roundtrip(&m) else { panic!() };
    assert_eq!(r.mmsi, 993_669_001);
    // The decoder ignores the name extension; the first 20 characters are in the base field.
    assert_eq!(r.name.trim(), "CANAVERAL APPROACH B");
    // 272 bits + 5 extension characters (30 bits), padded to a byte boundary.
    assert_eq!(protocols::ais::bit_len(&m), 304);
    assert!(r.off_position && r.virtual_aid);
    assert!(close(r.longitude, -80.45, 1e-5));
    assert_eq!(r.utc_second, 12);
}

#[test]
fn aton_reference_sentence_reencodes_identically() {
    // From the `ais` crate's documentation.
    let reference = "!AIVDM,1,1,,B,E>kb9O9aS@7PUh10dh19@;0Tah2cWrfP:l?M`00003vP100,0*01";
    let Decoded::AidToNavigationReport(r) = decode(&[reference.to_string()]) else { panic!() };
    let m = AisMessage::Aton {
        mmsi: r.mmsi,
        aton_type: aton_type_number(&format!("{:?}", r.aid_type)),
        name: r.name.to_string(),
        lat: r.latitude.unwrap() as f64,
        lon: r.longitude.unwrap() as f64,
        accuracy: format!("{:?}", r.accuracy) == "DGPS",
        dims: [r.dimension_to_bow, r.dimension_to_stern, r.dimension_to_port, r.dimension_to_starboard],
        epfd: epfd_number(&format!("{:?}", r.epfd_type)),
        second: r.utc_second,
        off_position: r.off_position,
        raim: r.raim,
        virtual_aton: r.virtual_aid,
        assigned: r.assigned_mode,
    };
    assert_eq!(r.regional_reserved, 0);
    let line = &sentences("AI", false, 'B', 0, &m)[0];
    let ours = unarmor(line.split(',').nth(5).unwrap());
    let theirs = unarmor(reference.split(',').nth(5).unwrap());
    // The reference transmitter pads 272 bits to 282 with zeros.
    assert!(theirs[272..].chars().all(|c| c == '0'));
    // Every field matches bit for bit, except the coordinates (bits 164..219), which the
    // decoder hands back as f32 and may be off by one unit of 1/10 000 minute.
    assert_eq!(&ours[..164], &theirs[..164]);
    assert_eq!(&ours[219..272], &theirs[219..272]);
    for (a, n) in [(164, 28), (192, 27)] {
        let v = |s: &str| i64::from_str_radix(&s[a..a + n], 2).unwrap();
        assert!((v(&ours) - v(&theirs)).abs() <= 2, "coordinate bits {a}");
    }
}

fn unarmor(payload: &str) -> String {
    payload
        .bytes()
        .map(|c| {
            let v = c - 48;
            format!("{:06b}", if v > 40 { v - 8 } else { v })
        })
        .collect()
}

fn aton_type_number(s: &str) -> u8 {
    // Only the type used in the reference sentence.
    match s {
        "Some(ReferencePoint)" => 1,
        "Some(Racon)" => 2,
        "Some(FixedStructure)" => 3,
        "Some(BeaconSpecialMark)" => 19,
        other => panic!("unexpected aid type {other}"),
    }
}

fn epfd_number(s: &str) -> u8 {
    match s {
        "None" => 0,
        "Some(Gps)" => 1,
        "Some(Surveyed)" => 7,
        other => panic!("unexpected epfd {other}"),
    }
}

#[test]
fn safety_text() {
    let m = AisMessage::SafetyText { mmsi: 970_000_001, text: "SART ACTIVE".into() };
    let Decoded::SafetyRelatedBroadcastMessage(r) = roundtrip(&m) else { panic!() };
    assert_eq!(r.text.trim(), "SART ACTIVE");
}

#[test]
fn every_class_produces_decodable_messages() {
    for &class in AisClass::ALL {
        let mut cfg = AisTargetConfig::new(class, 1, LatLon::new(28.4, -80.5));
        cfg.safety_text = "TEST".into();
        cfg.send_msg19 = true;
        if class.is_mobile() {
            cfg.motion = TargetMotion::Course { cog: 90.0, sog_kn: 8.0 };
        }
        let t = AisTarget::new(1, cfg);
        let mut msgs = t.position_messages(1.78e9);
        msgs.extend(t.static_messages());
        msgs.extend(t.safety_text());
        assert!(!msgs.is_empty());
        for m in msgs {
            let lines = sentences("AI", false, 'A', 0, &m);
            for l in &lines {
                assert!(protocols::nmea0183::verify(l), "{l}");
                assert!(l.len() + 2 <= 82, "{} chars + CR LF: {l}", l.len());
            }
            let d = decode(&lines);
            assert!(format!("{d:?}").contains(&m.mmsi().to_string()), "{class:?} {d:?}");
        }
    }
}
