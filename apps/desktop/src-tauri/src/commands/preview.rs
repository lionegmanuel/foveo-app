//! Comando Tauri de preview: renderiza un solo frame (decode -> camara ->
//! composite -> style) a baja resolucion para el editor de timeline (Fase
//! 2). Ver CLAUDE.md regla 1 (nunca bloquear el hilo de eventos) — el
//! comando es sync (no `async fn`), lo que ya lo corre en el threadpool
//! bloqueante de Tauri (ver el comentario sobre la funcion).

use std::io::Cursor;

use base64::Engine;
use compositor::{Compositor, RawFrameReader, apply_style, camera_rect_at};
use project::Project;

/// `#[tauri::command]` sobre una funcion sync (no `async fn`) ya corre en el
/// threadpool bloqueante de Tauri, nunca en el hilo de eventos (ver
/// `tauri-macros` `ExecutionContext::Async` + `sync_threadpool` — CLAUDE.md
/// regla 1 queda cubierta sin necesitar `spawn_blocking` manual aca).
#[tauri::command]
pub fn render_preview_frame(project_path: String, t_ms: u64, max_width: u32) -> Result<String, String> {
    let project = Project::load(project_path).map_err(|e| e.to_string())?;

    let mut keyframes = project.zoom_keyframes.clone();
    keyframes.sort_by_key(|k| k.start_ms);

    let in_width = project.raw_take.resolution.width;
    let in_height = project.raw_take.resolution.height;
    let aspect = in_height as f32 / in_width as f32;
    let out_width = max_width.max(2);
    let out_height = ((out_width as f32) * aspect).round().max(2.0) as u32;

    let mut reader = RawFrameReader::spawn_seeked(&project.raw_take.path, in_width, in_height, t_ms)
        .map_err(|e| e.to_string())?;
    let frame = reader
        .next_frame()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no se pudo leer ningun frame en ese timestamp".to_string())?;

    let mut compositor = Compositor::new(in_width, in_height, out_width, out_height).map_err(|e| e.to_string())?;
    let crop_rect = camera_rect_at(&keyframes, t_ms);
    let composed = compositor.composite_frame(&frame, crop_rect).map_err(|e| e.to_string())?;
    let styled = apply_style(&composed, out_width, out_height, &project.style);

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut png_bytes), out_width, out_height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;

        // png espera RGBA; el pipeline entero es BGRA — convertir en el
        // borde de salida, no en la pipeline compartida (ver
        // crates/compositor/src/render.rs: BGRA se eligio para no convertir
        // entre windows-capture/ffmpeg, esto es solo para mostrar en UI).
        let mut rgba = styled.bgra;
        for px in rgba.chunks_mut(4) {
            px.swap(0, 2);
        }
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }

    Ok(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
}
