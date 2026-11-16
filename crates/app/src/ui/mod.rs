//! The app's UI toolkit: nothing here knows about the simulation.
//!
//! - [`theme`]: palettes (Day, Dusk, Night), design tokens and the chart colours.
//! - [`form`]: label/control rows that line up across panels.
//! - [`i18n`]: the languages of the interface (`t("…")`, `tf!`).
//! - [`ribbon`]: tabs, groups and big/small buttons.
//! - [`search`]: the command search field and its list of matches.
//! - [`title_bar`]: the app's own title bar on macOS, under the traffic lights.
//! - [`widgets`]: values edited while the simulation changes them, overrides, formatting.

pub mod form;
pub mod i18n;
pub mod ribbon;
pub mod search;
pub mod theme;
pub mod title_bar;
pub mod widgets;

use egui::{CollapsingHeader, RichText, Ui};

/// A collapsible panel section with a bold title. `id` keeps its state when the title changes.
pub fn section<R>(
    ui: &mut Ui,
    id: &str,
    title: impl Into<String>,
    open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    CollapsingHeader::new(RichText::new(title).strong()).id_salt(id).default_open(open).show(ui, add).body_returned
}

/// A [`section`] with an eye at the right of its title, that shows or hides what the section
/// stands for on the chart.
pub fn section_eye<R>(
    ui: &mut Ui,
    id: &str,
    title: impl Into<String>,
    open: bool,
    visible: &mut bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(id), open);
    let title = title.into();
    let (_, _, body) = state
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, tip) = if *visible {
                    (RichText::new("👁"), crate::t!("Shown on the chart: click to hide"))
                } else {
                    (RichText::new("👁").weak().strikethrough(), crate::t!("Hidden from the chart: click to show"))
                };
                if ui.add(egui::Button::new(text).frame_when_inactive(false)).on_hover_text(tip).clicked() {
                    *visible = !*visible;
                }
            });
        })
        .body(add);
    body.map(|b| b.inner)
}
