//! The app's UI toolkit: nothing here knows about the simulation.
//!
//! - [`theme`]: palettes (Day, Dusk, Night), design tokens and the chart colours.
//! - [`form`]: label/control rows that line up across panels.
//! - [`i18n`]: the languages of the interface (`t("…")`, `tf!`).
//! - [`ribbon`]: tabs, groups and big/small buttons.
//! - [`title_bar`]: the app's own title bar on macOS, under the traffic lights.
//! - [`widgets`]: values edited while the simulation changes them, overrides, formatting.

pub mod form;
pub mod i18n;
pub mod ribbon;
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
