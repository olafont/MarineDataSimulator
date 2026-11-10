//! The app's own title bar, on macOS: the OS title bar is hidden and the window's content runs
//! under it, so the strip the OS would leave empty holds the app's name, quick commands and
//! status. The OS still draws its traffic lights over the bar's left end ([`TRAFFIC_LIGHTS`]),
//! and keeps the window's frame, corners and shadow.
//!
//! Empty space in the bar moves the window (with the OS's own move loop) and a double-click there
//! zooms it, as on a native title bar. Elsewhere the OS title bar is kept and [`integrated`] is
//! false.
//!
//! The approach is SolidCraft's (`crates/ui-egui/src/window_chrome.rs`).

use egui::{Id, PointerButton, Rect, Sense, Ui, UiBuilder, ViewportBuilder, ViewportCommand, pos2};

/// Room for the macOS traffic lights at the bar's left end.
pub const TRAFFIC_LIGHTS: f32 = 76.0;

/// True when the app draws the title bar (macOS).
pub const fn integrated() -> bool {
    cfg!(target_os = "macos")
}

/// The window settings that hide the OS title bar and run the content under it.
pub fn viewport(v: ViewportBuilder) -> ViewportBuilder {
    if !integrated() {
        return v;
    }
    v.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
}

/// Lays out the bar in `bar`: the drag area first (so the widgets added by `contents`, which are
/// registered later, take their clicks), then `contents` in the space right of the traffic lights.
pub fn show<R>(ui: &mut Ui, bar: Rect, contents: impl FnOnce(&mut Ui) -> R) -> R {
    drag_area(ui, bar);
    let inner = Rect::from_min_max(pos2(bar.left() + TRAFFIC_LIGHTS, bar.top()), bar.right_bottom());
    ui.scope_builder(UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), contents)
        .inner
}

/// The bar's background: a press-and-drag on empty space moves the window, a double-click zooms.
fn drag_area(ui: &Ui, rect: Rect) {
    let ctx = ui.ctx();
    let resp = ui.interact(rect, Id::unique("title-bar-drag"), Sense::click_and_drag());
    // egui hands a drag that starts on a click-only button to the drag-sensing background below
    // it, so remember whether the press itself landed on bare background.
    if ctx.input(|i| i.pointer.any_pressed()) {
        let bare = ctx.viewport(|v| v.hits.click.is_some_and(|w| w.id == resp.id));
        ctx.data_mut(|d| d.insert_temp(resp.id, bare));
    }
    if resp.double_clicked() {
        let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
    } else if resp.drag_started_by(PointerButton::Primary) && ctx.data(|d| d.get_temp(resp.id).unwrap_or(false)) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, RawInput, ViewportId, vec2};

    /// Frames of a press-drag-release from `from` to `to` over a bar with one button at its
    /// start; the viewport commands they sent.
    fn drag(from: Pos2, to: Pos2) -> Vec<ViewportCommand> {
        let ctx = egui::Context::default();
        let button = |p, pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let frames = [
            vec![Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![Event::PointerMoved(to)],
            vec![Event::PointerMoved(to)],
            vec![button(to, false)],
        ];
        let mut cmds = vec![];
        for (i, events) in frames.into_iter().enumerate() {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))),
                time: Some(i as f64 * 0.05),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let bar = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 30.0));
                show(ui, bar, |ui| ui.button("Open"));
            });
            out.textures_delta.clear();
            cmds.extend(out.viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default());
        }
        cmds
    }

    #[test]
    fn empty_space_moves_the_window_and_buttons_do_not() {
        let start_drag = |c: &Vec<ViewportCommand>| c.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count();
        assert_eq!(start_drag(&drag(pos2(500.0, 15.0), pos2(560.0, 40.0))), 1, "drag from empty space");
        assert_eq!(start_drag(&drag(pos2(TRAFFIC_LIGHTS + 12.0, 15.0), pos2(TRAFFIC_LIGHTS + 80.0, 40.0))), 0, "drag from the button");
    }
}
