//! Fase 0 spike: prueba que `rdev` puede escuchar eventos globales de mouse
//! (posicion + clicks) y timestampearlos, en paralelo a `spike_record`.
//! Ver docs/PROMPT_AGENTE_DEV.md seccion "Fase 0".
//!
//! Corre de forma independiente al spike de captura: cada evento se loguea con
//! su timestamp de reloj de pared (epoch ms), igual que `recording_started_wall_ms`
//! en `fase0_capture.meta.json`. La correlacion manual entre video y eventos se
//! hace restando ambos: offset_video_s = (evento.t_wall_ms - recording_started_wall_ms) / 1000.
//!
//! Ademas de escuchar eventos reales del usuario, este spike simula unos pocos
//! `MouseMove` (nunca clicks reales — inyectar un click de verdad en el escritorio
//! del usuario sin saber que hay debajo es riesgoso, ver CLAUDE.md raiz sobre
//! acciones con efectos en el mundo real) para que la corrida sea autoverificable
//! incluso si nadie toca el mouse durante la ventana de grabacion. El manejo de
//! clicks se prueba aparte con un test unitario puro (`cargo test`, sin tocar el
//! escritorio real) sobre `format_event_line`.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rdev::{listen, simulate, Event, EventType};

const DURATION_SECS: u64 = 20;

fn output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spikes-output/fase0_input_log.jsonl")
}

fn wall_ms(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn escape(s: &str) -> String {
    let backslash = char::from_u32(0x5c).unwrap();
    s.replace(backslash, "\\\\").replace('"', "\\\"")
}

#[derive(Clone, Copy)]
struct LastPos {
    x: f64,
    y: f64,
}

/// Logica pura de formateo, separada de la escucha real para poder testearla
/// con `cargo test` sin depender de un hook de mouse real (ver Fase 1 en
/// PROMPT_AGENTE_DEV.md: "cada crate debe tener tests unitarios reales").
fn format_event_line(t_wall_ms: u128, event_type: &EventType, last_pos: LastPos) -> Option<String> {
    match *event_type {
        EventType::MouseMove { x, y } => {
            Some(format!("{{\"t_wall_ms\":{t_wall_ms},\"kind\":\"move\",\"x\":{x},\"y\":{y}}}\n"))
        }
        EventType::ButtonPress(button) => Some(format!(
            "{{\"t_wall_ms\":{t_wall_ms},\"kind\":\"button_press\",\"button\":\"{}\",\"x\":{},\"y\":{}}}\n",
            escape(&format!("{button:?}")),
            last_pos.x,
            last_pos.y
        )),
        EventType::ButtonRelease(button) => Some(format!(
            "{{\"t_wall_ms\":{t_wall_ms},\"kind\":\"button_release\",\"button\":\"{}\",\"x\":{},\"y\":{}}}\n",
            escape(&format!("{button:?}")),
            last_pos.x,
            last_pos.y
        )),
        _ => None,
    }
}

fn main() {
    let out_path = output_path();
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("no se pudo crear spikes-output/");
    }

    let file = Mutex::new(File::create(&out_path).expect("no se pudo crear fase0_input_log.jsonl"));
    let last_pos = Mutex::new(LastPos { x: 0.0, y: 0.0 });

    println!("Logueando eventos de mouse por {DURATION_SECS}s en {}", out_path.display());

    // rdev::listen bloquea el hilo actual y no tiene forma nativa de "parar despues
    // de N segundos" en esta version del crate (no es un problema para un spike:
    // el hilo principal duerme y termina el proceso, el writer ya esta flusheado
    // linea por linea asi que no se pierde nada al salir).
    let listener = std::thread::spawn(move || {
        let callback = move |event: Event| {
            let t_wall_ms = wall_ms(event.time);

            let mut pos = last_pos.lock().unwrap();
            if let EventType::MouseMove { x, y } = event.event_type {
                pos.x = x;
                pos.y = y;
            }
            let snapshot = *pos;
            drop(pos);

            if let Some(line) = format_event_line(t_wall_ms, &event.event_type, snapshot) {
                let mut f = file.lock().unwrap();
                let _ = f.write_all(line.as_bytes());
                let _ = f.flush();
            }
        };

        if let Err(err) = listen(callback) {
            eprintln!("Error escuchando eventos de input: {err:?}");
        }
    });

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

    let remaining = 5u64;
    std::thread::sleep(Duration::from_secs(remaining));

    println!("Listo. Log de input escrito en {}", output_path().display());
    // rdev::listen no expone un stop() en 0.5.3; salimos del proceso explicitamente
    // en vez de esperar el join (que bloquearia para siempre) del hilo listener.
    let _ = listener;
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_mouse_move() {
        let line = format_event_line(1_000, &EventType::MouseMove { x: 12.5, y: 34.0 }, LastPos { x: 0.0, y: 0.0 })
            .unwrap();
        assert_eq!(line, "{\"t_wall_ms\":1000,\"kind\":\"move\",\"x\":12.5,\"y\":34}\n");
    }

    #[test]
    fn formats_button_press_with_last_known_position() {
        let line = format_event_line(
            2_000,
            &EventType::ButtonPress(rdev::Button::Left),
            LastPos { x: 111.0, y: 222.0 },
        )
        .unwrap();
        assert_eq!(line, "{\"t_wall_ms\":2000,\"kind\":\"button_press\",\"button\":\"Left\",\"x\":111,\"y\":222}\n");
    }

    #[test]
    fn formats_button_release() {
        let line = format_event_line(
            3_000,
            &EventType::ButtonRelease(rdev::Button::Right),
            LastPos { x: 5.0, y: 6.0 },
        )
        .unwrap();
        assert_eq!(line, "{\"t_wall_ms\":3000,\"kind\":\"button_release\",\"button\":\"Right\",\"x\":5,\"y\":6}\n");
    }

    #[test]
    fn ignores_wheel_events() {
        assert!(
            format_event_line(4_000, &EventType::Wheel { delta_x: 0, delta_y: 1 }, LastPos { x: 0.0, y: 0.0 })
                .is_none()
        );
    }

    #[test]
    fn escapes_backslash_and_quote_in_button_debug() {
        assert_eq!(escape("a\\b\"c"), "a\\\\b\\\"c");
    }
}
