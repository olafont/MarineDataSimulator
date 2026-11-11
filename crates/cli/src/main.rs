//! Headless runner: loads a scenario, serves its outputs, optionally echoes sentences to stdout
//! or serves the Model Context Protocol to AI agents.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use engine::transports::DataFormat;
use engine::{Engine, ReplayCommand, scenario};

#[derive(Parser, Debug)]
#[command(name = "marine-sim-cli", version, about = "Marine data simulator (headless)")]
struct Args {
    /// Scenario file (TOML). Defaults to the built-in Strait of Gibraltar crossing.
    #[arg(short, long)]
    scenario: Option<PathBuf>,
    /// Use a built-in scenario by file name (see --list).
    #[arg(short, long, conflicts_with = "scenario")]
    builtin: Option<String>,
    /// List built-in scenarios and exit.
    #[arg(long)]
    list: bool,
    /// Print every emitted sentence to stdout.
    #[arg(short, long)]
    print: bool,
    /// Format printed by --print: nmea0183, yd_raw, actisense_ascii, canboat_plain or signalk.
    #[arg(long, value_name = "FORMAT", default_value = "nmea0183", value_parser = parse_format)]
    print_format: DataFormat,
    /// Stop after this many wall-clock seconds.
    #[arg(short, long)]
    duration: Option<f64>,
    /// Override the scenario's time scale.
    #[arg(long)]
    time_scale: Option<f64>,
    /// Record every output line to this file (with TAG-block timestamps).
    #[arg(long, value_name = "FILE")]
    record: Option<PathBuf>,
    /// Replay a recorded NMEA/AIS log to the outputs (simulated data muted).
    #[arg(long, value_name = "FILE")]
    replay: Option<PathBuf>,
    /// Replay speed multiplier.
    #[arg(long, default_value_t = 1.0, requires = "replay")]
    replay_speed: f64,
    /// Loop the replayed log.
    #[arg(long, requires = "replay")]
    replay_loop: bool,
    /// Stop when the replayed log ends.
    #[arg(long, requires = "replay")]
    exit_after_replay: bool,
    /// Serve the Model Context Protocol on http://127.0.0.1:PORT/mcp, so that AI agents can
    /// watch and drive the simulation.
    #[arg(long, value_name = "PORT")]
    mcp_port: Option<u16>,
    /// Serve the Model Context Protocol on stdin/stdout (for an agent that starts this
    /// program); runs until the agent closes them.
    #[arg(long, conflicts_with_all = ["print", "mcp_port", "duration"])]
    mcp_stdio: bool,
    /// Write the built-in scenario to this path and exit.
    #[arg(long, value_name = "PATH")]
    write_default_scenario: Option<PathBuf>,
}

fn parse_format(s: &str) -> Result<DataFormat, String> {
    match s {
        "nmea0183" => Ok(DataFormat::Nmea0183),
        "yd_raw" => Ok(DataFormat::YdRaw),
        "actisense_ascii" => Ok(DataFormat::ActisenseAscii),
        "canboat_plain" => Ok(DataFormat::CanboatPlain),
        "signalk" => Ok(DataFormat::SignalK),
        _ => Err("expected nmea0183, yd_raw, actisense_ascii, canboat_plain or signalk".to_string()),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();

    if let Some(path) = args.write_default_scenario {
        std::fs::write(&path, scenario::DEFAULT_SCENARIO)?;
        eprintln!("wrote {}", path.display());
        return Ok(());
    }

    if args.list {
        for (name, _) in scenario::BUILTIN {
            println!("{name}");
        }
        return Ok(());
    }
    let mut sc = match (&args.scenario, &args.builtin) {
        (Some(p), _) => scenario::Scenario::load(p)?,
        (None, Some(b)) => scenario::builtin(b)?,
        (None, None) => scenario::default_scenario(),
    };
    if let Some(ts) = args.time_scale {
        sc.simulation.time_scale = ts;
    }
    tracing::info!("scenario: {}", if sc.name.is_empty() { "(unnamed)" } else { &sc.name });

    if let Some(path) = &args.record {
        sc.outputs.push(engine::transports::OutputConfig {
            name: "record".into(),
            enabled: true,
            kind: engine::transports::OutputKind::File { path: path.display().to_string(), tag_blocks: true },
            format: engine::transports::DataFormat::Nmea0183,
            filter: vec![],
        });
    }

    let log = args.replay.as_deref().map(engine::read_log).transpose()?;
    let replaying = log.is_some();
    let engine = Arc::new(Engine::start_with_replay(sc, None, log)?);
    engine.monitor().lock().expect("monitor lock").view = args.print_format;
    if replaying {
        engine.replay(ReplayCommand::Speed(args.replay_speed));
        engine.replay(ReplayCommand::Loop(args.replay_loop));
    }
    std::thread::sleep(Duration::from_millis(300));
    for o in engine.outputs().list() {
        match &o.status {
            engine::transports::Status::Warning(w) | engine::transports::Status::Failed(w) => {
                tracing::warn!("output `{}`: {} — {w}", o.config.name, o.config.kind.describe())
            }
            s => tracing::info!("output `{}`: {} — {s:?}", o.config.name, o.config.kind.describe()),
        }
    }

    let host: Arc<dyn mcp::Host> = Arc::new(mcp::EngineHost(engine.clone()));
    if args.mcp_stdio {
        return mcp::serve_stdio(host);
    }
    let _mcp = match args.mcp_port {
        Some(port) => {
            let server = mcp::McpServer::start_http(host, port)?;
            eprintln!("MCP server: {}", server.url());
            Some(server)
        }
        None => None,
    };

    let started = Instant::now();
    let mut seen = 0u64;
    loop {
        if args.print {
            let mon = engine.monitor().lock().expect("monitor lock");
            for l in mon.lines.iter().filter(|l| l.seq >= seen && l.format == args.print_format) {
                println!("{}", l.text);
            }
            seen = mon.next_seq;
        }
        if args.duration.is_some_and(|d| started.elapsed().as_secs_f64() >= d) {
            break;
        }
        if args.exit_after_replay && engine.snapshot().replay.as_ref().is_some_and(|r| !r.playing && r.cursor > 0) {
            // Let the outputs flush the last lines.
            std::thread::sleep(Duration::from_millis(300));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
