//! Form rows: a label column and a control column, with the sizes from the theme tokens.
//!
//! ```ignore
//! form(ui, "vessel", |f| {
//!     f.segmented("Propulsion", &mut c.propulsion, &[(Motor, "Motor"), (Sail, "Sail")]);
//!     changed |= f.number("Length", edits, id, &mut c.length_m, Num::new(2.0..=400.0, 0.1, " m"));
//!     f.readout("Optimum angles", "beat 42°");
//! });
//! ```
//!
//! Every row has the same height, every label column the same minimum width, so forms in one
//! panel line up. Rows that need several controls use [`Form::row`].

use std::hash::Hash;
use std::ops::RangeInclusive;

use egui::{ComboBox, DragValue, Grid, Id, Response, RichText, TextEdit, Ui, WidgetText};

use crate::ui::theme::{Tokens, Tone, tokens};
use crate::ui::widgets::Edits;

/// Shows a form. `id` must be unique among the forms drawn in the same parent.
pub fn form<R>(ui: &mut Ui, id: impl Hash + std::fmt::Debug, add: impl FnOnce(&mut Form<'_>) -> R) -> R {
    let t = tokens(ui);
    Grid::new(id)
        .num_columns(2)
        .spacing([t.space_lg, t.row_gap])
        .min_col_width(t.label_width)
        .min_row_height(t.row_height)
        .show(ui, |ui| add(&mut Form { ui, t }))
        .inner
}

/// Text field widths, from the tokens.
#[derive(Debug, Clone, Copy)]
pub enum Width {
    Short,
    Medium,
    Long,
}

/// How a number is edited: range, drag speed, unit suffix and decimals.
#[derive(Debug, Clone)]
pub struct Num<'s> {
    range: RangeInclusive<f64>,
    speed: f64,
    suffix: &'s str,
    decimals: usize,
}

impl<'s> Num<'s> {
    pub fn new(range: RangeInclusive<f64>, speed: f64, suffix: &'s str) -> Self {
        Self { range, speed, suffix, decimals: 3 }
    }

    pub fn decimals(mut self, n: usize) -> Self {
        self.decimals = n;
        self
    }

    pub fn drag<'a>(&self, v: &'a mut f64) -> DragValue<'a> {
        DragValue::new(v).range(self.range.clone()).speed(self.speed).suffix(self.suffix).max_decimals(self.decimals)
    }
}

pub struct Form<'a> {
    ui: &'a mut Ui,
    t: &'static Tokens,
}

