//! Trait de captura de pantalla + implementacion Windows (`windows-capture`).
//!
//! `ScreenCapturer`/`RecordingHandle` son la interfaz que el resto del sistema
//! usa para grabar; la implementacion concreta de Windows queda detras de
//! ella para que una futura implementacion de macOS (`screencapturekit-rs`,
//! Fase 4) se pueda sumar sin tocar nada fuera de este crate (ver
//! ARQUITECTURA.md 2.2 y CLAUDE.md regla 2).
//!
//! Este crate no importa `tauri`: los comandos de `apps/desktop/src-tauri`
//! son la unica capa que conoce Tauri y delega aca.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder, VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no se encontro el monitor primario: {0}")]
    NoPrimaryMonitor(String),
    #[error("no se pudo leer la geometria del monitor: {0}")]
    MonitorInfo(String),
    #[error("no se pudo iniciar la captura de pantalla: {0}")]
    StartFailed(String),
    #[error("no se pudo detener la captura de pantalla: {0}")]
    StopFailed(String),
    #[error("la sesion de captura termino sin recibir ni un frame")]
    NoFramesCaptured,
}

/// Metadata de la toma cruda resultante de una grabacion, necesaria para
/// armar el `.szproj` (ver `project::RawTake`) y para correlacionar el input
/// log contra el video (ambos comparten reloj de pared, no el reloj interno
/// del encoder).
#[derive(Debug, Clone, Copy)]
pub struct RawTakeInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub recording_started_wall_ms: u128,
    pub recording_stopped_wall_ms: u128,
}

/// Metadata minima de un monitor para el selector de fuente de la UI (ver
/// ARQUITECTURA.md 4.1: "Grabar pantalla completa o un monitor especifico").
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// Indice 1-based estable (mismo que espera `start_recording`), no un
    /// handle de OS — sirve para mostrarlo en un `<select>` y volver a
    /// pasarlo tal cual.
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// Abstraccion de plataforma para grabar pantalla completa a un archivo.
pub trait ScreenCapturer {
    type Handle: RecordingHandle;

    /// Lista los monitores disponibles, para el selector de fuente de la UI.
    fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError>;

    /// Arranca a grabar en `output_path` y devuelve de inmediato un handle
    /// para pararla despues; nunca bloquea el hilo que llama (ver CLAUDE.md
    /// regla 1: nada de video pesado en el hilo de UI). `monitor_index` usa
    /// los mismos indices que devuelve `list_monitors`; `None` graba el
    /// monitor primario.
    fn start_recording(&self, output_path: &Path, monitor_index: Option<usize>) -> Result<Self::Handle, CaptureError>;
}

/// Handle de una grabacion en curso. Consumirlo con `stop()` bloquea hasta
/// que el archivo de video quede completamente escrito y cerrado (fsync
/// implicito de `VideoEncoder::finish`), asi que quien lo llama debe hacerlo
/// desde un thread dedicado, no desde el hilo de eventos de Tauri/UI.
pub trait RecordingHandle {
    fn stop(self) -> Result<RawTakeInfo, CaptureError>;
}

/// Implementacion Windows via Windows.Graphics.Capture (crate
/// `windows-capture`). Selecciona monitor por indice (`list_monitors`/
/// `start_recording`), sin picker/dialogo interactivo. El picker de fuente
/// nativo (con manejo explicito de cancelacion — ver ARQUITECTURA.md seccion
/// 4) queda como posible mejora de UX, no bloquea la seleccion de monitor.
pub struct WindowsScreenCapturer;

impl ScreenCapturer for WindowsScreenCapturer {
    type Handle = WindowsRecordingHandle;

    fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError> {
        let monitors = Monitor::enumerate().map_err(|e| CaptureError::MonitorInfo(e.to_string()))?;
        monitors
            .into_iter()
            .enumerate()
            .map(|(i, m)| {
                Ok(MonitorInfo {
                    index: i + 1,
                    name: m.name().unwrap_or_else(|_| format!("Monitor {}", i + 1)),
                    width: m.width().map_err(|e| CaptureError::MonitorInfo(e.to_string()))?,
                    height: m.height().map_err(|e| CaptureError::MonitorInfo(e.to_string()))?,
                })
            })
            .collect()
    }

