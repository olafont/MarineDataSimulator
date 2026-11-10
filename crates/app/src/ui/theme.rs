//! Design tokens: every size and colour the app draws with, in one place.
//!
//! Panels and widgets read their spacing, field widths, fonts and colours from [`Tokens`]
//! (through [`tokens`]) instead of writing numbers inline. There is one set for egui's dark
//! theme and one for its light theme; [`apply`] installs the shared sizes and the status colours
//! into egui's own styles, so stock widgets agree with ours.

use egui::{Color32, Context, FontFamily, FontId, Id, Stroke, Style, TextStyle, Theme, Ui, Visuals, vec2};

/// Sizes and colours. Sizes are in points and identical in both themes.
#[derive(Debug, Clone)]
pub struct Tokens {
    // Spacing scale.
    pub space_xs: f32,
    pub space_sm: f32,
    pub space_md: f32,
    pub space_lg: f32,
    pub space_xl: f32,
    pub radius_sm: u8,
    pub radius_md: u8,
    /// Cards (the ribbon).
    pub radius_lg: u8,

    // Type.
    pub font_small: f32,
    pub font_body: f32,
    pub font_mono: f32,
    pub font_heading: f32,
    /// Icons on large ribbon buttons.
    pub font_icon: f32,

    // Forms: a label column, then the control.
    /// Minimum width of a form's label column, so forms in one panel line up.
    pub label_width: f32,
    /// Height of one form row.
    pub row_height: f32,
    /// Vertical space between form rows.
    pub row_gap: f32,
    /// Text field widths.
    pub field_short: f32,
    pub field_medium: f32,
    pub field_long: f32,
    /// Width of a drop-down that shows a code and its meaning.
    pub combo_wide: f32,
    /// Width of a drop-down that shows a short value (`10×`).
    pub combo_narrow: f32,
    /// Height of a scrolling list inside a panel.
    pub list_height: f32,

    // Ribbon.
    pub ribbon_tab_height: f32,
    /// Height of the band of commands under the tabs, captions included.
    pub ribbon_band_height: f32,
    /// Minimum width of a large ribbon button.
    pub ribbon_button_width: f32,

    // Status colours (text).
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
    /// A value replaced by an override.
    pub overridden: Color32,

    // Surfaces.
    /// The selected tab's fill and text.
    pub accent: Color32,
    pub accent_soft: Color32,
    /// Behind cards, between them and the window edge.
    pub canvas: Color32,
    pub card_shadow: Color32,

    /// Icon colours, by meaning.
    pub icons: IconInks,
}

/// Icon colours. Each command picks one by meaning, so a glance finds it.
#[derive(Debug, Clone)]
pub struct IconInks {
    /// Navigation and files.
    pub blue: Color32,
    /// Start, add.
    pub green: Color32,
    /// Pause, warnings, wind.
    pub amber: Color32,
    /// Record, remove.
    pub red: Color32,
    /// Display and view.
    pub purple: Color32,
    /// Water and data.
    pub teal: Color32,
}

/// Colours and sizes for what is drawn on the chart. The map tiles are light in both themes,
/// so there is one set ([`CHART`]).
#[derive(Debug, Clone)]
pub struct ChartColors {
    pub ship: Color32,
    pub track: Color32,
    pub route: Color32,
    pub predictor: Color32,
    /// The last position read from a replayed log.
    pub replay: Color32,
    pub selected: Color32,
    pub ais_a: Color32,
    pub ais_b: Color32,
    pub base_station: Color32,
    pub aton: Color32,
    pub sart: Color32,
    pub wind: Color32,
    pub current: Color32,
    pub waves: Color32,
    /// Text drawn on the chart and its translucent backing.
    pub ink: Color32,
    pub label_bg: Color32,
    pub marker_fill: Color32,
    /// Labels on a translucent backing (target names, attribution, scale).
    pub font_label: f32,
    /// Waypoint numbers and markers drawn straight on the map.
    pub font_marker: f32,
    pub font_compass: f32,
}

