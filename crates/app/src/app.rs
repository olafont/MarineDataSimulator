//! The main window: the ribbon on top, the chart in the middle and three panels around it.
//!
//! | Where  | What                                                             | Module         |
//! |--------|------------------------------------------------------------------|----------------|
//! | top    | ribbon: commands by task, simulation status                      | [`ribbon_bar`] |
//! | left   | navigation, vessel, AIS, environment, dynamics, instruments      | [`controls`], [`ais_panel`] |
//! | right  | outputs, replay, sensors                                         | [`outputs`], [`replay`], [`sensors`] |
//! | bottom | traffic monitor                                                  | [`monitor`]    |
//!
//! This file holds the window state and the commands that both the ribbon and the panels use.

use std::path::PathBuf;

use egui::{ScrollArea, Ui};
use engine::scenario::Scenario;
use engine::sim_core::{Channel, Command, LatLon, Simulation, Steering};
use engine::transports::{DataFormat, OutputConfig, OutputKind};
use engine::{Engine, Snapshot};

use crate::map::{ChartView, MapAction};
use crate::ui::form::{Form, Num};
use crate::ui::widgets::Edits;
use crate::ui::{ribbon, theme, title_bar};
use crate::t;

mod ais_panel;
mod controls;
mod local_outputs;
mod monitor;
mod outputs;
mod prefs;
mod replay;
mod ribbon_bar;
mod sensors;

const TIME_SCALES: &[f64] = &[0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0];
/// The app's title bar on macOS: a little taller than the OS one, for the buttons.
const TITLE_BAR_HEIGHT: f32 = 32.0;
const REPLAY_SPEEDS: &[f64] = &[0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0];

pub struct SimApp {
    engine: Engine,
    chart: ChartView,
    edits: Edits,
    /// Baseline used by "Reset".
    scenario: Scenario,
    scenario_path: Option<PathBuf>,
    /// Route kept while steering by heading, restored when switching back.
    saved_route: Steering,
    output_editor: Option<outputs::OutputDraft>,
    prefs: prefs::PrefsState,
    /// Outputs the user created, kept on this computer.
    local_outputs: local_outputs::LocalOutputs,
    monitor: monitor::MonitorState,
    ais: ais_panel::AisUi,
    ribbon: ribbon::State<ribbon_bar::Tab>,
    panels: ribbon_bar::Panels,
    /// Opens the Outputs section on the next frame (a ribbon command shows its editor there).
    reveal_outputs: bool,
    error: Option<String>,
}

/// How the vessel is steered, without the target value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SteerMode {
    Heading,
    WindAngle,
    Route,
}

impl SteerMode {
    fn of(s: &Steering) -> Self {
        match s {
            Steering::Heading { .. } => Self::Heading,
            Steering::WindAngle { .. } => Self::WindAngle,
            Steering::Route { .. } => Self::Route,
        }
    }
}

impl SimApp {
    pub fn new(cc: &eframe::CreationContext<'_>, scenario: Scenario, path: Option<PathBuf>) -> anyhow::Result<Self> {
        theme::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let engine = Engine::start(scenario.clone(), Some(Box::new(move || ctx.request_repaint())))?;
        let (prefs, prefs_error) = prefs::PrefsState::load();
        prefs.apply(&cc.egui_ctx);
        let mut app = Self {
            engine,
            chart: ChartView::new(&cc.egui_ctx),
            edits: Edits::default(),
            saved_route: route_in(&scenario),
            scenario,
            scenario_path: path,
            output_editor: None,
            prefs,
            local_outputs: local_outputs::LocalOutputs::new(),
            monitor: monitor::MonitorState::default(),
            ais: ais_panel::AisUi::default(),
            ribbon: ribbon::State { tab: ribbon_bar::Tab::Home, collapsed: false },
            panels: ribbon_bar::Panels::default(),
            reveal_outputs: false,
            error: prefs_error,
        };
        app.start_local_outputs();
        Ok(app)
    }

    /// Shows an error in a window until the user dismisses it.
    fn fail(&mut self, e: impl std::fmt::Display) {
        self.error = Some(format!("{e:#}"));
    }

    // -- Scenario ------------------------------------------------------------------------------

    fn load(&mut self, scenario: Scenario, path: Option<PathBuf>) {
        self.engine.load_scenario(scenario.clone());
        self.saved_route = route_in(&scenario);
        self.scenario = scenario;
        self.scenario_path = path;
        self.chart.clear_track();
        self.chart.selected_target = None;
        self.output_editor = None;
        self.start_local_outputs();
    }

