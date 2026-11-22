//! TikVoz como biblioteca.
//!
//! El ejecutable (`src/main.rs`) solo llama a [`run`]. Separarlo así permite que
//! los benchmarks (`benches/`) y los tests de integración usen los módulos
//! internos: el filtro de groserías, la limpieza de texto y el parseo de los
//! protocolos de TikTok y Kick.

mod app;
mod config;
mod demo;
pub mod filter;
pub mod kick;
pub mod live;
mod monitor;
pub mod profanity;
mod speech;
pub mod tiktok;

use eframe::egui;

/// Arranca la aplicación (ventana, hilos de red y voz).
pub fn run() -> eframe::Result {
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
            .with_title(format!("TikVoz v{}", env!("CARGO_PKG_VERSION")))
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
