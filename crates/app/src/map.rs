//! Chart view: OpenStreetMap base layer + OpenSeaMap seamarks, own ship, track, route,
//! predictors and an environment indicator.

use std::collections::VecDeque;

use egui::{Align2, Color32, FontId, Pos2, Shape, Stroke, Ui, Vec2, vec2};
use engine::sim_core::geo::Ne;
use engine::sim_core::ais::{AisClass, AisTarget, TargetMotion};
use engine::sim_core::{Channel, EnvParam, LatLon, Simulation, Steering};
use walkers::sources::{Attribution, OpenStreetMap, TileSource};
use walkers::{HttpOptions, HttpTiles, Map, MapMemory, Position, Projector, TileId, lat_lon};
use crate::t;

const C: &crate::ui::theme::ChartColors = &crate::ui::theme::CHART;
const R: &crate::ui::theme::Tokens = &crate::ui::theme::DAY;

/// Seamark overlay (lights, buoys, …) from OpenSeaMap.
struct OpenSeaMap;

impl TileSource for OpenSeaMap {
    fn tile_url(&self, t: TileId) -> String {
        format!("https://tiles.openseamap.org/seamark/{}/{}/{}.png", t.zoom, t.x, t.y)
    }
    fn attribution(&self) -> Attribution {
        Attribution {
            text: "OpenSeaMap contributors",
            url: "https://openseamap.org",
            logo_light: None,
            logo_dark: None,
        }
    }
    fn max_zoom(&self) -> u8 {
        18
    }
}

pub enum MapAction {
    AddWaypoint(LatLon),
    MoveVessel(LatLon),
    SteerTowards(LatLon),
    /// Select an AIS target (None = clear the selection).
    SelectTarget(Option<u64>),
    AddTarget(LatLon),
    MoveTarget(u64, LatLon),
    /// Append a waypoint to a target's route (switching it to route mode).
    AddTargetWaypoint(u64, LatLon),
    /// Set a target on a constant course towards a point.
    TargetTowards(u64, LatLon),
}

pub struct ChartView {
    base: HttpTiles,
    seamarks: HttpTiles,
    pub show_seamarks: bool,
    memory: MapMemory,
    /// Left-click adds a waypoint.
    pub add_waypoints: bool,
    /// Position where the context menu was opened.
    context_pos: Option<LatLon>,
    track: VecDeque<LatLon>,
    /// Highlighted AIS target.
    pub selected_target: Option<u64>,
}

fn http_options(sub: &str) -> HttpOptions {
    HttpOptions {
        cache: dirs::cache_dir().map(|d| d.join("marine-sim").join("tiles").join(sub)),
        user_agent: Some(walkers::HeaderValue::from_static(concat!(
            "MarineDataSimulator/",
            env!("CARGO_PKG_VERSION")
        ))),
        ..Default::default()
    }
}

fn to_pos(p: LatLon) -> Position {
    lat_lon(p.lat, p.lon)
}

fn from_pos(p: Position) -> LatLon {
    LatLon::new(p.y(), p.x())
}

impl ChartView {
    pub fn new(ctx: &egui::Context) -> Self {
        let mut memory = MapMemory::default();
        let _ = memory.set_zoom(12.0);
        Self {
            base: HttpTiles::with_options(OpenStreetMap, http_options("osm"), ctx.clone()),
            seamarks: HttpTiles::with_options(OpenSeaMap, http_options("seamarks"), ctx.clone()),
            show_seamarks: true,
            memory,
            add_waypoints: false,
            context_pos: None,
            track: VecDeque::new(),
            selected_target: None,
        }
    }

    pub fn clear_track(&mut self) {
        self.track.clear();
    }

