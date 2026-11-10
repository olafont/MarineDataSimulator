//! Network/file outputs. Each output is an async task fed from one broadcast channel; it filters
//! messages by tag (e.g. the NMEA sentence formatter) and keeps live statistics.
//!
//! Kinds: UDP (unicast/broadcast), TCP server, TCP client, WebSocket server, serial port and
//! file (recording, optionally with NMEA 4 TAG-block timestamps for replay).

mod file;
mod serial;
mod websocket;

pub use serial::available_ports;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::broadcast;
use tokio::task::{JoinHandle, JoinSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputKind {
    /// Sends each line as one datagram to `target` (`host:port`).
    Udp { target: String, #[serde(default)] broadcast: bool },
    /// Listens on `bind` (`addr:port`) and streams every line to all connected clients.
    TcpServer { bind: String },
    /// Connects to `target` (`host:port`) and streams to it, reconnecting when dropped.
    TcpClient { target: String },
    /// WebSocket server on `bind`: one text frame per line, to every connected client.
    WebSocket { bind: String },
    /// Serial port (`/dev/tty…`, `COM3`). NMEA 0183 is 4800 baud, AIS and high-speed 38400.
    Serial { port: String, #[serde(default = "default_baud")] baud: u32 },
    /// Appends every line to a file. `tag_blocks` prefixes each line with an NMEA 4 TAG block
    /// carrying the simulated time (`\c:<unix seconds>*hh\`), which replay uses for timing.
    File {
        path: String,
        #[serde(default = "yes")]
        tag_blocks: bool,
    },
}

fn default_baud() -> u32 {
    4800
}

/// What an output carries. NMEA 2000 is sent in one of the text formats gateways use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DataFormat {
    #[default]
    #[serde(rename = "nmea0183")]
    Nmea0183,
    /// NMEA 2000, Yacht Devices RAW (one CAN frame per line).
    #[serde(rename = "yd_raw")]
    YdRaw,
    /// NMEA 2000, Actisense N2K ASCII (one message per line).
    #[serde(rename = "actisense_ascii")]
    ActisenseAscii,
    /// NMEA 2000, canboat / Actisense plain CSV (one message per line).
    #[serde(rename = "canboat_plain")]
    CanboatPlain,
    /// Signal K delta messages (JSON, one per line).
    #[serde(rename = "signalk")]
    SignalK,
}

impl DataFormat {
    pub const ALL: &[DataFormat] =
        &[DataFormat::Nmea0183, DataFormat::YdRaw, DataFormat::ActisenseAscii, DataFormat::CanboatPlain, DataFormat::SignalK];

    pub fn label(self) -> &'static str {
        match self {
            DataFormat::Nmea0183 => "NMEA 0183",
            DataFormat::YdRaw => "NMEA 2000 · Yacht Devices RAW",
            DataFormat::ActisenseAscii => "NMEA 2000 · Actisense ASCII",
            DataFormat::CanboatPlain => "NMEA 2000 · canboat plain",
            DataFormat::SignalK => "Signal K",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            DataFormat::Nmea0183 => "0183",
            DataFormat::YdRaw => "N2K YD RAW",
            DataFormat::ActisenseAscii => "N2K Actisense",
            DataFormat::CanboatPlain => "N2K canboat",
            DataFormat::SignalK => "Signal K",
        }
    }

    /// What the output filter matches for this format.
    pub fn filter_hint(self) -> &'static str {
        match self {
            DataFormat::Nmea0183 => "sentences, e.g. RMC, GGA, VDM",
            DataFormat::SignalK => "sensors, e.g. gps, wind, ais",
            _ => "PGNs, e.g. 129025, 130306, 129038",
        }
    }

    pub fn is_n2k(self) -> bool {
        matches!(self, DataFormat::YdRaw | DataFormat::ActisenseAscii | DataFormat::CanboatPlain)
    }

    fn is_default(&self) -> bool {
        *self == DataFormat::Nmea0183
    }
}

