//! File output: appends lines to a file, optionally prefixed with an NMEA 4 TAG block holding
//! the line's (simulated) time, so a replay can reproduce the timing.

use anyhow::{Context as _, Result};
use tokio::io::{AsyncWriteExt as _, BufWriter};
use tokio::sync::broadcast;

use crate::{Message, OutputConfig, Stats, Status, lag_status};

/// `\c:<unix seconds>*hh\` (NMEA 0183 v4 TAG block, source time).
pub fn tag_block(time_unix: f64) -> String {
    let body = format!("c:{}", time_unix.floor() as i64);
    let cs = body.bytes().fold(0u8, |a, b| a ^ b);
    format!("\\{body}*{cs:02X}\\")
}

pub(crate) async fn run(
    cfg: &OutputConfig,
    path: &str,
    tag_blocks: bool,
    mut rx: broadcast::Receiver<Message>,
    stats: &Stats,
) -> Result<()> {
    let p = std::path::Path::new(path);
    if let Some(dir) = p.parent()
        && !dir.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(dir).await.with_context(|| format!("creating {}", dir.display()))?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .await
        .with_context(|| format!("opening {path}"))?;
    let mut w = BufWriter::new(file);
    stats.set_status(Status::Running);
    loop {
        match rx.recv().await {
            Ok(msg) if cfg.takes(&msg) => {
                let mut n = msg.data.len();
                if tag_blocks {
                    let tag = tag_block(msg.time_unix);
                    n += tag.len();
                    w.write_all(tag.as_bytes()).await.context("writing")?;
                }
                w.write_all(msg.data.as_bytes()).await.context("writing")?;
                stats.count(n);
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => lag_status(stats, n, "the disk"),
            Err(broadcast::error::RecvError::Closed) => {
                w.flush().await.context("writing")?;
                return Ok(());
            }
        }
        // Flush once the burst is written, so the file is complete if the output is stopped.
        if rx.is_empty() {
            w.flush().await.context("writing")?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_block_format() {
        // Checksum of "c:1782052215" computed independently.
        let t = tag_block(1_782_052_215.7);
        assert!(t.starts_with("\\c:1782052215*") && t.ends_with('\\'), "{t}");
        let cs = "c:1782052215".bytes().fold(0u8, |a, b| a ^ b);
        assert_eq!(t, format!("\\c:1782052215*{cs:02X}\\"));
    }
}