    /// Reloads the scenario's initial state.
    fn reset(&mut self) {
        self.load(self.scenario.clone(), self.scenario_path.clone());
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter(t!("Scenario"), &["toml"]).pick_file() {
            match Scenario::load(&path) {
                Ok(s) => self.load(s, Some(path)),
                Err(e) => self.fail(e),
            }
        }
    }

    fn save_dialog(&mut self) {
        let name = self.scenario_path.as_deref().and_then(file_name).unwrap_or_else(|| "scenario.toml".into());
        if let Some(path) = rfd::FileDialog::new().add_filter(t!("Scenario"), &["toml"]).set_file_name(name).save_file() {
            let captured = self.engine.capture_scenario();
            match captured.save(&path) {
                Ok(()) => {
                    self.scenario = captured;
                    self.scenario_path = Some(path);
                }
                Err(e) => self.fail(e),
            }
        }
    }

    /// The scenario's file name, or its name for a built-in one.
    fn scenario_label(&self) -> String {
        self.scenario_path
            .as_deref()
            .and_then(file_name)
            .unwrap_or_else(|| crate::tf!("{name} (built-in)", name = self.scenario.name))
    }

    // -- Steering ------------------------------------------------------------------------------

    /// Switches steering mode, starting from the current heading or wind angle. A route left
    /// behind is kept for when the user switches back.
    fn steer(&mut self, sim: &Simulation, mode: SteerMode) {
        if SteerMode::of(&sim.steering) == mode {
            return;
        }
        if let Steering::Route { .. } = sim.steering {
            self.saved_route = sim.steering.clone();
        }
        self.engine.send(Command::SetSteering(match mode {
            SteerMode::Heading => Steering::Heading { heading: sim.vessel.heading.round() },
            SteerMode::WindAngle => Steering::WindAngle { twa: sim.channels.get(Channel::Twa).round() },
            SteerMode::Route => self.saved_route.clone(),
        }));
    }

    fn set_route(&mut self, route: Steering) {
        self.saved_route = route.clone();
        self.engine.send(Command::SetSteering(route));
    }

    /// Current route (from the simulation when following one, else the saved one).
    fn route_of(&self, snap: &Snapshot) -> (Vec<LatLon>, usize, bool, f64) {
        let s = match &snap.sim.steering {
            r @ Steering::Route { .. } => r,
            _ => &self.saved_route,
        };
        match s {
            Steering::Route { waypoints, active, looped, arrival_radius_m } => {
                (waypoints.clone(), *active, *looped, *arrival_radius_m)
            }
            _ => (vec![], 0, true, 50.0),
        }
    }

    fn clear_route(&mut self, snap: &Snapshot) {
        let (_, _, looped, radius) = self.route_of(snap);
        self.set_route(Steering::Route { waypoints: vec![], active: 0, looped, arrival_radius_m: radius });
    }

    fn handle_map_action(&mut self, snap: &Snapshot, action: MapAction) {
        match action {
            MapAction::MoveVessel(p) => {
                self.engine.send(Command::Teleport(p));
                self.chart.clear_track();
            }
            MapAction::AddWaypoint(p) => {
                let (mut wps, active, looped, radius) = self.route_of(snap);
                wps.push(p);
                self.set_route(Steering::Route { waypoints: wps, active, looped, arrival_radius_m: radius });
            }
            MapAction::SteerTowards(p) => {
                let h = snap.sim.vessel.position.bearing_to(&p);
                self.engine.send(Command::SetSteering(Steering::Heading { heading: h.round() }));
            }
            other => self.handle_ais_action(snap, other),
        }
    }

    // -- Data ----------------------------------------------------------------------------------

    /// Opens the output editor on a new UDP output.
    fn new_output(&mut self) {
        let folder = self.output_folder();
        self.output_editor = Some(outputs::OutputDraft::from_config(None, &folder, &OutputConfig {
            name: t!("UDP broadcast").into(),
            enabled: true,
            kind: OutputKind::Udp { target: "255.255.255.255:10113".into(), broadcast: true },
            format: DataFormat::Nmea0183,
            filter: vec![],
        }));
        self.panels.io = true;
        self.reveal_outputs = true;
    }

    /// Adds a file output with TAG-block timestamps, for replay.
    fn record_to_file(&mut self) {
        let name = chrono::Utc::now().format("marine-sim-%Y%m%d-%H%M%S.nmea").to_string();
        let dialog = rfd::FileDialog::new()
            .add_filter(t!("NMEA log"), &["nmea", "log", "txt"])
            .set_directory(self.output_folder())
            .set_file_name(name);
        if let Some(path) = dialog.save_file() {
            self.engine.outputs().add(OutputConfig {
                name: crate::tf!("Record {file}", file = file_name(&path).unwrap_or_default()),
                enabled: true,
                kind: OutputKind::File { path: path.display().to_string(), tag_blocks: true },
                format: DataFormat::Nmea0183,
                filter: vec![],
            });
        }
    }

