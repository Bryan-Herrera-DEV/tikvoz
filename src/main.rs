// En release no abrimos una consola negra junto a la ventana.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod filter;
mod monitor;
mod profanity;
mod speech;
mod tiktok;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TikVoz")
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([680.0, 420.0]),
        // Sin vsync forzado ni MSAA: la UI es 2D y solo repinta ante eventos.
        vsync: true,
        multisampling: 0,
        // Solo guardamos los ajustes, no la geometría: si la app se cierra a la
        // fuerza durante el arranque eframe puede guardar una ventana de 64x64.
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "TikVoz",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
