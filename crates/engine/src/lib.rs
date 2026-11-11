//! Runtime glue shared by the GUI and the CLI: owns the simulation thread, the sensors, the
//! encoders and the outputs, and publishes read-only snapshots.
//!
//! ```text
//! GUI/CLI ──commands──▶ sim thread (10 Hz) ──▶ sensors ──▶ encoders ──▶ OutputManager ──▶ UDP/TCP…
//!                                                    (0183, N2K, Signal K)
//!    ▲                        │
//!    └──── Arc<Snapshot> ◀────┘ (arc-swap, lock-free reads)
//! ```
//!
//! A recorded log can be replayed to the same outputs (`replay`), alongside or instead of the
//! simulated data.

pub mod replay;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender};
use protocols::{n2k, nmea0183, signalk};
use scenario::Scenario;
use replay::{Player, ReplayLog, Timing};
use sim_core::{Command, LatLon, Simulation};
use sim_sensors::{SensorConfig, SensorKind, SensorSuite};
use transports::{DataFormat, OutputManager};

pub use scenario;
pub use sim_core;
pub use sim_sensors;
pub use transports;

/// Simulation loop frequency (wall clock).
pub const TICK_HZ: f64 = 10.0;
const MONITOR_CAPACITY: usize = 2000;

/// Read-only view of the engine, published after every tick.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub sim: Simulation,
    pub sensors: Vec<SensorConfig>,
    pub tick: u64,
    /// Lines sent per second, all formats (smoothed).
    pub sentences_per_sec: f64,
    pub replay: Option<ReplayStatus>,
}

/// State of the log being replayed, for display.
#[derive(Debug, Clone)]
pub struct ReplayStatus {
    pub name: String,
    pub timing: Timing,
    pub lines: usize,
    pub cursor: usize,
    /// Playback position and length, seconds.
    pub position: f64,
    pub duration: f64,
    /// UTC time at the playback position, when the log carries one.
    pub time_unix: Option<f64>,
    pub playing: bool,
    pub speed: f64,
    pub looped: bool,
    pub mute_simulation: bool,
    pub last_fix: Option<LatLon>,
}

impl ReplayStatus {
    fn of(p: &Player) -> Self {
        Self {
            name: p.log.name.clone(),
            timing: p.log.timing,
            lines: p.log.entries.len(),
            cursor: p.cursor,
            position: p.clock,
            duration: p.log.duration(),
            time_unix: p.time_unix(),
            playing: p.playing,
            speed: p.speed,
            looped: p.looped,
            mute_simulation: p.mute_simulation,
            last_fix: p.last_fix,
        }
    }
}

/// Where a monitored line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Sensor(SensorKind),
    /// Own-ship identity (Signal K).
    Vessel,
    Replay,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Sensor(k) => k.label(),
            Source::Vessel => "Vessel",
            Source::Replay => "Replay",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MonitorLine {
    pub seq: u64,
    pub sim_time_unix: f64,
    pub source: Source,
    pub format: DataFormat,
    /// Sentence formatter (`RMC`), PGN (`129025`) or Signal K sensor (`gps`), used for output
    /// filters.
    pub tag: String,
    pub text: String,
}

/// Replay controls.
#[derive(Debug, Clone)]
pub enum ReplayCommand {
    Load(Box<ReplayLog>),
    Close,
    Play(bool),
    /// Seconds from the start of the log.
    Seek(f64),
    Speed(f64),
    Loop(bool),
    MuteSimulation(bool),
}

/// Largest log accepted for replay (it is held in memory).
pub const MAX_REPLAY_BYTES: u64 = 512 * 1024 * 1024;

/// Recent output traffic for display, in one format at a time (`view`). The engine also
/// encodes the viewed format when no output uses it, so it can always be inspected.
#[derive(Debug, Default)]
pub struct Monitor {
    pub lines: VecDeque<MonitorLine>,
    pub next_seq: u64,
    pub view: DataFormat,
}

enum EngineCommand {
    Sim(Command),
    SetSensor(SensorConfig),
    Load(Box<Simulation>, Vec<SensorConfig>, u64),
    Replay(ReplayCommand),
}

