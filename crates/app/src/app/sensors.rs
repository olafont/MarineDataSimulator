//! Sensors section of the right panel: rate, talker, noise and errors of each sensor.

use egui::{Id, RichText, Ui};
use engine::Snapshot;
use engine::sim_sensors::SensorKind;

use super::SimApp;
use crate::ui::section;
use crate::ui::form::{Form, Num, Width, form};
use crate::t;
use crate::ui::i18n::tr;

impl SimApp {
    pub(super) fn sensors(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "sensors", t!("Sensors"), true, |ui| {
            for cfg in &snap.sensors {
                let mut c = cfg.clone();
                let changed = std::cell::Cell::new(false);
                let id = Id::unique(("sensor", c.kind));
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut c.enabled, RichText::new(tr(c.kind.label())).strong()).changed() {
                        changed.set(true);
                    }
                    ui.weak(sentences_of(c.kind));
                });
                let e = &mut self.edits;
                egui::CollapsingHeader::new(t!("settings")).id_salt(id.with("more")).show(ui, |ui| {
                    form(ui, id.with("form"), |f| {
                        let mut num = |f: &mut Form<'_>, label: &str, key: &str, v: &mut f64, n: Num<'_>| {
                            if f.number(label, e, id.with(key), v, n) {
                                changed.set(true);
                            }
                        };
                        let ais = c.kind == SensorKind::Ais;
                        if !ais {
                            num(f, t!("Rate"), "rate", &mut c.rate_hz, Num::new(0.01..=50.0, 0.05, " Hz"));
                        }
                        let mut talker = c.talker.clone();
                        if f.text(t!("Talker"), &mut talker, Width::Short, 2).changed()
                            && talker.len() == 2
                            && talker.chars().all(|ch| ch.is_ascii_uppercase())
                        {
                            c.talker = talker;
                            changed.set(true);
                        }
                        num(f, t!("Noise"), "noise", &mut c.noise_scale, Num::new(0.0..=5.0, 0.02, " ×"));
                        if ais {
                            return; // intervals are per station (AIS panel)
                        }
                        num(f, t!("Latency"), "lat", &mut c.latency_s, Num::new(0.0..=10.0, 0.02, " s"));
                        num(f, t!("Resolution"), "res", &mut c.resolution, Num::new(0.0..=100.0, 0.01, ""));
                        match c.kind {
                            SensorKind::Compass | SensorKind::Wind | SensorKind::Rudder | SensorKind::Attitude => {
                                let label = if c.kind == SensorKind::Wind { t!("Vane offset") } else { t!("Bias") };
                                num(f, label, "bias", &mut c.angle_bias_deg, Num::new(-45.0..=45.0, 0.1, " °"));
                            }
                            SensorKind::DepthSounder => {
                                num(f, t!("Transducer depth"), "xdcr", &mut c.transducer_depth_m, Num::new(0.0..=20.0, 0.05, " m"));
                            }
                            SensorKind::Gps => {
                                num(f, t!("Antenna altitude"), "alt", &mut c.antenna_altitude_m, Num::new(-100.0..=10000.0, 0.5, " m"));
                            }
                            _ => {}
                        }
                        if matches!(c.kind, SensorKind::SpeedLog | SensorKind::Wind) {
                            num(f, t!("Calibration"), "scale", &mut c.speed_scale, Num::new(0.5..=1.5, 0.002, " ×"));
                        }
                        if c.kind == SensorKind::Compass {
                            let hint = t!("A + B·sin h + C·cos h + D·sin 2h + E·cos 2h (degrees, h magnetic). Reported in HDG; HDT is corrected.");
                            f.row_hint(t!("Deviation"), Some(hint), |ui| {
                                for (key, v) in [
                                    ("A", &mut c.deviation.a),
                                    ("B", &mut c.deviation.b),
                                    ("C", &mut c.deviation.c),
                                    ("D", &mut c.deviation.d),
                                    ("E", &mut c.deviation.e),
                                ] {
                                    ui.weak(key);
                                    let live = *v;
                                    let n = Num::new(-20.0..=20.0, 0.05, "");
                                    if let Some(x) = e.value(ui, id.with(("dev", key)), live, |ui, x| ui.add(n.drag(x))) {
                                        *v = x;
                                        changed.set(true);
                                    }
                                }
                            });
                        }
                    });
                });
                if changed.get() {
                    self.engine.set_sensor(c);
                }
            }
            });
    }
}

fn sentences_of(kind: SensorKind) -> &'static str {
    match kind {
        SensorKind::Gps => "RMC GGA GLL VTG ZDA DTM",
        SensorKind::Compass => "HDG HDT ROT",
        SensorKind::SpeedLog => "VHW",
        SensorKind::DepthSounder => "DBT DPT",
        SensorKind::Wind => "MWV MWD",
        SensorKind::WaterTemp => "MTW",
        SensorKind::Weather => "MDA XDR",
        SensorKind::Rudder => "RSA",
        SensorKind::Attitude => t!("XDR (pitch, roll)"),
        SensorKind::Engine => "RPM",
        SensorKind::Ais => t!("VDM VDO (msgs 1 4 5 14 18 19 21 24)"),
    }
}
