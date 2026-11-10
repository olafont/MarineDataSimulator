//! AIS panel: own transponder, target list with range/CPA/TCPA, target editor and traffic
//! generators (single targets, collision course, random traffic).

use egui::{ComboBox, DragValue, Grid, RichText, Ui};
use engine::Snapshot;
use engine::sim_core::Command;
use engine::sim_core::ais::{
    ATON_TYPES, AisClass, AisTargetConfig, EPFD, NAV_STATUS, OwnAis, SHIP_TYPES, TargetMotion, collision_course,
    cpa_tcpa, ship_type_label,
};
use engine::sim_core::geo::norm_360;
use engine::sim_core::{Channel, LatLon};

use super::SimApp;
use crate::ui::form::{Form, Width, form};
use crate::map::MapAction;
use crate::ui::section;
use crate::ui::theme::tokens;
use crate::t;
use crate::ui::i18n::tr;

/// A local copy of a value that lives in the simulation, edited without fighting the
/// snapshot's one-tick lag. External changes (scenario load, map actions) replace it.
#[derive(Debug, Clone)]
pub struct Draft<T> {
    pub value: T,
    sent: T,
    last_live: T,
}

impl<T: Clone + PartialEq> Draft<T> {
    pub fn new(live: &T) -> Self {
        Self { value: live.clone(), sent: live.clone(), last_live: live.clone() }
    }

    /// Follows changes made elsewhere.
    pub fn sync(&mut self, live: &T) {
        if *live != self.last_live {
            self.last_live = live.clone();
            if *live != self.sent {
                *self = Self::new(live);
            }
        }
    }

