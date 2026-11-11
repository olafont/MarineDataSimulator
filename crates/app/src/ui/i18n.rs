//! Languages of the user interface.
//!
//! The code is written in English and the English text is the key: `t!("Pause")` shows "Pause"
//! in English and its entry of `i18n/fr.toml`, `i18n/de.toml` or `i18n/es.toml` in French, German
//! or Spanish. A missing
//! entry shows the English text (the `every_string_is_translated` test keeps the catalogs
//! complete).
//!
//! Text with values uses named placeholders, filled after translation, so a translation can
//! move them: `tf!("{n} targets", n = count)`.
//!
//! The language is global (any widget, even deep in a helper, can translate) and switches at
//! once: the UI is redrawn every frame.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    English,
    French,
    German,
    Spanish,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::English, Lang::French, Lang::German, Lang::Spanish];

    /// The language's own name, as a language menu shows it.
    pub fn name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::French => "Français",
            Lang::German => "Deutsch",
            Lang::Spanish => "Español",
        }
    }

    /// The system's language, when it is one of ours (else English).
    pub fn system() -> Lang {
        let locale = sys_locale::get_locale().unwrap_or_default().to_ascii_lowercase();
        if locale.starts_with("fr") {
            Lang::French
        } else if locale.starts_with("de") {
            Lang::German
        } else if locale.starts_with("es") {
            Lang::Spanish
        } else {
            Lang::English
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::French,
        2 => Lang::German,
        3 => Lang::Spanish,
        _ => Lang::English,
    }
}

pub fn set_lang(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

type Catalog = HashMap<String, String>;

/// The catalog of a language other than English, read once.
fn catalog(l: Lang) -> Option<&'static Catalog> {
    static FRENCH: OnceLock<Catalog> = OnceLock::new();
    static GERMAN: OnceLock<Catalog> = OnceLock::new();
    static SPANISH: OnceLock<Catalog> = OnceLock::new();
    match l {
        Lang::English => None,
        Lang::French => Some(FRENCH.get_or_init(|| toml::from_str(include_str!("i18n/fr.toml")).expect("i18n/fr.toml is valid TOML"))),
        Lang::German => Some(GERMAN.get_or_init(|| toml::from_str(include_str!("i18n/de.toml")).expect("i18n/de.toml is valid TOML"))),
        Lang::Spanish => Some(SPANISH.get_or_init(|| toml::from_str(include_str!("i18n/es.toml")).expect("i18n/es.toml is valid TOML"))),
    }
}

/// `s` in the current language.
pub fn t(s: &'static str) -> &'static str {
    catalog(lang()).and_then(|c| c.get(s)).map_or(s, String::as_str)
}

/// `s` in the current language, for text that is not a literal (labels from the engine).
pub fn tr(s: &str) -> String {
    catalog(lang()).and_then(|c| c.get(s)).cloned().unwrap_or_else(|| s.to_string())
}

/// Fills `{name}` placeholders of a translated template.
pub fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (name, v) in values {
        out = out.replace(&format!("{{{name}}}"), v);
    }
    out
}

/// `s` in the current language: `t!("Pause")`. A macro, so that it never clashes with a local
/// variable named `t`.
#[macro_export]
macro_rules! t {
    ($s:literal) => {
        $crate::ui::i18n::t($s)
    };
}