pub struct Engine {
    snapshot: Arc<ArcSwap<Snapshot>>,
    tx: Sender<EngineCommand>,
    outputs: Arc<OutputManager>,
    monitor: Arc<Mutex<Monitor>>,
    scenario: Mutex<Scenario>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub fn now_unix() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl Engine {
    /// Starts the simulation thread and the scenario's outputs. `on_tick` is called after each
    /// tick (e.g. to request a GUI repaint).
    pub fn start(scenario: Scenario, on_tick: Option<Box<dyn Fn() + Send>>) -> Result<Engine> {
        Self::start_with_replay(scenario, on_tick, None)
    }

    /// Like `start`, already playing `log` (so no simulated data goes out before it).
    pub fn start_with_replay(
        scenario: Scenario,
        on_tick: Option<Box<dyn Fn() + Send>>,
        log: Option<ReplayLog>,
    ) -> Result<Engine> {
        let outputs = Arc::new(OutputManager::new()?);
        for o in &scenario.outputs {
            outputs.add(o.clone());
        }
        let sim = scenario.build_simulation(now_unix());
        let suite = SensorSuite::new(scenario.sensors.clone(), scenario.simulation.seed);
        let snapshot = Arc::new(ArcSwap::from_pointee(Snapshot {
            sim: sim.clone(),
            sensors: suite.configs(),
            tick: 0,
            sentences_per_sec: 0.0,
            replay: None,
        }));
        let (tx, rx) = crossbeam_channel::unbounded();
        let monitor = Arc::new(Mutex::new(Monitor::default()));
        let stop = Arc::new(AtomicBool::new(false));

        let worker = Worker {
            sim,
            suite,
            rx,
            snapshot: snapshot.clone(),
            outputs: outputs.clone(),
            monitor: monitor.clone(),
            stop: stop.clone(),
            on_tick,
            player: log.map(Player::new),
            n2k: n2k::Encoder::new(),
            formats: vec![],
            next_n2k_info: None,
            next_sk_identity: None,
            sk_context: String::new(),
        };
        let thread = std::thread::Builder::new().name("simulation".into()).spawn(move || worker.run())?;

        Ok(Engine {
            snapshot,
            tx,
            outputs,
            monitor,
            scenario: Mutex::new(scenario),
            stop,
            thread: Some(thread),
        })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(EngineCommand::Sim(cmd));
    }

    pub fn replay(&self, cmd: ReplayCommand) {
        let _ = self.tx.send(EngineCommand::Replay(cmd));
    }

    /// Reads and parses a log, then starts playing it.
    pub fn load_replay(&self, path: &std::path::Path) -> Result<()> {
        self.replay(ReplayCommand::Load(Box::new(read_log(path)?)));
        Ok(())
    }

    pub fn set_sensor(&self, cfg: SensorConfig) {
        let _ = self.tx.send(EngineCommand::SetSensor(cfg));
    }

    pub fn outputs(&self) -> &OutputManager {
        &self.outputs
    }

    pub fn monitor(&self) -> &Mutex<Monitor> {
        &self.monitor
    }

    /// Replaces the whole state with a scenario: simulation, sensors and outputs.
    pub fn load_scenario(&self, scenario: Scenario) {
        let sim = scenario.build_simulation(now_unix());
        let _ = self.tx.send(EngineCommand::Load(
            Box::new(sim),
            scenario.sensors.clone(),
            scenario.simulation.seed,
        ));
        for info in self.outputs.list() {
            self.outputs.remove(info.id);
        }
        for o in &scenario.outputs {
            self.outputs.add(o.clone());
        }
        *self.scenario.lock().expect("scenario lock") = scenario;
    }

    /// The scenario last loaded (or adopted with [`Engine::rebase`]): what "Reset" goes back to.
    pub fn scenario(&self) -> Scenario {
        self.scenario.lock().expect("scenario lock").clone()
    }

    /// Makes `scenario` the baseline without touching the running state (after "Save as").
    pub fn rebase(&self, scenario: Scenario) {
        *self.scenario.lock().expect("scenario lock") = scenario;
    }

    /// The current state as a scenario, suitable for saving.
    pub fn capture_scenario(&self) -> Scenario {
        let snap = self.snapshot();
        let base = self.scenario.lock().expect("scenario lock").clone();
        base.capture(&snap.sim, snap.sensors.clone(), self.outputs.configs())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    sim: Simulation,
    suite: SensorSuite,
    rx: Receiver<EngineCommand>,
    snapshot: Arc<ArcSwap<Snapshot>>,
    outputs: Arc<OutputManager>,
    monitor: Arc<Mutex<Monitor>>,
    stop: Arc<AtomicBool>,
    on_tick: Option<Box<dyn Fn() + Send>>,
    player: Option<Player>,
    n2k: n2k::Encoder,
    /// Formats to encode this tick: those of the enabled outputs, plus the monitor view.
    formats: Vec<DataFormat>,
    /// Simulated time of the next NMEA 2000 device announcement (address claim, product
    /// information); `None` while no NMEA 2000 output is enabled.
    next_n2k_info: Option<f64>,
    /// Same for the Signal K own-ship identity delta.
    next_sk_identity: Option<f64>,
    sk_context: String,
}

impl Worker {
    fn run(mut self) {
        let period = Duration::from_secs_f64(1.0 / TICK_HZ);
        let mut last = Instant::now();
        let mut next = last + period;
        let mut tick = 0u64;
        let mut rate = 0.0;
        while !self.stop.load(Ordering::Relaxed) {
            while let Ok(cmd) = self.rx.try_recv() {
                match cmd {
                    EngineCommand::Sim(c) => self.sim.apply(c),
                    EngineCommand::SetSensor(cfg) => self.suite.set_config(cfg),
                    EngineCommand::Load(sim, sensors, seed) => {
                        self.sim = *sim;
                        self.suite = SensorSuite::new(sensors, seed);
                    }
                    EngineCommand::Replay(c) => self.replay_command(c),
                }
            }

            let now = Instant::now();
            // Clamp so a stalled thread (laptop sleep, debugger) does not jump the boat.
            let dt = now.duration_since(last).as_secs_f64().min(1.0);
            last = now;
            self.sim.advance(dt);

            self.refresh_formats();
            let muted = self.player.as_ref().is_some_and(|p| p.mute_simulation);
            let mut emitted = if muted {
                // Keep the sensors' schedules running so unmuting does not cause a burst.
                let _ = self.suite.poll(&self.sim);
                0
            } else {
                self.announce() + self.emit()
            };
            emitted += self.play(dt);
            // Exponential smoothing over ~2 s.
            rate += (emitted as f64 / dt.max(1e-3) - rate) * 0.05;

            tick += 1;
            self.snapshot.store(Arc::new(Snapshot {
                sim: self.sim.clone(),
                sensors: self.suite.configs(),
                tick,
                sentences_per_sec: rate,
                replay: self.player.as_ref().map(ReplayStatus::of),
            }));
            if let Some(cb) = &self.on_tick {
                cb();
            }

            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
                next += period;
            } else {
                // Overran: don't try to catch up with a burst of ticks.
                next = now + period;
            }
        }
    }

    /// Works out which formats to encode this tick, and keeps the Signal K hello current.
    fn refresh_formats(&mut self) {
        let mut formats = self.outputs.formats_in_use();
        let view = self.monitor.lock().expect("monitor lock").view;
        if !formats.contains(&view) {
            formats.push(view);
        }
        let now = self.sim.time_unix;
        // Announce devices as soon as a format appears, then periodically.
        let n2k = formats.iter().any(|f| f.is_n2k());
        self.next_n2k_info = match (n2k, self.next_n2k_info) {
            (false, _) => None,
            (true, None) => Some(now),
            (true, t) => t,
        };
        let sk = formats.contains(&DataFormat::SignalK);
        self.next_sk_identity = match (sk, self.next_sk_identity) {
            (false, _) => None,
            (true, None) => Some(now),
            (true, t) => t,
        };
        let ctx = signalk::vessel_context(self.sim.vessel_cfg.mmsi);
        if ctx != self.sk_context {
            self.outputs.set_signalk_hello(signalk::hello(&ctx, now));
            self.sk_context = ctx;
            self.next_sk_identity = self.next_sk_identity.map(|_| now);
        }
        self.formats = formats;
    }

    /// Periodic messages that are not measurements: NMEA 2000 address claims and product
    /// information (every minute), and the Signal K own-ship identity (every 30 s).
    fn announce(&mut self) -> usize {
        let now = self.sim.time_unix;
        let mut count = 0;
        let mut mon = self.monitor.lock().expect("monitor lock");
        if self.next_n2k_info.is_some_and(|t| now >= t) {
            self.next_n2k_info = Some(now + 60.0);
            let version = env!("CARGO_PKG_VERSION");
            for cfg in self.suite.configs().iter().filter(|c| c.enabled) {
                for m in n2k::device_info(cfg.kind, cfg.kind.label(), version) {
                    count += send_n2k(&mut self.n2k, &self.outputs, &mut mon, &self.formats, Source::Sensor(cfg.kind), &m, now);
                }
            }
        }
        if self.next_sk_identity.is_some_and(|t| now >= t) {
            self.next_sk_identity = Some(now + 30.0);
            let v = &self.sim.vessel_cfg;
            let delta = signalk::identity(
                &signalk::Identity {
                    mmsi: v.mmsi,
                    name: &v.name,
                    call_sign: &v.call_sign,
                    length_m: v.length_m,
                    beam_m: v.beam_m,
                    draft_m: v.draft_m,
                },
                now,
            );
            send_line(&self.outputs, &mut mon, Source::Vessel, DataFormat::SignalK, "vessel", &delta, now);
            count += 1;
        }
        count
    }

    /// Polls the sensors and sends their data in every format in use. Returns the number of
    /// lines sent.
    fn emit(&mut self) -> usize {
        let emissions = self.suite.poll(&self.sim);
        if emissions.is_empty() {
            return 0;
        }
        let mut count = 0;
        let mut mon = self.monitor.lock().expect("monitor lock");
        let n2k = self.formats.iter().any(|f| f.is_n2k());
        for e in &emissions {
            let source = Source::Sensor(e.sensor);
            if self.formats.contains(&DataFormat::Nmea0183) {
                for line in nmea0183::encode(e) {
                    let tag = nmea0183::sentence_id(&line).unwrap_or("").to_string();
                    send_line(&self.outputs, &mut mon, source, DataFormat::Nmea0183, &tag, &line, e.time_unix);
                    count += 1;
                }
            }
            if n2k {
                for m in self.n2k.encode(e) {
                    count += send_n2k(&mut self.n2k, &self.outputs, &mut mon, &self.formats, source, &m, e.time_unix);
                }
            }
            if self.formats.contains(&DataFormat::SignalK)
                && let Some(delta) = signalk::encode(e, &self.sk_context)
            {
                send_line(&self.outputs, &mut mon, source, DataFormat::SignalK, signalk::sensor_key(e.sensor), &delta, e.time_unix);
                count += 1;
            }
        }
        count
    }

    fn replay_command(&mut self, c: ReplayCommand) {
        if let ReplayCommand::Load(log) = c {
            // Keep the user's playback preferences across logs.
            let prefs = self.player.as_ref().map(|p| (p.speed, p.looped, p.mute_simulation));
            let mut p = Player::new(*log);
            if let Some((speed, looped, mute)) = prefs {
                (p.speed, p.looped, p.mute_simulation) = (speed, looped, mute);
            }
            self.player = Some(p);
            return;
        }
        if let ReplayCommand::Close = c {
            self.player = None;
            return;
        }
        let Some(p) = &mut self.player else { return };
        match c {
            ReplayCommand::Play(on) => {
                if on && !p.playing && p.cursor >= p.log.entries.len() {
                    p.seek(0.0); // play again from the start
                }
                p.playing = on;
            }
            ReplayCommand::Seek(t) => p.seek(t),
            ReplayCommand::Speed(s) => p.speed = s.clamp(0.01, 1000.0),
            ReplayCommand::Loop(l) => p.looped = l,
            ReplayCommand::MuteSimulation(m) => p.mute_simulation = m,
            ReplayCommand::Load(_) | ReplayCommand::Close => unreachable!(),
        }
    }

    /// Sends the replayed lines that are due. Returns how many were sent.
    fn play(&mut self, dt: f64) -> usize {
        let Some(p) = &mut self.player else { return 0 };
        let due = p.advance(dt);
        if due.is_empty() {
            return 0;
        }
        let log = p.log.clone();
        let base = log.start_unix;
        let mut mon = self.monitor.lock().expect("monitor lock");
        for e in &log.entries[due.clone()] {
            let t = base.map_or(self.sim.time_unix, |b| b + e.t);
            let tag = nmea0183::sentence_id(&e.line).unwrap_or("");
            send_line(&self.outputs, &mut mon, Source::Replay, DataFormat::Nmea0183, tag, &e.line, t);
        }
        p.sent += due.len() as u64;
        due.len()
    }
}

/// Reads and parses a log file for replay.
pub fn read_log(path: &std::path::Path) -> Result<ReplayLog> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    anyhow::ensure!(size <= MAX_REPLAY_BYTES, "{} is too large to replay ({} MB)", path.display(), size >> 20);
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    ReplayLog::parse(&name, &text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

fn send_line(
    outputs: &OutputManager,
    mon: &mut Monitor,
    source: Source,
    format: DataFormat,
    tag: &str,
    line: &str,
    time_unix: f64,
) {
    outputs.send_format(format, tag, line, time_unix);
    if format != mon.view {
        return;
    }
    let seq = mon.next_seq;
    mon.next_seq += 1;
    if mon.lines.len() >= MONITOR_CAPACITY {
        mon.lines.pop_front();
    }
    mon.lines.push_back(MonitorLine {
        seq,
        sim_time_unix: time_unix,
        source,
        format,
        tag: tag.to_string(),
        text: line.to_string(),
    });
}

/// Sends one NMEA 2000 message in every NMEA 2000 format in use. Returns the lines sent.
fn send_n2k(
    enc: &mut n2k::Encoder,
    outputs: &OutputManager,
    mon: &mut Monitor,
    formats: &[DataFormat],
    source: Source,
    m: &n2k::N2kMessage,
    time_unix: f64,
) -> usize {
    let tag = m.pgn.to_string();
    let mut count = 0;
    for &f in formats {
        let text = match f {
            DataFormat::YdRaw => n2k::TextFormat::YdRaw,
            DataFormat::ActisenseAscii => n2k::TextFormat::ActisenseAscii,
            DataFormat::CanboatPlain => n2k::TextFormat::CanboatPlain,
            _ => continue,
        };
        for line in enc.lines(text, m, time_unix) {
            send_line(outputs, mon, source, f, &tag, &line, time_unix);
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One simulation feeds NMEA 0183, NMEA 2000 and Signal K outputs at the same time.
    #[test]
    fn every_format_reaches_its_outputs() {
        let socket = || {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            s
        };
        let (rx0183, rxn2k, rxsk) = (socket(), socket(), socket());
        let out = |name: &str, rx: &std::net::UdpSocket, format| transports::OutputConfig {
            name: name.into(),
            enabled: true,
            kind: transports::OutputKind::Udp { target: rx.local_addr().unwrap().to_string(), broadcast: false },
            format,
            filter: vec![],
        };
        let mut s = scenario::default_scenario();
        s.outputs = vec![
            out("0183", &rx0183, DataFormat::Nmea0183),
            out("n2k", &rxn2k, DataFormat::CanboatPlain),
            out("sk", &rxsk, DataFormat::SignalK),
        ];
        let own_name = format!("\"name\":\"{}\"", s.vessel.name);
        let _engine = Engine::start(s, None).unwrap();
        let read = |rx: &std::net::UdpSocket, n: usize| -> Vec<String> {
            let mut buf = [0u8; 2048];
            (0..n).map(|_| {
                let k = rx.recv(&mut buf).unwrap();
                String::from_utf8_lossy(&buf[..k]).trim_end().to_string()
            }).collect()
        };
        let lines = read(&rx0183, 20);
        assert!(lines.iter().all(|l| l.starts_with('$') || l.starts_with('!')), "{lines:?}");
        // NMEA 2000: devices announce themselves first, then measurements follow.
        let lines = read(&rxn2k, 60);
        let pgns: Vec<u32> = lines.iter().map(|l| l.split(',').nth(2).unwrap().parse().unwrap()).collect();
        assert_eq!(pgns[..2], [60928, 126996], "{lines:?}");
        assert!(pgns.contains(&129025) && pgns.contains(&127250), "{pgns:?}");
        // Signal K: own-ship identity, then deltas in the own-ship context.
        let lines = read(&rxsk, 10);
        assert!(lines[0].contains("\"marine-sim.vessel\"") && lines[0].contains(&own_name), "{}", lines[0]);
        assert!(lines.iter().all(|l| l.starts_with("{\"context\":\"vessels.urn:mrn:imo:mmsi:")), "{lines:?}");
    }

    #[test]
    fn engine_emits_over_udp() {
        let rx = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut s = scenario::default_scenario();
        s.outputs = vec![transports::OutputConfig {
            name: "test".into(),
            enabled: true,
            kind: transports::OutputKind::Udp { target: rx.local_addr().unwrap().to_string(), broadcast: false },
            format: transports::DataFormat::Nmea0183,
            filter: vec!["RMC".into()],
        }];
        let engine = Engine::start(s, None).unwrap();
        let mut buf = [0u8; 512];
        let n = rx.recv(&mut buf).unwrap();
        let line = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(line.starts_with("$GPRMC,") && line.ends_with("\r\n"), "{line}");
        assert!(nmea0183::verify(line));
        assert!(engine.snapshot().tick > 0 || engine.monitor().lock().unwrap().next_seq > 0);
    }

    #[test]
    fn record_then_replay() {
        let path = std::env::temp_dir().join(format!("marine-sim-rec-{}.nmea", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // Record ~1.5 s of simulated data (10x speed: 15 simulated seconds).
        let mut s = scenario::default_scenario();
        s.simulation.time_scale = 10.0;
        s.outputs = vec![transports::OutputConfig {
            name: "rec".into(),
            enabled: true,
            kind: transports::OutputKind::File { path: path.display().to_string(), tag_blocks: true },
            format: transports::DataFormat::Nmea0183,
            filter: vec![],
        }];
        let engine = Engine::start(s, None).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        drop(engine);
        let recorded = std::fs::read_to_string(&path).unwrap();
        assert!(recorded.lines().count() > 50, "{} lines", recorded.lines().count());
        if let Some(bad) = recorded.lines().find(|l| !l.starts_with("\\c:")) {
            panic!("untagged line: {bad:?}");
        }

        // Replay it to UDP with the simulation muted: only recorded lines arrive, in order.
        let rx = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut s = scenario::default_scenario();
        s.outputs = vec![transports::OutputConfig {
            name: "udp".into(),
            enabled: true,
            kind: transports::OutputKind::Udp { target: rx.local_addr().unwrap().to_string(), broadcast: false },
            format: transports::DataFormat::Nmea0183,
            filter: vec![],
        }];
        // Loaded at start (as the CLI does), so the simulation is muted from the first tick.
        let engine = Engine::start_with_replay(s, None, Some(read_log(&path).unwrap())).unwrap();
        engine.replay(ReplayCommand::Speed(100.0));
        let expected: Vec<&str> = recorded.lines().map(|l| l.rsplit('\\').next().unwrap()).collect();
        let mut buf = [0u8; 1024];
        for (i, e) in expected.iter().take(30).enumerate() {
            let n = rx.recv(&mut buf).unwrap();
            let line = std::str::from_utf8(&buf[..n]).unwrap().trim_end();
            assert_eq!(line, *e, "line {i}");
        }
        // The snapshot is published after the tick that sent the lines; wait for it.
        let t = Instant::now();
        let status = loop {
            if let Some(r) = engine.snapshot().replay.clone() {
                break r;
            }
            assert!(t.elapsed() < Duration::from_secs(2), "no replay status");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.timing, Timing::TagBlocks);
        assert!(status.duration > 5.0, "{}", status.duration);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn commands_reach_the_simulation() {
        let mut s = scenario::default_scenario();
        s.outputs.clear();
        let engine = Engine::start(s, None).unwrap();
        engine.send(Command::SetChannelOverride(
            sim_core::Channel::Depth,
            Some((sim_core::OverrideMode::Replace, 3.0)),
        ));
        let t = Instant::now();
        while engine.snapshot().sim.channels.get(sim_core::Channel::Depth) != 3.0 {
            assert!(t.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(20));
        }
        let captured = engine.capture_scenario();
        assert!(captured.overrides.channels.contains_key("depth"));
    }
}