impl OutputKind {
    pub fn describe(&self) -> String {
        match self {
            OutputKind::Udp { target, broadcast: true } => format!("UDP broadcast → {target}"),
            OutputKind::Udp { target, .. } => format!("UDP → {target}"),
            OutputKind::TcpServer { bind } => format!("TCP server {bind}"),
            OutputKind::TcpClient { target } => format!("TCP client → {target}"),
            OutputKind::WebSocket { bind } => format!("WebSocket ws://{bind}"),
            OutputKind::Serial { port, baud } => format!("Serial {port} @ {baud}"),
            OutputKind::File { path, .. } => format!("File {path}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputConfig {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: OutputKind,
    #[serde(default, skip_serializing_if = "DataFormat::is_default")]
    pub format: DataFormat,
    /// Tags to send: sentence formatters (`RMC`), PGNs (`129025`) or Signal K sensors (`gps`).
    /// Empty = everything.
    #[serde(default)]
    pub filter: Vec<String>,
}

fn yes() -> bool {
    true
}

impl OutputConfig {
    pub fn default_outputs() -> Vec<OutputConfig> {
        vec![
            OutputConfig {
                name: "TCP 10113".into(),
                enabled: true,
                kind: OutputKind::TcpServer { bind: "0.0.0.0:10113".into() },
                format: DataFormat::Nmea0183,
                filter: vec![],
            },
            OutputConfig {
                name: "UDP localhost 10113".into(),
                enabled: true,
                kind: OutputKind::Udp { target: "127.0.0.1:10113".into(), broadcast: false },
                format: DataFormat::Nmea0183,
                filter: vec![],
            },
        ]
    }

    /// Whether this output sends a message of `format` with `tag`.
    pub fn accepts(&self, format: DataFormat, tag: &str) -> bool {
        self.format == format && (self.filter.is_empty() || self.filter.iter().any(|f| f.eq_ignore_ascii_case(tag)))
    }

    pub(crate) fn takes(&self, msg: &Message) -> bool {
        self.accepts(msg.format, &msg.tag)
    }
}

/// One line on the wire, with its format and the tag used for filtering.
#[derive(Debug, Clone)]
pub struct Message {
    pub format: DataFormat,
    pub tag: Arc<str>,
    /// Payload including line ending.
    pub data: Arc<str>,
    /// Simulated UTC time of the line, Unix seconds.
    pub time_unix: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Disabled,
    Starting,
    Running,
    /// Running, but with a problem the user should know about.
    Warning(String),
    Failed(String),
}

#[derive(Debug, Default)]
pub(crate) struct Stats {
    sent: AtomicU64,
    bytes: AtomicU64,
    errors: AtomicU64,
    clients: AtomicU64,
    status: Mutex<Option<Status>>,
}

impl Stats {
    pub(crate) fn set_status(&self, s: Status) {
        *self.status.lock().expect("stats lock") = Some(s);
    }

    pub(crate) fn count(&self, bytes: usize) {
        self.sent.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub(crate) fn error(&self, n: u64) {
        self.errors.fetch_add(n, Ordering::Relaxed);
    }
}

/// Snapshot of an output for display.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub id: u64,
    pub config: OutputConfig,
    pub status: Status,
    pub sent: u64,
    pub bytes: u64,
    pub errors: u64,
    pub clients: u64,
}

struct Running {
    config: OutputConfig,
    stats: Arc<Stats>,
    task: Option<JoinHandle<()>>,
    /// Stops outputs that run on a blocking thread (serial), which `abort` cannot reach.
    cancel: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

pub struct OutputManager {
    runtime: tokio::runtime::Runtime,
    tx: broadcast::Sender<Message>,
    outputs: Mutex<BTreeMap<u64, Running>>,
    next_id: AtomicU64,
    signalk_hello: Arc<Mutex<String>>,
}

impl Drop for OutputManager {
    fn drop(&mut self) {
        // Cancel every output before the runtime shuts down: dropping a runtime waits for its
        // blocking threads (serial outputs), which only stop once cancelled.
        self.outputs.lock().map(|mut o| o.clear()).ok();
    }
}

impl OutputManager {
    pub fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("outputs")
            .enable_all()
            .build()
            .context("starting output runtime")?;
        let (tx, _) = broadcast::channel(8192);
        Ok(Self {
            runtime,
            tx,
            outputs: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            signalk_hello: Arc::new(Mutex::new(String::new())),
        })
    }

    /// Queues a line (without line ending) for every output whose filter accepts `tag`,
    /// stamped with the wall-clock time.
    pub fn send(&self, tag: &str, line: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.send_at(tag, line, now);
    }

    /// Like `send`, with the line's (simulated) time.
    pub fn send_at(&self, tag: &str, line: &str, time_unix: f64) {
        self.send_format(DataFormat::Nmea0183, tag, line, time_unix);
    }

    /// Queues a line of `format` for the outputs of that format whose filter accepts `tag`.
    pub fn send_format(&self, format: DataFormat, tag: &str, line: &str, time_unix: f64) {
        // No receivers is fine (no outputs running).
        let _ = self.tx.send(Message { format, tag: tag.into(), data: format!("{line}\r\n").into(), time_unix });
    }

    /// Names of the enabled outputs that send a `format` message with `tag`.
    pub fn accepting(&self, format: DataFormat, tag: &str) -> Vec<String> {
        self.outputs
            .lock()
            .expect("outputs lock")
            .values()
            .filter(|r| r.config.enabled && r.config.accepts(format, tag))
            .map(|r| r.config.name.clone())
            .collect()
    }

    /// The formats of the enabled outputs (so the engine only encodes what is needed).
    pub fn formats_in_use(&self) -> Vec<DataFormat> {
        let mut f: Vec<DataFormat> = self
            .outputs
            .lock()
            .expect("outputs lock")
            .values()
            .filter(|r| r.config.enabled)
            .map(|r| r.config.format)
            .collect();
        f.sort_by_key(|f| *f as u8);
        f.dedup();
        f
    }

    /// The message Signal K outputs send first on every TCP or WebSocket connection.
    pub fn set_signalk_hello(&self, hello: String) {
        *self.signalk_hello.lock().expect("hello lock") = hello;
    }

    pub fn add(&self, config: OutputConfig) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let running = self.start(config);
        self.outputs.lock().expect("outputs lock").insert(id, running);
        id
    }

    /// Replaces an output's configuration, restarting it.
    pub fn update(&self, id: u64, config: OutputConfig) {
        let mut outputs = self.outputs.lock().expect("outputs lock");
        if outputs.contains_key(&id) {
            // Drop the old task first so a TCP port can be re-bound.
            outputs.remove(&id);
            outputs.insert(id, self.start(config));
        }
    }

    pub fn remove(&self, id: u64) {
        self.outputs.lock().expect("outputs lock").remove(&id);
    }

    pub fn configs(&self) -> Vec<OutputConfig> {
        self.outputs.lock().expect("outputs lock").values().map(|r| r.config.clone()).collect()
    }

    pub fn list(&self) -> Vec<OutputInfo> {
        self.outputs
            .lock()
            .expect("outputs lock")
            .iter()
            .map(|(&id, r)| OutputInfo {
                id,
                config: r.config.clone(),
                status: r.stats.status.lock().expect("stats lock").clone().unwrap_or(Status::Starting),
                sent: r.stats.sent.load(Ordering::Relaxed),
                bytes: r.stats.bytes.load(Ordering::Relaxed),
                errors: r.stats.errors.load(Ordering::Relaxed),
                clients: r.stats.clients.load(Ordering::Relaxed),
            })
            .collect()
    }

    fn start(&self, config: OutputConfig) -> Running {
        let stats = Arc::new(Stats::default());
        let cancel = Arc::new(AtomicBool::new(false));
        if !config.enabled {
            stats.set_status(Status::Disabled);
            return Running { config, stats, task: None, cancel };
        }
        stats.set_status(Status::Starting);
        let rx = self.tx.subscribe();
        let cfg = config.clone();
        let st = stats.clone();
        let cn = cancel.clone();
        let hello = (config.format == DataFormat::SignalK).then(|| self.signalk_hello.clone());
        let task = self.runtime.spawn(async move {
            let result = match &cfg.kind {
                OutputKind::Udp { target, broadcast } => {
                    run_udp(&cfg, target, *broadcast, rx, &st).await
                }
                OutputKind::TcpServer { bind } => run_tcp(&cfg, bind, rx, &st, hello).await,
                OutputKind::TcpClient { target } => run_tcp_client(&cfg, target, rx, &st).await,
                OutputKind::WebSocket { bind } => websocket::run(&cfg, bind, rx, &st, hello).await,
                OutputKind::Serial { port, baud } => {
                    let (cfg2, port, baud, st2) = (cfg.clone(), port.clone(), *baud, st.clone());
                    tokio::task::spawn_blocking(move || serial::run(&cfg2, &port, baud, rx, &st2, &cn))
                        .await
                        .unwrap_or_else(|e| Err(anyhow::anyhow!("serial thread: {e}")))
                }
                OutputKind::File { path, tag_blocks } => {
                    // TAG blocks are NMEA 0183; the other formats carry their own timestamps.
                    let tags = *tag_blocks && cfg.format == DataFormat::Nmea0183;
                    file::run(&cfg, path, tags, rx, &st).await
                }
            };
            if let Err(e) = result {
                tracing::warn!(output = %cfg.name, "output failed: {e:#}");
                st.set_status(Status::Failed(format!("{e:#}")));
            }
        });
        Running { config, stats, task: Some(task), cancel }
    }
}

/// Status while an output is up, noting lines dropped because it could not keep up.
pub(crate) fn lag_status(stats: &Stats, dropped: u64, what: &str) {
    if dropped > 0 {
        stats.error(dropped);
        stats.set_status(Status::Warning(format!("{dropped} lines dropped: {what} too slow for the traffic")));
    }
}

/// Streams to a remote TCP server, reconnecting every 2 s while it is unreachable.
async fn run_tcp_client(
    cfg: &OutputConfig,
    target: &str,
    mut rx: broadcast::Receiver<Message>,
    stats: &Stats,
) -> Result<()> {
    loop {
        stats.set_status(Status::Warning(format!("connecting to {target}…")));
        let mut stream = match tokio::net::TcpStream::connect(target).await {
            Ok(s) => s,
            Err(e) => {
                stats.set_status(Status::Warning(format!("{target}: {e}; retrying")));
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        stats.set_status(Status::Running);
        stats.clients.store(1, Ordering::Relaxed);
        // Lines queued while disconnected are stale.
        rx = rx.resubscribe();
        loop {
            match rx.recv().await {
                Ok(msg) if cfg.takes(&msg) => {
                    if let Err(e) = stream.write_all(msg.data.as_bytes()).await {
                        tracing::info!(output = %cfg.name, "{target}: {e}");
                        break;
                    }
                    stats.count(msg.data.len());
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => lag_status(stats, n, "the connection"),
                Err(broadcast::error::RecvError::Closed) => return Ok(()),
            }
        }
        stats.clients.store(0, Ordering::Relaxed);
    }
}

async fn run_udp(
    cfg: &OutputConfig,
    target: &str,
    broadcast: bool,
    mut rx: broadcast::Receiver<Message>,
    stats: &Stats,
) -> Result<()> {
    let addr: SocketAddr = tokio::net::lookup_host(target)
        .await
        .with_context(|| format!("resolving {target}"))?
        .next()
        .with_context(|| format!("no address for {target}"))?;
    let bind = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let sock = UdpSocket::bind(bind).await.context("binding UDP socket")?;
    sock.set_broadcast(broadcast).context("enabling broadcast")?;
    stats.set_status(Status::Running);
    loop {
        match rx.recv().await {
            Ok(msg) if cfg.takes(&msg) => match sock.send_to(msg.data.as_bytes(), addr).await {
                Ok(n) => {
                    stats.sent.fetch_add(1, Ordering::Relaxed);
                    stats.bytes.fetch_add(n as u64, Ordering::Relaxed);
                }
                Err(e) => {
                    // Typical: no route / host unreachable. Keep going; it may recover.
                    stats.errors.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!("udp send to {addr}: {e}");
                }
            },
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                stats.errors.fetch_add(n, Ordering::Relaxed);
            }
            Err(broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

/// The Signal K hello line, when this output sends one.
pub(crate) fn hello_line(hello: Option<&Arc<Mutex<String>>>) -> Option<String> {
    let h = hello?.lock().expect("hello lock").clone();
    (!h.is_empty()).then(|| format!("{h}\r\n"))
}

/// Binds a listener; a restarted output may race the previous listener's (asynchronous)
/// shutdown, so `AddrInUse` is retried for about 2 s.
pub(crate) async fn bind_listener(bind: &str) -> Result<TcpListener> {
    let mut attempt = 0;
    loop {
        match TcpListener::bind(bind).await {
            Ok(l) => return Ok(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempt < 20 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e).with_context(|| format!("binding {bind}")),
        }
    }
}

async fn run_tcp(
    cfg: &OutputConfig,
    bind: &str,
    rx: broadcast::Receiver<Message>,
    stats: &Arc<Stats>,
    hello: Option<Arc<Mutex<String>>>,
) -> Result<()> {
    let listener = bind_listener(bind).await?;
    stats.set_status(match loopback_conflict(&listener) {
        Some(w) => Status::Warning(w),
        None => Status::Running,
    });
    // Owning the client tasks here means aborting this task disconnects every client.
    let mut clients = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (mut stream, peer) = match accepted {
                    Ok(a) => a,
                    Err(e) => {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!("accept: {e}");
                        continue;
                    }
                };
                let _ = stream.set_nodelay(true);
                tracing::info!(output = %cfg.name, "client connected: {peer}");
                let mut rx = rx.resubscribe();
                let stats = stats.clone();
                let greeting = hello_line(hello.as_ref());
                let cfg = cfg.clone();
                stats.clients.fetch_add(1, Ordering::Relaxed);
                clients.spawn(async move {
                    if let Some(g) = greeting
                        && stream.write_all(g.as_bytes()).await.is_err()
                    {
                        stats.clients.fetch_sub(1, Ordering::Relaxed);
                        return;
                    }
                    loop {
                        match rx.recv().await {
                            Ok(msg) if cfg.takes(&msg) => {
                                if stream.write_all(msg.data.as_bytes()).await.is_err() {
                                    break;
                                }
                                stats.sent.fetch_add(1, Ordering::Relaxed);
                                stats.bytes.fetch_add(msg.data.len() as u64, Ordering::Relaxed);
                            }
                            Ok(_) => {}
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                stats.errors.fetch_add(n, Ordering::Relaxed);
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    stats.clients.fetch_sub(1, Ordering::Relaxed);
                    tracing::info!(output = %cfg.name, "client disconnected: {peer}");
                });
            }
            Some(_) = clients.join_next(), if !clients.is_empty() => {}
        }
    }
}

/// A wildcard listener (`0.0.0.0:P`) can coexist with another process bound to
/// `127.0.0.1:P`; local clients then reach that process, not us. Detect it by trying to bind
/// the loopback address ourselves.
///
/// Linux needs no check (and the probe would misfire): its kernel refuses overlapping
/// wildcard/loopback binds while either socket listens, so the shadowing cannot happen and our
/// own listener would make the probe fail.
fn loopback_conflict(listener: &TcpListener) -> Option<String> {
    if cfg!(any(target_os = "linux", target_os = "android")) {
        return None;
    }
    let addr = listener.local_addr().ok()?;
    if !addr.ip().is_unspecified() {
        return None;
    }
    let lo: SocketAddr = if addr.is_ipv4() {
        ([127, 0, 0, 1], addr.port()).into()
    } else {
        (std::net::Ipv6Addr::LOCALHOST, addr.port()).into()
    };
    match std::net::TcpListener::bind(lo) {
        Ok(_) => None,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Some(format!(
            "another program is listening on {lo}; local clients connecting to {lo} reach it, not the simulator"
        )),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_running(m: &OutputManager, id: u64) {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if m.list().iter().any(|o| o.id == id && o.status == Status::Running) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("output {id} not running: {:?}", m.list());
    }

    #[test]
    fn warns_when_loopback_port_is_taken_by_someone_else() {
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = other.local_addr().unwrap().port();
        let m = OutputManager::new().unwrap();
        m.add(OutputConfig {
            name: "t".into(),
            enabled: true,
            kind: OutputKind::TcpServer { bind: format!("0.0.0.0:{port}") },
            format: DataFormat::Nmea0183,
            filter: vec![],
        });
        let t = Instant::now();
        loop {
            match &m.list()[0].status {
                Status::Warning(w) => {
                    assert!(w.contains("127.0.0.1"), "{w}");
                    break;
                }
                // Some platforms refuse the wildcard bind outright; that is also a visible error.
                Status::Failed(_) => break,
                s => assert!(t.elapsed() < Duration::from_secs(5), "status {s:?}"),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn no_warning_when_the_port_is_free() {
        let port = {
            let l = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let m = OutputManager::new().unwrap();
        let id = m.add(OutputConfig {
            name: "t".into(),
            enabled: true,
            kind: OutputKind::TcpServer { bind: format!("0.0.0.0:{port}") },
            format: DataFormat::Nmea0183,
            filter: vec![],
        });
        // Fails (status Warning) if the conflict probe mistakes our own listener for another
        // program's, as it would on Linux.
        wait_running(&m, id);
    }

    fn add(m: &OutputManager, kind: OutputKind, filter: &[&str]) -> u64 {
        m.add(OutputConfig { name: "t".into(), enabled: true, kind, format: DataFormat::Nmea0183, filter: filter.iter().map(|s| s.to_string()).collect() })
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn file_records_with_tag_blocks() {
        let path = std::env::temp_dir().join(format!("marine-sim-file-{}.nmea", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let m = OutputManager::new().unwrap();
        let id = add(&m, OutputKind::File { path: path.display().to_string(), tag_blocks: true }, &["RMC"]);
        wait_running(&m, id);
        m.send_at("RMC", "$GPRMC,a", 1_782_052_215.2);
        m.send_at("GGA", "$GPGGA,dropped", 1_782_052_215.3);
        m.send_at("RMC", "$GPRMC,b", 1_782_052_216.0);
        let t = Instant::now();
        let text = loop {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.lines().count() >= 2 || t.elapsed() > Duration::from_secs(5) {
                break text;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].starts_with("\\c:1782052215*") && lines[0].ends_with("\\$GPRMC,a"), "{}", lines[0]);
        assert!(lines[1].contains("c:1782052216") && lines[1].ends_with("$GPRMC,b"));
        m.remove(id);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn websocket_sends_one_frame_per_line() {
        use futures_util::StreamExt as _;
        let port = free_port();
        let m = OutputManager::new().unwrap();
        let id = add(&m, OutputKind::WebSocket { bind: format!("127.0.0.1:{port}") }, &[]);
        wait_running(&m, id);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/")).await.unwrap();
            let t = Instant::now();
            while m.list()[0].clients == 0 {
                assert!(t.elapsed() < Duration::from_secs(5));
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            m.send("HDT", "$HCHDT,123.4,T*00");
            let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
            assert_eq!(msg.into_text().unwrap().as_str(), "$HCHDT,123.4,T*00");
        });
    }

    #[test]
    fn outputs_only_receive_their_format() {
        let rx = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let m = OutputManager::new().unwrap();
        let id = m.add(OutputConfig {
            name: "n2k".into(),
            enabled: true,
            kind: OutputKind::Udp { target: rx.local_addr().unwrap().to_string(), broadcast: false },
            format: DataFormat::YdRaw,
            filter: vec!["129025".into()],
        });
        wait_running(&m, id);
        assert_eq!(m.formats_in_use(), [DataFormat::YdRaw]);
        assert_eq!(m.accepting(DataFormat::YdRaw, "129025"), ["n2k"]);
        assert!(m.accepting(DataFormat::Nmea0183, "RMC").is_empty());
        m.send("RMC", "$GPRMC,not for this output");
        m.send_format(DataFormat::YdRaw, "127250", "filtered out", 0.0);
        m.send_format(DataFormat::YdRaw, "129025", "12:00:00.000 R 09F8010A 00 00 00 00 00 00 00 00", 0.0);
        let mut buf = [0u8; 256];
        let n = rx.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"12:00:00.000 R 09F8010A 00 00 00 00 00 00 00 00\r\n");
    }

    #[test]
    fn signalk_websocket_says_hello_and_answers_discovery() {
        use futures_util::StreamExt as _;
        use std::io::{Read as _, Write as _};
        let port = free_port();
        let m = OutputManager::new().unwrap();
        m.set_signalk_hello("{\"self\":\"vessels.urn:mrn:imo:mmsi:1\"}".into());
        let id = m.add(OutputConfig {
            name: "sk".into(),
            enabled: true,
            kind: OutputKind::WebSocket { bind: format!("127.0.0.1:{port}") },
            format: DataFormat::SignalK,
            filter: vec![],
        });
        wait_running(&m, id);

        // Discovery over plain HTTP.
        let mut http = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        http.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        http.write_all(format!("GET /signalk HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n").as_bytes()).unwrap();
        let mut response = String::new();
        http.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(&format!("\"signalk-ws\":\"ws://localhost:{port}/signalk/v1/stream\"")), "{response}");

        // The stream: hello first, then deltas (and nothing in other formats).
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let url = format!("ws://127.0.0.1:{port}/signalk/v1/stream?subscribe=all");
            let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
            let mut next = async || {
                let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
                msg.into_text().unwrap().to_string()
            };
            assert_eq!(next().await, "{\"self\":\"vessels.urn:mrn:imo:mmsi:1\"}");
            m.send("RMC", "$GPRMC,0183 line");
            m.send_format(DataFormat::SignalK, "gps", "{\"updates\":[]}", 0.0);
            assert_eq!(next().await, "{\"updates\":[]}");
        });
    }

    #[test]
    fn tcp_client_connects_and_reconnects() {
        use std::io::{BufRead, BufReader};
        let port = free_port();
        let m = OutputManager::new().unwrap();
        let id = add(&m, OutputKind::TcpClient { target: format!("127.0.0.1:{port}") }, &[]);
        // Nobody listening yet: the output waits and retries.
        std::thread::sleep(Duration::from_millis(300));
        assert!(matches!(m.list()[0].status, Status::Warning(_)), "{:?}", m.list()[0].status);
        let server = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        let (stream, _) = server.accept().unwrap();
        wait_running(&m, id);
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let t = Instant::now();
        while line.is_empty() {
            assert!(t.elapsed() < Duration::from_secs(5));
            m.send("RMC", "$GPRMC,x");
            let _ = reader.read_line(&mut line);
        }
        assert_eq!(line, "$GPRMC,x\r\n");
    }

    // A pseudo-terminal pair stands in for the port. macOS ptys reject the baud-rate ioctl
    // serialport uses ("Not a typewriter"), so this runs on Linux (CI) only.
    #[cfg(target_os = "linux")]
    #[test]
    fn serial_writes_to_the_port() {
        use std::io::Read as _;
        use serialport::SerialPort as _;
        let (mut master, slave) = serialport::TTYPort::pair().unwrap();
        let name = slave.name().unwrap();
        // Our handle holds an exclusive flock on the slave; close it so the output can open the
        // port by name (the master keeps the pty alive).
        drop(slave);
        let m = OutputManager::new().unwrap();
        let id = add(&m, OutputKind::Serial { port: name, baud: 38_400 }, &[]);
        wait_running(&m, id);
        master.set_timeout(Duration::from_millis(200)).unwrap();
        let mut got = Vec::new();
        let t = Instant::now();
        while !got.ends_with(b"\r\n") {
            assert!(t.elapsed() < Duration::from_secs(5), "{got:?}");
            m.send("VDM", "!AIVDM,1,1,,A,x,0*00");
            let mut buf = [0u8; 256];
            if let Ok(n) = master.read(&mut buf) {
                got.extend_from_slice(&buf[..n]);
            }
        }
        assert!(String::from_utf8_lossy(&got).contains("!AIVDM,1,1,,A,x,0*00\r\n"));
        // Removing the output closes the port (the blocking thread sees the cancel flag).
        m.remove(id);
    }

    #[test]
    fn missing_serial_port_retries() {
        let m = OutputManager::new().unwrap();
        add(&m, OutputKind::Serial { port: "/dev/does-not-exist-marine-sim".into(), baud: 4800 }, &[]);
        let t = Instant::now();
        loop {
            if let Status::Warning(w) = &m.list()[0].status {
                assert!(w.contains("retrying"), "{w}");
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(5), "{:?}", m.list()[0].status);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn udp_delivers_filtered_lines() {
        let rx = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let m = OutputManager::new().unwrap();
        let id = m.add(OutputConfig {
            name: "t".into(),
            enabled: true,
            kind: OutputKind::Udp { target: rx.local_addr().unwrap().to_string(), broadcast: false },
            format: DataFormat::Nmea0183,
            filter: vec!["RMC".into()],
        });
        wait_running(&m, id);
        m.send("GGA", "$GPGGA,dropped");
        m.send("RMC", "$GPRMC,kept");
        let mut buf = [0u8; 256];
        let n = rx.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"$GPRMC,kept\r\n");
    }

    #[test]
    fn tcp_streams_to_clients() {
        use std::io::{BufRead, BufReader};
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let m = OutputManager::new().unwrap();
        let id = m.add(OutputConfig {
            name: "t".into(),
            enabled: true,
            kind: OutputKind::TcpServer { bind: format!("127.0.0.1:{port}") },
            format: DataFormat::Nmea0183,
            filter: vec![],
        });
        wait_running(&m, id);
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let t = Instant::now();
        while m.list()[0].clients == 0 && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        m.send("HDT", "$HCHDT,123.4,T*00");
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        assert_eq!(line, "$HCHDT,123.4,T*00\r\n");

        // Removing the output closes the port so it can be rebound.
        m.remove(id);
        let t = Instant::now();
        while std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            assert!(t.elapsed() < Duration::from_secs(5), "port still bound");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
