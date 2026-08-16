//! Estado compartido de la app (`AppState`, gestionado por Tauri via
//! `.manage()`). Solo esta capa conoce tanto Tauri como los crates de
//! `crates/` — ver CLAUDE.md regla 2.

use std::path::PathBuf;
use std::sync::Mutex;

use capture::WindowsRecordingHandle;
use input_tracker::RdevRecordingHandle;

/// Handles de una grabacion en curso. Vive en `AppState.recording` mientras
/// dura la grabacion; `stop_recording` lo consume.
pub struct RecordingSession {
    pub capture_handle: WindowsRecordingHandle,
    pub input_handle: RdevRecordingHandle,
    pub raw_take_path: PathBuf,
    pub input_log_path: PathBuf,
}

#[derive(Default)]
pub struct AppState {
    pub recording: Mutex<Option<RecordingSession>>,
}
