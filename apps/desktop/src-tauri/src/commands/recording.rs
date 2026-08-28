//! Comandos Tauri de grabacion: `list_monitors`, `start_recording`,
//! `stop_recording`. Unica capa que conoce tanto Tauri como los crates de
//! `crates/` (ver CLAUDE.md regla 2) — arma el `.szproj` a partir de la
//! toma cruda + el log de input crudo.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use capture::{CaptureSource, RecordingHandle as _, ScreenCapturer, WindowsScreenCapturer};
use input_tracker::{InputRecordingHandle as _, InputSource, RdevInputSource};
use project::{CursorPathPoint, InputLog, Project, RawTake, Resolution};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use zoom_engine::{InputEvent, InputEventKind};

use crate::state::{AppState, RecordingSession};

/// Cada cuanto se guarda una muestra de cursor en el `.szproj`, como mucho
/// (ver `build_cursor_path`). 30Hz alcanza para una reconstruccion suave y
/// mantiene el `.szproj` chico incluso en grabaciones largas.
const CURSOR_SAMPLE_INTERVAL_MS: u64 = 33;

#[derive(Debug, Serialize)]
pub struct MonitorDto {
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize)]
pub struct WindowDto {
    pub index: usize,
    pub title: String,
}

#[tauri::command]
pub fn list_monitors() -> Result<Vec<MonitorDto>, String> {
    let capturer = WindowsScreenCapturer;
    let monitors = capturer.list_monitors().map_err(|e| e.to_string())?;
    Ok(monitors
        .into_iter()
        .map(|m| MonitorDto { index: m.index, name: m.name, width: m.width, height: m.height })
        .collect())
}

#[tauri::command]
pub fn list_windows() -> Result<Vec<WindowDto>, String> {
    let capturer = WindowsScreenCapturer;
    let windows = capturer.list_windows().map_err(|e| e.to_string())?;
    Ok(windows.into_iter().map(|w| WindowDto { index: w.index, title: w.title }).collect())
}

fn takes_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("takes");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn now_wall_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("reloj del sistema antes de epoch").as_millis()
}

#[tauri::command]
pub fn start_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_index: Option<usize>,
    window_index: Option<usize>,
) -> Result<(), String> {
    let mut guard = state.recording.lock().map_err(|_| "estado de grabacion envenenado".to_string())?;
    if guard.is_some() {
        return Err("ya hay una grabacion en curso".to_string());
    }

    let dir = takes_dir(&app)?;
    let stamp = now_wall_ms();
    let raw_take_path = dir.join(format!("take_{stamp}.mp4"));
    let input_log_path = dir.join(format!("take_{stamp}.input.jsonl"));

    // `window_index` gana si esta presente (seleccionar una ventana especifica
    // en la UI desactiva el selector de monitor); ver CaptureSource.
    let source = match (window_index, monitor_index) {
        (Some(w), _) => CaptureSource::Window(w),
        (None, Some(m)) => CaptureSource::Monitor(m),
        (None, None) => CaptureSource::PrimaryMonitor,
    };

    let capture_handle = WindowsScreenCapturer.start_recording(&raw_take_path, source).map_err(|e| e.to_string())?;
    let input_handle = RdevInputSource.start_logging(&input_log_path).map_err(|e| e.to_string())?;

    *guard = Some(RecordingSession { capture_handle, input_handle, raw_take_path, input_log_path });

    Ok(())
}

/// Parsea el JSONL crudo del input-tracker (timestamps de reloj de pared,
/// pixeles) a `zoom_engine::InputEvent` (timestamps relativos al inicio de
/// la grabacion, coordenadas normalizadas 0..1) — ver el contrato de entrada
/// documentado en `zoom_engine::lib`.
fn parse_input_log(
    path: &Path,
    recording_started_wall_ms: u128,
    width: u32,
    height: u32,
) -> Result<Vec<InputEvent>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut events = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let Some(kind) = value["kind"].as_str().and_then(|k| match k {
            "move" => Some(InputEventKind::Move),
            "button_press" => Some(InputEventKind::ButtonPress),
            "button_release" => Some(InputEventKind::ButtonRelease),
            _ => None,
        }) else {
            continue;
        };

        let t_wall_ms = value["t_wall_ms"].as_u64().ok_or("evento sin t_wall_ms")? as u128;
        let x = value["x"].as_f64().ok_or("evento sin x")?;
        let y = value["y"].as_f64().ok_or("evento sin y")?;

        let t_ms = t_wall_ms.saturating_sub(recording_started_wall_ms) as u64;
        let x_norm = ((x as f32) / (width as f32)).clamp(0.0, 1.0);
        let y_norm = ((y as f32) / (height as f32)).clamp(0.0, 1.0);

        events.push(InputEvent { t_ms, x: x_norm, y: y_norm, kind });
    }

    Ok(events)
}

/// Downsamplea los `InputEvent::Move` a lo sumo cada `CURSOR_SAMPLE_INTERVAL_MS`
/// para la reconstruccion de cursor suavizado (`project::CursorPathPoint`,
/// consumido por `compositor::cursor_path`). Los clicks (`ButtonPress`/
/// `ButtonRelease`) siempre se conservan sin importar el intervalo, para que
/// el efecto de click quede timesteado con precision.
fn build_cursor_path(events: &[InputEvent]) -> Vec<CursorPathPoint> {
    let mut path = Vec::new();
    let mut pressed = false;
    let mut last_kept_t_ms: Option<u64> = None;

    for event in events {
        match event.kind {
            InputEventKind::ButtonPress => pressed = true,
            InputEventKind::ButtonRelease => pressed = false,
            InputEventKind::Move => {}
        }

        let is_click = matches!(event.kind, InputEventKind::ButtonPress | InputEventKind::ButtonRelease);
        let due = last_kept_t_ms.is_none_or(|t| event.t_ms.saturating_sub(t) >= CURSOR_SAMPLE_INTERVAL_MS);

        if is_click || due {
            path.push(CursorPathPoint { t_ms: event.t_ms, x: event.x, y: event.y, pressed });
            last_kept_t_ms = Some(event.t_ms);
        }
    }

    path
}

