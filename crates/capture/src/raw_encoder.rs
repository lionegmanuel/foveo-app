//! Encoder de la toma cruda via un proceso hijo de `ffmpeg`, en vez del
//! `VideoEncoder` interno de `windows-capture`.
//!
//! Por que: el `VideoEncoder` de `windows-capture` solo deja el `moov` (el
//! indice que hace que un mp4 sea reproducible) escrito al final, en
//! `finish()`. Si la app crashea a mitad de una grabacion, el archivo queda
//! con frames validos pero sin indice — no reproducible (viola CLAUDE.md
//! regla 7: "el archivo parcial ya escrito debe seguir siendo un video valido
//! y reproducible hasta el ultimo frame flusheado"). Pedirle a ffmpeg un mp4
//! **fragmentado** (`frag_keyframe+empty_moov`) resuelve esto: cada fragmento
//! (aprox. cada keyframe, ver `-g` mas abajo) se cierra y queda reproducible
//! por si mismo apenas ffmpeg lo flushea, sin esperar al cierre del archivo.
//!
//! Bitrate alto (`-b:v`) a proposito: esta es la toma cruda/intermedia (ver
//! ARQUITECTURA.md 3.1, diagrama: "Encoder intermedio, hw encoder, bitrate
//! alto"), no la entrega final — prioriza fidelidad para poder re-exportar
//! despues, no tamanio de archivo.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::CaptureError;

/// Bitrate de la toma cruda. Alto a proposito (ver doc del modulo) — el
/// tamanio en disco de la toma cruda no es una preocupacion de UX (nunca se
/// le muestra al usuario final, solo alimenta el export).
const RAW_TAKE_BITRATE: &str = "50M";

fn find_ffmpeg_binary() -> String {
    std::env::var("FFMPEG_BINARY_PATH").unwrap_or_else(|_| "ffmpeg".to_string())
}

/// Recibe frames BGRA8 crudos (el mismo formato que entrega
/// `windows_capture::frame::FrameBuffer::as_nopadding_buffer`) y los encodea
/// a un mp4 fragmentado, resiliente a crashes.
pub struct RawFragmentedEncoder {
    child: Child,
    stdin: ChildStdin,
}

impl RawFragmentedEncoder {
    pub fn spawn(output_path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, CaptureError> {
        let mut child = Command::new(find_ffmpeg_binary())
            .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "bgra"])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &fps.to_string()])
            .args(["-i", "-"])
            .args(["-c:v", "libopenh264", "-pix_fmt", "yuv420p", "-b:v", RAW_TAKE_BITRATE])
            // Un keyframe por segundo: acota cuanto se puede perder si la app
            // crashea entre dos fragmentos (ver doc del modulo).
            .args(["-g", &fps.to_string()])
            .args(["-movflags", "frag_keyframe+empty_moov+default_base_moof"])
            .arg(output_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| CaptureError::StartFailed(format!("no se pudo arrancar ffmpeg para la toma cruda: {e}")))?;

        let stdin = child.stdin.take().expect("stdin fue pedido como piped");

        Ok(Self { child, stdin })
    }

    /// Escribe un frame BGRA8 (`width*height*4` bytes). Bloqueante: si el
    /// pipe hacia ffmpeg se llena (encoder no da abasto), este llamado —
    /// hecho desde `on_frame_arrived` del capturador — se frena hasta que
    /// haya lugar, aplicando backpressure natural en vez de acumular frames
    /// sin encodear en RAM.
    pub fn write_frame(&mut self, bgra: &[u8]) -> Result<(), CaptureError> {
        self.stdin.write_all(bgra).map_err(|e| CaptureError::StartFailed(format!("error escribiendo frame: {e}")))
    }

    /// Cierra el pipe (EOF para ffmpeg) y espera a que termine de escribir el
    /// ultimo fragmento. Sigue al patron ya usado por
    /// `exporter::FfmpegExporter::finish`.
    pub fn finish(self) -> Result<(), CaptureError> {
        let Self { mut child, stdin } = self;
        drop(stdin);

        let status = child
            .wait()
            .map_err(|e| CaptureError::StopFailed(format!("error esperando a que ffmpeg termine: {e}")))?;
        if !status.success() {
            let mut message = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut message);
            }
            return Err(CaptureError::StopFailed(format!("ffmpeg termino con error: {message}")));
        }

        Ok(())
    }
}
