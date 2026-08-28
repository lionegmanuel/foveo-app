//! Orquestacion del sidecar `ffmpeg`: recibe frames ya compuestos (de
//! `compositor::Compositor`) y los pipea para encoding final + mux a MP4
//! (ver ARQUITECTURA.md 3.3). Tambien expone `render_and_export`, que ata
//! todo el pipeline Fase 1 (decode -> interpolar camara -> componer ->
//! encodear) para un `project::Project` completo.
//!
//! Este crate no importa `tauri`.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

use compositor::{Compositor, CursorMarker, CursorPath, RawFrameReader, apply_style_ex, camera_rect_at, content_uv_to_canvas_px};
use project::{ExportResolution, Project, Rect};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("no se pudo arrancar ffmpeg para exportar: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("ffmpeg termino con error: {0}")]
    FfmpegFailed(String),
    #[error("error decodificando la toma cruda: {0}")]
    Decode(#[from] compositor::DecodeError),
    #[error("error en el compositor: {0}")]
    Compositor(#[from] compositor::CompositorError),
}

/// Resuelve el binario de ffmpeg a usar (mismo criterio que
/// `compositor::decode`: `FFMPEG_BINARY_PATH` si esta seteada, si no
/// cualquier `ffmpeg` en PATH — la resolucion del sidecar empaquetado en
/// produccion es responsabilidad de `apps/desktop/src-tauri`).
fn find_ffmpeg_binary() -> String {
    std::env::var("FFMPEG_BINARY_PATH").unwrap_or_else(|_| "ffmpeg".to_string())
}

/// Dimensiones en pixeles (16:9) de cada resolucion soportada de export (ver
/// ARQUITECTURA.md 4.1: "Exportar a MP4 (H.264) en 1080p, 1440p o 4K").
#[must_use]
pub fn resolution_pixels(resolution: ExportResolution) -> (u32, u32) {
    match resolution {
        ExportResolution::P1080 => (1920, 1080),
        ExportResolution::P1440 => (2560, 1440),
        ExportResolution::P4k => (3840, 2160),
    }
}

/// Encoder/mux de frames BGRA8 crudos a MP4 via un proceso hijo de ffmpeg.
///
/// Usa `libopenh264` (LGPL, no `libx264`/GPL) como codec por defecto — ver
/// ARQUITECTURA.md 3.6, riesgo de licencia si se linkea `libx264` directo en
/// el sidecar bundleado. Seleccion de encoder de hardware (NVENC/QSV/AMF)
/// segun `hw_accel: "auto"` queda para cuando haga falta, no es parte de la
/// pipeline minima de Fase 1.
pub struct FfmpegExporter {
    child: Child,
    stdin: ChildStdin,
}

impl FfmpegExporter {
    pub fn spawn(output_path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, ExportError> {
        let mut child = Command::new(find_ffmpeg_binary())
            .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "bgra"])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(["-c:v", "libopenh264", "-pix_fmt", "yuv420p"])
            .arg(output_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin fue pedido como piped");

        Ok(Self { child, stdin })
    }

    /// Envia un frame BGRA8 compuesto (`Compositor::composite_frame`) a
    /// ffmpeg para encodear. El tamanio debe coincidir con `width*height*4`
    /// pasado a `spawn`; ffmpeg va a fallar en `finish()` si no coincide.
    pub fn write_frame(&mut self, bgra: &[u8]) -> Result<(), ExportError> {
        self.stdin.write_all(bgra).map_err(ExportError::Spawn)
    }

    /// Cierra el pipe de entrada (asi ffmpeg ve EOF) y espera a que termine
    /// de muxear el archivo final.
    pub fn finish(self) -> Result<(), ExportError> {
        let Self { mut child, stdin } = self;
        drop(stdin);

        let status = child.wait()?;
        if !status.success() {
            let mut message = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut message);
            }
            return Err(ExportError::FfmpegFailed(message));
        }

        Ok(())
    }
}

/// Pipeline completa de Fase 1: decodifica `project.raw_take`, interpola la
/// camara con `project.zoom_keyframes` frame a frame, compone en GPU, y
/// encodea el resultado a `output_path` segun `project.export_settings`.
///
/// `on_progress` se llama despues de cada frame exportado con el numero de
/// frame procesado hasta el momento, para que quien llama (un comando Tauri)
/// pueda emitir eventos de progreso sin que este crate conozca Tauri.
pub fn render_and_export(
    project: &Project,
    output_path: &Path,
    mut on_progress: impl FnMut(u64),
) -> Result<(), ExportError> {
    let mut keyframes = project.zoom_keyframes.clone();
    keyframes.sort_by_key(|k| k.start_ms);
    let cursor_path = CursorPath::build(&project.cursor_path);

    let (out_width, out_height) = resolution_pixels(project.export_settings.resolution);
    let in_width = project.raw_take.resolution.width;
    let in_height = project.raw_take.resolution.height;
    let in_fps = project.raw_take.fps;

    let mut reader = RawFrameReader::spawn(&project.raw_take.path, in_width, in_height)?;
    let mut compositor = Compositor::new(in_width, in_height, out_width, out_height)?;
    let mut exporter = FfmpegExporter::spawn(output_path, out_width, out_height, project.export_settings.fps)?;

    let mut frame_index: u64 = 0;
    let mut prev_crop_rect = Rect::FULL_FRAME;
    while let Some(frame) = reader.next_frame()? {
        let t_ms = frame_index * 1000 / u64::from(in_fps.max(1));
        let crop_rect = camera_rect_at(&keyframes, t_ms);

        let motion_blur_strength = if project.style.motion_blur {
            motion_blur_strength_from_velocity(prev_crop_rect, crop_rect)
        } else {
            0.0
        };
        let cursor = if project.style.cursor_smoothing {
            cursor_marker_at(&cursor_path, t_ms, crop_rect, out_width, out_height, &project.style)
        } else {
            None
        };
        prev_crop_rect = crop_rect;

        let composed = compositor.composite_frame(&frame, crop_rect)?;
        let styled = apply_style_ex(&composed, out_width, out_height, &project.style, motion_blur_strength, cursor);
        exporter.write_frame(&styled.bgra)?;

        frame_index += 1;
        on_progress(frame_index);
    }

    exporter.finish()
}

