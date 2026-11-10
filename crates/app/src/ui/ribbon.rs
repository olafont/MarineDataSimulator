//! Ribbon widgets: a floating card with a tab strip and, under it, a band of captioned groups.
//!
//! ```ignore
//! egui::Panel::top("ribbon").frame(ribbon::outer_frame(ui)).show(ui, |ui| {
//!     ribbon::card(ui, |ui| {
//!         ribbon::tabs(ui, &mut state, TABS, |ui| status(ui));
//!         ribbon::band(ui, |ui| {
//!             ribbon::group_menu(ui, "Simulation", |ui| {
//!                 if ribbon::big(ui, Icon::green("▶"), "Run").clicked() { … }
//!             }, |ui| { /* the ⏷ menu */ });
//!         });
//!     });
//! });
//! ```
//!
//! The widgets know nothing about the simulator: the app supplies the tabs and dispatches the
//! clicks. Sizes and colours come from the theme tokens.

use egui::containers::menu::MenuButton;
use egui::{
    Align, Align2, Button, Color32, FontId, Frame, Layout, Margin, Rect, Response, RichText, Sense, Shadow,
    Ui, vec2,
};

use crate::ui::theme::{Tokens, tokens};
use crate::t;

/// Which tab is shown and whether the band is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State<T> {
    pub tab: T,
    pub collapsed: bool,
}

/// The colour of an icon, by meaning (see [`crate::ui::theme::IconInks`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Blue,
    Green,
    Amber,
    Red,
    Purple,
    Teal,
}

/// A glyph and its colour.
#[derive(Debug, Clone, Copy)]
pub struct Icon {
    pub glyph: &'static str,
    pub ink: Ink,
}

impl Icon {
    pub const fn new(glyph: &'static str, ink: Ink) -> Self {
        Self { glyph, ink }
    }
    pub const fn blue(glyph: &'static str) -> Self {
        Self::new(glyph, Ink::Blue)
    }
    pub const fn green(glyph: &'static str) -> Self {
        Self::new(glyph, Ink::Green)
    }
    pub const fn amber(glyph: &'static str) -> Self {
        Self::new(glyph, Ink::Amber)
    }
    pub const fn red(glyph: &'static str) -> Self {
        Self::new(glyph, Ink::Red)
    }
    pub const fn purple(glyph: &'static str) -> Self {
        Self::new(glyph, Ink::Purple)
    }
    pub const fn teal(glyph: &'static str) -> Self {
        Self::new(glyph, Ink::Teal)
    }

    pub fn color(self, t: &Tokens) -> Color32 {
        match self.ink {
            Ink::Blue => t.icons.blue,
            Ink::Green => t.icons.green,
            Ink::Amber => t.icons.amber,
            Ink::Red => t.icons.red,
            Ink::Purple => t.icons.purple,
            Ink::Teal => t.icons.teal,
        }
    }
}

/// The frame of the top panel that holds the card: the canvas colour, with a margin.
pub fn outer_frame(ui: &Ui) -> Frame {
    let t = tokens(ui);
    let m = t.space_md as i8;
    Frame::NONE.fill(t.canvas).inner_margin(Margin { left: m, right: m, top: m, bottom: m / 2 })
}

/// A rounded, shadowed card.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = tokens(ui);
    Frame::new()
        .fill(ui.visuals().panel_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(t.radius_lg)
        .shadow(Shadow { offset: [0, 2], blur: 10, spread: 0, color: t.card_shadow })
        .inner_margin(Margin::symmetric(t.space_md as i8, t.space_sm as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// The tab strip. Clicking the current tab, or double-clicking any tab, hides or shows the band;
/// clicking a tab while hidden shows it. `right` draws on the right of the strip (status).
pub fn tabs<T: Copy + PartialEq>(ui: &mut Ui, state: &mut State<T>, tabs: &[(T, &str)], right: impl FnOnce(&mut Ui)) {
    let t = tokens(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(t.ribbon_tab_height);
        tab_pills(ui, state, tabs);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            collapse_button(ui, state);
            ui.add_space(t.space_md);
            right(ui);
        });
    });
}

/// The tabs alone, left to right in the current layout (e.g. in the title bar).
pub fn tab_pills<T: Copy + PartialEq>(ui: &mut Ui, state: &mut State<T>, tabs: &[(T, &str)]) {
    let t = tokens(ui);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = t.space_xs;
        for &(tab, label) in tabs {
            let selected = state.tab == tab;
            let r = tab_button(ui, selected && !state.collapsed, label);
            if r.double_clicked() {
                state.tab = tab;
                state.collapsed = !state.collapsed;
            } else if r.clicked() {
                if selected {
                    state.collapsed = !state.collapsed;
                } else {
                    state.tab = tab;
                    state.collapsed = false;
                }
            }
        }
    });
}

