//! The ribbon: tabs of commands above the chart, with the simulation status on the right.
//!
//! Every command here does what an existing control in a panel does; the ribbon only gathers
//! them by task. A group's `⏷` menu holds its less common commands and settings.

use egui::{ComboBox, DragValue, RichText, Ui};
use engine::scenario;
use engine::sim_core::ais::AisClass;
use engine::sim_core::{Command, Propulsion, Steering};
use engine::transports::DataFormat;
use engine::{ReplayCommand, Snapshot};

use super::outputs::status_badge;
use super::{REPLAY_SPEEDS, SimApp, SteerMode, TIME_SCALES, steering_rows};
use crate::ui::form::form;
use crate::ui::ribbon::{self, Icon, big, big_enabled, big_toggle, column, group, group_menu, small, small_toggle};
use crate::ui::theme::{self, Palette, tokens};
use crate::ui::title_bar;
use crate::ui::widgets::format_time;
use crate::t;
use crate::ui::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Scenario,
    Chart,
    Traffic,
    Data,
    View,
}

const TABS: &[(Tab, &str)] = &[
    (Tab::Home, "Home"),
    (Tab::Scenario, "Scenario"),
    (Tab::Chart, "Chart"),
    (Tab::Traffic, "AIS traffic"),
    (Tab::Data, "Data"),
    (Tab::View, "View"),
];

/// Which panels are shown around the chart.
#[derive(Debug, Clone, Copy)]
pub struct Panels {
    pub controls: bool,
    pub io: bool,
    pub monitor: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self { controls: true, io: true, monitor: true }
    }
}

impl SimApp {
    /// Shown or not: on macOS the tabs are in the title bar, and a hidden ribbon has nothing left.
    pub(super) fn ribbon_shown(&self) -> bool {
        !(title_bar::integrated() && self.ribbon.collapsed)
    }

    pub(super) fn ribbon(&mut self, ui: &mut Ui, snap: &Snapshot) {
        ribbon::card(ui, |ui| {
            let mut state = self.ribbon;
            if !title_bar::integrated() {
                ribbon::tabs(ui, &mut state, TABS, |ui| self.status(ui, snap));
            }
            self.ribbon = state;
            if state.collapsed {
                return;
            }
            ribbon::band(ui, |ui| match state.tab {
                Tab::Home => self.home_tab(ui, snap),
                Tab::Scenario => self.scenario_tab(ui),
                Tab::Chart => self.chart_tab(ui, snap),
                Tab::Traffic => self.traffic_tab(ui, snap),
                Tab::Data => self.data_tab(ui, snap),
                Tab::View => self.view_tab(ui),
            });
        });
    }