pub const CHART: ChartColors = ChartColors {
    ship: Color32::from_rgb(214, 48, 49),
    track: Color32::from_rgb(9, 132, 227),
    route: Color32::from_rgb(225, 112, 0),
    predictor: Color32::from_rgb(0, 148, 50),
    replay: Color32::from_rgb(200, 0, 160),
    selected: Color32::from_rgb(255, 170, 0),
    ais_a: Color32::from_rgb(0, 140, 70),
    ais_b: Color32::from_rgb(0, 110, 200),
    base_station: Color32::from_rgb(90, 90, 90),
    aton: Color32::from_rgb(170, 0, 170),
    sart: Color32::from_rgb(230, 0, 0),
    wind: Color32::from_rgb(108, 92, 231),
    current: Color32::from_rgb(0, 168, 168),
    waves: Color32::from_rgb(0, 112, 192),
    ink: Color32::from_rgb(30, 30, 30),
    label_bg: Color32::from_rgba_premultiplied(255, 255, 255, 215),
    marker_fill: Color32::WHITE,
    font_label: 11.0,
    font_marker: 13.0,
    font_compass: 10.0,
};

const fn base(ok: Color32, warn: Color32, error: Color32) -> Tokens {
    Tokens {
        space_xs: 2.0,
        space_sm: 4.0,
        space_md: 8.0,
        space_lg: 12.0,
        space_xl: 16.0,
        radius_sm: 2,
        radius_md: 3,
        radius_lg: 8,
        font_small: 10.5,
        font_body: 13.0,
        font_mono: 12.0,
        font_heading: 16.0,
        font_icon: 24.0,
        label_width: 136.0,
        row_height: 20.0,
        row_gap: 4.0,
        field_short: 80.0,
        field_medium: 160.0,
        field_long: 220.0,
        combo_wide: 200.0,
        combo_narrow: 64.0,
        list_height: 220.0,
        ribbon_tab_height: 26.0,
        ribbon_band_height: 76.0,
        ribbon_button_width: 52.0,
        ok,
        warn,
        error,
        overridden: warn,
        accent: Color32::TRANSPARENT,
        accent_soft: Color32::TRANSPARENT,
        canvas: Color32::TRANSPARENT,
        card_shadow: Color32::TRANSPARENT,
        icons: IconInks {
            blue: Color32::TRANSPARENT,
            green: Color32::TRANSPARENT,
            amber: Color32::TRANSPARENT,
            red: Color32::TRANSPARENT,
            purple: Color32::TRANSPARENT,
            teal: Color32::TRANSPARENT,
        },
    }
}

/// Dusk: the dark blue-grey palette.
pub const DUSK: Tokens = Tokens {
    accent: Color32::from_rgb(147, 197, 253),
    accent_soft: Color32::from_rgb(30, 58, 95),
    canvas: Color32::from_rgb(14, 16, 20),
    card_shadow: Color32::from_black_alpha(90),
    icons: IconInks {
        blue: Color32::from_rgb(96, 165, 250),
        green: Color32::from_rgb(74, 222, 128),
        amber: Color32::from_rgb(251, 191, 36),
        red: Color32::from_rgb(248, 113, 113),
        purple: Color32::from_rgb(167, 139, 250),
        teal: Color32::from_rgb(45, 212, 191),
    },
    ..base(Color32::from_rgb(90, 200, 120), Color32::from_rgb(255, 170, 60), Color32::from_rgb(255, 100, 100))
};
/// Day: the light palette.
pub const DAY: Tokens = Tokens {
    accent: Color32::from_rgb(29, 78, 216),
    accent_soft: Color32::from_rgb(219, 234, 254),
    canvas: Color32::from_rgb(226, 229, 234),
    card_shadow: Color32::from_black_alpha(28),
    icons: IconInks {
        blue: Color32::from_rgb(37, 99, 235),
        green: Color32::from_rgb(22, 163, 74),
        amber: Color32::from_rgb(217, 119, 6),
        red: Color32::from_rgb(220, 38, 38),
        purple: Color32::from_rgb(124, 58, 237),
        teal: Color32::from_rgb(13, 148, 136),
    },
    ..base(Color32::from_rgb(0, 140, 70), Color32::from_rgb(190, 105, 0), Color32::from_rgb(200, 30, 30))
};

