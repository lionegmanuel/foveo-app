// Evita que se abra una consola extra en Windows en builds de release; en dev
// (`cargo build` sin --release) sigue mostrando la consola para poder ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppState;

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::recording::list_monitors,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::export::export_project,
        ])
        .run(tauri::generate_context!())
        .expect("error corriendo la app de tauri");
}