/// Translates a template, then fills its placeholders:
/// `tf!("{n} targets", n = count)`, `tf!("to {a} of {b}", a = i + 1, b = len)`.
#[macro_export]
macro_rules! tf {
    ($template:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::ui::i18n::fill($crate::ui::i18n::t($template), &[$((stringify!($name), ($value).to_string())),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_and_falls_back_to_english() {
        set_lang(Lang::French);
        assert_eq!(t("Pause"), "Pause");
        assert_eq!(t("Preferences"), "Préférences");
        assert_eq!(t("not in the catalog"), "not in the catalog");
        assert_eq!(tf!("{n} targets", n = 3), "3 cibles");
        set_lang(Lang::German);
        assert_eq!(t("Preferences"), "Einstellungen");
        assert_eq!(tf!("{n} targets", n = 3), "3 Ziele");
        set_lang(Lang::Spanish);
        assert_eq!(t("Preferences"), "Preferencias");
        assert_eq!(tf!("{n} targets", n = 3), "3 blancos");
        assert_eq!(tr("not in the catalog"), "not in the catalog");
        set_lang(Lang::English);
        assert_eq!(t("Preferences"), "Preferences");
        assert_eq!(tf!("{n} targets", n = 3), "3 targets");
    }

    /// Every `t!("…")` and `tf!("…")` in the app's sources has an entry in every catalog, and
    /// every placeholder of a template is kept in its translation.
    #[test]
    fn every_string_is_translated() {
        let mut missing = std::collections::BTreeSet::new();
        for lang in Lang::ALL {
            let Some(cat) = catalog(lang) else { continue };
            let name = lang.name();
            for (file, src) in SOURCES {
                for key in literal_args(src) {
                    match cat.get(&key) {
                        None => {
                            missing.insert(format!("{name}: {file}: {key:?}"));
                        }
                        Some(v) => {
                            for p in placeholders(&key) {
                                assert!(v.contains(&p), "{name}: {file}: {key:?} → {v:?} lost {p}");
                            }
                        }
                    }
                }
            }
            for key in engine_labels() {
                if !cat.contains_key(key) {
                    missing.insert(format!("{name}: engine label: {key:?}"));
                }
            }
        }
        assert!(missing.is_empty(), "{} strings without a translation:\n{}", missing.len(), missing.into_iter().collect::<Vec<_>>().join("\n"));
    }

    /// The string literal that each `t!(` or `tf!(` starts with.
    fn literal_args(src: &str) -> Vec<String> {
        let mut out = vec![];
        for marker in ["t!(", "tf!("] {
            let mut rest = src;
            while let Some(i) = rest.find(marker) {
                // Only whole words: `t!(` and not `format!(`.
                let before = rest[..i].chars().next_back();
                rest = &rest[i + marker.len()..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                // The template may be on the next line.
                let Some(r) = rest.trim_start().strip_prefix('"') else { continue };
                rest = r;
                let mut s = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some(c) => s.push(c),
                            None => break,
                        },
                        '"' => break,
                        c => s.push(c),
                    }
                }
                out.push(s);
            }
        }
        out
    }

    fn placeholders(s: &str) -> Vec<String> {
        s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| format!("{{{n}}}"))).collect()
    }

    /// The app's sources, for the catalog test.
    const SOURCES: &[(&str, &str)] = &[
        ("app.rs", include_str!("../app.rs")),
        ("app/ais_panel.rs", include_str!("../app/ais_panel.rs")),
        ("app/controls.rs", include_str!("../app/controls.rs")),
        ("app/local_outputs.rs", include_str!("../app/local_outputs.rs")),
        ("app/monitor.rs", include_str!("../app/monitor.rs")),
        ("app/outputs.rs", include_str!("../app/outputs.rs")),
        ("app/prefs.rs", include_str!("../app/prefs.rs")),
        ("app/replay.rs", include_str!("../app/replay.rs")),
        ("app/ribbon_bar.rs", include_str!("../app/ribbon_bar.rs")),
        ("app/sensors.rs", include_str!("../app/sensors.rs")),
        ("map.rs", include_str!("../map.rs")),
        ("ui/form.rs", include_str!("form.rs")),
        ("ui/ribbon.rs", include_str!("ribbon.rs")),
        ("ui/widgets.rs", include_str!("widgets.rs")),
    ];

    /// Text the interface shows that is not a literal in the app: names from the engine (sensors,
    /// instruments, AIS tables, formats), the ribbon's tabs and the palettes. Shown through [`tr`].
    fn engine_labels() -> Vec<&'static str> {
        use engine::Source;
        use engine::replay::Timing;
        use engine::sim_core::ais::{ATON_TYPES, AisClass, EPFD, NAV_STATUS, SHIP_TYPES, ship_type_label};
        use engine::sim_core::{Channel, EnvParam};
        use engine::sim_sensors::SensorKind;
        use engine::transports::DataFormat;

        let mut v: Vec<&'static str> = vec![];
        v.extend(EnvParam::ALL.iter().map(|p| p.meta().label));
        v.extend(Channel::ALL.iter().map(|c| c.meta().label));
        v.extend(SensorKind::ALL.iter().map(|k| k.label()));
        v.extend(AisClass::ALL.iter().map(|c| c.label()));
        v.extend(NAV_STATUS);
        v.extend(SHIP_TYPES.iter().map(|(_, l)| *l));
        v.extend((0..=99).map(ship_type_label));
        v.extend(EPFD);
        v.extend(ATON_TYPES.iter().map(|(_, l)| *l));
        v.extend(DataFormat::ALL.iter().map(|f| f.label()));
        v.extend([Timing::TagBlocks, Timing::SentenceTimes, Timing::FixedRate].map(Timing::label));
        v.extend([Source::Vessel, Source::Replay].map(Source::label));
        v.extend(crate::ui::theme::Palette::ALL.map(|p| p.label()));
        v.extend(["Home", "Scenario", "Chart", "AIS traffic", "Data", "View"]); // ribbon tabs
        v
    }
}
