//! Preferences: settings of this computer, kept between runs in `preferences.toml` in the user's
//! config folder (on macOS `~/Library/Application Support/marine-sim/`), next to the kept
//! outputs (`outputs.toml`, see [`super::local_outputs`]).
//!
//! The file is written whenever a preference changes, wherever it was changed (the ribbon's
//! palette buttons or the Preferences window): [`SimApp::save_prefs_if_changed`] compares with
//! what was last written, once per frame.

use std::path::{Path, PathBuf};

use egui::{Context, RichText, Ui};
use serde::{Deserialize, Serialize};

use super::SimApp;
use crate::ui::form::form;
use crate::ui::i18n::{self, Lang};
use crate::ui::theme::{self, FontWeight, Palette};
use crate::t;

/// The folder that holds the app's settings files, if the system has one.
pub(super) fn config_file(name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("marine-sim").join(name))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Prefs {
    /// Day, Dusk or Night; `None` follows the system's light or dark mode.
    pub palette: Option<Palette>,
    /// The interface's language; `None` follows the system's.
    pub language: Option<Lang>,
    /// How heavy the interface's text is.
    pub font_weight: FontWeight,
    /// Where recordings and file outputs go, and where logs to replay are looked for.
    pub output_folder: PathBuf,
    /// Serve the Model Context Protocol to AI agents on this computer (off unless asked for).
    pub mcp_enabled: bool,
    pub mcp_port: u16,
    /// Scenario files saved or opened in the app, listed after the built-in ones.
    pub scenarios: Vec<PathBuf>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { palette: None, language: None, font_weight: FontWeight::default(), output_folder: default_output_folder(), mcp_enabled: false, mcp_port: mcp::DEFAULT_PORT, scenarios: vec![] }
    }
}

/// Documents, else home, else the current folder.
pub(super) fn default_output_folder() -> PathBuf {
    dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

fn read(path: &Path) -> anyhow::Result<Prefs> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Prefs::default()),
        Err(e) => Err(e.into()),
    }
}

fn write(path: &Path, prefs: &Prefs) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml::to_string_pretty(prefs)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The Preferences window's size, centred on the main window.
const PREFS_WIDTH: f32 = 640.0;
const PREFS_HEIGHT: f32 = 560.0;

/// The preferences, as last written, and whether the Preferences window is open.
pub(super) struct PrefsState {
    path: Option<PathBuf>,
    pub prefs: Prefs,
    saved: Prefs,
    pub window_open: bool,
}

impl PrefsState {
    /// Reads the file; an unreadable one is reported and the defaults are used.
    pub(super) fn load() -> (Self, Option<String>) {
        let path = config_file("preferences.toml");
        let (mut prefs, error) = match path.as_deref().map(read) {
            Some(Ok(p)) => (p, None),
            Some(Err(e)) => (Prefs::default(), Some(crate::tf!("Could not read the preferences: {e}", e = format!("{e:#}")))),
            None => (Prefs::default(), None),
        };
        let saved = prefs.clone();
        // Files deleted or moved since the last run leave the list (written on the first frame).
        prefs.scenarios.retain(|p| p.is_file());
        (Self { path, saved, prefs, window_open: false }, error)
    }

    /// Applies the preferences that live in egui (the palette).
    pub(super) fn apply(&self, ctx: &Context) {
        i18n::set_lang(self.prefs.language.unwrap_or_else(Lang::system));
        theme::set_font_weight(ctx, self.prefs.font_weight);
        if let Some(p) = self.prefs.palette {
            theme::set_palette(ctx, p);
        }
    }
}

impl SimApp {
    /// Where recordings and file outputs go.
    pub(super) fn output_folder(&self) -> PathBuf {
        self.prefs.prefs.output_folder.clone()
    }

    /// Adds a scenario file to the list in the Scenario tab, once.
    pub(super) fn remember_scenario(&mut self, path: &Path) {
        let list = &mut self.prefs.prefs.scenarios;
        if !list.iter().any(|p| p == path) {
            list.push(path.to_path_buf());
        }
    }

    /// Takes a scenario file off that list.
    pub(super) fn forget_scenario(&mut self, path: &Path) {
        self.prefs.prefs.scenarios.retain(|p| p != path);
    }

    /// Picks up preferences changed elsewhere (the palette) and writes the file when anything
    /// differs from what was last written.
    pub(super) fn save_prefs_if_changed(&mut self, ctx: &Context) {
        let s = &mut self.prefs;
        // The palette lives in egui; until the user picks one, it follows the system.
        if let Some(p) = theme::chosen_palette(ctx) {
            s.prefs.palette = Some(p);
        }
        if s.prefs == s.saved {
            return;
        }
        let Some(path) = s.path.clone() else { return };
        s.saved = s.prefs.clone();
        if let Err(e) = write(&path, &s.prefs) {
            self.fail(crate::tf!("Could not save the preferences to {path}: {e}", path = path.display(), e = format!("{e:#}")));
        }
    }

