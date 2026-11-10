//! Left panel: navigation and route, vessel, environment, dynamics and instruments.

use std::sync::Arc;

use egui::{DragValue, Grid, Id, RichText, Ui};
use engine::Snapshot;
use engine::scenario;
use engine::sim_core::{Channel, Command, EnvParam, Propulsion, Steering};


use super::{SimApp, SteerMode, steering_rows};
use crate::ui::form::{Form, Num, form};
use crate::ui::theme::{Tone, tokens};
use crate::ui::section;
use crate::ui::widgets::{format_value, override_cells, quantity_drag};
use crate::t;
use crate::ui::i18n::tr;

impl SimApp {
    pub(super) fn navigation(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "navigation", t!("Navigation"), true, |ui| {
            let sim = &snap.sim;
            let mut mode = SteerMode::of(&sim.steering);
            let mut send = vec![];
            form(ui, "nav", |f| {
                if sim.vessel_cfg.propulsion == Propulsion::Sail {
                    // Same control as Steering below: pick one, the choice is highlighted.
                    let mut on = sim.engine_on;
                    if f.segmented(t!("Under"), &mut on, &[(false, t!("⛵ Sail")), (true, t!("⚙ Engine"))]) {
                        send.push(Command::SetEngine(on));
                    }
                }
                f.segmented(
                    t!("Steering"),
                    &mut mode,
                    &[(SteerMode::Heading, t!("Heading")), (SteerMode::WindAngle, t!("Wind angle")), (SteerMode::Route, t!("Route"))],
                );
                send.extend(steering_rows(f, &mut self.edits, sim, "nav"));
                if let Steering::Route { .. } = sim.steering {
                    if sim.route_complete {
                        let hint = Some(t!("Click a waypoint below to resume the route."));
                        f.status(t!("Status"), t!("Route complete: stopped, drifting"), Tone::Warn, hint);
                    } else if sim.is_sailing() && sim.tack != 0.0 {
                        f.readout(t!("Tack"), if sim.tack > 0.0 { t!("starboard") } else { t!("port") });
                    }
                }
            });
            self.steer(sim, mode);
            for c in send {
                self.engine.send(c);
            }
            if mode == SteerMode::Route {
                self.route_editor(ui, snap);
            }
            ui.weak(t!("Right-click the chart to move the vessel or add waypoints."));
        });
    }

    fn route_editor(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let (mut wps, mut active, mut looped, mut radius) = self.route_of(snap);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.toggle_value(&mut self.chart.add_waypoints, t!("+ Add on click"));
            changed |= ui.checkbox(&mut looped, t!("Loop")).changed();
            ui.label(t!("Arrival"));
            changed |= ui.add(DragValue::new(&mut radius).range(5.0..=2000.0).suffix(" m")).changed();
            if ui.button(t!("Clear")).clicked() {
                wps.clear();
                active = 0;
                changed = true;
            }
        });
        let here = snap.sim.vessel.position;
        let mut remove = None;
        let t = tokens(ui);
        Grid::new("route").num_columns(4).striped(true).spacing([t.space_md, t.space_xs]).show(ui, |ui| {
            for (i, w) in wps.iter().enumerate() {
                let label = if i == active { RichText::new(format!("▶ {}", i + 1)).strong() } else { RichText::new(format!("  {}", i + 1)) };
                if ui.selectable_label(i == active, label).on_hover_text(t!("Go to this waypoint")).clicked() {
                    self.engine.send(Command::SetActiveWaypoint(i));
                }
                ui.monospace(format!("{:.5}, {:.5}", w.lat, w.lon));
                ui.weak(format!("{:.2} nm", here.distance_to(w) / 1852.0));
                if ui.small_button("×").on_hover_text(t!("Remove waypoint")).clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            wps.remove(i);
            if active >= wps.len() {
                active = 0;
            }
            changed = true;
        }
        if changed {
            self.set_route(Steering::Route { waypoints: wps, active, looped, arrival_radius_m: radius });
        }
    }

    pub(super) fn vessel(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "vessel", t!("Vessel"), false, |ui| {
            let sim = &snap.sim;
            let mut c = sim.vessel_cfg.clone();
            let mut changed = false;
            let e = &mut self.edits;
            form(ui, "vessel", |f| {
                changed |= f.segmented(
                    t!("Propulsion"),
                    &mut c.propulsion,
                    &[(Propulsion::Motor, t!("Motor")), (Propulsion::Sail, t!("Sail"))],
                );
                if c.propulsion == Propulsion::Sail {
                    f.row(t!("Polar"), |ui| {
                        let name = c
                            .polar_file
                            .as_deref()
                            .map(|p| {
                                std::path::Path::new(p)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default()
                            })
                            .unwrap_or_else(|| t!("built-in cruiser").into());
                        ui.monospace(name);
                        if ui.small_button(t!("Load…")).clicked()
                            && let Some(path) =
                                rfd::FileDialog::new().add_filter(t!("Polar"), &["pol", "csv", "txt"]).pick_file()
                        {
                            match scenario::load_polar(&path) {
                                Ok(p) => {
                                    self.engine.send(Command::SetPolar(Arc::new(p)));
                                    c.polar_file = Some(path.display().to_string());
                                    changed = true;
                                }
                                Err(err) => self.error = Some(format!("{err:#}")), // not `fail`: `edits` is borrowed
                            }
                        }
                        if c.polar_file.is_some() && ui.small_button(t!("Built-in")).clicked() {
                            self.engine.send(Command::SetPolar(Arc::new(engine::sim_core::Polar::default_cruiser())));
                            c.polar_file = None;
                            changed = true;
                        }
                    });
                    let (tws, _) = sim.true_wind_over_water();
                    f.readout(
                        t!("Optimum angles"),
                        crate::tf!(
                            "beat {beat}° · run {run}° at {tws} kn",
                            beat = format!("{:.0}", sim.polar.beat_angle(tws)),
                            run = format!("{:.0}", sim.polar.run_angle(tws)),
                            tws = format!("{tws:.0}"),
                        ),
                    );
                }
                let mut num = |f: &mut Form<'_>, label: &str, key: &str, v: &mut f64, n: Num<'_>| {
                    changed |= f.number(label, e, Id::unique(("vessel", key)), v, n);
                };
                if c.propulsion == Propulsion::Sail {
                    num(f, t!("Polar efficiency"), "eff", &mut c.polar_efficiency, Num::new(0.1..=1.5, 0.01, ""));
                    num(f, t!("Heel coefficient"), "heel", &mut c.heel_coefficient, Num::new(0.0..=0.5, 0.002, " °/kn²"));
                    num(f, t!("Max heel"), "maxheel", &mut c.max_heel_deg, Num::new(0.0..=60.0, 0.5, " °"));
                    num(f, t!("Leeway coefficient"), "lee", &mut c.leeway_coefficient, Num::new(0.0..=30.0, 0.1, ""));
                }
                num(f, t!("Max engine speed"), "maxspd", &mut c.max_speed_kn, Num::new(1.0..=60.0, 0.1, " kn"));
                num(f, t!("Length"), "loa", &mut c.length_m, Num::new(2.0..=400.0, 0.1, " m"));
                num(f, t!("Beam"), "beam", &mut c.beam_m, Num::new(1.0..=80.0, 0.05, " m"));
                num(f, t!("Draft"), "draft", &mut c.draft_m, Num::new(0.1..=30.0, 0.05, " m"));
                num(f, t!("Max rate of turn"), "rot", &mut c.max_rot_dpm, Num::new(5.0..=720.0, 1.0, " °/min"));
                num(f, t!("Speed time constant"), "tau", &mut c.speed_time_constant_s, Num::new(1.0..=600.0, 0.5, " s"));
                num(f, t!("Roll period"), "rollp", &mut c.roll_period_s, Num::new(1.0..=30.0, 0.1, " s"));
                num(f, t!("Idle rpm"), "idle", &mut c.idle_rpm, Num::new(0.0..=3000.0, 10.0, ""));
                num(f, t!("Max rpm"), "maxrpm", &mut c.max_rpm, Num::new(100.0..=10000.0, 10.0, ""));
            });
            if changed {
                self.engine.send(Command::SetVesselConfig(Box::new(c)));
            }
        });
    }

    pub(super) fn dynamics(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "dynamics", t!("Dynamics (gusts, tide)"), false, |ui| {
            let sim = &snap.sim;
            let mut d = sim.dynamics.clone();
            let mut changed = false;
            let e = &mut self.edits;
            form(ui, "dynamics", |f| {
                let mut num = |f: &mut Form<'_>, label: &str, key: &str, v: &mut f64, n: Num<'_>| {
                    changed |= f.number(label, e, Id::unique(("dyn", key)), v, n);
                };
                let mut gust_pct = d.wind.gust_fraction * 100.0;
                num(f, t!("Gusts (std dev)"), "gust", &mut gust_pct, Num::new(0.0..=60.0, 0.5, " %"));
                d.wind.gust_fraction = gust_pct / 100.0;
                num(f, t!("Gust period"), "gustp", &mut d.wind.gust_period_s, Num::new(1.0..=600.0, 0.5, " s"));
                num(f, t!("Shifts (std dev)"), "shift", &mut d.wind.shift_deg, Num::new(0.0..=60.0, 0.2, " °"));
                num(f, t!("Shift period"), "shiftp", &mut d.wind.shift_period_s, Num::new(5.0..=3600.0, 1.0, " s"));
                num(
                    f,
                    t!("Pressure trend"),
                    "ptrend",
                    &mut d.pressure_trend_hpa_h,
                    Num::new(-10.0..=10.0, 0.05, " hPa/h"),
                );

                let tide_toggled = f.heading(t!("Tide"), |ui| ui.checkbox(&mut d.tide.enabled, "enabled").changed());
                if d.tide.enabled {
                    num(f, t!("Range"), "range", &mut d.tide.range_m, Num::new(0.0..=15.0, 0.05, " m"));
                    num(f, t!("Period"), "period", &mut d.tide.period_h, Num::new(1.0..=30.0, 0.01, " h"));
                    num(
                        f,
                        t!("Flood sets towards"),
                        "flood",
                        &mut d.tide.flood_set_deg,
                        Num::new(0.0..=359.9, 0.5, " °T"),
                    );
                    num(f, t!("Max stream"), "rate", &mut d.tide.max_rate_kn, Num::new(0.0..=10.0, 0.05, " kn"));
                    let mut next_hw_h = d.tide.next_high_water_in(sim.time_unix) / 3600.0;
                    num(f, t!("Next high water in"), "hw", &mut next_hw_h, Num::new(0.0..=d.tide.period_h, 0.02, " h"));
                    d.tide.high_water_unix = sim.time_unix + next_hw_h * 3600.0;
                    let stream = d.tide.stream(sim.time_unix);
                    f.readout(
                        t!("Now"),
                        crate::tf!(
                            "height {h} m · stream {rate} kn to {dir}°",
                            h = format!("{:.2}", sim.tide_height_m),
                            rate = format!("{:.1}", stream.magnitude()),
                            dir = format!("{:03.0}", stream.direction()),
                        ),
                    );
                }
                changed |= tide_toggled;
        });
                if changed {
                    self.engine.send(Command::SetDynamics(Box::new(d)));
                }
            });
    }

    pub(super) fn environment(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "environment", t!("Environment"), true, |ui| {
            let t = tokens(ui);
            ui.weak(t!("Base values you set; the live column adds gusts, tide and soundings. Overrides here change how the vessel behaves."));
            Grid::new("env").num_columns(6).spacing([t.space_md, t.space_sm]).striped(true).show(ui, |ui| {
                ui.weak("");
                ui.weak(t!("base"));
                ui.weak(t!("live"));
                ui.weak(t!("override"));
                ui.end_row();
                for &p in EnvParam::ALL {
                    let meta = p.meta();
                    let param = snap.sim.env.param(p);
                    ui.label(tr(meta.label));
                    let id = Id::unique(("env", meta.key));
                    if let Some(v) = self.edits.value(ui, id, snap.sim.env_base.get(p), |ui, v| ui.add(quantity_drag(v, &meta, false))) {
                        self.engine.send(Command::SetEnvBase(p, v));
                    }
                    let live = RichText::new(format_value(&meta, param.value())).monospace();
                    if param.is_overridden() {
                        ui.label(live.color(t.overridden));
                    } else {
                        ui.label(live.weak());
                    }
                    if let Some(o) = override_cells(ui, &mut self.edits, id.with("ovr"), param, &meta) {
                        self.engine.send(Command::SetEnvOverride(p, o));
                    }
                    ui.end_row();
                }
        });
            });
    }

    pub(super) fn instruments(&mut self, ui: &mut Ui, snap: &Snapshot) {
        section(ui, "instruments", t!("Instruments"), true, |ui| {
            let t = tokens(ui);
            ui.weak(t!("Values the physics produces, before sensor errors. Overrides change only what is reported."));
            Grid::new("channels").num_columns(5).spacing([t.space_md, t.space_sm]).striped(true).show(ui, |ui| {
                for &c in Channel::ALL {
                    let meta = c.meta();
                    let param = snap.sim.channels.param(c);
                    ui.label(tr(meta.label));
                    let text = format_value(&meta, param.value());
                    if param.is_overridden() {
                        ui.label(RichText::new(text).monospace().color(t.overridden))
                            .on_hover_text(crate::tf!("modeled: {v}", v = format_value(&meta, param.modeled)));
                    } else {
                        ui.monospace(text);
                    }
                    let id = Id::unique(("ch", meta.key));
                    if let Some(o) = override_cells(ui, &mut self.edits, id, param, &meta) {
                        self.engine.send(Command::SetChannelOverride(c, o));
                    }
                    ui.end_row();
                }
            });
        });
    }
}
