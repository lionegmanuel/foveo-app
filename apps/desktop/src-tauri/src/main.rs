// Evita que se abra una consola extra en Windows en builds de release; en dev
// (`cargo build` sin --release) sigue mostrando la consola para poder ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error corriendo la app de tauri");
}
