// En release no abrimos una consola negra junto a la ventana.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    tikvoz::run()
}
