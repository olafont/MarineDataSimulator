//! The commands the search finds (⌘K): the ribbon's, each with its name and icon as the ribbon
//! shows them, where it lives (`Tab › Group`), whether it can run now, and what it does.

use engine::sim_core::{Command, Propulsion};
use engine::{ReplayCommand, Snapshot};

use super::{SimApp, SteerMode};
use crate::t;
use crate::ui::ribbon::Icon;
use crate::ui::theme::{self, Palette};

/// The shortcut of the search.
pub const SEARCH_KEY: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::K);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    RunPause,
    Reset,
    RealTime,
    SteerHeading,
    SteerWind,
    SteerRoute,
    Engine,
    Open,
    SaveAs,
    AddWaypoints,
    ClearRoute,
    Seamarks,
    ClearTrack,
    AddTarget,
    Collision,
    RandomTraffic,
    RemoveTargets,
    AddOutput,
    Record,
    OpenLog,
    ReplayPlay,
    ReplayStart,
    ReplayClose,
    Freeze,
    LeftPanel,
    RightPanel,
    MonitorPanel,
    Day,
    Dusk,
    Night,
    Preferences,
}

impl Cmd {
    /// Every command, in the order the search lists them.
    pub const ALL: &[Cmd] = &[
        Cmd::RunPause,
        Cmd::Reset,
        Cmd::RealTime,
        Cmd::SteerHeading,
        Cmd::SteerWind,
        Cmd::SteerRoute,
        Cmd::Engine,
        Cmd::Open,
        Cmd::SaveAs,
        Cmd::AddWaypoints,
        Cmd::ClearRoute,
        Cmd::Seamarks,
        Cmd::ClearTrack,
        Cmd::AddTarget,
        Cmd::Collision,
        Cmd::RandomTraffic,
        Cmd::RemoveTargets,
        Cmd::AddOutput,
        Cmd::Record,
        Cmd::OpenLog,
        Cmd::ReplayPlay,
        Cmd::ReplayStart,
        Cmd::ReplayClose,
        Cmd::Freeze,
        Cmd::LeftPanel,
        Cmd::RightPanel,
        Cmd::MonitorPanel,
        Cmd::Day,
        Cmd::Dusk,
        Cmd::Night,
        Cmd::Preferences,
    ];

