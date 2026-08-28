//! Reconstruccion de cursor suavizado a partir de `project::CursorPathPoint`
//! (ver ARQUITECTURA.md 3.1: "el cursor real... no se graba tal cual... se
//! reconstruye digitalmente a partir de la posicion logica capturada,
//! generando un movimiento suave y recto entre puntos"). Logica pura, sin
//! GPU ni IO — el `.szproj` ya trae los puntos precomputados
//! (`commands::recording::build_cursor_path`, capa de Tauri).

use project::CursorPathPoint;

/// Constante de tiempo (ms) de la media movil exponencial usada para suavizar
/// el cursor: mas alto = cursor mas "flotante"/rezagado respecto al mouse
/// real; mas bajo = sigue casi 1:1 (jittery, como el cursor crudo del SO).
/// 90ms da un movimiento suave sin sentirse desconectado del click real.
const SMOOTHING_TAU_MS: f32 = 90.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorAt {
    pub x: f32,
    pub y: f32,
    pub pressed: bool,
}

/// Trayectoria de cursor ya suavizada, lista para samplear por timestamp.
pub struct CursorPath {
    // (t_ms, x, y, pressed), ordenado por t_ms.
    smoothed: Vec<(u64, f32, f32, bool)>,
}

impl CursorPath {
    /// Aplica una media movil exponencial sobre `points` (se asume ordenado
    /// por `t_ms`, que es como lo arma `build_cursor_path`). Vacio si
    /// `points` esta vacio (grabaciones sin ningun movimiento de mouse, o
    /// `.szproj` viejos sin este campo).
    #[must_use]
    pub fn build(points: &[CursorPathPoint]) -> Self {
        let Some(first) = points.first() else {
            return Self { smoothed: Vec::new() };
        };

        let mut smoothed = Vec::with_capacity(points.len());
        let (mut sx, mut sy) = (first.x, first.y);
        smoothed.push((first.t_ms, sx, sy, first.pressed));

        for pair in points.windows(2) {
            let (prev, cur) = (pair[0], pair[1]);
            let dt = cur.t_ms.saturating_sub(prev.t_ms).max(1) as f32;
            let alpha = 1.0 - (-dt / SMOOTHING_TAU_MS).exp();
            sx += (cur.x - sx) * alpha;
            sy += (cur.y - sy) * alpha;
            smoothed.push((cur.t_ms, sx, sy, cur.pressed));
        }

        Self { smoothed }
    }

    /// Posicion normalizada (0..1, mismo eje que `project::Rect`) del cursor
    /// en `t_ms`, o `None` si el proyecto no tiene datos de cursor.
    #[must_use]
    pub fn position_at(&self, t_ms: u64) -> Option<CursorAt> {
        let (first, last) = (*self.smoothed.first()?, *self.smoothed.last()?);

        if t_ms <= first.0 {
            return Some(CursorAt { x: first.1, y: first.2, pressed: first.3 });
        }
        if t_ms >= last.0 {
            return Some(CursorAt { x: last.1, y: last.2, pressed: last.3 });
        }

        for pair in self.smoothed.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t_ms >= a.0 && t_ms <= b.0 {
                let span = (b.0 - a.0).max(1) as f32;
                let t = (t_ms - a.0) as f32 / span;
                return Some(CursorAt {
                    x: a.1 + (b.1 - a.1) * t,
                    y: a.2 + (b.2 - a.2) * t,
                    pressed: if t < 0.5 { a.3 } else { b.3 },
                });
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_points_has_no_position() {
        let path = CursorPath::build(&[]);
        assert!(path.position_at(0).is_none());
        assert!(path.position_at(1_000).is_none());
    }

    #[test]
    fn single_point_stays_fixed_everywhere() {
        let path = CursorPath::build(&[CursorPathPoint { t_ms: 500, x: 0.3, y: 0.4, pressed: false }]);
        assert_eq!(path.position_at(0), Some(CursorAt { x: 0.3, y: 0.4, pressed: false }));
        assert_eq!(path.position_at(999_999), Some(CursorAt { x: 0.3, y: 0.4, pressed: false }));
    }

    #[test]
    fn before_first_and_after_last_clamp_to_endpoints() {
        let points = [
            CursorPathPoint { t_ms: 100, x: 0.0, y: 0.0, pressed: false },
            CursorPathPoint { t_ms: 200, x: 1.0, y: 1.0, pressed: false },
        ];
        let path = CursorPath::build(&points);
        assert_eq!(path.position_at(0).unwrap().x, 0.0);
        assert_eq!(path.position_at(10_000).unwrap().x, path.position_at(200).unwrap().x);
    }

    #[test]
    fn smoothing_lags_behind_a_sudden_jump_then_converges() {
        // Salto brusco de 0.0 a 1.0 en un solo paso de 16ms (~60fps): la media
        // movil no deberia saltar instantaneamente a 1.0 en el primer punto
        // post-salto, pero si converger cerca de 1.0 varios pasos despues.
        let mut points = vec![CursorPathPoint { t_ms: 0, x: 0.0, y: 0.0, pressed: false }];
        points.push(CursorPathPoint { t_ms: 16, x: 1.0, y: 1.0, pressed: false });
        for i in 2..40u64 {
            points.push(CursorPathPoint { t_ms: i * 16, x: 1.0, y: 1.0, pressed: false });
        }

        let path = CursorPath::build(&points);
        let right_after_jump = path.position_at(16).unwrap().x;
        let long_after = path.position_at(39 * 16).unwrap().x;

        assert!(right_after_jump < 0.5, "deberia rezagarse justo despues del salto, fue {right_after_jump}");
        assert!(long_after > 0.95, "deberia converger cerca de 1.0 varios frames despues, fue {long_after}");
    }

    #[test]
    fn interpolates_linearly_between_smoothed_samples() {
        let points = [
            CursorPathPoint { t_ms: 0, x: 0.0, y: 0.0, pressed: false },
            CursorPathPoint { t_ms: 1_000, x: 0.0, y: 0.0, pressed: false },
        ];
        // Sin cambio de posicion real (ambos puntos en 0,0): el suavizado no
        // debe inventar movimiento, y cualquier t intermedio debe dar 0,0.
        let path = CursorPath::build(&points);
        let mid = path.position_at(500).unwrap();
        assert_eq!(mid.x, 0.0);
        assert_eq!(mid.y, 0.0);
    }

    #[test]
    fn pressed_state_switches_at_the_matching_sample() {
        let points = [
            CursorPathPoint { t_ms: 0, x: 0.5, y: 0.5, pressed: false },
            CursorPathPoint { t_ms: 100, x: 0.5, y: 0.5, pressed: true },
        ];
        let path = CursorPath::build(&points);
        assert!(!path.position_at(0).unwrap().pressed);
        assert!(path.position_at(100).unwrap().pressed);
    }
}
