//! Fase 0 spike: prueba que `rdev` puede escuchar eventos globales de mouse
//! (posicion + clicks) y timestampearlos, en paralelo a `spike_record`.
//! Ver docs/PROMPT_AGENTE_DEV.md seccion "Fase 0".
//!
//! La logica de formateo/escucha ahora vive en el crate (`input_tracker::`,
//! Fase 1 paso 2 — trait `InputSource` + impl `RdevInputSource`); este binario
//! solo orquesta la corrida cronometrada del spike y, ademas de escuchar
//! eventos reales del usuario, simula unos pocos `MouseMove` (nunca clicks
//! reales — inyectar un click de verdad en el escritorio del usuario sin
//! saber que hay debajo es riesgoso, ver CLAUDE.md raiz sobre acciones con
//! efectos en el mundo real) para que la corrida sea autoverificable incluso
//! si nadie toca el mouse durante la ventana de grabacion.

use std::path::PathBuf;
use std::time::Duration;

use input_tracker::{InputRecordingHandle, InputSource, RdevInputSource};
use rdev::{simulate, EventType};

const DURATION_SECS: u64 = 20;

fn output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes-output/fase0_input_log.jsonl")
}

fn main() {
    let out_path = output_path();
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("no se pudo crear spikes-output/");
    }

    println!("Logueando eventos de mouse por {DURATION_SECS}s en {}", out_path.display());

    let source = RdevInputSource;
    let handle = source.start_logging(&out_path).expect("no se pudo arrancar el listener de input");

    // Dar tiempo a que el hook global quede instalado antes de simular movimiento.
    std::thread::sleep(Duration::from_millis(500));

    // Movimientos sinteticos deterministicos (SOLO move, nunca click) para que
    // el log tenga contenido verificable incluso sin interaccion humana. Se
    // reparten a lo largo de la ventana de grabacion para poder correlacionarlos
    // despues contra frames especificos del video.
    let waypoints: [(f64, f64); 4] = [(200.0, 200.0), (800.0, 450.0), (1500.0, 800.0), (960.0, 540.0)];
    for (x, y) in waypoints {
        if let Err(err) = simulate(&EventType::MouseMove { x, y }) {
            eprintln!("No se pudo simular movimiento de mouse: {err:?}");
        }
        std::thread::sleep(Duration::from_secs(3));
    }

    std::thread::sleep(Duration::from_secs(5));

    handle.stop().expect("no se pudo detener el listener de input");
    println!("Listo. Log de input escrito en {}", output_path().display());
}
