//! Marine data simulator — desktop app.

mod app;
mod map;
mod ui;

use std::path::PathBuf;


fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    #[cfg(target_os = "macos")]
    name_process();

    // Optional first argument: a scenario file (else the app reopens the last one chosen).
    let path = std::env::args().nth(1).map(PathBuf::from);

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
        Box::new(move |cc| Ok(Box::new(app::SimApp::new(cc, path)?))),
    )
}

/// macOS builds the app menu ("About …", "Hide …", "Quit …") from the process name. Inside the
/// .app bundle that is the app's name already; run as a plain binary (`cargo run`), it would be
/// `marine-sim`.
#[cfg(target_os = "macos")]
fn name_process() {
    use objc2_foundation::{NSProcessInfo, NSString};
    NSProcessInfo::processInfo().setProcessName(&NSString::from_str("Marine Data Simulator"));
}
