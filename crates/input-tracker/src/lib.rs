//! Trait de tracking de input + implementacion via `rdev`.
//!
//! `InputSource`/`InputRecordingHandle` son la interfaz que el resto del
//! sistema usa; la implementacion concreta con `rdev` queda detras de ella
//! (mismo patron que `crates/capture`, ver ARQUITECTURA.md 3.6: si la
//! latencia/precision de `rdev` no alcanza, el upgrade path documentado es
//! reemplazarla por un hook nativo `WH_MOUSE_LL` sin tocar el resto del
//! sistema).
//!
//! Este crate no importa `tauri`.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use rdev::{listen, Event, EventType};

#[derive(Debug, thiserror::Error)]
pub enum InputTrackerError {
    #[error("no se pudo crear el archivo de log de input: {0}")]
    Io(#[from] std::io::Error),
}

/// Abstraccion de plataforma para loguear eventos globales de mouse a un
/// archivo JSONL (posicion + clicks, timestampeados a reloj de pared — mismo
/// reloj que usa `capture::RawTakeInfo` para poder correlacionar ambos).
pub trait InputSource {
    type Handle: InputRecordingHandle;

    fn start_logging(&self, output_path: &Path) -> Result<Self::Handle, InputTrackerError>;
}

pub trait InputRecordingHandle {
    fn stop(self) -> Result<(), InputTrackerError>;
}

/// Implementacion con `rdev::listen`.
pub struct RdevInputSource;

impl InputSource for RdevInputSource {
    type Handle = RdevRecordingHandle;

    fn start_logging(&self, output_path: &Path) -> Result<Self::Handle, InputTrackerError> {
        let file = Arc::new(Mutex::new(File::create(output_path)?));
        let last_pos = Arc::new(Mutex::new(LastPos { x: 0.0, y: 0.0 }));
        let active = Arc::new(AtomicBool::new(true));

        let active_for_thread = Arc::clone(&active);
        let thread = thread::spawn(move || {
            let callback = move |event: Event| {
                if !active_for_thread.load(Ordering::Relaxed) {
                    return;
                }

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

        Ok(RdevRecordingHandle { active, _thread: thread })
    }
}

pub struct RdevRecordingHandle {
    active: Arc<AtomicBool>,
    _thread: JoinHandle<()>,
}

impl InputRecordingHandle for RdevRecordingHandle {
    fn stop(self) -> Result<(), InputTrackerError> {
        // rdev 0.5.3 no expone un stop real del hook global de OS (no hay
        // `unlisten()`), asi que esto solo apaga la escritura de nuevos
        // eventos; el hilo que corre `listen()` sigue vivo (idle) hasta que
        // el proceso termina. Ver el comentario del modulo sobre el upgrade
        // path a un hook nativo si esto llega a ser un problema real.
        self.active.store(false, Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct LastPos {
    pub x: f64,
    pub y: f64,
}

fn wall_ms(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn escape(s: &str) -> String {
    let backslash = char::from_u32(0x5c).unwrap();
    s.replace(backslash, "\\\\").replace('"', "\\\"")
}

/// Logica pura de formateo de un evento a una linea JSONL, separada de la
/// escucha real para poder testearla sin depender de un hook de mouse real.
pub fn format_event_line(t_wall_ms: u128, event_type: &EventType, last_pos: LastPos) -> Option<String> {
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
