// En release no abrimos una consola negra junto a la ventana.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod demo;
mod filter;
mod kick;
mod live;
mod monitor;
mod profanity;
mod speech;
mod tiktok;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // Modo portable: `tikvoz.exe --config <carpeta>` guarda los ajustes ahí
    // (útil en una USB o para tener varios perfiles).
    let args: Vec<String> = std::env::args().collect();
    let persistence_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(|dir| {
            let _ = std::fs::create_dir_all(dir);
            std::path::PathBuf::from(dir).join("tikvoz.ron")
        });

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
        persistence_path,
        ..Default::default()
    };

    eframe::run_native(
        "TikVoz",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