/// Night: dim red on black, to keep the eyes adapted to the dark on watch. Every colour is
/// a shade of red or amber; meanings show by brightness and symbol as well as colour.
pub const NIGHT: Tokens = Tokens {
    accent: Color32::from_rgb(240, 90, 70),
    accent_soft: Color32::from_rgb(60, 14, 12),
    canvas: Color32::from_rgb(4, 1, 1),
    card_shadow: Color32::from_black_alpha(140),
    overridden: Color32::from_rgb(220, 120, 40),
    icons: IconInks {
        blue: Color32::from_rgb(190, 60, 50),
        green: Color32::from_rgb(210, 90, 50),
        amber: Color32::from_rgb(220, 120, 40),
        red: Color32::from_rgb(255, 60, 40),
        purple: Color32::from_rgb(170, 55, 45),
        teal: Color32::from_rgb(200, 80, 60),
    },
    ..base(Color32::from_rgb(200, 90, 50), Color32::from_rgb(220, 120, 40), Color32::from_rgb(255, 60, 40))
};

/// The three palettes of a bridge display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Palette {
    Day,
    Dusk,
    Night,
}

impl Palette {
    pub const ALL: [Palette; 3] = [Palette::Day, Palette::Dusk, Palette::Night];

    pub fn label(self) -> &'static str {
        match self {
            Palette::Day => "Day",
            Palette::Dusk => "Dusk",
            Palette::Night => "Night",
        }
    }

    pub fn tokens(self) -> &'static Tokens {
        match self {
            Palette::Day => &DAY,
            Palette::Dusk => &DUSK,
            Palette::Night => &NIGHT,
        }
    }

    /// What is laid over the chart tiles: nothing by day, darker at dusk, dark red at night.
    pub fn chart_shade(self) -> Option<Color32> {
        match self {
            Palette::Day => None,
            Palette::Dusk => Some(Color32::from_black_alpha(90)),
            Palette::Night => Some(Color32::from_rgba_unmultiplied(30, 0, 0, 200)),
        }
    }

    fn visuals(self) -> Visuals {
        match self {
            Palette::Day => Visuals::light(),
            Palette::Dusk => Visuals::dark(),
            Palette::Night => night_visuals(),
        }
    }
}

fn palette_id() -> Id {
    Id::unique("marine_sim_palette")
}

/// The palette in use. Before [`set_palette`], follows egui's dark or light theme.
pub fn palette(ctx: &Context) -> Palette {
    ctx.data(|d| d.get_temp(palette_id()))
        .unwrap_or_else(|| if ctx.global_style().visuals.dark_mode { Palette::Dusk } else { Palette::Day })
}

/// The palette the user picked, if any (else the app follows the system's light or dark mode).
pub fn chosen_palette(ctx: &Context) -> Option<Palette> {
    ctx.data(|d| d.get_temp(palette_id()))
}

/// Switches the whole app to a palette.
pub fn set_palette(ctx: &Context, p: Palette) {
    ctx.data_mut(|d| d.insert_temp(palette_id(), p));
    let theme = if p == Palette::Day { Theme::Light } else { Theme::Dark };
    ctx.style_mut_of(theme, |s| {
        s.visuals = p.visuals();
        install(s, p.tokens());
    });
    ctx.set_theme(theme);
}

/// The tokens of the palette in use.
pub fn tokens(ui: &Ui) -> &'static Tokens {
    palette(ui.ctx()).tokens()
}

fn night_visuals() -> Visuals {
    let red = Color32::from_rgb;
    let mut v = Visuals::dark();
    v.panel_fill = red(12, 4, 4);
    v.window_fill = red(16, 5, 5);
    v.extreme_bg_color = red(6, 2, 2);
    v.faint_bg_color = red(20, 7, 7);
    v.code_bg_color = red(24, 8, 8);
    v.window_stroke = Stroke::new(1.0, red(60, 16, 14));
    v.hyperlink_color = red(220, 70, 50);
    v.selection.bg_fill = red(90, 22, 18);
    v.selection.stroke = Stroke::new(1.0, red(240, 80, 60));
    let w = &mut v.widgets;
    for (wv, bg, weak, stroke, fg, fg_w) in [
        (&mut w.noninteractive, red(16, 5, 5), red(16, 5, 5), red(50, 14, 12), red(170, 40, 30), 1.0),
        (&mut w.inactive, red(34, 10, 9), red(30, 9, 8), red(40, 12, 10), red(190, 48, 36), 1.0),
        (&mut w.hovered, red(52, 14, 12), red(48, 13, 11), red(110, 30, 24), red(225, 60, 45), 1.5),
        (&mut w.active, red(70, 18, 15), red(70, 18, 15), red(140, 36, 28), red(240, 70, 50), 2.0),
        (&mut w.open, red(52, 14, 12), red(48, 13, 11), red(110, 30, 24), red(225, 60, 45), 1.0),
    ] {
        wv.bg_fill = bg;
        wv.weak_bg_fill = weak;
        wv.bg_stroke = Stroke::new(1.0, stroke);
        wv.fg_stroke = Stroke::new(fg_w, fg);
    }
    v
}