/// Cuanto pesa el blur radial en este frame, a partir de cuanto cambio de
/// tamanio el rect de camara desde el frame anterior (proxy de "velocidad de
/// zoom" — un pan puro, sin cambio de escala, no genera blur). Ajustable:
/// `VELOCITY_TO_STRENGTH` mas alto = blur mas intenso durante las
/// transiciones; se eligio para que una transicion default del zoom-engine
/// (0.58 de cambio de ancho en 600ms a 60fps) llegue a media intensidad.
const VELOCITY_TO_STRENGTH: f32 = 20.0;

fn motion_blur_strength_from_velocity(prev: Rect, current: Rect) -> f32 {
    let delta_w = (current.w - prev.w).abs();
    (delta_w * VELOCITY_TO_STRENGTH).clamp(0.0, 1.0)
}

/// Posicion del cursor reconstruido en coordenadas de pixel del canvas de
/// salida para este frame, o `None` si no hay dato de cursor o si el mouse
/// esta fuera del `crop_rect` vigente (zoomeado fuera de vista). Publica
/// porque tambien la usa el comando de preview (`apps/desktop/src-tauri`)
/// para mostrar el cursor en el editor sin duplicar esta logica.
pub fn cursor_marker_at(
    cursor_path: &CursorPath,
    t_ms: u64,
    crop_rect: Rect,
    out_width: u32,
    out_height: u32,
    style: &project::Style,
) -> Option<CursorMarker> {
    let at = cursor_path.position_at(t_ms)?;
    if crop_rect.w.abs() < 1e-6 || crop_rect.h.abs() < 1e-6 {
        return None;
    }

    let u = (at.x - crop_rect.x) / crop_rect.w;
    let v = (at.y - crop_rect.y) / crop_rect.h;
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }

    let (x_px, y_px) = content_uv_to_canvas_px(u, v, out_width, out_height, style);
    Some(CursorMarker { x_px, y_px, pressed: at.pressed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::CursorPathPoint;

    #[test]
    fn motion_blur_strength_is_zero_when_the_camera_is_static() {
        let rect = Rect { x: 0.2, y: 0.2, w: 0.4, h: 0.4 };
        assert_eq!(motion_blur_strength_from_velocity(rect, rect), 0.0);
    }

    #[test]
    fn motion_blur_strength_grows_with_zoom_speed_and_saturates_at_one() {
        let slow = motion_blur_strength_from_velocity(
            Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            Rect { x: 0.0, y: 0.0, w: 0.99, h: 0.99 },
        );
        let fast = motion_blur_strength_from_velocity(
            Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            Rect { x: 0.0, y: 0.0, w: 0.1, h: 0.1 },
        );
        assert!(slow > 0.0 && slow < fast);
        assert_eq!(fast, 1.0, "un cambio de ancho grande debe saturar en 1.0, no desbordar");
    }

    #[test]
    fn cursor_marker_is_none_without_cursor_data() {
        let empty_path = CursorPath::build(&[]);
        let full_frame = Rect::FULL_FRAME;
        let style = project::Style::default();
        assert!(cursor_marker_at(&empty_path, 0, full_frame, 1920, 1080, &style).is_none());
    }

    #[test]
    fn cursor_marker_is_none_when_the_mouse_is_outside_the_current_crop_rect() {
        let path = CursorPath::build(&[CursorPathPoint { t_ms: 0, x: 0.9, y: 0.9, pressed: false }]);
        // El crop actual solo cubre el cuadrante superior izquierdo: el mouse
        // en (0.9, 0.9) queda fuera de vista.
        let crop = Rect { x: 0.0, y: 0.0, w: 0.5, h: 0.5 };
        let style = project::Style::default();
        assert!(cursor_marker_at(&path, 0, crop, 1920, 1080, &style).is_none());
    }

    #[test]
    fn cursor_marker_maps_a_centered_mouse_in_a_full_frame_crop_to_the_canvas_center() {
        let path = CursorPath::build(&[CursorPathPoint { t_ms: 0, x: 0.5, y: 0.5, pressed: true }]);
        let style = project::Style { padding: 0.0, ..project::Style::default() };
        let marker = cursor_marker_at(&path, 0, Rect::FULL_FRAME, 200, 100, &style).unwrap();
        assert!((marker.x_px - 100.0).abs() < 1.0);
        assert!((marker.y_px - 50.0).abs() < 1.0);
        assert!(marker.pressed);
    }
}