/// ⏶ / ⏷: hides or shows the band.
pub fn collapse_button<T>(ui: &mut Ui, state: &mut State<T>) {
    let (icon, tip) = if state.collapsed { ("⏷", t!("Show the ribbon")) } else { ("⏶", t!("Hide the ribbon")) };
    if ui.add(Button::new(icon).frame_when_inactive(false)).on_hover_text(tip).clicked() {
        state.collapsed = !state.collapsed;
    }
}

/// A tab: text on a filled pill when selected.
/// `label` is the English name; it is shown in the interface's language.
fn tab_button(ui: &mut Ui, selected: bool, label: &str) -> Response {
    let t = tokens(ui);
    let font = FontId::proportional(t.font_body);
    let label = super::i18n::tr(label);
    let galley = ui.painter().layout_no_wrap(label.clone(), font.clone(), ui.visuals().text_color());
    let size = vec2(galley.size().x + 2.0 * t.space_lg, t.ribbon_tab_height);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let pill = rect.shrink2(vec2(0.0, t.space_xs));
        let ink = if selected {
            ui.painter().rect_filled(pill, t.radius_md + 2, t.accent_soft);
            t.accent
        } else if r.hovered() {
            ui.painter().rect_filled(pill, t.radius_md + 2, v.widgets.hovered.weak_bg_fill);
            v.strong_text_color()
        } else {
            v.text_color()
        };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, ink);
    }
    r
}

/// The band under the tabs, hidden while collapsed: groups side by side, scrolling sideways if
/// the window is narrow.
pub fn band(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = tokens(ui);
    egui::ScrollArea::horizontal().id_salt("ribbon_band").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(t.ribbon_band_height);
            ui.spacing_mut().item_spacing.x = t.space_sm;
            add(ui);
        });
    });
}

/// A group of commands with its caption under it, followed by a divider.
pub fn group<R>(ui: &mut Ui, caption: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    group_inner(ui, &mut (), caption, |ui, _| add(ui), None::<fn(&mut Ui, &mut ())>)
}

/// A group whose caption opens a menu (`Caption ⏷`) with more of its commands and settings.
/// `state` (typically the app) is lent to the commands, then to the menu.
pub fn group_menu<S, R>(
    ui: &mut Ui,
    state: &mut S,
    caption: &str,
    add: impl FnOnce(&mut Ui, &mut S) -> R,
    menu: impl FnOnce(&mut Ui, &mut S),
) -> R {
    group_inner(ui, state, caption, add, Some(menu))
}

fn group_inner<S, R>(
    ui: &mut Ui,
    state: &mut S,
    caption: &str,
    add: impl FnOnce(&mut Ui, &mut S) -> R,
    menu: Option<impl FnOnce(&mut Ui, &mut S)>,
) -> R {
    let t = tokens(ui);
    let caption_h = t.font_small + t.space_sm + t.space_xs;
    let inner = ui
        .vertical(|ui| {
            let body = ui.horizontal(|ui| {
                ui.set_height(t.ribbon_band_height - caption_h);
                add(ui, state)
            });
            // The caption is centred under the commands, as wide as they are (or as itself).
            let text_w = ui
                .painter()
                .layout_no_wrap(format!("{caption} ⏷"), FontId::proportional(t.font_small), Color32::WHITE)
                .size()
                .x;
            let w = body.response.rect.width().max(text_w + 2.0 * t.space_md);
            ui.allocate_ui_with_layout(vec2(w, caption_h), Layout::top_down(Align::Center), |ui| {
                let text = |s: String| RichText::new(s).size(t.font_small).color(ui.visuals().weak_text_color());
                match menu {
                    Some(menu) => {
                        let button = Button::new(text(format!("{caption} ⏷"))).frame_when_inactive(false).small();
                        MenuButton::from_button(button).ui(ui, |ui| {
                            ui.set_min_width(t.field_long);
                            menu(ui, state)
                        });
                    }
                    None => {
                        ui.label(text(caption.to_string()));
                    }
                }
            });
            body.inner
        })
        .inner;
    ui.add_space(t.space_xs);
    ui.separator();
    inner
}

/// Several small commands stacked in one column of a group.
pub fn column<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = tokens(ui).space_xs;
        add(ui)
    })
    .inner
}

/// A large command: an icon above its label.
pub fn big(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    big_button(ui, icon, label, false, true)
}

/// A large on/off command, drawn selected while on.
pub fn big_toggle(ui: &mut Ui, on: bool, icon: Icon, label: &str) -> Response {
    big_button(ui, icon, label, on, true)
}

/// A large command that may be unavailable.
pub fn big_enabled(ui: &mut Ui, enabled: bool, icon: Icon, label: &str) -> Response {
    big_button(ui, icon, label, false, enabled)
}