    fn update_track(&mut self, p: LatLon) {
        match self.track.back() {
            None => self.track.push_back(p),
            Some(last) => {
                let d = last.distance_to(&p);
                if d > 2_000.0 {
                    // Teleport or reset: start a new track.
                    self.track.clear();
                    self.track.push_back(p);
                } else if d > 5.0 {
                    self.track.push_back(p);
                    if self.track.len() > 20_000 {
                        self.track.pop_front();
                    }
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, sim: &Simulation, replay_fix: Option<LatLon>) -> Option<MapAction> {
        let vessel = sim.vessel.position;
        self.update_track(vessel);

        let track = &self.track;
        let add_waypoints = self.add_waypoints;
        let context_pos = &mut self.context_pos;
        let selected = self.selected_target.and_then(|id| sim.ais_targets.iter().find(|t| t.id == id));
        let mut action = None;

        let mut map = Map::new(Some(&mut self.base), &mut self.memory, to_pos(vessel));
        if self.show_seamarks {
            map = map.with_layer(&mut self.seamarks, 1.0);
        }
        let shade = crate::ui::theme::palette(ui.ctx()).chart_shade();
        let inner = map.show(ui, |ui, response, projector, _memory| {
            let painter = ui.painter();
            if let Some(shade) = shade {
                // Dim the tiles (dusk, night); what is drawn on them stays bright.
                painter.rect_filled(response.rect, 0.0, shade);
            }
            draw_track(painter, projector, track);
            draw_route(painter, projector, sim);
            draw_targets(painter, projector, sim, selected.map(|t| t.id));
            draw_vessel(painter, projector, sim);
            if let Some(p) = replay_fix {
                // Position from the log being replayed.
                let c = screen(projector, p);
                let color = C.replay;
                painter.circle_stroke(c, 9.0, Stroke::new(2.5, color));
                painter.circle_filled(c, 3.0, color);
                painter.text(c + vec2(12.0, -12.0), Align2::LEFT_BOTTOM, "replay", FontId::proportional(C.font_marker), color);
            }

            if response.clicked()
                && let Some(p) = response.interact_pointer_pos()
            {
                if add_waypoints {
                    action = Some(MapAction::AddWaypoint(from_pos(projector.unproject(p.to_vec2()))));
                } else {
                    // Select the nearest target within 14 px, or clear the selection.
                    let hit = sim
                        .ais_targets
                        .iter()
                        .filter(|t| t.cfg.enabled)
                        .map(|t| (t.id, screen(projector, t.state.position).distance(p)))
                        .filter(|(_, d)| *d < 14.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(id, _)| id);
                    action = Some(MapAction::SelectTarget(hit));
                }
            }
            if response.secondary_clicked()
                && let Some(p) = response.interact_pointer_pos()
            {
                *context_pos = Some(from_pos(projector.unproject(p.to_vec2())));
            }
            response.context_menu(|ui| {
                let Some(p) = *context_pos else { return };
                ui.weak(format!(
                    "{}  {}",
                    crate::ui::widgets::format_latlon(p.lat, 'N', 'S'),
                    crate::ui::widgets::format_latlon(p.lon, 'E', 'W')
                ));
                if ui.button(t!("Move vessel here")).clicked() {
                    action = Some(MapAction::MoveVessel(p));
                    ui.close();
                }
                if ui.button(t!("Add waypoint here")).clicked() {
                    action = Some(MapAction::AddWaypoint(p));
                    ui.close();
                }
                if ui.button(t!("Steer towards here")).clicked() {
                    action = Some(MapAction::SteerTowards(p));
                    ui.close();
                }
                ui.separator();
                if ui.button(t!("Add AIS target here")).clicked() {
                    action = Some(MapAction::AddTarget(p));
                    ui.close();
                }
                if let Some(t) = selected {
                    let name = if t.cfg.name.trim().is_empty() { t.cfg.mmsi.to_string() } else { t.cfg.name.clone() };
                    if ui.button(crate::tf!("Move {name} here", name = name)).clicked() {
                        action = Some(MapAction::MoveTarget(t.id, p));
                        ui.close();
                    }
                    if t.cfg.class.is_mobile() {
                        if ui.button(crate::tf!("Add waypoint to {name}'s route", name = name)).clicked() {
                            action = Some(MapAction::AddTargetWaypoint(t.id, p));
                            ui.close();
                        }
                        if ui.button(crate::tf!("Send {name} towards here", name = name)).clicked() {
                            action = Some(MapAction::TargetTowards(t.id, p));
                            ui.close();
                        }
                    }
                }
            });
            draw_environment(painter, response.rect, sim);
        });

        let rect = inner.response.rect;
        // Overlay controls.
        let follow_rect = egui::Rect::from_min_size(rect.left_top() + vec2(10.0, 10.0), vec2(300.0, 24.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(follow_rect), |ui| {
            ui.horizontal(|ui| {
                if self.memory.detached().is_some() && ui.button(t!("Follow vessel")).clicked() {
                    self.memory.follow_my_position();
                }
                if ui.button("+").clicked() {
                    let _ = self.memory.zoom_in();
                }
                if ui.button("−").clicked() {
                    let _ = self.memory.zoom_out();
                }
            });
        });
        let painter = ui.painter_at(rect);
        let attribution = if self.show_seamarks {
            "© OpenStreetMap contributors · © OpenSeaMap contributors"
        } else {
            "© OpenStreetMap contributors"
        };
        let galley = painter.layout_no_wrap(attribution.into(), FontId::proportional(C.font_label), C.ink);
        let at = rect.right_bottom() - galley.size() - vec2(6.0, 4.0);
        painter.rect_filled(egui::Rect::from_min_size(at - vec2(4.0, 2.0), galley.size() + vec2(8.0, 4.0)), R.radius_md, C.label_bg);
        painter.galley(at, galley, C.ink);
        if self.add_waypoints {
            painter.text(
                rect.center_top() + vec2(0.0, 14.0),
                Align2::CENTER_TOP,
                t!("Click to add waypoints"),
                FontId::proportional(C.font_marker),
                C.route,
            );
        }
        action
    }
}

fn screen(projector: &Projector, p: LatLon) -> Pos2 {
    projector.project(to_pos(p)).to_pos2()
}

fn draw_track(painter: &egui::Painter, projector: &Projector, track: &VecDeque<LatLon>) {
    if track.len() < 2 {
        return;
    }
    let pts: Vec<Pos2> = track.iter().map(|p| screen(projector, *p)).collect();
    painter.add(Shape::line(pts, Stroke::new(2.0, C.track.gamma_multiply(0.8))));
}

fn draw_route(painter: &egui::Painter, projector: &Projector, sim: &Simulation) {
    let Steering::Route { waypoints, active, looped, .. } = &sim.steering else { return };
    if waypoints.is_empty() {
        return;
    }
    let mut pts: Vec<Pos2> = waypoints.iter().map(|w| screen(projector, *w)).collect();
    if *looped && pts.len() > 2 {
        pts.push(pts[0]);
    }
    painter.add(Shape::dashed_line(&pts, Stroke::new(2.0, C.route), 8.0, 5.0));
    for (i, w) in waypoints.iter().enumerate() {
        let p = screen(projector, *w);
        let is_active = i == *active;
        painter.circle(p, if is_active { 8.0 } else { 6.0 }, C.marker_fill, Stroke::new(2.0, C.route));
        painter.text(p + vec2(10.0, -10.0), Align2::LEFT_BOTTOM, format!("{}", i + 1), FontId::proportional(C.font_marker), C.route);
    }
    // Leg to the active waypoint.
    if let Some(w) = waypoints.get(*active) {
        painter.line_segment([screen(projector, sim.vessel.position), screen(projector, *w)], Stroke::new(1.0, C.route));
    }
}

fn class_color(class: AisClass) -> Color32 {
    match class {
        AisClass::A => C.ais_a,
        AisClass::B => C.ais_b,
        AisClass::BaseStation => C.base_station,
        AisClass::Aton => C.aton,
        AisClass::Sart => C.sart,
    }
}

fn draw_targets(painter: &egui::Painter, projector: &Projector, sim: &Simulation, selected: Option<u64>) {
    let label_all = sim.ais_targets.len() <= 40;
    for t in sim.ais_targets.iter().filter(|t| t.cfg.enabled) {
        let is_selected = Some(t.id) == selected;
        if is_selected && let TargetMotion::Route { waypoints, active, looped, .. } = &t.cfg.motion {
            let mut pts: Vec<Pos2> = waypoints.iter().map(|w| screen(projector, *w)).collect();
            if *looped && pts.len() > 2 {
                pts.push(pts[0]);
            }
            let color = class_color(t.cfg.class);
            painter.add(Shape::dashed_line(&pts, Stroke::new(1.5, color), 6.0, 4.0));
            for (i, p) in pts.iter().take(waypoints.len()).enumerate() {
                painter.circle(*p, if i == *active { 5.0 } else { 3.5 }, C.marker_fill, Stroke::new(1.5, color));
            }
        }
        draw_target(painter, projector, t, is_selected, label_all || is_selected);
    }
}

fn draw_target(painter: &egui::Painter, projector: &Projector, t: &AisTarget, selected: bool, label: bool) {
    let c = screen(projector, t.state.position);
    let color = class_color(t.cfg.class);
    let stroke = Stroke::new(1.5, color);
    let dir = |deg: f64| {
        let a = deg.to_radians() as f32;
        vec2(a.sin(), -a.cos())
    };
    match t.cfg.class {
        AisClass::A | AisClass::B => {
            // Standard AIS symbol: an isosceles triangle pointing along the heading.
            let ppm = projector.scale_pixel_per_meter(to_pos(t.state.position));
            let len = (t.cfg.statics.length() as f32 * ppm).max(if t.cfg.class == AisClass::A { 20.0 } else { 15.0 });
            let fwd = dir(t.state.heading);
            let stb = vec2(-fwd.y, fwd.x);
            let tri = vec![c + fwd * len * 0.6, c - fwd * len * 0.4 + stb * len * 0.3, c - fwd * len * 0.4 - stb * len * 0.3];
            let fill = if t.cfg.class == AisClass::A { color.gamma_multiply(0.35) } else { Color32::TRANSPARENT };
            painter.add(Shape::convex_polygon(tri, fill, stroke));
            if t.state.sog_kn > 0.2 {
                // Six-minute COG/SOG vector.
                let ahead = t.state.position.destination(t.state.cog, t.state.sog_kn * 1852.0 / 10.0);
                painter.line_segment([c + fwd * len * 0.6, screen(projector, ahead)], Stroke::new(1.5, color));
            }
        }
        AisClass::Aton => {
            let r = 8.0;
            let pts = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
            if t.cfg.virtual_aton {
                let mut closed = pts.clone();
                closed.push(pts[0]);
                painter.add(Shape::dashed_line(&closed, stroke, 3.0, 2.0));
            } else {
                painter.add(Shape::convex_polygon(pts, color.gamma_multiply(0.3), stroke));
            }
            painter.line_segment([c + vec2(-3.0, 0.0), c + vec2(3.0, 0.0)], stroke);
            painter.line_segment([c + vec2(0.0, -3.0), c + vec2(0.0, 3.0)], stroke);
        }
        AisClass::BaseStation => {
            painter.rect_stroke(egui::Rect::from_center_size(c, vec2(14.0, 14.0)), 1.0, stroke, egui::StrokeKind::Middle);
            painter.circle_filled(c, 2.5, color);
        }
        AisClass::Sart => {
            painter.circle_stroke(c, 9.0, Stroke::new(2.0, color));
            painter.line_segment([c + vec2(-6.0, -6.0), c + vec2(6.0, 6.0)], Stroke::new(2.0, color));
            painter.line_segment([c + vec2(-6.0, 6.0), c + vec2(6.0, -6.0)], Stroke::new(2.0, color));
        }
    }
    if selected {
        painter.circle_stroke(c, 16.0, Stroke::new(2.0, C.selected));
    }
    if label {
        let name = if t.cfg.name.trim().is_empty() { t.cfg.mmsi.to_string() } else { t.cfg.name.trim().to_string() };
        let galley = painter.layout_no_wrap(name, FontId::proportional(C.font_label), C.ink);
        let at = c + vec2(12.0, 6.0);
        painter.rect_filled(egui::Rect::from_min_size(at - vec2(2.0, 1.0), galley.size() + vec2(4.0, 2.0)), R.radius_sm, C.label_bg);
        painter.galley(at, galley, C.ink);
    }
}

fn draw_vessel(painter: &egui::Painter, projector: &Projector, sim: &Simulation) {
    let pos = sim.vessel.position;
    let c = screen(projector, pos);
    let ppm = projector.scale_pixel_per_meter(to_pos(pos));

    // COG/SOG predictor: where the vessel will be in 6 minutes.
    let sog = sim.channels.get(Channel::Sog);
    let cog = sim.channels.get(Channel::Cog);
    let ahead = pos.destination(cog, sog * 1852.0 / 10.0);
    painter.line_segment([c, screen(projector, ahead)], Stroke::new(2.0, C.predictor));

    // Hull outline: real size when zoomed in, minimum 18 px otherwise.
    let len = (sim.vessel_cfg.length_m as f32 * ppm).max(18.0);
    let beam = (sim.vessel_cfg.beam_m as f32 * ppm).max(len * 0.38);
    let h = sim.vessel.heading.to_radians() as f32;
    // Screen axes: +x east, +y south.
    let fwd = vec2(h.sin(), -h.cos());
    let stb = vec2(h.cos(), h.sin());
    let p = |f: f32, s: f32| c + fwd * f * len + stb * s * beam;
    let hull = vec![p(0.5, 0.0), p(0.15, 0.5), p(-0.5, 0.4), p(-0.5, -0.4), p(0.15, -0.5)];
    painter.add(Shape::convex_polygon(hull, C.ship, Stroke::new(1.5, C.marker_fill)));
    // Heading line.
    painter.line_segment([p(0.5, 0.0), c + fwd * (len * 0.5 + 40.0)], Stroke::new(1.5, C.ship));
    if sim.grounded {
        painter.circle_stroke(c, len * 0.8, Stroke::new(3.0, C.ship));
        painter.text(c + vec2(0.0, len * 0.8 + 4.0), Align2::CENTER_TOP, "AGROUND", FontId::proportional(C.font_marker), C.ship);
    }
}

/// Compass-style indicator in the top-right corner: true wind (from) and current (towards).
fn draw_environment(painter: &egui::Painter, rect: egui::Rect, sim: &Simulation) {
    let r = 42.0;
    let c = rect.right_top() + vec2(-r - 16.0, r + 16.0);
    painter.circle(c, r + 8.0, C.label_bg, Stroke::new(1.0, C.ink.gamma_multiply(0.3)));
    painter.text(c - vec2(0.0, r + 2.0), Align2::CENTER_BOTTOM, "N", FontId::proportional(C.font_compass), C.ink);

    let dir = |deg: f64| {
        let a = deg.to_radians() as f32;
        vec2(a.sin(), -a.cos())
    };
    // Wind arrow points downwind, drawn from the upwind edge.
    let twd = sim.env.get(EnvParam::WindDirection);
    let tws = sim.env.get(EnvParam::WindSpeed);
    let from = c + dir(twd) * r;
    arrow(painter, from, c + dir(twd) * (r * 0.1), Stroke::new(3.0, C.wind));
    let cur = Ne::from_polar(sim.env.get(EnvParam::CurrentSpeed), sim.env.get(EnvParam::CurrentSet));
    if cur.magnitude() > 0.01 {
        arrow(painter, c, c + dir(cur.direction()) * (r * 0.75), Stroke::new(2.0, C.current));
    }
    let hs = sim.env.get(EnvParam::WaveHeight);
    let wave_from = sim.env.get(EnvParam::WaveDirection);
    if hs > 0.01 {
        // Short dashed arrow from the wave side.
        let from = c + dir(wave_from) * (r * 0.95);
        painter.add(Shape::dashed_line(&[from, c + dir(wave_from) * (r * 0.35)], Stroke::new(2.0, C.waves), 4.0, 3.0));
    }
    let mut text = crate::tf!(
        "wind {speed} kn / {dir}°\ncurrent {cspeed} kn to {cdir}°",
        speed = format!("{tws:.0}"),
        dir = format!("{twd:03.0}"),
        cspeed = format!("{:.1}", cur.magnitude()),
        cdir = format!("{:03.0}", cur.direction()),
    );
    if hs > 0.01 {
        text += "\n";
        text += &crate::tf!("waves {h} m / {dir}°", h = format!("{hs:.1}"), dir = format!("{wave_from:03.0}"));
    }
    if sim.dynamics.tide.enabled {
        text += "\n";
        text += &crate::tf!("tide {h} m", h = format!("{:.2}", sim.tide_height_m));
    }
    let galley = painter.layout_no_wrap(text, FontId::proportional(C.font_label), C.ink);
    // Centred under the rose, but kept inside the chart.
    let clip = painter.clip_rect();
    let x = (c.x - galley.size().x / 2.0).min(clip.right() - galley.size().x - 8.0).max(clip.left() + 8.0);
    let at = Pos2::new(x, c.y + r + 12.0);
    painter.rect_filled(egui::Rect::from_min_size(at - vec2(4.0, 2.0), galley.size() + vec2(8.0, 4.0)), R.radius_md, C.label_bg);
    painter.galley(at, galley, C.ink);
}

fn arrow(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    let d = to - from;
    if d.length() < 1.0 {
        return;
    }
    let n = d.normalized();
    let side = Vec2::new(-n.y, n.x);
    painter.line_segment([from, to], stroke);
    let head = 7.0;
    painter.add(Shape::convex_polygon(
        vec![to, to - n * head + side * head * 0.6, to - n * head - side * head * 0.6],
        stroke.color,
        Stroke::NONE,
    ));
}
