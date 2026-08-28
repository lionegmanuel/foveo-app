// Evita que se abra una consola extra en Windows en builds de release; en dev
// (`cargo build` sin --release) sigue mostrando la consola para poder ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppState;

/// Apunta `FFMPEG_BINARY_PATH` (leida por `crates/compositor` y
/// `crates/exporter`, ver CLAUDE.md regla 5) al sidecar de ffmpeg bundleado
/// por Tauri, si no fue seteada ya a mano (`.env`). `tauri-build` copia el
/// sidecar declarado en `bundle.externalBin` junto al ejecutable principal
/// SIEMPRE (dev y build final) y le saca el sufijo de target-triple del
/// nombre (ver `tauri-build::copy_binaries`), asi que en cualquiera de los
/// dos casos termina siendo literalmente `ffmpeg.exe` al lado de nuestro exe.
///
/// `set_var` es `unsafe` en esta edicion (2024) por el aliasing de env vars
/// entre threads — se llama una sola vez aca, antes de que `tauri::Builder`
/// arranque ningun thread, asi que es seguro.
fn resolve_and_set_ffmpeg_env_var() {
    if std::env::var_os("FFMPEG_BINARY_PATH").is_some() {
        return; // override explicito del usuario, no lo pisamos
    }

    let Ok(exe_path) = std::env::current_exe() else { return };
    let Some(exe_dir) = exe_path.parent() else { return };

    let sidecar = exe_dir.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    if sidecar.is_file() {
        // Seguro: unico punto de escritura, antes de spawnear ningun thread.
        unsafe { std::env::set_var("FFMPEG_BINARY_PATH", &sidecar) };
    }
}

fn main() {
    resolve_and_set_ffmpeg_env_var();

    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::recording::list_monitors,
            commands::recording::list_windows,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::export::export_project,
            commands::preview::render_preview_frame,
            commands::keyframes::get_project,
            commands::keyframes::update_keyframe,
            commands::keyframes::add_keyframe,
            commands::keyframes::delete_keyframe,
            commands::keyframes::update_style,
        ])
        .run(tauri::generate_context!())
        .expect("error corriendo la app de tauri");
}
