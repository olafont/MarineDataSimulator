//! Small reusable widgets and formatting helpers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{ComboBox, DragValue, Id, Response, Ui};
use engine::sim_core::{Kind, Meta, OverrideMode, Param};
use crate::t;

/// Local edit buffers for values that live in the simulation thread.
///
/// The GUI reads values from snapshots that lag a tick behind. While the user drags or types,
/// the widget must show what the user is entering, not the (stale) snapshot. An edit is kept
/// until the snapshot catches up or a short timeout passes.
#[derive(Default)]
pub struct Edits {
    map: HashMap<Id, (f64, Instant)>,
}

impl Edits {
    /// Shows a numeric widget bound to `live`. Returns the new value when the user changed it.
    pub fn value(
        &mut self,
        ui: &mut Ui,
        id: Id,
        live: f64,
        add: impl FnOnce(&mut Ui, &mut f64) -> Response,
    ) -> Option<f64> {
        let mut v = self.map.get(&id).map_or(live, |e| e.0);
        let r = add(ui, &mut v);
        if r.changed() || r.dragged() || r.has_focus() {
            self.map.insert(id, (v, Instant::now()));
        } else if let Some((pending, at)) = self.map.get(&id)
            && ((pending - live).abs() < 1e-9 || at.elapsed() > Duration::from_millis(600))
        {
            self.map.remove(&id);
        }
        r.changed().then_some(v)
    }
}

pub fn mode_label(m: OverrideMode) -> &'static str {
    match m {
        OverrideMode::Replace => t!("Replace"),
        OverrideMode::Offset => t!("Offset"),
        OverrideMode::Freeze => t!("Freeze"),
    }
}

/// Reasonable drag speed for a quantity.
pub fn drag_speed(meta: &Meta) -> f64 {
    match meta.kind {
        Kind::Latitude | Kind::Longitude => 0.0001,
        _ => ((meta.max - meta.min) / 1000.0).max(0.01),
    }
}

fn decimals(meta: &Meta) -> usize {
    match meta.kind {
        Kind::Latitude | Kind::Longitude => 6,
        _ => 2,
    }
}

/// A drag value configured for a quantity (range, speed, unit).
pub fn quantity_drag<'a>(v: &'a mut f64, meta: &Meta, offset: bool) -> DragValue<'a> {
    let d = DragValue::new(v).speed(drag_speed(meta)).max_decimals(decimals(meta));
    let d = if offset { d } else { d.range(meta.min..=meta.max) };
    if meta.unit.is_empty() { d } else { d.suffix(format!(" {}", meta.unit)) }
}

/// Three grid cells: override checkbox, mode, value. Returns `Some(change)` when the user
/// changed the override (`Some(None)` = cleared).
pub fn override_cells(
    ui: &mut Ui,
    edits: &mut Edits,
    id: Id,
    p: &Param,
    meta: &Meta,
) -> Option<Option<(OverrideMode, f64)>> {
    let mut out = None;
    let mut on = p.ovr.is_some();
    if ui.checkbox(&mut on, "").on_hover_text(t!("Override this value")).changed() {
        out = Some(on.then_some((OverrideMode::Replace, p.value())));
    }
    match p.ovr {
        Some(o) => {
            let mut mode = o.mode;
            ComboBox::from_id_salt(id.with("mode"))
                .width(72.0)
                .selected_text(mode_label(mode))
                .show_ui(ui, |ui| {
                    for m in [OverrideMode::Replace, OverrideMode::Offset, OverrideMode::Freeze] {
                        ui.selectable_value(&mut mode, m, mode_label(m));
                    }
                });
            if mode != o.mode {
                let v = match mode {
                    OverrideMode::Replace => p.value(),
                    OverrideMode::Offset | OverrideMode::Freeze => 0.0,
                };
                out = Some(Some((mode, v)));
                ui.label("");
            } else if mode == OverrideMode::Freeze {
                ui.weak(format_value(meta, o.value));
            } else {
                let offset = mode == OverrideMode::Offset;
                if let Some(v) =
                    edits.value(ui, id.with("v"), o.value, |ui, v| ui.add(quantity_drag(v, meta, offset)))
                {
                    out = Some(Some((mode, v)));
                }
            }
        }
        None => {
            ui.label("");
            ui.label("");
        }
    }
    out
}

pub fn format_value(meta: &Meta, v: f64) -> String {
    match meta.kind {
        Kind::Latitude => format_latlon(v, 'N', 'S'),
        Kind::Longitude => format_latlon(v, 'E', 'W'),
        Kind::Bearing => format!("{v:05.1}{}", meta.unit),
        _ => format!("{v:.2} {}", meta.unit),
    }
}

/// `43°15.302' N`
pub fn format_latlon(v: f64, pos: char, neg: char) -> String {
    let h = if v < 0.0 { neg } else { pos };
    let a = v.abs();
    let mut d = a.trunc();
    let mut m = ((a - d) * 60.0 * 1000.0).round() / 1000.0;
    if m >= 60.0 {
        d += 1.0;
        m -= 60.0;
    }
    format!("{d:.0}°{m:06.3}' {h}")
}

pub fn format_time(unix: f64) -> String {
    chrono::DateTime::from_timestamp(unix.floor() as i64, 0)
        .map(|t| t.format(t!("%Y-%m-%d %H:%M:%S UTC")).to_string())
        .unwrap_or_default()
}
