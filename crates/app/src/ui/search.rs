//! The command search: a field with its shortcut in a key cap, and under it the commands that
//! match, picked with the arrow keys and Enter or with a click.
//!
//! The widget knows nothing about the commands: the app gives it the matches as [`Hit`]s and
//! runs the one picked.

use egui::{Align2, FontId, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use super::ribbon::Icon;
use super::theme::{palette, tokens};

/// Width of the field.
pub const WIDTH: f32 = 200.0;
const ROW_H: f32 = 24.0;

/// One match.
pub struct Hit<'a> {
    pub icon: Icon,
    pub name: &'a str,
    /// Where it lives: `Tab › Group`.
    pub place: &'a str,
    pub enabled: bool,
}

/// What the search remembers between frames.
pub struct Search {
    pub text: String,
    open: bool,
    selected: usize,
    /// Takes the keyboard on the next frame (⌘K).
    pub focus: bool,
    /// The field, and the list under it, as of the last frame.
    rects: [Rect; 2],
}

impl Default for Search {
    fn default() -> Self {
        Self { text: String::new(), open: false, selected: 0, focus: false, rects: [Rect::NOTHING; 2] }
    }
}

fn field_id() -> Id {
    Id::unique("command-search")
}

impl Search {
    fn close(&mut self, ctx: &egui::Context) {
        self.open = false;
        self.text.clear();
        self.rects[1] = Rect::NOTHING;
        ctx.memory_mut(|m| m.surrender_focus(field_id()));
    }

    /// The field and, while it is in use, the list of `hits` under it. Returns the index of the
    /// hit picked (Enter or a click); the search then closes.
    pub fn show(&mut self, ui: &mut Ui, shortcut: &str, hits: &[Hit<'_>]) -> Option<usize> {
        let picked = self.field(ui, shortcut, hits.len());
        let clicked = self.results(ui.ctx(), hits);
        let picked = picked.or(clicked);
        if picked.is_some() {
            self.close(ui.ctx());
        }
        picked
    }

    fn field(&mut self, ui: &mut Ui, shortcut: &str, hits: usize) -> Option<usize> {
        let t = tokens(ui);
        let v = ui.visuals().clone();
        let ctx = ui.ctx().clone();
        let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, t.row_height + 2.0), Sense::hover());
        let focused = ctx.memory(|m| m.has_focus(field_id()));
        let stroke = if focused { Stroke::new(1.0, t.accent) } else { v.widgets.inactive.bg_stroke };
        ui.painter().rect(rect, t.radius_md + 2, v.extreme_bg_color, stroke, egui::StrokeKind::Inside);
        ui.painter().text(pos2(rect.left() + 11.0, rect.center().y), Align2::CENTER_CENTER, "🔍", FontId::proportional(t.font_small), v.weak_text_color());
        let key = ui.painter().layout_no_wrap(shortcut.to_owned(), FontId::proportional(t.font_small), v.weak_text_color());
        let cap = Rect::from_min_max(pos2(rect.right() - key.size().x - 12.0, rect.top() + 3.0), pos2(rect.right() - 4.0, rect.bottom() - 3.0));
        ui.painter().rect(cap, t.radius_md, v.widgets.inactive.weak_bg_fill, v.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
        ui.painter().galley(cap.center() - key.size() / 2.0, key, v.weak_text_color());
        let inner = Rect::from_min_max(pos2(rect.left() + 20.0, rect.top()), pos2(cap.left() - 2.0, rect.bottom()));
        let edit = egui::TextEdit::singleline(&mut self.text)
            .id(field_id())
            .frame(egui::Frame::NONE)
            .hint_text(crate::t!("Search commands"))
            .desired_width(inner.width());
        let response = ui.put(inner, edit);
        if std::mem::take(&mut self.focus) {
            response.request_focus();
        }
        if response.gained_focus() || response.changed() {
            self.open = true;
            self.selected = 0;
        }
        self.rects[0] = rect;
        if !self.open {
            return None;
        }
        let last = hits.saturating_sub(1);
        let (up, down, enter, escape, pressed) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
                i.pointer.any_pressed().then(|| i.pointer.interact_pos()),
            )
        });
        if down {
            self.selected = (self.selected + 1).min(last);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if enter && hits > 0 {
            return Some(self.selected.min(last));
        }
        // A press outside the field and its list closes the list.
        let outside = pressed.is_some_and(|pos| pos.is_none_or(|pos| !self.rects.iter().any(|r| r.contains(pos))));
        if escape || outside {
            self.close(&ctx);
        }
        None
    }

    fn results(&mut self, ctx: &egui::Context, hits: &[Hit<'_>]) -> Option<usize> {
        if !self.open {
            return None;
        }
        let t = palette(ctx).tokens();
        let field = self.rects[0];
        let width = 320.0;
        let selected = self.selected.min(hits.len().saturating_sub(1));
        let mut run = None;
        let area = egui::Area::new(Id::unique("command-search-results"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(field.right() - width, field.bottom() + 4.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(width - 12.0);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let v = ui.visuals().clone();
                    if hits.is_empty() {
                        ui.weak(crate::t!("No command by that name"));
                        return;
                    }
                    egui::ScrollArea::vertical().max_height(ROW_H * 10.0).show(ui, |ui| {
                        for (i, hit) in hits.iter().enumerate() {
                            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                            if i == selected {
                                ui.painter().rect_filled(rect, t.radius_md, t.accent_soft);
                            } else if response.hovered() {
                                ui.painter().rect_filled(rect, t.radius_md, v.widgets.hovered.weak_bg_fill);
                            }
                            let ink = if hit.enabled { v.text_color() } else { v.weak_text_color() };
                            let icon = if hit.enabled { hit.icon.color(t) } else { v.weak_text_color() };
                            let y = rect.center().y;
                            ui.painter().text(pos2(rect.left() + 13.0, y), Align2::CENTER_CENTER, hit.icon.glyph, FontId::proportional(t.font_body), icon);
                            ui.painter().text(pos2(rect.left() + 28.0, y), Align2::LEFT_CENTER, hit.name, FontId::proportional(t.font_body), ink);
                            ui.painter().text(pos2(rect.right() - 6.0, y), Align2::RIGHT_CENTER, hit.place, FontId::proportional(t.font_small), v.weak_text_color());
                            if response.clicked() {
                                run = Some(i);
                            }
                        }
                    });
                });
            });
        self.rects[1] = area.response.rect;
        run
    }
}