fn big_button(ui: &mut Ui, icon: Icon, label: &str, on: bool, enabled: bool) -> Response {
    let t = tokens(ui);
    let label_font = FontId::proportional(t.font_small + 1.0);
    let label_w = ui.painter().layout_no_wrap(label.to_string(), label_font.clone(), ui.visuals().text_color()).size().x;
    let caption_h = t.font_small + t.space_sm + t.space_xs;
    let size = vec2((label_w + 2.0 * t.space_sm).max(t.ribbon_button_width), t.ribbon_band_height - caption_h);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, r) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let fill = if on {
            Some(t.accent_soft)
        } else if enabled && r.is_pointer_button_down_on() {
            Some(v.widgets.active.weak_bg_fill)
        } else if enabled && r.hovered() {
            Some(v.widgets.hovered.weak_bg_fill)
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, t.radius_md + 2, fill);
        }
        let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.35) };
        let text_ink = dim(if on { t.accent } else { v.text_color() });
        let icon_rect = Rect::from_min_max(rect.min, rect.min + vec2(rect.width(), rect.height() * 0.62));
        ui.painter().text(
            icon_rect.center(),
            Align2::CENTER_CENTER,
            icon.glyph,
            FontId::proportional(t.font_icon),
            dim(icon.color(t)),
        );
        ui.painter().text(rect.center_bottom() - vec2(0.0, t.space_sm), Align2::CENTER_BOTTOM, label, label_font, text_ink);
    }
    r
}

/// A small command for a [`column`]: coloured icon and label on one line.
pub fn small(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    let atoms = (icon_text(ui, icon), label.to_string());
    ui.add(Button::new(atoms).frame_when_inactive(false))
}

/// A small on/off command for a [`column`].
pub fn small_toggle(ui: &mut Ui, on: &mut bool, icon: Icon, label: &str) -> Response {
    let atoms = (icon_text(ui, icon), label.to_string());
    ui.toggle_value(on, atoms)
}

fn icon_text(ui: &Ui, icon: Icon) -> RichText {
    let t = tokens(ui);
    RichText::new(icon.glyph).color(icon.color(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, add: impl FnMut(&mut Ui)) {
        let mut out = ctx.run_ui(egui::RawInput::default(), add);
        out.textures_delta.clear();
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Tab {
        A,
        B,
    }

    #[test]
    fn band_has_a_fixed_height_and_buttons_fit_in_it() {
        let ctx = egui::Context::default();
        crate::ui::theme::apply(&ctx);
        let mut heights = (0.0, 0.0);
        for _ in 0..2 {
            frame(&ctx, |ui| {
                let top = ui.cursor().top();
                band(ui, |ui| {
                    group_menu(
                        ui,
                        &mut heights.1,
                        "One",
                        |ui, h| {
                            *h = big(ui, Icon::green("▶"), "Run").rect.height();
                        },
                        |ui, _| {
                            ui.label(crate::t!("more"));
                        },
                    );
                });
                heights.0 = ui.cursor().top() - top;
            });
        }
        let t = &crate::ui::theme::DUSK;
        assert!(heights.0 >= t.ribbon_band_height, "band {heights:?}");
        assert!(heights.1 < t.ribbon_band_height, "button {heights:?}");
    }

    /// Each group is as wide as its commands, so the next one starts right after it.
    #[test]
    fn groups_sit_side_by_side() {
        let ctx = egui::Context::default();
        crate::ui::theme::apply(&ctx);
        let mut rects = vec![];
        for _ in 0..2 {
            rects.clear();
            frame(&ctx, |ui| {
                band(ui, |ui| {
                    for caption in ["One", "Two"] {
                        let r = group_menu(ui, &mut (), caption, |ui, _| big(ui, Icon::green("▶"), "Run").rect, |_, _| {});
                        rects.push(r);
                    }
                    rects.push(group(ui, "Three", |ui| big(ui, Icon::blue("⟲"), "Reset").rect));
                });
            });
        }
        assert_eq!(rects.len(), 3);
        for w in rects.windows(2) {
            let gap = w[1].left() - w[0].right();
            assert!(gap > 0.0 && gap < 60.0, "groups too far apart: {rects:?}");
        }
    }

    #[test]
    fn tab_state_toggles() {
        let mut s = State { tab: Tab::A, collapsed: false };
        let ctx = egui::Context::default();
        frame(&ctx, |ui| tabs(ui, &mut s, &[(Tab::A, "A"), (Tab::B, "B")], |_| {}));
        assert_eq!(s, State { tab: Tab::A, collapsed: false }, "no input, no change");
    }

    #[test]
    fn icon_inks_are_set_in_both_themes() {
        for t in [&crate::ui::theme::DAY, &crate::ui::theme::DUSK, &crate::ui::theme::NIGHT] {
            let i = &t.icons;
            for c in [i.blue, i.green, i.amber, i.red, i.purple, i.teal, t.accent, t.accent_soft, t.canvas] {
                assert_ne!(c, Color32::TRANSPARENT);
            }
        }
    }
}