    fn start_recording(
        &self,
        output_path: &Path,
        monitor_index: Option<usize>,
    ) -> Result<Self::Handle, CaptureError> {
        let monitor = match monitor_index {
            Some(index) => Monitor::from_index(index).map_err(|e| CaptureError::MonitorInfo(e.to_string()))?,
            None => Monitor::primary().map_err(|e| CaptureError::NoPrimaryMonitor(e.to_string()))?,
        };
        let width = monitor.width().map_err(|e| CaptureError::MonitorInfo(e.to_string()))?;
        let height = monitor.height().map_err(|e| CaptureError::MonitorInfo(e.to_string()))?;
        let fps = monitor.refresh_rate().map_err(|e| CaptureError::MonitorInfo(e.to_string()))?;

        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::Default,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            (width, height, fps, output_path.to_path_buf()),
        );

        let control =
            CaptureHandler::start_free_threaded(settings).map_err(|e| CaptureError::StartFailed(e.to_string()))?;

        Ok(WindowsRecordingHandle { control, width, height, fps })
    }
}

pub struct WindowsRecordingHandle {
    control: CaptureControl<CaptureHandler, Box<dyn std::error::Error + Send + Sync>>,
    width: u32,
    height: u32,
    fps: u32,
}

impl RecordingHandle for WindowsRecordingHandle {
    fn stop(self) -> Result<RawTakeInfo, CaptureError> {
        // `callback()` clona el Arc<Mutex<CaptureHandler>> ANTES de consumir
        // `control` en `.stop()`, para poder leer los timestamps finales que
        // `on_closed` escribe cuando la sesion termina.
        let callback = self.control.callback();
        self.control.stop().map_err(|e| CaptureError::StopFailed(e.to_string()))?;

        // `callback` es un Arc<parking_lot::Mutex<_>> (traido por windows-capture),
        // no std::sync::Mutex: no hay poisoning, `.lock()` no devuelve Result.
        let handler = callback.lock();
        let started = handler.first_frame_wall_ms.ok_or(CaptureError::NoFramesCaptured)?;
        let stopped = handler.stopped_wall_ms.unwrap_or(started);

        Ok(RawTakeInfo {
            width: self.width,
            height: self.height,
            fps: self.fps,
            recording_started_wall_ms: started,
            recording_stopped_wall_ms: stopped,
        })
    }
}

fn now_wall_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("reloj del sistema antes de epoch").as_millis()
}

/// Implementacion interna de `GraphicsCaptureApiHandler`. `finish()` del
/// encoder se llama en `on_closed`, no en `on_frame_arrived`: asi el archivo
/// queda cerrado correctamente sin importar si la sesion termino porque el
/// handle externo pidio `stop()` o porque la fuente de captura se cerro sola
/// (ver ARQUITECTURA.md seccion 4, manejo de cancelacion/cierre).
struct CaptureHandler {
    encoder: Option<VideoEncoder>,
    first_frame_wall_ms: Option<u128>,
    stopped_wall_ms: Option<u128>,
    _start: Option<Instant>,
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = (u32, u32, u32, PathBuf);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (width, height, fps, output_path) = ctx.flags;

        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(width, height).frame_rate(fps).sub_type(VideoSettingsSubType::H264),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &output_path,
        )?;

        Ok(Self { encoder: Some(encoder), first_frame_wall_ms: None, stopped_wall_ms: None, _start: None })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.first_frame_wall_ms.is_none() {
            self.first_frame_wall_ms = Some(now_wall_ms());
            self._start = Some(Instant::now());
        }

        if let Some(encoder) = self.encoder.as_mut() {
            encoder.send_frame(frame)?;
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        if let Some(encoder) = self.encoder.take() {
            encoder.finish()?;
        }
        self.stopped_wall_ms = Some(now_wall_ms());
        Ok(())
    }
}