    pub(super) fn prefs_window(&mut self, ctx: &Context) {
        let mut open = self.prefs.window_open;
        let t = theme::palette(ctx).tokens();
        egui::Window::new(t!("Preferences"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([PREFS_WIDTH, PREFS_HEIGHT])
            .min_size([PREFS_WIDTH * 0.75, PREFS_HEIGHT * 0.6])
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(t.space_xl))
            .show(ctx, |ui| {
                ui.set_min_size(ui.available_size());
                self.prefs_form(ui);
            });
        // Closed with × or with Done.
        self.prefs.window_open &= open;
    }

    fn prefs_form(&mut self, ui: &mut Ui) {
        let t = theme::tokens(ui);
        let ctx = ui.ctx().clone();
        let p = &mut self.prefs.prefs;
        let heading = |ui: &mut Ui, title: &str, about: &str| {
            ui.label(RichText::new(title).strong().size(t.font_heading));
            ui.weak(about);
            ui.add_space(t.space_sm);
        };

        heading(ui, t!("Appearance"), t!("Palette: Day for daylight, Dusk for dim rooms, Night (red on black) to keep night vision."));
        form(ui, "prefs_look", |f| {
            let mut palette = theme::palette(&ctx);
            let options: Vec<(Palette, String)> = Palette::ALL.iter().map(|&p| (p, crate::ui::i18n::tr(p.label()))).collect();
            if f.segmented(t!("Palette"), &mut palette, &options) {
                theme::set_palette(&ctx, palette);
            }
            let mut lang = i18n::lang();
            let names: Vec<(Lang, &str)> = Lang::ALL.iter().map(|&l| (l, l.name())).collect();
            if f.segmented(t!("Language"), &mut lang, &names) {
                i18n::set_lang(lang);
                p.language = Some(lang);
            }
            let weights: Vec<(FontWeight, &str)> = FontWeight::ALL
                .iter()
                .map(|&w| {
                    let label = match w {
                        FontWeight::Light => t!("Light"),
                        FontWeight::Regular => t!("Regular"),
                        FontWeight::Medium => t!("Medium"),
                    };
                    (w, label)
                })
                .collect();
            if f.segmented(t!("Text weight"), &mut p.font_weight, &weights) {
                theme::set_font_weight(&ctx, p.font_weight);
            }
        });
        ui.add_space(t.space_lg);
        ui.separator();
        ui.add_space(t.space_lg);

        heading(ui, t!("Files"), t!("Recordings and new file outputs are written here; Open log… starts here."));
        form(ui, "prefs_files", |f| {
            f.row(t!("Output folder"), |ui| {
                let text = RichText::new(p.output_folder.display().to_string()).monospace();
                ui.add(egui::Label::new(text).truncate()).on_hover_text(p.output_folder.display().to_string());
            });
            f.row("", |ui| {
                if ui.button(t!("Choose…")).clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_directory(&p.output_folder).pick_folder()
                {
                    p.output_folder = dir;
                }
                let default = default_output_folder();
                let tip = crate::tf!("Back to {path}", path = default.display());
                if ui.add_enabled(p.output_folder != default, egui::Button::new(t!("Default"))).on_hover_text(tip).clicked()
                {
                    p.output_folder = default;
                }
            });
        });

        ui.add_space(t.space_lg);
        ui.separator();
        ui.add_space(t.space_lg);

        heading(ui, t!("AI agents"), t!("An MCP server lets AI agents such as Claude watch and drive the simulation. Only programs on this computer can connect."));
        form(ui, "prefs_agents", |f| self.agents_form(f));

        // Footer: where this is kept, and Done.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                if let Some(path) = &self.prefs.path {
                    ui.weak(crate::tf!("Kept in {path}", path = path.display()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t!("Done")).clicked() {
                        self.prefs.window_open = false;
                    }
                });
            });
            ui.separator();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_and_fill_in_missing_fields() {
        let dir = std::env::temp_dir().join(format!("marine-sim-prefs-{}", std::process::id()));
        let path = dir.join("preferences.toml");
        assert_eq!(read(&path).unwrap(), Prefs::default(), "no file: defaults");
        let p = Prefs { palette: Some(Palette::Night), language: Some(Lang::French), font_weight: FontWeight::Medium, output_folder: dir.join("logs"), mcp_enabled: true, mcp_port: 12000, scenarios: vec![dir.join("my boat.toml")] };
        write(&path, &p).unwrap();
        assert_eq!(read(&path).unwrap(), p);
        // A file from an older version, with fewer fields, still reads.
        std::fs::write(&path, "palette = \"Day\"\n").unwrap();
        let old = read(&path).unwrap();
        assert_eq!(old.palette, Some(Palette::Day));
        assert_eq!(old.output_folder, default_output_folder());
        assert!(!old.mcp_enabled, "the MCP server is off unless asked for");
        assert_eq!(old.mcp_port, mcp::DEFAULT_PORT);
        assert!(old.scenarios.is_empty());
        assert_eq!(old.font_weight, FontWeight::Regular);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
