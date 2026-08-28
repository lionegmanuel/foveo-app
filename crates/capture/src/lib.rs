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

pub mod raw_encoder;

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, GraphicsCaptureItemType,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use raw_encoder::RawFragmentedEncoder;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no se encontro el monitor primario: {0}")]
    NoPrimaryMonitor(String),
    #[error("no se pudo leer la geometria del monitor: {0}")]
    MonitorInfo(String),
    #[error("no se pudo leer/encontrar la ventana: {0}")]
    WindowInfo(String),
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

/// Metadata minima de una ventana capturable, para el selector de fuente de
/// la UI (Fase 3, ver ARQUITECTURA.md seccion 6: "Seleccion de ventana
/// especifica como fuente de captura").
#[derive(Debug, Clone)]
pub struct WindowInfo {
    /// Indice 1-based estable **dentro de una misma llamada a
    /// `list_windows`** — el set de ventanas abiertas puede cambiar entre
    /// llamadas (se cierran/abren ventanas). Si el usuario tarda en arrancar
    /// a grabar y la ventana elegida ya cerro, `start_recording` devuelve
    /// `WindowInfo` (indice no encontrado o corrido), no un crash.
    pub index: usize,
    pub title: String,
}

/// De donde capturar: pantalla completa (un monitor) o una ventana especifica.
/// `monitor_index`/`window_index` usan los mismos indices que devuelven
/// `list_monitors`/`list_windows`.
#[derive(Debug, Clone, Copy)]
pub enum CaptureSource {
    PrimaryMonitor,
    Monitor(usize),
    Window(usize),
}

/// Abstraccion de plataforma para grabar pantalla completa a un archivo.
pub trait ScreenCapturer {
    type Handle: RecordingHandle;

    /// Lista los monitores disponibles, para el selector de fuente de la UI.
    fn list_monitors(&self) -> Result<Vec<MonitorInfo>, CaptureError>;

    /// Lista las ventanas capturables en este momento, para el selector de
    /// fuente de la UI.
    fn list_windows(&self) -> Result<Vec<WindowInfo>, CaptureError>;

    /// Arranca a grabar en `output_path` segun `source`, y devuelve de
    /// inmediato un handle para pararla despues; nunca bloquea el hilo que
    /// llama (ver CLAUDE.md regla 1: nada de video pesado en el hilo de UI).
    fn start_recording(&self, output_path: &Path, source: CaptureSource) -> Result<Self::Handle, CaptureError>;
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

    fn list_windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        let windows = Window::enumerate().map_err(|e| CaptureError::WindowInfo(e.to_string()))?;
        Ok(windows
            .into_iter()
            .enumerate()
            .map(|(i, w)| WindowInfo {
                index: i + 1,
                title: w.title().unwrap_or_else(|_| format!("Ventana {}", i + 1)),
            })
            .collect())
    }

    fn start_recording(&self, output_path: &Path, source: CaptureSource) -> Result<Self::Handle, CaptureError> {
        match source {
            CaptureSource::PrimaryMonitor => {
                let monitor = Monitor::primary().map_err(|e| CaptureError::NoPrimaryMonitor(e.to_string()))?;
                start_recording_item(monitor, output_path)
            }
            CaptureSource::Monitor(index) => {
                let monitor = Monitor::from_index(index).map_err(|e| CaptureError::MonitorInfo(e.to_string()))?;
                start_recording_item(monitor, output_path)
            }
            CaptureSource::Window(index) => {
                if index < 1 {
                    return Err(CaptureError::WindowInfo("el indice de ventana empieza en 1".to_string()));
                }
                let window = Window::enumerate()
                    .map_err(|e| CaptureError::WindowInfo(e.to_string()))?
                    .into_iter()
                    .nth(index - 1)
                    .ok_or_else(|| CaptureError::WindowInfo(format!("no se encontro la ventana con indice {index}")))?;
                start_recording_item(window, output_path)
            }
        }
    }
}

