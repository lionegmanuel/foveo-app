//! Comando Tauri de export. Corre `exporter::render_and_export` en un thread
//! dedicado (nunca en el hilo de eventos de Tauri/UI — CLAUDE.md regla 1) y
//! reporta progreso via eventos (`app.emit`), nunca bloqueando la respuesta
//! del comando en si (ver ARQUITECTURA.md 3.4: progreso por eventos, no
//! polling).

use std::path::PathBuf;

use project::Project;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
struct ExportProgressPayload {
    frames_done: u64,
}

#[derive(Debug, Clone, Serialize)]
struct ExportErrorPayload {
    message: String,
}

#[tauri::command]
pub fn export_project(app: AppHandle, project_path: String, output_path: String) -> Result<(), String> {
    let project = Project::load(&project_path).map_err(|e| e.to_string())?;
    let output = PathBuf::from(output_path);

    // El export puede tardar (decodificar + componer en GPU + encodear frame
    // a frame): corre en su propio thread y el comando vuelve enseguida, la
    // UI se entera del progreso/resultado por eventos.
    std::thread::spawn(move || {
        let progress_app = app.clone();
        let result = exporter::render_and_export(&project, &output, move |frames_done| {
            let _ = progress_app.emit("export-progress", ExportProgressPayload { frames_done });
        });

        match result {
            Ok(()) => {
                let _ = app.emit("export-finished", output.to_string_lossy().into_owned());
            }
            Err(err) => {
                let _ = app.emit("export-error", ExportErrorPayload { message: err.to_string() });
            }
        }
    });

    Ok(())
}
