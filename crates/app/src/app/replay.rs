//! Replay section of the right panel: plays a recorded log to the outputs.

use egui::{ComboBox, Ui};
use engine::{ReplayCommand, Snapshot};

use super::{REPLAY_SPEEDS, SimApp};
use crate::ui::section;
use crate::ui::theme::tokens;
use crate::ui::widgets::format_time;
use crate::t;
use crate::ui::i18n::tr;

impl SimApp {
    pub(super) fn replay(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let title = match &snap.replay {
            Some(r) if r.playing => t!("Replay (playing)"),
            Some(_) => t!("Replay (loaded)"),
            None => t!("Replay"),
        };
        section(ui, "replay", title, false, |ui| {
            ui.horizontal(|ui| {
                let tip = t!("NMEA 0183 / AIS log; TAG-block or sentence times set the pace");
                if ui.button(t!("Open log…")).on_hover_text(tip).clicked() {
                    self.open_log();
                }
                if snap.replay.is_some() && ui.button(t!("Close")).clicked() {
                    self.engine.replay(ReplayCommand::Close);
                }
            });
            let Some(r) = &snap.replay else {
                ui.weak(t!("Plays a recorded log to the outputs. Record one with ⏺ in Outputs."));
                return;
            };
            ui.horizontal_wrapped(|ui| {
                ui.monospace(&r.name);
                ui.weak(crate::tf!("{n} lines · {d} · {timing}", n = r.lines, d = format_duration(r.duration), timing = tr(r.timing.label())));
            });
            ui.horizontal(|ui| {
                if ui.button(if r.playing { t!("⏸ Pause") } else { t!("▶ Play") }).clicked() {
                    self.engine.replay(ReplayCommand::Play(!r.playing));
                }
                if ui.button("⏮").on_hover_text(t!("Back to the start")).clicked() {
                    self.engine.replay(ReplayCommand::Seek(0.0));
                }
                let speed = ComboBox::from_id_salt("replay_speed").selected_text(format!("{}×", r.speed));
                speed.width(tokens(ui).combo_narrow).show_ui(ui, |ui| {
                    for &s in REPLAY_SPEEDS {
                        if ui.selectable_label(s == r.speed, format!("{s}×")).clicked() {
                            self.engine.replay(ReplayCommand::Speed(s));
                        }
                    }
                });
                let mut looped = r.looped;
                if ui.checkbox(&mut looped, t!("Loop")).changed() {
                    self.engine.replay(ReplayCommand::Loop(looped));
                }
            });
            let mut pos = r.position;
            let slider = egui::Slider::new(&mut pos, 0.0..=r.duration.max(0.001)).show_value(false);
            if ui.add(slider).changed() {
                self.engine.replay(ReplayCommand::Seek(pos));
            }
            ui.horizontal(|ui| {
                ui.monospace(format!("{} / {}", format_duration(r.position), format_duration(r.duration)));
                if let Some(t) = r.time_unix {
                    ui.weak(format_time(t));
                }
                ui.weak(crate::tf!("line {i} of {n}", i = r.cursor, n = r.lines));
            });
            let mut mute = r.mute_simulation;
            if ui.checkbox(&mut mute, t!("Mute simulated data while a log is loaded")).changed() {
                self.engine.replay(ReplayCommand::MuteSimulation(mute));
            }
        });
    }
}

fn format_duration(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}
