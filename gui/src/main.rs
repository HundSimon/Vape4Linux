//! OpenVape — a minimalist native Linux launcher for the Vape 4.21 agent.
//!
//! On start it brings up the local VapeService, lists the Minecraft JVMs that
//! this user owns, and injects the agent into the selected one using either
//! HotSpot's Attach API or the native `ptrace` injector — the same two paths
//! `tools/inject.sh` uses.

mod app;
mod cli;
mod inject;
mod runtime;
mod service;
mod targets;
mod theme;
mod ui;
mod util;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::dispatch(&args) {
        std::process::exit(code);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OpenVape")
            .with_app_id("openvape")
            .with_inner_size([760.0, 500.0])
            .with_min_inner_size([640.0, 420.0])
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "OpenVape",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
