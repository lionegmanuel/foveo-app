//! Fase 0 spike: prueba que `windows-capture` puede grabar la pantalla completa
//! y producir un mp4 reproducible, sin ninguna logica de zoom/composicion/UI.
//! Ver docs/PROMPT_AGENTE_DEV.md seccion "Fase 0".
//!
//! No usa el picker interactivo (GraphicsCapturePicker) a proposito: este binario
//! esta pensado para correr de forma no interactiva durante la validacion del
//! spike. El picker (con manejo explicito de cancelacion, ver ARQUITECTURA.md
//! seccion 4) se implementa recien en la Fase 1, conectado a la UI real.

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
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

const DURATION_SECS: u64 = 60;

fn output_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/capture. La raiz del repo esta dos niveles arriba.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes-output")
}

fn now_wall_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock antes de epoch").as_millis()
}

struct Capture {
    encoder: Option<VideoEncoder>,
    start: Option<Instant>,
    first_frame_wall_ms: Option<u128>,
    meta_path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
}

impl GraphicsCaptureApiHandler for Capture {
    // (width, height, fps, output mp4 path, output meta path) via ctx.flags
    type Flags = (u32, u32, u32, PathBuf, PathBuf);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (width, height, fps, mp4_path, meta_path) = ctx.flags;

        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(width, height).frame_rate(fps).sub_type(VideoSettingsSubType::H264),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &mp4_path,
        )?;

        Ok(Self { encoder: Some(encoder), start: None, first_frame_wall_ms: None, meta_path, width, height, fps })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.start.is_none() {
            self.start = Some(Instant::now());
            self.first_frame_wall_ms = Some(now_wall_ms());
            println!("Primer frame recibido, arranca la ventana de {DURATION_SECS}s.");
        }

        self.encoder.as_mut().unwrap().send_frame(frame)?;

        let elapsed = self.start.unwrap().elapsed();
        print!("\rGrabando: {}s / {DURATION_SECS}s", elapsed.as_secs());
        io::stdout().flush()?;

        if elapsed.as_secs() >= DURATION_SECS {
            self.encoder.take().unwrap().finish()?;

            let stop_wall_ms = now_wall_ms();
            let meta = serde_json_meta(
                self.first_frame_wall_ms.unwrap(),
                stop_wall_ms,
                self.width,
                self.height,
                self.fps,
            );
            std::fs::write(&self.meta_path, meta)?;

            capture_control.stop();
            println!("\nListo. Video + metadata escritos.");
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        println!("Sesion de captura finalizada.");
        Ok(())
    }
}

// JSON minimo a mano, sin traer serde_json solo para esto (el resto de la app
// SI lo usa; este spike es intencionalmente autocontenido, ver "que no hacer"
// en PROMPT_AGENTE_DEV.md sobre no anticipar dependencias que no hacen falta hoy).
fn serde_json_meta(first_frame_wall_ms: u128, stop_wall_ms: u128, width: u32, height: u32, fps: u32) -> String {
    format!(
        "{{\n  \"recording_started_wall_ms\": {first_frame_wall_ms},\n  \"recording_stopped_wall_ms\": {stop_wall_ms},\n  \"width\": {width},\n  \"height\": {height},\n  \"fps\": {fps}\n}}\n"
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let monitor = Monitor::primary().map_err(|e| format!("No se encontro el monitor primario: {e}"))?;

    let width = monitor.width()?;
    let height = monitor.height()?;
    let fps = monitor.refresh_rate()?;

    println!("Monitor primario: {width}x{height} @ {fps}Hz");

    let out_dir = output_dir();
    std::fs::create_dir_all(&out_dir)?;
    let mp4_path = out_dir.join("fase0_capture.mp4");
    let meta_path = out_dir.join("fase0_capture.meta.json");

    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        (width, height, fps, mp4_path, meta_path),
    );

    Capture::start(settings).map_err(|e| format!("Fallo la captura de pantalla: {e}"))?;

    Ok(())
}