    /// The edited value, once per change.
    pub fn take_change(&mut self) -> Option<T> {
        (self.value != self.sent).then(|| {
            self.sent = self.value.clone();
            self.value.clone()
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct OwnIdentity {
    name: String,
    mmsi: u32,
    call_sign: String,
    ais: OwnAis,
}

pub struct AisUi {
    pub(super) new_class: AisClass,
    pub(super) collision_minutes: f64,
    pub(super) collision_rel_bearing: f64,
    pub(super) collision_speed: f64,
    pub(super) random_count: u32,
    pub(super) random_radius_nm: f64,
    own: Option<Draft<OwnIdentity>>,
    target: Option<(u64, Draft<AisTargetConfig>)>,
    rng: u64,
    added: u32,
}

impl Default for AisUi {
    fn default() -> Self {
        Self {
            new_class: AisClass::A,
            collision_minutes: 12.0,
            collision_rel_bearing: 45.0,
            collision_speed: 14.0,
            random_count: 10,
            random_radius_nm: 6.0,
            own: None,
            target: None,
            rng: 0x2545_F491_4F6C_DD1D,
            added: 0,
        }
    }
}

impl AisUi {
    /// xorshift64*: uniform in [0, 1).
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn next_n(&mut self, snap: &Snapshot) -> u32 {
        self.added += 1;
        snap.sim.next_ais_target_id() as u32 + self.added
    }
}

const NAMES: &[&str] = &[
    "ATLANTIC STAR", "OCEAN PRIDE", "SEA BREEZE", "NORTHERN LIGHT", "BLUE HORIZON", "CAROLINA", "MARIA ELENA",
    "SAVANNAH", "GULF RUNNER", "CORAL QUEEN", "SILVER WAVE", "ISLAND SPIRIT", "MORNING TIDE", "PELICAN",
    "WINDSONG", "SEA HAWK", "TRADE WIND", "ENDEAVOUR", "SOUTHERN CROSS", "POLARIS",
];

impl SimApp {
    pub(super) fn ais(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let n = snap.sim.ais_targets.len();
        section(ui, "ais", crate::tf!("AIS ({n} targets)", n = n), false, |ui| {
            self.own_transponder(ui, snap);
            self.traffic_tools(ui, snap);
            ui.separator();
            self.target_list(ui, snap);
            self.target_editor(ui, snap);
        });
    }

    fn own_transponder(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let v = &snap.sim.vessel_cfg;
        let live = OwnIdentity { name: v.name.clone(), mmsi: v.mmsi, call_sign: v.call_sign.clone(), ais: v.ais.clone() };
        let draft = self.ais.own.get_or_insert_with(|| Draft::new(&live));
        draft.sync(&live);
        let d = &mut draft.value;
        egui::CollapsingHeader::new(t!("Own ship transponder (VDO)")).id_salt("own_ais").show(ui, |ui| {
            form(ui, "own_ais", |f| {
                f.check(t!("Transmit"), &mut d.ais.enabled, t!("!AIVDO own-ship reports"));
                f.text(t!("Name"), &mut d.name, Width::Medium, 20);
                f.drag("MMSI", DragValue::new(&mut d.mmsi).range(0..=999_999_999).speed(1.0));
                f.text(t!("Call sign"), &mut d.call_sign, Width::Short, 7);
                f.segmented(t!("Class"), &mut d.ais.class, &[(AisClass::A, "A"), (AisClass::B, "B")]);
                if d.ais.class == AisClass::A {
                    nav_status_row(f, &mut d.ais.nav_status);
                }
                statics_rows(f, &mut d.ais.statics, d.ais.class);
                flags_row(f, &mut d.ais.flags, d.ais.class);
                interval_rows(f, &mut d.ais.intervals, false);
            });
        });
        if let Some(c) = draft.take_change() {
            self.engine.send(Command::SetOwnAis { name: c.name, mmsi: c.mmsi, call_sign: c.call_sign, ais: Box::new(c.ais) });
        }
    }

    fn traffic_tools(&mut self, ui: &mut Ui, snap: &Snapshot) {
        ui.horizontal(|ui| {
            ComboBox::from_id_salt("new_class").selected_text(tr(self.ais.new_class.label())).show_ui(ui, |ui| {
                for &c in AisClass::ALL {
                    ui.selectable_value(&mut self.ais.new_class, c, tr(c.label()));
                }
            });
            if ui.button(t!("Add target")).on_hover_text(t!("2 nm from own ship; right-click the chart to place one")).clicked() {
                self.add_target_nearby(snap);
            }
        });
        ui.horizontal(|ui| {
            if ui.button(t!("Collision course")).on_hover_text(t!("A Class A target that reaches own ship's predicted position")).clicked() {
                self.add_collision_target(snap);
            }
            ui.add(DragValue::new(&mut self.ais.collision_minutes).range(1.0..=120.0).speed(0.5).suffix(" min"));
            ui.label(t!("from"));
            ui.add(DragValue::new(&mut self.ais.collision_rel_bearing).range(0.0..=359.0).speed(1.0).suffix("° rel"));
            ui.add(DragValue::new(&mut self.ais.collision_speed).range(1.0..=40.0).speed(0.2).suffix(" kn"));
        });
        ui.horizontal(|ui| {
            if ui.button(t!("Random traffic")).clicked() {
                self.add_random_traffic(snap);
            }
            ui.add(DragValue::new(&mut self.ais.random_count).range(1..=200).speed(0.2));
            ui.label(t!("within"));
            ui.add(DragValue::new(&mut self.ais.random_radius_nm).range(0.5..=50.0).speed(0.1).suffix(" nm"));
        });
        if !snap.sim.ais_targets.is_empty() && ui.button(t!("Remove all targets")).clicked() {
            self.remove_all_targets(snap);
        }
    }

    /// A target of the class chosen in the panel, 2 nm from own ship in a random direction.
    pub(super) fn add_target_nearby(&mut self, snap: &Snapshot) {
        let b = (self.ais.random() * 360.0).round();
        let p = snap.sim.vessel.position.destination(b, 2.0 * 1852.0);
        self.add_target_at(snap, p);
    }

    /// A Class A target that reaches own ship's predicted position (settings in the AIS panel).
    pub(super) fn add_collision_target(&mut self, snap: &Snapshot) {
        let own = snap.sim.vessel.position;
        let n = self.ais.next_n(snap);
        let heading = snap.sim.channels.get(Channel::HeadingTrue);
        let from = norm_360(heading + self.ais.collision_rel_bearing);
        let (start, cog) =
            collision_course(own, snap.sim.ground_velocity, self.ais.collision_minutes, from, self.ais.collision_speed);
        let mut cfg = AisTargetConfig::new(AisClass::A, n, start);
        cfg.name = format!("COLLISION {n}");
        cfg.motion = TargetMotion::Course { cog: cog.round(), sog_kn: self.ais.collision_speed };
        cfg.heading = cog;
        self.select_new(snap, cfg);
    }

    /// Random Class A and B traffic around own ship (count and radius in the AIS panel).
    pub(super) fn add_random_traffic(&mut self, snap: &Snapshot) {
        let own = snap.sim.vessel.position;
        for _ in 0..self.ais.random_count {
            let n = self.ais.next_n(snap);
            let class = if self.ais.random() < 0.6 { AisClass::A } else { AisClass::B };
            let p = own.destination(self.ais.random() * 360.0, self.ais.random().sqrt() * self.ais.random_radius_nm * 1852.0);
            let mut cfg = AisTargetConfig::new(class, n, p);
            cfg.name = NAMES[(self.ais.random() * NAMES.len() as f64) as usize % NAMES.len()].to_string();
            let anchored = class == AisClass::A && self.ais.random() < 0.15;
            if anchored {
                cfg.nav_status = 1;
                cfg.motion = TargetMotion::Stationary;
            } else {
                let cog = (self.ais.random() * 360.0).round();
                let sog = if class == AisClass::A { 8.0 + self.ais.random() * 14.0 } else { 3.0 + self.ais.random() * 5.0 };
                cfg.motion = TargetMotion::Course { cog, sog_kn: (sog * 10.0).round() / 10.0 };
                cfg.heading = cog;
                if class == AisClass::A {
                    cfg.statics.ship_type = [70, 80, 60, 30, 52][(self.ais.random() * 5.0) as usize % 5];
                }
            }
            self.engine.send(Command::AddAisTarget(Box::new(cfg)));
        }
    }

    pub(super) fn remove_all_targets(&mut self, snap: &Snapshot) {
        for t in &snap.sim.ais_targets {
            self.engine.send(Command::RemoveAisTarget(t.id));
        }
        self.ais.target = None;
        self.chart.selected_target = None;
    }

    pub(super) fn add_target_at(&mut self, snap: &Snapshot, p: LatLon) {
        let n = self.ais.next_n(snap);
        let mut cfg = AisTargetConfig::new(self.ais.new_class, n, p);
        if cfg.class.is_mobile() {
            // Head roughly across own ship's track.
            let cog = norm_360(p.bearing_to(&snap.sim.vessel.position) + 60.0).round();
            cfg.heading = cog;
            cfg.motion = TargetMotion::Course { cog, sog_kn: cfg.motion.speed() };
        }
        self.select_new(snap, cfg);
    }

    fn select_new(&mut self, snap: &Snapshot, cfg: AisTargetConfig) {
        // The simulation numbers targets in order, so this is the id the new one gets (unless
        // several are added within one tick).
        let id = snap.sim.next_ais_target_id();
        self.engine.send(Command::AddAisTarget(Box::new(cfg)));
        self.chart.selected_target = Some(id);
        self.ais.target = None;
    }

    fn target_list(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let sim = &snap.sim;
        if sim.ais_targets.is_empty() {
            ui.weak(t!("No targets. Add one above, or right-click the chart."));
            return;
        }
        let own = sim.vessel.position;
        let own_vel = sim.ground_velocity;
        let tk = tokens(ui);
        egui::ScrollArea::vertical().id_salt("ais_list").max_height(tk.list_height).show(ui, |ui| {
            Grid::new("ais_targets").num_columns(6).striped(true).spacing([tk.space_md, tk.space_xs]).show(ui, |ui| {
                ui.weak("");
                ui.weak(t!("Name"));
                ui.weak(t!("Range"));
                ui.weak(t!("Brg"));
                ui.weak("CPA");
                ui.weak("TCPA");
                ui.end_row();
                for t in &sim.ais_targets {
                    let mut enabled = t.cfg.enabled;
                    if ui.checkbox(&mut enabled, "").on_hover_text(t!("Transmitting")).changed() {
                        let mut c = t.cfg.clone();
                        c.enabled = enabled;
                        self.engine.send(Command::UpdateAisTarget(t.id, Box::new(c)));
                    }
                    let name = if t.cfg.name.trim().is_empty() { t.cfg.mmsi.to_string() } else { t.cfg.name.trim().to_string() };
                    let selected = self.chart.selected_target == Some(t.id);
                    let label = format!("{name} · {}", short_class(t.cfg.class));
                    if ui.selectable_label(selected, label).on_hover_text(format!("MMSI {}", t.cfg.mmsi)).clicked() {
                        self.chart.selected_target = if selected { None } else { Some(t.id) };
                    }
                    let range = own.distance_to(&t.state.position) / 1852.0;
                    ui.monospace(format!("{range:5.2}"));
                    ui.monospace(format!("{:03.0}°", own.bearing_to(&t.state.position)));
                    if t.cfg.class.is_mobile() || own_vel.magnitude() > 0.05 {
                        let (cpa, tcpa) = cpa_tcpa(own, own_vel, t.state.position, t.velocity());
                        let danger = cpa < 0.5 && (0.0..30.0).contains(&tcpa);
                        let color = if danger { tk.error } else { ui.visuals().text_color() };
                        ui.label(RichText::new(format!("{cpa:4.2}")).monospace().color(color));
                        if tcpa >= 0.0 {
                            ui.label(RichText::new(format!("{tcpa:5.1}m")).monospace().color(color));
                        } else {
                            ui.weak(RichText::new(t!("past")).monospace());
                        }
                    } else {
                        ui.weak("–");
                        ui.weak("–");
                    }
                    ui.end_row();
                }
            });
        });
        ui.weak(t!("Range and CPA in nm, TCPA in minutes; red = CPA < 0.5 nm within 30 min."));
    }

    fn target_editor(&mut self, ui: &mut Ui, snap: &Snapshot) {
        let Some(id) = self.chart.selected_target else {
            self.ais.target = None;
            return;
        };
        let Some(t) = snap.sim.ais_targets.iter().find(|t| t.id == id) else {
            // Not in the snapshot yet (just added) or removed.
            return;
        };
        let draft = match &mut self.ais.target {
            Some((did, d)) if *did == id => {
                d.sync(&t.cfg);
                d
            }
            slot => &mut slot.insert((id, Draft::new(&t.cfg))).1,
        };
        let c = &mut draft.value;
        let mut remove = false;
        let mut duplicate = None;
        ui.separator();
        ui.horizontal(|ui| {
            let name = if c.name.is_empty() { t!("(no name)") } else { &c.name };
            ui.strong(format!("{name} · {}", tr(c.class.label())));
            ui.weak(crate::tf!(
                "{sog} kn {cog}° · every {s} s",
                sog = format!("{:.1}", t.state.sog_kn),
                cog = format!("{:03.0}", t.state.cog),
                s = format!("{:.0}", t.position_interval())
            ));
        });
        form(ui, ("ais_edit", id), |f| {
            f.check(t!("Transmitting"), &mut c.enabled, "");
            f.text(t!("Name"), &mut c.name, Width::Medium, if c.class == AisClass::Aton { 34 } else { 20 });
            f.drag("MMSI", DragValue::new(&mut c.mmsi).range(0..=999_999_999).speed(1.0));
            let classes: Vec<(AisClass, String)> = AisClass::ALL.iter().map(|&k| (k, tr(k.label()))).collect();
            f.choice(t!("Class"), &mut c.class, &classes);
            if matches!(c.class, AisClass::A | AisClass::B) {
                f.text(t!("Call sign"), &mut c.call_sign, Width::Short, 7);
            }
            if c.class == AisClass::A {
                nav_status_row(f, &mut c.nav_status);
            }
            if c.class.is_mobile() {
                let current = c.motion.label();
                let mut picked = current;
                f.choice(
                    t!("Motion"),
                    &mut picked,
                    &[("Course", t!("Course")), ("Route", t!("Route")), ("Stationary", t!("Stationary"))],
                );
                if picked != current {
                    let speed = c.motion.speed().max(if c.class == AisClass::Sart { 0.5 } else { 10.0 });
                    c.motion = match picked {
                        "Course" => TargetMotion::Course { cog: t.state.cog.round(), sog_kn: speed },
                        "Route" => TargetMotion::Route { waypoints: vec![], sog_kn: speed, looped: false, active: 0 },
                        _ => TargetMotion::Stationary,
                    };
                }
                match &mut c.motion {
                    TargetMotion::Course { cog, sog_kn } => {
                        f.row("COG / SOG", |ui| {
                            ui.add(DragValue::new(cog).range(0.0..=359.9).speed(1.0).suffix("°"));
                            ui.add(DragValue::new(sog_kn).range(0.0..=102.0).speed(0.1).suffix(" kn"));
                        });
                    }
                    TargetMotion::Route { waypoints, sog_kn, looped, active } => {
                        f.drag("SOG", DragValue::new(sog_kn).range(0.0..=102.0).speed(0.1).suffix(" kn"));
                        f.row(t!("Route"), |ui| {
                            if waypoints.is_empty() {
                                ui.weak(t!("right-click the chart to add waypoints"));
                            } else {
                                ui.label(crate::tf!("{n} waypoints, to #{i}", n = waypoints.len(), i = *active + 1));
                                ui.checkbox(looped, "loop");
                                if ui.small_button(t!("Clear")).clicked() {
                                    waypoints.clear();
                                    *active = 0;
                                }
                            }
                        });
                    }
                    TargetMotion::Stationary => {}
                }
                f.drag(
                    t!("Max rate of turn"),
                    DragValue::new(&mut c.max_rot_dpm).range(1.0..=720.0).speed(1.0).suffix(" °/min"),
                );
            }
            if c.class == AisClass::Aton {
                let types: Vec<(u8, String)> = ATON_TYPES.iter().map(|(n, l)| (*n, format!("{n} {}", tr(l)))).collect();
                f.choice_width(t!("Type"), &mut c.aton_type, &types, Some(f.tokens().combo_wide));
                f.check(t!("Virtual"), &mut c.virtual_aton, t!("no physical mark"));
                f.row_hint(t!("Off position"), Some(t!("> 0 sets the off-position flag")), |ui| {
                    ui.add(DragValue::new(&mut c.off_position_m).range(0.0..=10_000.0).speed(1.0).suffix(" m"))
                });
            }
            if matches!(c.class, AisClass::A | AisClass::B | AisClass::Sart) || !c.safety_text.is_empty() {
                f.text_hint(t!("Safety text (14)"), &mut c.safety_text, Width::Medium, 161, "none");
            }
            if c.class == AisClass::B {
                f.check(t!("Message 19"), &mut c.send_msg19, t!("send with static data"));
            }
            if !matches!(c.class, AisClass::A | AisClass::B | AisClass::Aton | AisClass::BaseStation) {
                interval_rows(f, &mut c.intervals, true);
            }
        });
        if matches!(c.class, AisClass::A | AisClass::B | AisClass::Aton | AisClass::BaseStation) {
            egui::CollapsingHeader::new(t!("Static data, flags and intervals")).id_salt(("more", id)).show(ui, |ui| {
                form(ui, ("ais_more", id), |f| {
                    statics_rows(f, &mut c.statics, c.class);
                    flags_row(f, &mut c.flags, c.class);
                    interval_rows(f, &mut c.intervals, !c.safety_text.is_empty() || c.class == AisClass::Sart);
                });
            });
        }
        ui.horizontal(|ui| {
            if ui.button(t!("Duplicate")).clicked() {
                let mut d = c.clone();
                d.mmsi = d.mmsi.wrapping_add(1);
                d.name = format!("{} 2", d.name).chars().take(20).collect();
                d.position = t.state.position.destination(norm_360(t.state.heading + 90.0), 300.0);
                duplicate = Some(d);
            }
            if ui.button(t!("Remove")).clicked() {
                remove = true;
            }
        });
        if let Some(cfg) = draft.take_change() {
            self.engine.send(Command::UpdateAisTarget(id, Box::new(cfg)));
        }
        if let Some(d) = duplicate {
            self.select_new(snap, d);
        }
        if remove {
            self.engine.send(Command::RemoveAisTarget(id));
            self.chart.selected_target = None;
            self.ais.target = None;
        }
    }

    /// Map actions that concern AIS targets.
    pub(super) fn handle_ais_action(&mut self, snap: &Snapshot, action: MapAction) {
        let update = |this: &mut Self, id: u64, f: &dyn Fn(&mut AisTargetConfig)| {
            if let Some(t) = snap.sim.ais_targets.iter().find(|t| t.id == id) {
                let mut c = t.cfg.clone();
                f(&mut c);
                this.engine.send(Command::UpdateAisTarget(id, Box::new(c)));
            }
        };
        match action {
            MapAction::SelectTarget(id) => self.chart.selected_target = id,
            MapAction::AddTarget(p) => self.add_target_at(snap, p),
            MapAction::MoveTarget(id, p) => update(self, id, &|c| c.position = p),
            MapAction::AddTargetWaypoint(id, p) => update(self, id, &|c| match &mut c.motion {
                TargetMotion::Route { waypoints, .. } => waypoints.push(p),
                other => {
                    let sog = other.speed().max(5.0);
                    *other = TargetMotion::Route { waypoints: vec![p], sog_kn: sog, looped: false, active: 0 };
                }
            }),
            MapAction::TargetTowards(id, p) => {
                if let Some(t) = snap.sim.ais_targets.iter().find(|t| t.id == id) {
                    let cog = t.state.position.bearing_to(&p).round();
                    let sog = t.cfg.motion.speed().max(5.0);
                    update(self, id, &|c| c.motion = TargetMotion::Course { cog, sog_kn: sog });
                }
            }
            _ => {}
        }
    }
}

fn short_class(c: AisClass) -> &'static str {
    match c {
        AisClass::A => "A",
        AisClass::B => "B",
        AisClass::BaseStation => t!("base"),
        AisClass::Aton => "AtoN",
        AisClass::Sart => "SART",
    }
}

fn nav_status_row(f: &mut Form<'_>, status: &mut u8) {
    let options: Vec<(u8, String)> = NAV_STATUS.iter().enumerate().map(|(i, l)| (i as u8, format!("{i} {}", tr(l)))).collect();
    let width = f.tokens().combo_wide;
    f.choice_width(t!("Status"), status, &options, Some(width));
}

fn statics_rows(f: &mut Form<'_>, s: &mut engine::sim_core::ais::AisStatic, class: AisClass) {
    if matches!(class, AisClass::A | AisClass::B) {
        f.row(t!("Ship type"), |ui| {
            ui.add(DragValue::new(&mut s.ship_type).range(0..=99).speed(0.2));
            ComboBox::from_id_salt("ship_type").selected_text(tr(ship_type_label(s.ship_type))).show_ui(ui, |ui| {
                for (n, l) in SHIP_TYPES {
                    ui.selectable_value(&mut s.ship_type, *n, format!("{n} {}", tr(l)));
                }
            });
        });
    }
    if class == AisClass::A {
        f.drag("IMO", DragValue::new(&mut s.imo).range(0..=999_999_999).speed(1.0));
        f.text(t!("Destination"), &mut s.destination, Width::Medium, 20);
        f.text_hint("ETA (UTC)", &mut s.eta, Width::Short, 11, "MM-DD HH:MM");
        f.drag(t!("Draught"), DragValue::new(&mut s.draught_m).range(0.0..=25.5).speed(0.1).suffix(" m"));
    }
    if class == AisClass::B {
        f.text(t!("Vendor ID"), &mut s.vendor_id, Width::Short, 7);
    }
    if class != AisClass::BaseStation {
        f.row(t!("Antenna to bow/stern"), |ui| {
            ui.add(DragValue::new(&mut s.to_bow).range(0..=511).suffix(" m"));
            ui.add(DragValue::new(&mut s.to_stern).range(0..=511).suffix(" m"));
        });
        f.row(t!("Antenna to port/stbd"), |ui| {
            ui.add(DragValue::new(&mut s.to_port).range(0..=63).suffix(" m"));
            ui.add(DragValue::new(&mut s.to_starboard).range(0..=63).suffix(" m"));
        });
    }
    let epfd: Vec<(u8, &str)> = EPFD.iter().enumerate().map(|(i, l)| (i as u8, *l)).collect();
    f.choice(t!("Position device"), &mut s.epfd, &epfd);
    if matches!(class, AisClass::A | AisClass::B) {
        f.check("DTE", &mut s.dte_unavailable, t!("not available"));
    }
}

fn flags_row(f: &mut Form<'_>, fl: &mut engine::sim_core::ais::AisFlags, class: AisClass) {
    f.row(t!("Flags"), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut fl.position_accuracy, t!("High accuracy"));
            ui.checkbox(&mut fl.raim, "RAIM");
            ui.checkbox(&mut fl.assigned, t!("Assigned mode"));
            if class == AisClass::B {
                ui.checkbox(&mut fl.cs_unit, t!("CS unit"));
                ui.checkbox(&mut fl.display, t!("Display"));
                ui.checkbox(&mut fl.dsc, "DSC");
                ui.checkbox(&mut fl.band, t!("Whole band"));
                ui.checkbox(&mut fl.msg22, t!("Msg 22"));
            }
        });
    });
}

fn interval_rows(f: &mut Form<'_>, i: &mut engine::sim_core::ais::AisIntervals, text: bool) {
    f.row(t!("Position interval"), |ui| {
        let mut auto = i.position_s.is_none();
        if ui.checkbox(&mut auto, "auto").on_hover_text(t!("ITU-R M.1371: from class, speed, turning and status")).changed() {
            i.position_s = if auto { None } else { Some(10.0) };
        }
        if let Some(v) = &mut i.position_s {
            ui.add(DragValue::new(v).range(1.0..=3600.0).speed(0.5).suffix(" s"));
        }
    });
    f.drag(t!("Static interval"), DragValue::new(&mut i.static_s).range(1.0..=3600.0).speed(1.0).suffix(" s"));
    if text {
        f.drag(t!("Text interval"), DragValue::new(&mut i.safety_text_s).range(1.0..=3600.0).speed(1.0).suffix(" s"));
    }
}
