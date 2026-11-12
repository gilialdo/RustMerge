// Sem janela de console no build release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod diff_view;
mod editor;
mod find_bar;
mod settings;
mod tree_view;

use eframe::egui;

fn main() -> eframe::Result {
    // Uso opcional: rustmerge <pasta_esquerda> <pasta_direita>
    let args: Vec<String> = std::env::args().skip(1).collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([700.0, 400.0])
            .with_title("Comparador de arquivos"),
        ..Default::default()
    };

    eframe::run_native(
        "RustMerge",
        options,
        Box::new(move |cc| Ok(Box::new(app::RustMergeApp::new(cc, args)))),
    )
}
