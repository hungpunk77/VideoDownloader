#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cast;
mod config;
mod downloader;
mod engines;
mod i18n;
mod search;
mod types;

use eframe::egui;
use std::sync::Arc;

fn load_app_icon() -> Option<Arc<egui::IconData>> {
    let icon_bytes = include_bytes!("../assets/icons/icon.png");
    let img = image::load_from_memory(icon_bytes).ok()?.into_rgba8();
    let width = img.width();
    let height = img.height();
    Some(Arc::new(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }))
}

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1240.0, 800.0])
        .with_min_inner_size([960.0, 640.0])
        .with_decorations(false)
        .with_title(format!("VideoDownloader v{}", env!("CARGO_PKG_VERSION")))
        .with_app_id("videodownloader");

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "VideoDownloader",
        options,
        Box::new(|cc| Ok(Box::new(app::VideoDownloaderApp::new(cc)))),
    )
}