    /// The title bar (macOS): the commands used most, the ribbon's tabs and the status.
    pub(super) fn title_bar(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let t = tokens(ui);
        let quick = |ui: &mut Ui, icon: Icon, tip: &str| {
            let text = RichText::new(icon.glyph).size(t.font_body + 2.0).color(icon.color(t));
            ui.add(egui::Button::new(text).frame_when_inactive(false)).on_hover_text(tip).clicked()
        };
        if quick(ui, Icon::blue("📂"), t!("Open a scenario…")) {
            self.open_dialog();
        }
        if quick(ui, Icon::blue("💾"), t!("Save the scenario as…")) {
            self.save_dialog();
        }
        ui.separator();
        let paused = snap.sim.paused;
        let (icon, tip) = if paused { (Icon::green("▶"), t!("Run")) } else { (Icon::amber("⏸"), t!("Pause")) };
        if quick(ui, icon, tip) {
            self.engine.send(Command::SetPaused(!paused));
        }
        if quick(ui, Icon::blue("⟲"), t!("Reset to the scenario's initial state")) {
            self.reset();
        }
        ui.separator();
        let mut state = self.ribbon;
        ribbon::tab_pills(ui, &mut state, TABS);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(t.space_md);
            ribbon::collapse_button(ui, &mut state);
            if quick(ui, Icon::purple("⚙"), t!("Preferences (⌘,)")) {
                self.prefs.window_open = true;
            }
            ui.add_space(t.space_md);
            self.status(ui, snap);
        });
        self.ribbon = state;
    }

    /// Scenario name, simulated time, data rate and warnings, laid out right to left.
    fn status(&self, ui: &mut Ui, snap: &Snapshot) {
        let t = tokens(ui);
        // Right to left.
        ui.label(crate::tf!("{n} lines/s", n = format!("{:.0}", snap.sentences_per_sec)));
        ui.separator();
        ui.monospace(format_time(snap.sim.time_unix));
        ui.separator();
        let r = ui.weak(self.scenario_label());
        if let Some(p) = &self.scenario_path {
            r.on_hover_text(p.display().to_string());
        }
        if snap.sim.grounded {
            ui.separator();
            ui.label(RichText::new(t!("AGROUND")).strong().color(t.error))
                .on_hover_text(t!("Water depth is less than the draft. Steer towards deeper water or move the vessel."));
        }
        if snap.sim.paused {
            ui.separator();
            ui.label(RichText::new(t!("PAUSED")).strong().color(t.warn));
        }
    }

    fn home_tab(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let sim = &snap.sim;
        group_menu(
            ui,
            self,
            t!("Simulation"),
            |ui, app: &mut Self| {
                let paused = sim.paused;
                let (icon, label) = if paused { (Icon::green("▶"), t!("Run")) } else { (Icon::amber("⏸"), t!("Pause")) };
                if big(ui, icon, label).on_hover_text(t!("Start or stop simulated time")).clicked() {
                    app.engine.send(Command::SetPaused(!paused));
                }
                if big(ui, Icon::blue("⟲"), t!("Reset")).on_hover_text(t!("Reload the scenario's initial state")).clicked() {
                    app.reset();
                }
                column(ui, |ui| {
                    ui.weak(t!("Time scale"));
                    let ts = sim.time_scale;
                    ComboBox::from_id_salt("ribbon_time_scale")
                        .width(tokens(ui).combo_narrow)
                        .selected_text(format!("{ts}×"))
                        .show_ui(ui, |ui| {
                            for &s in TIME_SCALES {
                                if ui.selectable_label((s - ts).abs() < 1e-9, format!("{s}×")).clicked() {
                                    app.engine.send(Command::SetTimeScale(s));
                                }
                            }
                        });
                    if small(ui, Icon::blue("⏵"), t!("Real time")).clicked() {
                        app.engine.send(Command::SetTimeScale(1.0));
                    }
                });
            },
            |ui, app: &mut Self| {
                ui.weak(t!("Time scale"));
                for &s in TIME_SCALES {
                    if ui.selectable_label((s - sim.time_scale).abs() < 1e-9, format!("{s}×")).clicked() {
                        app.engine.send(Command::SetTimeScale(s));
                    }
                }
            },
        );
        group_menu(
            ui,
            self,
            t!("Steering"),
            |ui, app: &mut Self| {
                let mode = SteerMode::of(&sim.steering);
                for (m, icon, label, tip) in [
                    (SteerMode::Heading, Icon::blue("⬆"), t!("Heading"), t!("Hold the current heading")),
                    (SteerMode::WindAngle, Icon::amber("💨"), t!("Wind angle"), t!("Hold the current true wind angle")),
                    (SteerMode::Route, Icon::red("🚩"), t!("Route"), t!("Follow the waypoints")),
                ] {
                    if big_toggle(ui, mode == m, icon, label).on_hover_text(tip).clicked() {
                        app.steer(sim, m);
                    }
                }
            },
            |ui, app: &mut Self| app.steering_menu(ui, snap),
        );
        if sim.vessel_cfg.propulsion == Propulsion::Sail {
            group(ui, t!("Propulsion"), |ui| {
                let on = sim.engine_on;
                let tip = if on { t!("Motoring: switch the engine off to sail") } else { t!("Sailing: switch the engine on") };
                if big_toggle(ui, on, Icon::teal("⚙"), t!("Engine")).on_hover_text(tip).clicked() {
                    self.engine.send(Command::SetEngine(!on));
                }
            });
        }
    }

    /// Target speed and the target of the current steering mode.
    fn steering_menu(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let sim = &snap.sim;
        let send = form(ui, "steering_menu", |f| {
            let send = steering_rows(f, &mut self.edits, sim, "menu");
            if let Steering::Route { waypoints, active, .. } = &sim.steering {
                let text = match waypoints.len() {
                    0 => t!("no waypoints").to_string(),
                    n => crate::tf!("to {i} of {n}", i = active + 1, n = n),
                };
                f.readout(t!("Route"), text);
            }
            send
        });
        for c in send {
            self.engine.send(c);
        }
    }

    fn scenario_tab(&mut self, ui: &mut Ui) {
        group(ui, t!("File"), |ui| {
            if big(ui, Icon::blue("📂"), t!("Open…")).on_hover_text(t!("Open a scenario file (.toml)")).clicked() {
                self.open_dialog();
            }
            if big(ui, Icon::blue("💾"), t!("Save as…")).on_hover_text(t!("Save the current state as a scenario file")).clicked() {
                self.save_dialog();
            }
            if big(ui, Icon::blue("⟲"), t!("Reset")).on_hover_text(t!("Reload the scenario's initial state")).clicked() {
                self.reset();
            }
        });
        group(ui, t!("Built-in scenarios"), |ui| {
            for (name, _) in scenario::BUILTIN {
                let label = name.trim_end_matches(".toml").replace('-', " ");
                if big(ui, Icon::teal("🗺"), &label).on_hover_text(*name).clicked() {
                    match scenario::builtin(name) {
                        Ok(s) => self.load(s, None),
                        Err(e) => self.fail(e),
                    }
                }
            }
        });
    }

    fn chart_tab(&mut self, ui: &mut Ui, snap: &Snapshot) {
        group_menu(
            ui,
            self,
            t!("Route"),
            |ui, app: &mut Self| {
                let on = app.chart.add_waypoints;
                if big_toggle(ui, on, Icon::green("➕"), t!("Add waypoints"))
                    .on_hover_text(t!("Click the chart to add waypoints"))
                    .clicked()
                {
                    app.chart.add_waypoints = !on;
                }
                let has_route = !app.route_of(snap).0.is_empty();
                if big_enabled(ui, has_route, Icon::red("🗑"), t!("Clear route")).clicked() {
                    app.clear_route(snap);
                }
            },
            |ui, app: &mut Self| {
                let (wps, active, mut looped, mut radius) = app.route_of(snap);
                let mut changed = false;
                form(ui, "route_menu", |f| {
                    f.readout(t!("Waypoints"), wps.len().to_string());
                    changed |= f.check(t!("Loop"), &mut looped, t!("back to the first waypoint")).changed();
                    changed |=
                        f.drag(t!("Arrival radius"), DragValue::new(&mut radius).range(5.0..=2000.0).suffix(" m")).changed();
                });
                if changed {
                    app.set_route(Steering::Route { waypoints: wps, active, looped, arrival_radius_m: radius });
                }
            },
        );
        group(ui, t!("Display"), |ui| {
            let on = self.chart.show_seamarks;
            if big_toggle(ui, on, Icon::purple("⚓"), t!("Seamarks")).on_hover_text(t!("OpenSeaMap overlay: lights, buoys, …")).clicked()
            {
                self.chart.show_seamarks = !on;
            }
            if big(ui, Icon::purple("〰"), t!("Clear track")).on_hover_text(t!("Erase the vessel's track")).clicked() {
                self.chart.clear_track();
            }
        });
    }

    fn traffic_tab(&mut self, ui: &mut Ui, snap: &Snapshot) {
        group_menu(
            ui,
            self,
            t!("Add"),
            |ui, app: &mut Self| {
                column(ui, |ui| {
                    ui.weak(t!("Class"));
                    let t = tokens(ui);
                    ComboBox::from_id_salt("ribbon_new_class")
                        .width(t.field_short + t.space_xl)
                        .selected_text(tr(app.ais.new_class.label()))
                        .show_ui(ui, |ui| {
                            for &c in AisClass::ALL {
                                ui.selectable_value(&mut app.ais.new_class, c, tr(c.label()));
                            }
                        });
                });
                if big(ui, Icon::green("➕"), t!("Target"))
                    .on_hover_text(t!("2 nm from own ship; right-click the chart to place one"))
                    .clicked()
                {
                    app.add_target_nearby(snap);
                }
                if big(ui, Icon::red("⚠"), t!("Collision"))
                    .on_hover_text(t!("A Class A target that reaches own ship's predicted position (settings under Add ⏷)"))
                    .clicked()
                {
                    app.add_collision_target(snap);
                }
                if big(ui, Icon::amber("🎲"), t!("Random"))
                    .on_hover_text(t!("Random traffic around own ship (settings under Add ⏷)"))
                    .clicked()
                {
                    app.add_random_traffic(snap);
                }
            },
            |ui, app: &mut Self| {
                let a = &mut app.ais;
                form(ui, "traffic_menu", |f| {
                    f.heading(t!("Collision course"), |_| {});
                    f.drag(
                        t!("Meets own ship in"),
                        DragValue::new(&mut a.collision_minutes).range(1.0..=120.0).speed(0.5).suffix(" min"),
                    );
                    f.drag(
                        t!("Comes from"),
                        DragValue::new(&mut a.collision_rel_bearing).range(0.0..=359.0).speed(1.0).suffix(t!("° relative")),
                    );
                    f.drag(t!("Speed"), DragValue::new(&mut a.collision_speed).range(1.0..=40.0).speed(0.2).suffix(" kn"));
                    f.heading(t!("Random traffic"), |_| {});
                    f.drag(t!("Targets"), DragValue::new(&mut a.random_count).range(1..=200).speed(0.2));
                    f.drag(t!("Within"), DragValue::new(&mut a.random_radius_nm).range(0.5..=50.0).speed(0.1).suffix(" nm"));
                });
            },
        );
        group_menu(
            ui,
            self,
            t!("Targets"),
            |ui, app: &mut Self| {
                let any = !snap.sim.ais_targets.is_empty();
                if big_enabled(ui, any, Icon::red("🗑"), t!("Remove all")).clicked() {
                    app.remove_all_targets(snap);
                }
                ui.weak(crate::tf!("{n} targets", n = snap.sim.ais_targets.len()));
            },
            |ui, app: &mut Self| {
                if snap.sim.ais_targets.is_empty() {
                    ui.weak(t!("No targets."));
                }
                egui::ScrollArea::vertical().max_height(tokens(ui).list_height).show(ui, |ui| {
                    for t in &snap.sim.ais_targets {
                        let name =
                            if t.cfg.name.trim().is_empty() { t.cfg.mmsi.to_string() } else { t.cfg.name.trim().to_string() };
                        let selected = app.chart.selected_target == Some(t.id);
                        if ui.selectable_label(selected, format!("{name} · {}", tr(t.cfg.class.label()))).clicked() {
                            app.chart.selected_target = Some(t.id);
                            app.panels.controls = true;
                            ui.close();
                        }
                    }
                });
            },
        );
    }

    fn data_tab(&mut self, ui: &mut Ui, snap: &Snapshot) {
        group_menu(
            ui,
            self,
            t!("Outputs"),
            |ui, app: &mut Self| {
                if big(ui, Icon::green("➕"), t!("Add output")).on_hover_text(t!("UDP, TCP, WebSocket, serial or file")).clicked() {
                    app.new_output();
                }
                if big(ui, Icon::red("⏺"), t!("Record…"))
                    .on_hover_text(t!("Record everything to a file, with timestamps for replay"))
                    .clicked()
                {
                    app.record_to_file();
                }
            },
            |ui, app: &mut Self| app.outputs_menu(ui),
        );
        group_menu(
            ui,
            self,
            t!("Replay"),
            |ui, app: &mut Self| {
                if big(ui, Icon::blue("📂"), t!("Open log…"))
                    .on_hover_text(t!("Play a recorded NMEA 0183 / AIS log to the outputs"))
                    .clicked()
                {
                    app.open_log();
                }
                let loaded = snap.replay.as_ref();
                let playing = loaded.is_some_and(|r| r.playing);
                let (icon, label) = if playing { (Icon::amber("⏸"), t!("Pause")) } else { (Icon::green("▶"), t!("Play")) };
                if big_enabled(ui, loaded.is_some(), icon, label).clicked() {
                    app.engine.replay(ReplayCommand::Play(!playing));
                }
                if big_enabled(ui, loaded.is_some(), Icon::blue("⏮"), t!("Start"))
                    .on_hover_text(t!("Back to the start of the log"))
                    .clicked()
                {
                    app.engine.replay(ReplayCommand::Seek(0.0));
                }
                if big_enabled(ui, loaded.is_some(), Icon::red("⏏"), t!("Close")).clicked() {
                    app.engine.replay(ReplayCommand::Close);
                }
            },
            |ui, app: &mut Self| {
                let Some(r) = &snap.replay else {
                    ui.weak(t!("No log loaded."));
                    return;
                };
                ui.weak(t!("Speed"));
                for &s in REPLAY_SPEEDS {
                    if ui.selectable_label((s - r.speed).abs() < 1e-9, format!("{s}×")).clicked() {
                        app.engine.replay(ReplayCommand::Speed(s));
                    }
                }
                ui.separator();
                let mut looped = r.looped;
                if ui.checkbox(&mut looped, t!("Loop")).changed() {
                    app.engine.replay(ReplayCommand::Loop(looped));
                }
                let mut mute = r.mute_simulation;
                if ui.checkbox(&mut mute, t!("Mute simulated data")).changed() {
                    app.engine.replay(ReplayCommand::MuteSimulation(mute));
                }
            },
        );
        group_menu(
            ui,
            self,
            t!("Traffic monitor"),
            |ui, app: &mut Self| {
                let frozen = app.monitor.paused;
                if big_toggle(ui, frozen, Icon::teal("❄"), t!("Freeze")).on_hover_text(t!("Stop scrolling the traffic monitor")).clicked()
                {
                    app.freeze_monitor(!frozen);
                }
            },
            |ui, app: &mut Self| {
                ui.weak(t!("Show"));
                let current = app.monitor_view();
                for &f in DataFormat::ALL {
                    if ui.selectable_label(f == current, tr(f.label())).clicked() {
                        app.set_monitor_view(f);
                    }
                }
            },
        );
    }

    /// Each output with its switch and state.
    fn outputs_menu(&mut self, ui: &mut Ui) {
        let t = tokens(ui);
        let list = self.engine.outputs().list();
        if list.is_empty() {
            ui.weak(t!("No outputs."));
        }
        for o in list {
            ui.horizontal(|ui| {
                let mut enabled = o.config.enabled;
                if ui.checkbox(&mut enabled, &o.config.name).changed() {
                    let mut cfg = o.config.clone();
                    cfg.enabled = enabled;
                    self.update_output(o.id, cfg);
                }
                let (tone, text) = status_badge(&o.status);
                ui.colored_label(t.tone(ui, tone), "⚫").on_hover_text(text);
                ui.weak(o.config.format.short());
            });
        }
    }

    fn view_tab(&mut self, ui: &mut Ui) {
        group(ui, t!("Panels"), |ui| {
            let p = &mut self.panels;
            column(ui, |ui| {
                small_toggle(ui, &mut p.controls, Icon::purple("⬅"), t!("Vessel and environment"));
                small_toggle(ui, &mut p.io, Icon::purple("➡"), t!("Outputs and sensors"));
                small_toggle(ui, &mut p.monitor, Icon::purple("⬇"), t!("Traffic monitor"));
            });
        });
        group(ui, t!("Palette"), |ui| {
            let current = theme::palette(ui.ctx());
            for p in Palette::ALL {
                let (icon, tip) = match p {
                    Palette::Day => (Icon::amber("☀"), t!("Light, for daylight")),
                    Palette::Dusk => (Icon::purple("🌆"), t!("Dark, for dusk and dim rooms")),
                    Palette::Night => (Icon::red("🌙"), t!("Dim red on black, to keep night vision")),
                };
                if big_toggle(ui, current == p, icon, &tr(p.label())).on_hover_text(tip).clicked() {
                    theme::set_palette(ui.ctx(), p);
                }
            }
        });
        group(ui, t!("Settings"), |ui| {
            if big(ui, Icon::purple("⚙"), t!("Preferences…")).on_hover_text(t!("Palette, language and output folder (⌘,)")).clicked() {
                self.prefs.window_open = true;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    /// Every symbol the app writes must exist in the bundled fonts, or it shows as a box (or, on
    /// some systems, in another font and colour).
    #[test]
    fn icons_are_in_the_fonts() {
        let src = [
            include_str!("../app.rs"),
            include_str!("ais_panel.rs"),
            include_str!("controls.rs"),
            include_str!("monitor.rs"),
            include_str!("local_outputs.rs"),
            include_str!("outputs.rs"),
            include_str!("prefs.rs"),
            include_str!("replay.rs"),
            include_str!("ribbon_bar.rs"),
            include_str!("sensors.rs"),
            include_str!("../map.rs"),
            include_str!("../ui/form.rs"),
            include_str!("../ui/ribbon.rs"),
            include_str!("../ui/widgets.rs"),
        ];
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let font = egui::FontId::proportional(20.0);
        let mut missing = std::collections::BTreeSet::new();
        // Every non-ASCII character inside a string literal (file by file: a stray quote in a
        // comment or char literal must not shift the next file).
        for text in src.iter().flat_map(|s| s.split('"').skip(1).step_by(2)) {
            for c in text.chars().filter(|c| !c.is_ascii()) {
                if !ctx.fonts_mut(|f| f.has_glyph(&font, c)) {
                    missing.insert(c);
                }
            }
        }
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }
}
