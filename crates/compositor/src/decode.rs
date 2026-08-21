//! Decodifica la toma cruda (mp4) a frames BGRA8 crudos via un proceso hijo
//! de `ffmpeg` (ver ARQUITECTURA.md 3.3: "Compositor... Decodifica la toma
//! cruda"). No conoce Tauri: la resolucion del binario real de ffmpeg
//! empaquetado como sidecar (vs. un `ffmpeg` de PATH en desarrollo) es
//! responsabilidad de `apps/desktop/src-tauri` via `FFMPEG_BINARY_PATH`
//! (ver CLAUDE.md regla 5); aca solo se respeta esa env var si esta seteada.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("no se pudo arrancar ffmpeg para decodificar: {0}")]
    SpawnFailed(#[from] std::io::Error),
}

/// Resuelve el binario de ffmpeg a usar. En desarrollo, cualquier `ffmpeg` en
/// PATH sirve; `FFMPEG_BINARY_PATH` permite forzar uno especifico (ver
/// `.env.example`).
pub(crate) fn find_ffmpeg_binary() -> String {
    std::env::var("FFMPEG_BINARY_PATH").unwrap_or_else(|_| "ffmpeg".to_string())
}

/// Formatea `ms` como segundos con precision de milisegundos para el flag
/// `-ss` de ffmpeg (ej. `1500` -> `"1.500"`).
fn seek_arg(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Lee frames BGRA8 crudos de una toma grabada, uno a la vez, pipeando desde
/// un proceso `ffmpeg -f rawvideo -pix_fmt bgra`.
pub struct RawFrameReader {
    child: Child,
    stdout: ChildStdout,
    frame_size: usize,
}

impl RawFrameReader {
    /// `width`/`height` deben coincidir con la resolucion real de la toma
    /// cruda (`project::RawTake::resolution`) — si no coinciden, los frames
    /// leidos van a quedar corridos/mezclados sin que ffmpeg lo detecte.
    pub fn spawn(input_path: &Path, width: u32, height: u32) -> Result<Self, DecodeError> {
        let mut child = Command::new(find_ffmpeg_binary())
            .args(["-v", "error", "-i"])
            .arg(input_path)
            .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take().expect("stdout fue pedido como piped");
        let frame_size = (width as usize) * (height as usize) * 4;

        Ok(Self { child, stdout, frame_size })
    }

    /// Como `spawn`, pero arranca la decodificacion desde `seek_ms` en vez del
    /// principio del archivo — usa el seek rapido de ffmpeg (`-ss` antes de
    /// `-i`, por keyframes, no frame-exacto) para no tener que leer toda la
    /// toma cruda solo para renderizar un frame de preview cerca del final.
    pub fn spawn_seeked(input_path: &Path, width: u32, height: u32, seek_ms: u64) -> Result<Self, DecodeError> {
        let mut child = Command::new(find_ffmpeg_binary())
            .args(["-v", "error", "-ss", &seek_arg(seek_ms), "-i"])
            .arg(input_path)
            .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take().expect("stdout fue pedido como piped");
        let frame_size = (width as usize) * (height as usize) * 4;

        Ok(Self { child, stdout, frame_size })
    }

    /// Devuelve el proximo frame decodificado, o `None` cuando la toma se
    /// termino. Un frame final truncado (menos bytes de los esperados) se
    /// trata como fin de stream, no como error — evita fallar el export
    /// entero por el ultimo frame parcial de una grabacion.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, DecodeError> {
        let mut buf = vec![0u8; self.frame_size];
        let mut read_total = 0usize;

        while read_total < self.frame_size {
            let n = self.stdout.read(&mut buf[read_total..])?;
            if n == 0 {
                return Ok(None); // EOF, sin frame parcial utilizable
            }
            read_total += n;
        }

        Ok(Some(buf))
    }
}

impl Drop for RawFrameReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::seek_arg;

    #[test]
    fn seek_arg_formats_milliseconds_as_seconds_with_millisecond_precision() {
        assert_eq!(seek_arg(0), "0.000");
        assert_eq!(seek_arg(1_500), "1.500");
        assert_eq!(seek_arg(12_345), "12.345");
    }
}
