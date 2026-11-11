//! The MCP server: lets AI agents watch and drive the simulation (see the `mcp` crate).
//!
//! It is off until the user switches it on in Preferences, listens on this computer only, and
//! works on the engine directly, so an agent's commands take effect even while the window is
//! hidden. What the window itself must follow (a scenario loaded by an agent, the vessel
//! moved) comes back as events, handled at the start of the next frame.

use std::collections::HashSet;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use egui::{Context, RichText, Ui};
use engine::Engine;
use engine::scenario::Scenario;
use mcp::McpServer;

use super::SimApp;
use super::local_outputs;
use super::route_in;
use crate::t;
use crate::ui::form::{Form, Num};
use crate::ui::theme;

enum Event {
    /// With the ids of the user's kept outputs, already started again.
    ScenarioLoaded(Box<Scenario>, HashSet<u64>),
    VesselMoved,
}

/// What the MCP server drives: the engine, and the window through events.
struct AppHost {
    engine: Arc<Engine>,
    events: Sender<Event>,
    ctx: Context,
}

impl AppHost {
    fn notify(&self, e: Event) {
        let _ = self.events.send(e);
        self.ctx.request_repaint();
    }
}

impl mcp::Host for AppHost {
    fn engine(&self) -> &Engine {
        &self.engine
    }

    fn load_scenario(&self, scenario: Scenario) {
        self.engine.load_scenario(scenario.clone());
        // Loading stops every output: bring the user's kept ones back now, not when the window
        // is next drawn (a hidden window is not drawn at all).
        let kept = match local_outputs::file_path().map(|p| local_outputs::start_saved(self.engine.outputs(), &p)) {
            Some(Ok(ids)) => ids,
            Some(Err(e)) => {
                tracing::warn!("kept outputs: {e:#}");
                HashSet::new()
            }
            None => HashSet::new(),
        };
        self.notify(Event::ScenarioLoaded(Box::new(scenario), kept));
    }

    fn vessel_moved(&self) {
        self.notify(Event::VesselMoved);
    }
}

pub(super) struct Agents {
    server: Option<McpServer>,
    /// Why the server could not start.
    error: Option<String>,
    /// The port last asked for (`None` = off), to notice a change in the preferences.
    applied: Option<u16>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Agents {
    pub(super) fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self { server: None, error: None, applied: None, tx, rx }
    }
}

impl SimApp {
    /// Starts, stops or moves the server when the preferences ask for it.
    pub(super) fn sync_agents(&mut self, ctx: &Context) {
        let p = &self.prefs.prefs;
        let wanted = p.mcp_enabled.then_some(p.mcp_port);
        if wanted == self.agents.applied {
            return;
        }
        self.agents.applied = wanted;
        self.agents.server = None; // frees the port before the new one starts
        self.agents.error = None;
        if let Some(port) = wanted {
            let host = Arc::new(AppHost { engine: self.engine.clone(), events: self.agents.tx.clone(), ctx: ctx.clone() });
            match McpServer::start_http(host, port) {
                Ok(s) => self.agents.server = Some(s),
                Err(e) => self.agents.error = Some(format!("{e:#}")),
            }
        }
    }

    /// Brings the window in step with what agents did since the last frame.
    pub(super) fn handle_agent_events(&mut self) {
        while let Ok(e) = self.agents.rx.try_recv() {
            match e {
                // The engine already runs it: only the window's own state follows.
                Event::ScenarioLoaded(scenario, kept) => {
                    self.saved_route = route_in(&scenario);
                    self.scenario = *scenario;
                    self.scenario_path = None;
                    self.chart.clear_track();
                    self.chart.selected_target = None;
                    self.output_editor = None;
                    self.local_outputs.adopt(kept);
                }
                Event::VesselMoved => self.chart.clear_track(),
            }
        }
    }

    /// The "AI agents" rows of the Preferences window.
    pub(super) fn agents_form(&mut self, f: &mut Form<'_>) {
        let p = &mut self.prefs.prefs;
        f.check(t!("MCP server"), &mut p.mcp_enabled, t!("Let agents connect"));
        let mut port = f64::from(p.mcp_port);
        if f.number(t!("Port"), &mut self.edits, egui::Id::unique("mcp_port"), &mut port, Num::new(1024.0..=65535.0, 1.0, "").decimals(0)) {
            p.mcp_port = port as u16;
        }
        if let Some(server) = &self.agents.server {
            let url = server.url();
            f.row(t!("Address"), |ui: &mut Ui| {
                ui.label(RichText::new(&url).monospace());
                if ui.small_button(t!("Copy")).clicked() {
                    ui.ctx().copy_text(url.clone());
                }
            });
        } else if let Some(e) = &self.agents.error {
            let text = crate::tf!("Could not start: {e}", e = e);
            f.row("", |ui: &mut Ui| ui.label(RichText::new(text).color(theme::tokens(ui).error)));
        }
    }
}
