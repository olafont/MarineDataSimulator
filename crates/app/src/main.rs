//! Marine data simulator — desktop app.

mod app;
mod map;
mod ui;

use std::path::PathBuf;

use engine::scenario::{self, Scenario};

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Optional first argument: a scenario file.
    let path = std::env::args().nth(1).map(PathBuf::from);
    let (sc, path) = match &path {
        Some(p) => match Scenario::load(p) {
            Ok(s) => (s, path),
            Err(e) => {
                eprintln!("{e:#}\nfalling back to the built-in scenario");
                (scenario::default_scenario(), None)
            }
        },
        None => (scenario::default_scenario(), None),
    };

    let mut viewport = ui::title_bar::viewport(egui::ViewportBuilder::default());
    match eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png")) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => tracing::warn!("app icon: {e}"),
    }
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title("Marine Data Simulator")
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "marine-data-simulator",
        options,
        Box::new(move |cc| Ok(Box::new(app::SimApp::new(cc, sc, path)?))),
    )
}
