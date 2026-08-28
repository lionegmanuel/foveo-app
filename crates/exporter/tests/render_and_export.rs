//! Test de integracion real: recorta un clip corto de la toma cruda grabada
//! en la Fase 0 (`spikes-output/fase0_capture.mp4`, si esta presente),
//! arma un `Project` con un keyframe de zoom, corre `render_and_export` de
//! punta a punta (decode real -> interpolar camara -> componer en GPU real
//! -> encodear con ffmpeg real) y valida el mp4 resultante con ffmpeg.
//!
//! `#[ignore]` por defecto: depende del archivo dejado por la corrida manual
//! de Fase 0 y tarda unos segundos (decodifica + compone + encodea frames de
//! verdad). Correr con:
//!
//!   cargo test -p exporter --test render_and_export -- --ignored --nocapture

use std::path::PathBuf;
use std::process::Command;

use exporter::render_and_export;
use project::{
    Codec, Easing, ExportSettings, InputLog, KeyframeSource, Project, RawTake, Rect, Resolution, ZoomKeyframe,
};

fn ffmpeg_path() -> String {
    std::env::var("FFMPEG_BINARY_PATH").unwrap_or_else(|_| "ffmpeg".to_string())
}

#[test]
#[ignore = "depende del mp4 de Fase 0 en spikes-output/ y tarda unos segundos; correr a mano con --ignored"]
fn renders_a_short_real_clip_end_to_end() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
    let source_take = repo_root.join("spikes-output/fase0_capture.mp4");
    assert!(
        source_take.exists(),
        "falta spikes-output/fase0_capture.mp4 (correr el spike de Fase 0 antes de este test)"
    );

    let work_dir = std::env::temp_dir().join(format!("screenzoom-exporter-test-{}", std::process::id()));
    std::fs::create_dir_all(&work_dir).unwrap();
    let short_take = work_dir.join("short_take.mp4");
    let output = work_dir.join("export.mp4");

    // Recorta 1s del principio (stream copy, rapido, sin recodificar) para
    // no tener que procesar los 60s completos en este test.
    let status = Command::new(ffmpeg_path())
        .args(["-y", "-v", "error", "-i"])
        .arg(&source_take)
        .args(["-t", "1", "-c", "copy"])
        .arg(&short_take)
        .status()
        .expect("no se pudo correr ffmpeg para recortar el clip de prueba");
    assert!(status.success(), "el recorte con ffmpeg fallo");

    let project = Project {
        version: project::CURRENT_VERSION,
        raw_take: RawTake {
            path: short_take.clone(),
            fps: 60,
            resolution: Resolution { width: 1920, height: 1080 },
            duration_ms: 1_000,
        },
        input_log: InputLog { path: work_dir.join("unused.input.jsonl") },
        zoom_keyframes: vec![ZoomKeyframe {
            id: "kf_001".to_string(),
            start_ms: 0,
            duration_ms: 500,
            target_rect: Rect { x: 0.1, y: 0.1, w: 0.5, h: 0.5 },
            easing: Easing::EaseInOutCubic,
            source: KeyframeSource::Auto,
        }],
        cursor_path: Vec::new(),
        style: Default::default(),
        export_settings: ExportSettings {
            resolution: project::ExportResolution::P1080,
            fps: 60,
            codec: Codec::H264,
            hw_accel: "auto".to_string(),
        },
    };

    let mut frames_reported = 0u64;
    render_and_export(&project, &output, |n| frames_reported = n).expect("render_and_export fallo");

    assert!(frames_reported > 0, "deberia haber procesado al menos un frame");

    let metadata = std::fs::metadata(&output).expect("el mp4 exportado deberia existir");
    assert!(metadata.len() > 0, "el mp4 exportado no deberia estar vacio");

    let decode_check = Command::new(ffmpeg_path())
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-f", "null", "-"])
        .output()
        .expect("no se pudo correr ffmpeg para validar el mp4 exportado");
    assert!(
        decode_check.status.success() && decode_check.stderr.is_empty(),
        "el mp4 exportado no decodifica limpio: {}",
        String::from_utf8_lossy(&decode_check.stderr)
    );

    std::fs::remove_dir_all(&work_dir).ok();
}
