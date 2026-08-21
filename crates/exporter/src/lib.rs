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

use compositor::{Compositor, RawFrameReader, apply_style, camera_rect_at};
use project::{ExportResolution, Project};

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

    let (out_width, out_height) = resolution_pixels(project.export_settings.resolution);
    let in_width = project.raw_take.resolution.width;
    let in_height = project.raw_take.resolution.height;
    let in_fps = project.raw_take.fps;

    let mut reader = RawFrameReader::spawn(&project.raw_take.path, in_width, in_height)?;
    let mut compositor = Compositor::new(in_width, in_height, out_width, out_height)?;
    let mut exporter = FfmpegExporter::spawn(output_path, out_width, out_height, project.export_settings.fps)?;

    let mut frame_index: u64 = 0;
    while let Some(frame) = reader.next_frame()? {
        let t_ms = frame_index * 1000 / u64::from(in_fps.max(1));
        let crop_rect = camera_rect_at(&keyframes, t_ms);

        let composed = compositor.composite_frame(&frame, crop_rect)?;
        let styled = apply_style(&composed, out_width, out_height, &project.style);
        exporter.write_frame(&styled.bgra)?;

        frame_index += 1;
        on_progress(frame_index);
    }

    exporter.finish()
}
