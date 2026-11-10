//! Serial output on a blocking thread. Reopens the port every 2 s while it is missing (USB
//! adapters come and go); lines that do not fit the baud rate are dropped and reported.

use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::broadcast::{self, error::TryRecvError};

use crate::{Message, OutputConfig, Stats, Status, lag_status};

/// Serial ports the system reports (may be empty when enumeration is unsupported).
pub fn available_ports() -> Vec<String> {
    serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default()
}

fn sleep_unless_cancelled(cancel: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(50);
    let mut waited = Duration::ZERO;
    while waited < total && !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(step);
        waited += step;
    }
}

pub(crate) fn run(
    cfg: &OutputConfig,
    port: &str,
    baud: u32,
    mut rx: broadcast::Receiver<Message>,
    stats: &Stats,
    cancel: &AtomicBool,
) -> Result<()> {
    while !cancel.load(Ordering::Relaxed) {
        let mut sp = match serialport::new(port, baud).timeout(Duration::from_millis(500)).open() {
            Ok(p) => p,
            Err(e) => {
                stats.set_status(Status::Warning(format!("{port}: {e}; retrying")));
                sleep_unless_cancelled(cancel, Duration::from_secs(2));
                continue;
            }
        };
        tracing::info!(output = %cfg.name, "opened {port} at {baud} baud");
        stats.set_status(Status::Running);
        stats.clients.store(1, Ordering::Relaxed);
        // Lines queued while the port was closed are stale.
        rx = rx.resubscribe();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            match rx.try_recv() {
                Ok(msg) if cfg.takes(&msg) => match sp.write_all(msg.data.as_bytes()) {
                    Ok(()) => stats.count(msg.data.len()),
                    Err(e) => {
                        stats.error(1);
                        tracing::info!(output = %cfg.name, "{port}: {e}");
                        stats.set_status(Status::Warning(format!("{port}: {e}; reopening")));
                        break;
                    }
                },
                Ok(_) => {}
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(10)),
                Err(TryRecvError::Lagged(n)) => lag_status(stats, n, &format!("{baud} baud")),
                Err(TryRecvError::Closed) => return Ok(()),
            }
        }
        stats.clients.store(0, Ordering::Relaxed);
        sleep_unless_cancelled(cancel, Duration::from_secs(1));
    }
    Ok(())
}