#[tauri::command]
pub fn stop_recording(state: State<'_, AppState>) -> Result<String, String> {
    let session = {
        let mut guard = state.recording.lock().map_err(|_| "estado de grabacion envenenado".to_string())?;
        guard.take().ok_or("no hay ninguna grabacion en curso")?
    };

    let RecordingSession { capture_handle, input_handle, raw_take_path, input_log_path } = session;

    let raw_take_info = capture_handle.stop().map_err(|e| e.to_string())?;
    input_handle.stop().map_err(|e| e.to_string())?;

    let duration_ms =
        (raw_take_info.recording_stopped_wall_ms.saturating_sub(raw_take_info.recording_started_wall_ms)) as u64;

    let events = parse_input_log(
        &input_log_path,
        raw_take_info.recording_started_wall_ms,
        raw_take_info.width,
        raw_take_info.height,
    )?;
    let zoom_keyframes = zoom_engine::generate_keyframes(&events, duration_ms);
    let cursor_path = build_cursor_path(&events);

    let mut project = Project::new(
        RawTake {
            path: raw_take_path.clone(),
            fps: raw_take_info.fps,
            resolution: Resolution { width: raw_take_info.width, height: raw_take_info.height },
            duration_ms,
        },
        InputLog { path: input_log_path },
    );
    project.zoom_keyframes = zoom_keyframes;
    project.cursor_path = cursor_path;

    let project_path = raw_take_path.with_extension("szproj");
    project.save(&project_path).map_err(|e| e.to_string())?;

    Ok(project_path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_move_and_click_lines_into_relative_normalized_events() {
        let dir = std::env::temp_dir().join(format!("screenzoom-parse-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.jsonl");
        std::fs::write(
            &path,
            "{\"t_wall_ms\":1000,\"kind\":\"move\",\"x\":960,\"y\":540}\n\
             {\"t_wall_ms\":1500,\"kind\":\"button_press\",\"button\":\"Left\",\"x\":960,\"y\":540}\n\
             {\"t_wall_ms\":1600,\"kind\":\"unknown_kind\",\"x\":0,\"y\":0}\n",
        )
        .unwrap();

        let events = parse_input_log(&path, 500, 1920, 1080).unwrap();

        assert_eq!(events.len(), 2, "la linea de kind desconocido deberia ignorarse");
        assert_eq!(events[0].t_ms, 500);
        assert!((events[0].x - 0.5).abs() < 1e-5);
        assert!((events[0].y - 0.5).abs() < 1e-5);
        assert_eq!(events[0].kind, InputEventKind::Move);

        assert_eq!(events[1].t_ms, 1000);
        assert_eq!(events[1].kind, InputEventKind::ButtonPress);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clamps_out_of_bounds_coordinates() {
        let dir = std::env::temp_dir().join(format!("screenzoom-parse-clamp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.jsonl");
        // x/y fuera de rango (ej. cursor movido a un segundo monitor mas ancho
        // que el grabado) no deberian romper el normalizado 0..1.
        std::fs::write(&path, "{\"t_wall_ms\":0,\"kind\":\"move\",\"x\":5000,\"y\":-100}\n").unwrap();

        let events = parse_input_log(&path, 0, 1920, 1080).unwrap();

        assert_eq!(events[0].x, 1.0);
        assert_eq!(events[0].y, 0.0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_cursor_path_downsamples_moves_but_keeps_every_click() {
        let events = vec![
            InputEvent { t_ms: 0, x: 0.0, y: 0.0, kind: InputEventKind::Move },
            InputEvent { t_ms: 10, x: 0.1, y: 0.1, kind: InputEventKind::Move }, // descartado: <33ms del anterior
            InputEvent { t_ms: 20, x: 0.2, y: 0.2, kind: InputEventKind::ButtonPress }, // click: siempre se guarda
            InputEvent { t_ms: 25, x: 0.25, y: 0.25, kind: InputEventKind::ButtonRelease }, // click: siempre se guarda
            InputEvent { t_ms: 50, x: 0.5, y: 0.5, kind: InputEventKind::Move }, // >=33ms del ultimo guardado (25)
        ];

        let path = build_cursor_path(&events);

        assert_eq!(path.len(), 4, "deberia descartar solo el move de t=10");
        assert_eq!(path[0], CursorPathPoint { t_ms: 0, x: 0.0, y: 0.0, pressed: false });
        assert_eq!(path[1], CursorPathPoint { t_ms: 20, x: 0.2, y: 0.2, pressed: true });
        assert_eq!(path[2], CursorPathPoint { t_ms: 25, x: 0.25, y: 0.25, pressed: false });
        assert_eq!(path[3], CursorPathPoint { t_ms: 50, x: 0.5, y: 0.5, pressed: false });
    }

    #[test]
    fn build_cursor_path_handles_no_events() {
        assert!(build_cursor_path(&[]).is_empty());
    }
}