    /// Loads a recorded log for replay.
    fn open_log(&mut self) {
        let dialog = rfd::FileDialog::new().set_directory(self.output_folder());
        if let Some(path) = dialog.add_filter(t!("NMEA log"), &["nmea", "log", "txt", "csv"]).pick_file()
            && let Err(e) = self.engine.load_replay(&path)
        {
            self.fail(e);
        }
    }
}

/// The route a scenario starts with, or an empty one to fill in.
fn route_in(scenario: &Scenario) -> Steering {
    match &scenario.steering {
        r @ Steering::Route { .. } => r.clone(),
        _ => Steering::Route { waypoints: vec![], active: 0, looped: true, arrival_radius_m: 50.0 },
    }
}

fn file_name(p: &std::path::Path) -> Option<String> {
    p.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Target speed, and the heading or wind angle to hold. Used by the Navigation section and the
/// ribbon's Steering menu (`salt` keeps their edits apart).
fn steering_rows(f: &mut Form<'_>, edits: &mut Edits, sim: &Simulation, salt: &str) -> Vec<Command> {
    let id = |key: &str| egui::Id::unique((salt, key));
    let mut send = vec![];
    let mut speed = sim.target_stw_kn;
    let label = if sim.is_sailing() { t!("Engine speed target") } else { t!("Target speed") };
    let num = Num::new(0.0..=sim.vessel_cfg.max_speed_kn, 0.1, " kn").decimals(1);
    if f.number(label, edits, id("speed"), &mut speed, num) {
        send.push(Command::SetTargetSpeed(speed));
    }
    match sim.steering {
        Steering::Heading { mut heading } => {
            let num = Num::new(0.0..=359.9, 0.5, " °T").decimals(1);
            if f.number(t!("Heading"), edits, id("heading"), &mut heading, num) {
                send.push(Command::SetSteering(Steering::Heading { heading }));
            }
        }
        Steering::WindAngle { mut twa } => {
            let num = Num::new(-180.0..=180.0, 0.5, t!("° (+stbd)")).decimals(1);
            if f.number(t!("True wind angle"), edits, id("twa"), &mut twa, num) {
                send.push(Command::SetSteering(Steering::WindAngle { twa }));
            }
        }
        Steering::Route { .. } => {}
    }
    send
}

impl eframe::App for SimApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let snap = self.engine.snapshot();

        if title_bar::integrated() {
            let frame = egui::Frame::NONE.fill(theme::tokens(ui).canvas);
            egui::Panel::top("title_bar").frame(frame).exact_size(TITLE_BAR_HEIGHT).show_separator_line(false).show(
                ui,
                |ui| {
                    let bar = ui.max_rect();
                    title_bar::show(ui, bar, |ui| self.title_bar(ui, &snap));
                },
            );
        }
        if self.ribbon_shown() {
            egui::Panel::top("ribbon").frame(ribbon::outer_frame(ui)).show(ui, |ui| self.ribbon(ui, &snap));
        }
        let panels = self.panels;
        if panels.controls {
            egui::Panel::left("controls").resizable(true).default_size(390.0).show(ui, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    self.navigation(ui, &snap);
                    self.vessel(ui, &snap);
                    self.ais(ui, &snap);
                    self.environment(ui, &snap);
                    self.dynamics(ui, &snap);
                    self.instruments(ui, &snap);
                });
            });
        }
        if panels.io {
            egui::Panel::right("io").resizable(true).default_size(330.0).show(ui, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    self.outputs(ui);
                    self.replay(ui, &snap);
                    self.sensors(ui, &snap);
                });
            });
        }
        if panels.monitor {
            egui::Panel::bottom("monitor").resizable(true).default_size(200.0).show(ui, |ui| self.monitor(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let replay_fix = snap.replay.as_ref().and_then(|r| r.last_fix);
            if let Some(action) = self.chart.ui(ui, &snap.sim, replay_fix) {
                self.handle_map_action(&snap, action);
            }
        });

        if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.prefs.window_open = true;
        }
        self.prefs_window(ui.ctx());
        self.save_prefs_if_changed(ui.ctx());

        if let Some(err) = self.error.clone() {
            egui::Window::new(t!("Error")).collapsible(false).resizable(false).show(ui.ctx(), |ui| {
                ui.label(err);
                if ui.button(t!("OK")).clicked() {
                    self.error = None;
                }
            });
        }
    }
}