impl Form<'_> {
    /// The tokens this form draws with.
    pub fn tokens(&self) -> &'static Tokens {
        self.t
    }

    fn label(&mut self, label: impl Into<WidgetText>, hint: Option<&str>) {
        let r = self.ui.label(label);
        if let Some(h) = hint {
            r.on_hover_text(h);
        }
    }

    /// A label and any controls, laid out left to right.
    pub fn row<R>(&mut self, label: impl Into<WidgetText>, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.row_hint(label, None, add)
    }

    /// A row whose label explains itself on hover.
    pub fn row_hint<R>(
        &mut self,
        label: impl Into<WidgetText>,
        hint: Option<&str>,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        self.label(label, hint);
        let r = self.ui.horizontal(add).inner;
        self.ui.end_row();
        r
    }

    /// A bold label that starts a group of rows, with optional controls beside it.
    pub fn heading<R>(&mut self, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.row(RichText::new(label).strong(), add)
    }

    /// A value that cannot be edited.
    pub fn readout(&mut self, label: &str, text: impl Into<String>) {
        self.row(label, |ui| ui.weak(text.into()));
    }

    /// A coloured message, e.g. a warning.
    pub fn status(&mut self, label: &str, text: impl Into<String>, tone: Tone, hint: Option<&str>) {
        let t = self.t;
        self.row(label, |ui| {
            let r = ui.label(RichText::new(text.into()).color(t.tone(ui, tone)));
            if let Some(h) = hint {
                r.on_hover_text(h);
            }
        });
    }

    /// A number bound to a value that lives in the simulation (see [`Edits`]). Writes the new
    /// value to `v` and returns true when the user changed it.
    pub fn number(&mut self, label: &str, edits: &mut Edits, id: Id, v: &mut f64, num: Num<'_>) -> bool {
        self.number_hint(label, None, edits, id, v, num)
    }

    pub fn number_hint(
        &mut self,
        label: &str,
        hint: Option<&str>,
        edits: &mut Edits,
        id: Id,
        v: &mut f64,
        num: Num<'_>,
    ) -> bool {
        let live = *v;
        let changed = self.row_hint(label, hint, |ui| edits.value(ui, id, live, |ui, x| ui.add(num.drag(x))));
        if let Some(n) = changed {
            *v = n;
        }
        changed.is_some()
    }

    /// A drag value on a local value (no simulation lag to hide).
    pub fn drag(&mut self, label: &str, drag: DragValue<'_>) -> Response {
        self.row(label, |ui| ui.add(drag))
    }

    /// A single-line text field.
    pub fn text(&mut self, label: &str, s: &mut String, width: Width, max_chars: usize) -> Response {
        self.text_hint(label, s, width, max_chars, "")
    }

    /// A single-line text field with grey placeholder text.
    pub fn text_hint(
        &mut self,
        label: &str,
        s: &mut String,
        width: Width,
        max_chars: usize,
        placeholder: &str,
    ) -> Response {
        let w = match width {
            Width::Short => self.t.field_short,
            Width::Medium => self.t.field_medium,
            Width::Long => self.t.field_long,
        };
        self.row(label, |ui| {
            let mut edit = TextEdit::singleline(s).desired_width(w).hint_text(placeholder);
            if max_chars > 0 {
                edit = edit.char_limit(max_chars);
            }
            ui.add(edit)
        })
    }

    /// A checkbox with text beside it.
    pub fn check(&mut self, label: &str, v: &mut bool, text: impl Into<WidgetText>) -> Response {
        self.row(label, |ui| ui.checkbox(v, text))
    }

    /// A drop-down. Returns true when the user picked another option.
    pub fn choice<T: PartialEq + Copy, S: AsRef<str>>(&mut self, label: &str, v: &mut T, options: &[(T, S)]) -> bool {
        self.choice_width(label, v, options, None)
    }

    /// A drop-down of a given width (`None`: the default).
    pub fn choice_width<T: PartialEq + Copy, S: AsRef<str>>(
        &mut self,
        label: &str,
        v: &mut T,
        options: &[(T, S)],
        width: Option<f32>,
    ) -> bool {
        let before = *v;
        let selected = options.iter().find(|(o, _)| *o == *v).map_or("", |(_, s)| s.as_ref()).to_string();
        self.row(label, |ui| {
            let mut combo = ComboBox::from_id_salt(("choice", label)).selected_text(selected);
            if let Some(w) = width {
                combo = combo.width(w);
            }
            combo.show_ui(ui, |ui| {
                for (o, s) in options {
                    ui.selectable_value(v, *o, s.as_ref());
                }
            });
        });
        *v != before
    }

    /// A few options side by side, one selected. Returns true when the user picked another.
    pub fn segmented<T: PartialEq + Copy, S: AsRef<str>>(&mut self, label: &str, v: &mut T, options: &[(T, S)]) -> bool {
        let before = *v;
        self.row(label, |ui| {
            for (o, s) in options {
                ui.selectable_value(v, *o, s.as_ref());
            }
        });
        *v != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{DUSK, apply};

    fn frame(ctx: &egui::Context, add: impl FnMut(&mut Ui)) {
        let mut out = ctx.run_ui(egui::RawInput::default(), add);
        out.textures_delta.clear();
    }

    #[test]
    fn labels_line_up_across_forms() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Dark);
        apply(&ctx);
        let mut controls = vec![];
        for _ in 0..2 {
            controls.clear();
            frame(&ctx, |ui| {
                form(ui, "a", |f| controls.push(f.row("Short", |ui| ui.label("x")).rect.left()));
                form(ui, "b", |f| controls.push(f.row("A longer label", |ui| ui.label("y")).rect.left()));
            });
        }
        assert_eq!(controls.len(), 2);
        assert_eq!(controls[0], controls[1], "control columns start at the same x");
        assert!(controls[0] >= DUSK.label_width + DUSK.space_lg);
    }

    #[test]
    fn choice_reports_no_change_without_input() {
        let ctx = egui::Context::default();
        let mut v = 2u8;
        let mut changed = true;
        frame(&ctx, |ui| {
            changed = form(ui, "c", |f| f.choice("Pick", &mut v, &[(1u8, "one"), (2, "two")]));
        });
        assert!(!changed);
        assert_eq!(v, 2);
    }
}
