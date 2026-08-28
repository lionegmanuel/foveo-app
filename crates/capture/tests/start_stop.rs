//! Test de integracion real: graba pantalla un par de segundos usando
//! `WindowsScreenCapturer` y verifica que `stop()` deja un mp4 valido con
//! metadata plausible. Queda `#[ignore]` por defecto porque graba la pantalla
//! de verdad (side effect real, no apto para correr en cada `cargo test`
//! silenciosamente) — correrlo explicito con:
//!
//!   cargo test -p capture --test start_stop -- --ignored --nocapture

use std::thread;
use std::time::Duration;

use capture::{CaptureSource, RecordingHandle, ScreenCapturer, WindowsScreenCapturer};

#[test]
#[ignore = "graba la pantalla real; correr a mano con --ignored"]
fn start_then_stop_produces_a_playable_take() {
    let dir = std::env::temp_dir().join(format!("screenzoom-capture-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output_path = dir.join("take.mp4");

    let capturer = WindowsScreenCapturer;
    let handle =
        capturer.start_recording(&output_path, CaptureSource::PrimaryMonitor).expect("start_recording fallo");

    thread::sleep(Duration::from_secs(2));

    let info = handle.stop().expect("stop fallo");

    assert!(info.width > 0);
    assert!(info.height > 0);
    assert!(info.fps > 0);
    assert!(
        info.recording_stopped_wall_ms >= info.recording_started_wall_ms,
        "el timestamp de fin no puede ser anterior al de inicio"
    );

    let metadata = std::fs::metadata(&output_path).expect("el archivo de video deberia existir");
    assert!(metadata.len() > 0, "el mp4 no deberia estar vacio");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_monitors_returns_at_least_one_real_monitor() {
    // Solo consulta geometria, no graba ni escribe nada: no hace falta
    // `#[ignore]`, corre en cada `cargo test` normal.
    let capturer = WindowsScreenCapturer;
    let monitors = capturer.list_monitors().expect("list_monitors fallo");

    assert!(!monitors.is_empty(), "esta maquina deberia tener al menos un monitor");
    for m in &monitors {
        assert!(m.index >= 1);
        assert!(m.width > 0 && m.height > 0, "geometria invalida para {}: {}x{}", m.name, m.width, m.height);
    }
}

#[test]
fn list_windows_returns_at_least_one_real_window() {
    // Igual que `list_monitors_returns_at_least_one_real_monitor`: solo
    // enumera, no graba nada — corre en cada `cargo test` normal. Siempre
    // deberia haber al menos una ventana visible en un runner con sesion de
    // escritorio activa (Explorer, la terminal que corre el test, etc).
    let capturer = WindowsScreenCapturer;
    let windows = capturer.list_windows().expect("list_windows fallo");

    assert!(!windows.is_empty(), "esta maquina deberia tener al menos una ventana capturable");
    for w in &windows {
        assert!(w.index >= 1);
    }
}