    /// The name, in the interface's language. Some names follow the state, as on the ribbon.
    pub fn name(self, snap: &Snapshot) -> &'static str {
        let playing = snap.replay.as_ref().is_some_and(|r| r.playing);
        match self {
            Cmd::RunPause if snap.sim.paused => t!("Run"),
            Cmd::RunPause => t!("Pause"),
            Cmd::Reset => t!("Reset"),
            Cmd::RealTime => t!("Real time"),
            Cmd::SteerHeading => t!("Heading"),
            Cmd::SteerWind => t!("Wind angle"),
            Cmd::SteerRoute => t!("Route"),
            Cmd::Engine => t!("Engine"),
            Cmd::Open => t!("Open…"),
            Cmd::SaveAs => t!("Save as…"),
            Cmd::AddWaypoints => t!("Add waypoints"),
            Cmd::ClearRoute => t!("Clear route"),
            Cmd::Seamarks => t!("Seamarks"),
            Cmd::ClearTrack => t!("Clear track"),
            Cmd::AddTarget => t!("Target"),
            Cmd::Collision => t!("Collision"),
            Cmd::RandomTraffic => t!("Random"),
            Cmd::RemoveTargets => t!("Remove all"),
            Cmd::AddOutput => t!("Add output"),
            Cmd::Record => t!("Record…"),
            Cmd::OpenLog => t!("Open log…"),
            Cmd::ReplayPlay if playing => t!("Pause"),
            Cmd::ReplayPlay => t!("Play"),
            Cmd::ReplayStart => t!("Start"),
            Cmd::ReplayClose => t!("Close"),
            Cmd::Freeze => t!("Freeze"),
            Cmd::LeftPanel => t!("Vessel and environment"),
            Cmd::RightPanel => t!("Outputs and sensors"),
            Cmd::MonitorPanel => t!("Traffic monitor"),
            Cmd::Day => t!("Day"),
            Cmd::Dusk => t!("Dusk"),
            Cmd::Night => t!("Night"),
            Cmd::Preferences => t!("Preferences…"),
        }
    }

    /// The icon, as on the ribbon.
    pub fn icon(self, snap: &Snapshot) -> Icon {
        let playing = snap.replay.as_ref().is_some_and(|r| r.playing);
        match self {
            Cmd::RunPause if snap.sim.paused => Icon::green("▶"),
            Cmd::RunPause => Icon::amber("⏸"),
            Cmd::Reset => Icon::blue("⟲"),
            Cmd::RealTime => Icon::blue("⏵"),
            Cmd::SteerHeading => Icon::blue("⬆"),
            Cmd::SteerWind => Icon::amber("💨"),
            Cmd::SteerRoute => Icon::red("🚩"),
            Cmd::Engine => Icon::teal("⚙"),
            Cmd::Open | Cmd::OpenLog => Icon::blue("📂"),
            Cmd::SaveAs => Icon::blue("💾"),
            Cmd::AddWaypoints | Cmd::AddTarget | Cmd::AddOutput => Icon::green("➕"),
            Cmd::ClearRoute | Cmd::RemoveTargets => Icon::red("🗑"),
            Cmd::Seamarks => Icon::purple("⚓"),
            Cmd::ClearTrack => Icon::purple("〰"),
            Cmd::Collision => Icon::red("⚠"),
            Cmd::RandomTraffic => Icon::amber("🎲"),
            Cmd::Record => Icon::red("⏺"),
            Cmd::ReplayPlay if playing => Icon::amber("⏸"),
            Cmd::ReplayPlay => Icon::green("▶"),
            Cmd::ReplayStart => Icon::blue("⏮"),
            Cmd::ReplayClose => Icon::red("⏏"),
            Cmd::Freeze => Icon::teal("❄"),
            Cmd::LeftPanel => Icon::purple("⬅"),
            Cmd::RightPanel => Icon::purple("➡"),
            Cmd::MonitorPanel => Icon::purple("⬇"),
            Cmd::Day => Icon::amber("☀"),
            Cmd::Dusk => Icon::purple("🌆"),
            Cmd::Night => Icon::red("🌙"),
            Cmd::Preferences => Icon::purple("⚙"),
        }
    }

    /// The ribbon's hint for the command, searched as well as its name.
    fn tip(self) -> &'static str {
        match self {
            Cmd::RunPause => t!("Start or stop simulated time"),
            Cmd::Reset => t!("Reload the scenario's initial state"),
            Cmd::SteerHeading => t!("Hold the current heading"),
            Cmd::SteerWind => t!("Hold the current true wind angle"),
            Cmd::SteerRoute => t!("Follow the waypoints"),
            Cmd::Open => t!("Open a scenario file (.toml)"),
            Cmd::SaveAs => t!("Save the current state as a scenario file"),
            Cmd::AddWaypoints => t!("Click the chart to add waypoints"),
            Cmd::Seamarks => t!("OpenSeaMap overlay: lights, buoys, …"),
            Cmd::ClearTrack => t!("Erase the vessel's track"),
            Cmd::AddTarget => t!("2 nm from own ship; right-click the chart to place one"),
            Cmd::Collision => t!("A Class A target that reaches own ship's predicted position (settings under Add ⏷)"),
            Cmd::RandomTraffic => t!("Random traffic around own ship (settings under Add ⏷)"),
            Cmd::AddOutput => t!("UDP, TCP, WebSocket, serial or file"),
            Cmd::Record => t!("Record everything to a file, with timestamps for replay"),
            Cmd::OpenLog => t!("Play a recorded NMEA 0183 / AIS log to the outputs"),
            Cmd::ReplayStart => t!("Back to the start of the log"),
            Cmd::Freeze => t!("Stop scrolling the traffic monitor"),
            Cmd::Day => t!("Light, for daylight"),
            Cmd::Dusk => t!("Dark, for dusk and dim rooms"),
            Cmd::Night => t!("Dim red on black, to keep night vision"),
            Cmd::Preferences => t!("Palette, language and output folder (⌘,)"),
            _ => "",
        }
    }

    /// Where the command lives on the ribbon.
    pub fn place(self) -> &'static str {
        match self {
            Cmd::RunPause | Cmd::Reset | Cmd::RealTime => t!("Home › Simulation"),
            Cmd::SteerHeading | Cmd::SteerWind | Cmd::SteerRoute => t!("Home › Steering"),
            Cmd::Engine => t!("Home › Propulsion"),
            Cmd::Open | Cmd::SaveAs => t!("Scenario › File"),
            Cmd::AddWaypoints | Cmd::ClearRoute => t!("Chart › Route"),
            Cmd::Seamarks | Cmd::ClearTrack => t!("Chart › Display"),
            Cmd::AddTarget | Cmd::Collision | Cmd::RandomTraffic => t!("AIS traffic › Add"),
            Cmd::RemoveTargets => t!("AIS traffic › Targets"),
            Cmd::AddOutput | Cmd::Record => t!("Data › Outputs"),
            Cmd::OpenLog | Cmd::ReplayPlay | Cmd::ReplayStart | Cmd::ReplayClose => t!("Data › Replay"),
            Cmd::Freeze => t!("Data › Traffic monitor"),
            Cmd::LeftPanel | Cmd::RightPanel | Cmd::MonitorPanel => t!("View › Panels"),
            Cmd::Day | Cmd::Dusk | Cmd::Night => t!("View › Palette"),
            Cmd::Preferences => t!("View › Settings"),
        }
    }

    /// Whether the command can run now (the ribbon greys the same ones out).
    pub fn enabled(self, app: &SimApp, snap: &Snapshot) -> bool {
        match self {
            Cmd::Engine => snap.sim.vessel_cfg.propulsion == Propulsion::Sail,
            Cmd::ClearRoute => !app.route_of(snap).0.is_empty(),
            Cmd::RemoveTargets => !snap.sim.ais_targets.is_empty(),
            Cmd::ReplayPlay | Cmd::ReplayStart | Cmd::ReplayClose => snap.replay.is_some(),
            _ => true,
        }
    }

    /// Runs the command, if it can run now.
    pub fn run(self, app: &mut SimApp, snap: &Snapshot, ctx: &egui::Context) {
        if !self.enabled(app, snap) {
            return;
        }
        let sim = &snap.sim;
        match self {
            Cmd::RunPause => app.engine.send(Command::SetPaused(!sim.paused)),
            Cmd::Reset => app.reset(),
            Cmd::RealTime => app.engine.send(Command::SetTimeScale(1.0)),
            Cmd::SteerHeading => app.steer(sim, SteerMode::Heading),
            Cmd::SteerWind => app.steer(sim, SteerMode::WindAngle),
            Cmd::SteerRoute => app.steer(sim, SteerMode::Route),
            Cmd::Engine => app.engine.send(Command::SetEngine(!sim.engine_on)),
            Cmd::Open => app.open_dialog(),
            Cmd::SaveAs => app.save_dialog(),
            Cmd::AddWaypoints => app.chart.add_waypoints = !app.chart.add_waypoints,
            Cmd::ClearRoute => app.clear_route(snap),
            Cmd::Seamarks => app.chart.show_seamarks = !app.chart.show_seamarks,
            Cmd::ClearTrack => app.chart.clear_track(),
            Cmd::AddTarget => app.add_target_nearby(snap),
            Cmd::Collision => app.add_collision_target(snap),
            Cmd::RandomTraffic => app.add_random_traffic(snap),
            Cmd::RemoveTargets => app.remove_all_targets(snap),
            Cmd::AddOutput => app.new_output(),
            Cmd::Record => app.record_to_file(),
            Cmd::OpenLog => app.open_log(),
            Cmd::ReplayPlay => {
                let playing = snap.replay.as_ref().is_some_and(|r| r.playing);
                app.engine.replay(ReplayCommand::Play(!playing));
            }
            Cmd::ReplayStart => app.engine.replay(ReplayCommand::Seek(0.0)),
            Cmd::ReplayClose => app.engine.replay(ReplayCommand::Close),
            Cmd::Freeze => {
                let frozen = app.monitor.paused;
                app.freeze_monitor(!frozen);
            }
            Cmd::LeftPanel => app.panels.controls = !app.panels.controls,
            Cmd::RightPanel => app.panels.io = !app.panels.io,
            Cmd::MonitorPanel => app.panels.monitor = !app.panels.monitor,
            Cmd::Day => theme::set_palette(ctx, Palette::Day),
            Cmd::Dusk => theme::set_palette(ctx, Palette::Dusk),
            Cmd::Night => theme::set_palette(ctx, Palette::Night),
            Cmd::Preferences => app.prefs.window_open = true,
        }
    }
}

/// The commands whose name, place or hint contains every word of `query`, ignoring case.
pub fn search(query: &str, snap: &Snapshot) -> Vec<Cmd> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    Cmd::ALL
        .iter()
        .copied()
        .filter(|c| {
            let hay = format!("{} {} {}", c.name(snap), c.place(), c.tip()).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        let mut scenario = engine::scenario::default_scenario();
        scenario.simulation.paused = true;
        let engine = engine::Engine::start(scenario, None).expect("engine");
        engine.snapshot().as_ref().clone()
    }

    #[test]
    fn search_matches_every_word_in_name_place_or_hint() {
        let snap = snapshot();
        assert_eq!(search("", &snap).len(), Cmd::ALL.len());
        assert_eq!(search("clear route", &snap), vec![Cmd::ClearRoute]);
        let replay = search("replay", &snap);
        assert!(replay.contains(&Cmd::OpenLog) && replay.contains(&Cmd::ReplayClose), "{replay:?}");
        assert!(search("RUN", &snap).contains(&Cmd::RunPause), "case is ignored");
        assert!(search("no such command", &snap).is_empty());
    }
}