/// Comun a `Monitor` y `Window`: arma los `Settings` y arranca el handler.
/// Generico sobre `T` porque `windows_capture::settings::Settings<Flags, T>`
/// (y por lo tanto `GraphicsCaptureApiHandler::start_free_threaded`) lo son —
/// `T: TryInto<GraphicsCaptureItemType>` es el unico bound real que pide la
/// libreria (verificado leyendo `windows-capture-2.0.1/src/capture.rs`: la
/// conversion de `T::Error` se descarta con `map_err(|_| ...)`, no exige
/// `Send`/`std::error::Error` en el error).
fn start_recording_item<T>(item: T, output_path: &Path) -> Result<WindowsRecordingHandle, CaptureError>
where
    T: TryInto<GraphicsCaptureItemType> + Send + 'static,
    T: CaptureItemDimensions,
{
    let width = item.capture_width()?;
    let height = item.capture_height()?;
    let fps = item.capture_fps()?;

    let settings = Settings::new(
        item,
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

/// Dimensiones/fps de una fuente de captura, homogeneizando `Monitor`
/// (`width()`/`height()`/`refresh_rate()` en `u32`) y `Window` (`width()`/
/// `height()` en `i32`, sin concepto propio de refresh rate).
trait CaptureItemDimensions {
    fn capture_width(&self) -> Result<u32, CaptureError>;
    fn capture_height(&self) -> Result<u32, CaptureError>;
    fn capture_fps(&self) -> Result<u32, CaptureError>;
}

impl CaptureItemDimensions for Monitor {
    fn capture_width(&self) -> Result<u32, CaptureError> {
        self.width().map_err(|e| CaptureError::MonitorInfo(e.to_string()))
    }
    fn capture_height(&self) -> Result<u32, CaptureError> {
        self.height().map_err(|e| CaptureError::MonitorInfo(e.to_string()))
    }
    fn capture_fps(&self) -> Result<u32, CaptureError> {
        self.refresh_rate().map_err(|e| CaptureError::MonitorInfo(e.to_string()))
    }
}

impl CaptureItemDimensions for Window {
    fn capture_width(&self) -> Result<u32, CaptureError> {
        Ok(self.width().map_err(|e| CaptureError::WindowInfo(e.to_string()))?.max(1) as u32)
    }
    fn capture_height(&self) -> Result<u32, CaptureError> {
        Ok(self.height().map_err(|e| CaptureError::WindowInfo(e.to_string()))?.max(1) as u32)
    }
    fn capture_fps(&self) -> Result<u32, CaptureError> {
        // Las ventanas no tienen un refresh rate propio: se usa el del
        // monitor primario, ya que en la practica el contenido de la ventana
        // se redibuja en sincronia con la pantalla que la muestra.
        Monitor::primary()
            .and_then(|m| m.refresh_rate())
            .map_err(|e| CaptureError::MonitorInfo(e.to_string()))
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
///
/// El encoder es `RawFragmentedEncoder` (ffmpeg propio, mp4 fragmentado), no
/// el `VideoEncoder` interno de `windows-capture` — ver
/// `raw_encoder.rs` para el porque (resiliencia a crashes, CLAUDE.md regla 7).
struct CaptureHandler {
    encoder: Option<RawFragmentedEncoder>,
    // Buffer reusado entre frames para `as_nopadding_buffer` (evita reasignar
    // en cada frame si el buffer de la sesion viene con padding).
    scratch: Vec<u8>,
    first_frame_wall_ms: Option<u128>,
    stopped_wall_ms: Option<u128>,
    _start: Option<Instant>,
}

impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = (u32, u32, u32, PathBuf);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (width, height, fps, output_path) = ctx.flags;

        let encoder = RawFragmentedEncoder::spawn(&output_path, width, height, fps)?;

        Ok(Self {
            encoder: Some(encoder),
            scratch: Vec::new(),
            first_frame_wall_ms: None,
            stopped_wall_ms: None,
            _start: None,
        })
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
            let buffer = frame.buffer()?;
            let bgra = buffer.as_nopadding_buffer(&mut self.scratch);
            encoder.write_frame(bgra)?;
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
