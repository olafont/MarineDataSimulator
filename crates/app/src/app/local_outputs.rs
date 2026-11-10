//! Outputs the user creates are kept on this computer, apart from the scenario, and come back
//! at every start and after loading another scenario.
//!
//! They live in `outputs.toml` in the user's config folder (on macOS
//! `~/Library/Application Support/marine-sim/`). Outputs that come from the scenario stay in
//! the scenario; recordings are not kept (each one is a new file).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use engine::transports::OutputConfig;
use serde::{Deserialize, Serialize};

use super::SimApp;

#[derive(Default)]
pub(super) struct LocalOutputs {
    /// Where the file is; `None` when the system has no config folder (nothing is kept).
    path: Option<PathBuf>,
    /// The running outputs that are kept in the file.
    ids: HashSet<u64>,
}

impl LocalOutputs {
    pub(super) fn new() -> Self {
        Self { path: super::prefs::config_file("outputs.toml"), ids: HashSet::new() }
    }

    pub(super) fn contains(&self, id: u64) -> bool {
        self.ids.contains(&id)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default, rename = "output")]
    outputs: Vec<OutputConfig>,
}

fn read(path: &Path) -> anyhow::Result<Vec<OutputConfig>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str::<File>(&text)?.outputs),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

fn write(path: &Path, outputs: Vec<OutputConfig>) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(&File { outputs })?;
    // Write then rename, so a crash never leaves half a file.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl SimApp {
    /// Starts the kept outputs, except those the scenario already has (a scenario saved with
    /// "Save as" contains them).
    pub(super) fn start_local_outputs(&mut self) {
        self.local_outputs.ids.clear();
        let Some(path) = self.local_outputs.path.clone() else { return };
        match read(&path) {
            Ok(saved) => {
                let running = self.engine.outputs().configs();
                for cfg in saved.into_iter().filter(|c| !running.contains(c)) {
                    let id = self.engine.outputs().add(cfg);
                    self.local_outputs.ids.insert(id);
                }
            }
            Err(e) => self.fail(crate::tf!("Could not read the saved outputs in {path}: {e}", path = path.display(), e = format!("{e:#}"))),
        }
    }

    /// Adds an output and keeps it on this computer.
    pub(super) fn add_local_output(&mut self, cfg: OutputConfig) {
        let id = self.engine.outputs().add(cfg);
        self.local_outputs.ids.insert(id);
        self.save_local_outputs();
    }

    /// Changes an output; a kept one is saved again.
    pub(super) fn update_output(&mut self, id: u64, cfg: OutputConfig) {
        self.engine.outputs().update(id, cfg);
        if self.local_outputs.contains(id) {
            self.save_local_outputs();
        }
    }

    /// Removes an output; a kept one is forgotten.
    pub(super) fn remove_output(&mut self, id: u64) {
        self.engine.outputs().remove(id);
        if self.local_outputs.ids.remove(&id) {
            self.save_local_outputs();
        }
    }

    fn save_local_outputs(&mut self) {
        let Some(path) = self.local_outputs.path.clone() else { return };
        let keep: Vec<OutputConfig> = self
            .engine
            .outputs()
            .list()
            .into_iter()
            .filter(|o| self.local_outputs.contains(o.id))
            .map(|o| o.config)
            .collect();
        if let Err(e) = write(&path, keep) {
            self.fail(crate::tf!("Could not save the outputs to {path}: {e}", path = path.display(), e = format!("{e:#}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::transports::{DataFormat, OutputKind};

    #[test]
    fn outputs_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("marine-sim-test-{}", std::process::id()));
        let path = dir.join("outputs.toml");
        assert!(read(&path).unwrap().is_empty(), "no file yet: nothing kept");
        let outputs = vec![
            OutputConfig {
                name: "Plotter".into(),
                enabled: true,
                kind: OutputKind::TcpServer { bind: "0.0.0.0:10113".into() },
                format: DataFormat::Nmea0183,
                filter: vec!["RMC".into()],
            },
            OutputConfig {
                name: "Serial".into(),
                enabled: false,
                kind: OutputKind::Serial { port: "/dev/tty.usb".into(), baud: 38_400 },
                format: DataFormat::Nmea0183,
                filter: vec![],
            },
        ];
        write(&path, outputs.clone()).unwrap();
        assert_eq!(read(&path).unwrap(), outputs);
        write(&path, vec![]).unwrap();
        assert!(read(&path).unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