/// The meaning of a coloured text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Error,
    Muted,
}

impl Tokens {
    pub fn tone(&self, ui: &Ui, tone: Tone) -> Color32 {
        match tone {
            Tone::Ok => self.ok,
            Tone::Warn => self.warn,
            Tone::Error => self.error,
            Tone::Muted => ui.visuals().weak_text_color(),
        }
    }
}

/// Installs the tokens into egui's dark and light styles (Dusk and Day). Call once at start;
/// [`set_palette`] then switches between Day, Dusk and Night.
pub fn apply(ctx: &Context) {
    for (theme, t) in [(Theme::Dark, &DUSK), (Theme::Light, &DAY)] {
        ctx.style_mut_of(theme, |s| install(s, t));
    }
}

fn install(s: &mut Style, t: &Tokens) {
    s.spacing.item_spacing = vec2(t.space_md, t.space_sm);
    s.spacing.button_padding = vec2(t.space_sm + t.space_xs, t.space_xs);
    s.spacing.interact_size.y = t.row_height;
    s.spacing.indent = t.space_xl;
    s.visuals.menu_corner_radius = t.radius_md.into();
    s.spacing.combo_width = t.field_medium;
    s.spacing.text_edit_width = t.field_medium;
    s.text_styles = [
        (TextStyle::Small, FontId::new(t.font_small, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(t.font_body, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(t.font_body, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(t.font_mono, FontFamily::Monospace)),
        (TextStyle::Heading, FontId::new(t.font_heading, FontFamily::Proportional)),
    ]
    .into();
    s.visuals.warn_fg_color = t.warn;
    s.visuals.error_fg_color = t.error;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Night keeps every colour red or amber: red is the largest channel, blue never above green.
    #[test]
    fn night_has_no_blue_or_green() {
        let v = night_visuals();
        let t = &NIGHT;
        let i = &t.icons;
        for c in [
            t.ok, t.warn, t.error, t.overridden, t.accent, t.accent_soft, t.canvas, i.blue, i.green, i.amber, i.red,
            i.purple, i.teal, v.panel_fill, v.window_fill, v.selection.bg_fill, v.widgets.inactive.fg_stroke.color,
            v.widgets.noninteractive.fg_stroke.color, v.hyperlink_color,
        ] {
            assert!(c.r() >= c.g() && c.g() >= c.b(), "{c:?}");
        }
    }

    #[test]
    fn set_palette_switches_tokens_and_style() {
        let ctx = Context::default();
        apply(&ctx);
        for p in Palette::ALL {
            set_palette(&ctx, p);
            assert_eq!(palette(&ctx), p);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(std::ptr::eq(tokens(ui), p.tokens()));
                assert_eq!(ui.visuals().panel_fill, p.visuals().panel_fill);
                assert_eq!(ui.visuals().warn_fg_color, p.tokens().warn);
            });
            out.textures_delta.clear();
        }
    }

    #[test]
    fn spacing_scale_increases() {
        for t in [&DUSK, &DAY, &NIGHT] {
            let s = [t.space_xs, t.space_sm, t.space_md, t.space_lg, t.space_xl];
            assert!(s.windows(2).all(|w| w[0] < w[1]), "{s:?}");
            assert!(t.field_short < t.field_medium && t.field_medium < t.field_long);
        }
    }

    #[test]
    fn status_colours_differ_between_themes_and_from_each_other() {
        for t in [&DUSK, &DAY] {
            assert!(t.ok != t.warn && t.warn != t.error && t.ok != t.error);
        }
        assert_ne!(DUSK.warn, DAY.warn);
    }

    #[test]
    fn apply_sets_both_styles() {
        let ctx = Context::default();
        apply(&ctx);
        for (theme, t) in [(Theme::Dark, &DUSK), (Theme::Light, &DAY)] {
            let s = ctx.style_of(theme);
            assert_eq!(s.visuals.warn_fg_color, t.warn);
            assert_eq!(s.spacing.interact_size.y, t.row_height);
            assert_eq!(s.text_styles[&TextStyle::Body].size, t.font_body);
        }
    }
}
