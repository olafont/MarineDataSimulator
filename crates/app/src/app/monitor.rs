//! Bottom panel: the traffic monitor, every line sent and the outputs it goes to.

use egui::{ComboBox, DragValue, RichText, ScrollArea, TextEdit, Ui};
use engine::MonitorLine;
use engine::transports::DataFormat;

use super::SimApp;
use crate::ui::theme::tokens;
use crate::t;
use crate::ui::i18n::tr;

pub(super) struct MonitorState {
    /// Frozen: shows `frozen` instead of the live lines.
    pub(super) paused: bool,
    frozen: Vec<MonitorLine>,
    filter: String,
    /// How many of the last lines to show.
    show: usize,
}

impl Default for MonitorState {
    fn default() -> Self {
        Self { paused: false, frozen: vec![], filter: String::new(), show: 300 }
    }
}

impl SimApp {
    /// Stops or restarts scrolling.
    pub(super) fn freeze_monitor(&mut self, frozen: bool) {
        self.monitor.paused = frozen;
        if frozen {
            self.monitor.frozen = self.engine.monitor().lock().expect("monitor").lines.iter().cloned().collect();
        }
    }

    pub(super) fn monitor_view(&self) -> DataFormat {
        self.engine.monitor().lock().expect("monitor").view
    }

    /// Shows the traffic in another format, starting afresh.
    pub(super) fn set_monitor_view(&mut self, view: DataFormat) {
        let mut mon = self.engine.monitor().lock().expect("monitor");
        if mon.view != view {
            mon.view = view;
            mon.lines.clear();
            self.monitor.frozen.clear();
        }
    }

    pub(super) fn monitor(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.strong(t!("Traffic"));
            let mut view = self.monitor_view();
            ComboBox::from_id_salt("monitor_view").selected_text(tr(view.label())).show_ui(ui, |ui| {
                for &f in DataFormat::ALL {
                    ui.selectable_value(&mut view, f, tr(f.label()));
                }
            });
            self.set_monitor_view(view);
            let mut frozen = self.monitor.paused;
            if ui.toggle_value(&mut frozen, t!("❄ Freeze")).changed() {
                self.freeze_monitor(frozen);
            }
            let m = &mut self.monitor;
            let filter = TextEdit::singleline(&mut m.filter).hint_text(t!("filter: text, sentence or PGN"));
            ui.add(filter.desired_width(tokens(ui).field_long));
            ui.label(t!("Show"));
            ui.add(DragValue::new(&mut m.show).range(50..=2000).speed(10));
            ui.label(t!("lines"));
        });
        let m = &self.monitor;
        let filter = m.filter.trim().to_ascii_uppercase();
        let matches = |l: &&MonitorLine| {
            filter.is_empty() || l.tag.eq_ignore_ascii_case(&filter) || l.text.to_ascii_uppercase().contains(&filter)
        };
        let lines: Vec<MonitorLine> = if m.paused {
            m.frozen.iter().filter(matches).cloned().collect()
        } else {
            let mon = self.engine.monitor().lock().expect("monitor");
            mon.lines.iter().filter(matches).cloned().collect()
        };
        let start = lines.len().saturating_sub(m.show);
        // Which outputs each tag goes to (current filters).
        let mut routes: std::collections::HashMap<&str, Vec<String>> = std::collections::HashMap::new();
        for l in &lines[start..] {
            routes.entry(l.tag.as_str()).or_insert_with(|| self.engine.outputs().accepting(l.format, &l.tag));
        }
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show_rows(ui, row_h, lines.len() - start, |ui, range| {
                for l in &lines[start + range.start..start + range.end] {
                    ui.horizontal(|ui| {
                        ui.weak(RichText::new(format!("{:<15}", tr(l.source.label()))).monospace());
                        let to = &routes[l.tag.as_str()];
                        let r = ui.weak(RichText::new(format!("»{:<2}", to.len())).monospace());
                        r.on_hover_text(if to.is_empty() { t!("not sent: no output accepts it").to_string() } else { to.join("\n") });
                        ui.monospace(&l.text);
                    });
                }
            });
    }
}
