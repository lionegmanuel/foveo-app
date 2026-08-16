//! Interpolacion de la "camara" (rect de crop/zoom) para un instante dado a
//! partir de los `ZoomKeyframe` del proyecto. Logica pura, sin GPU — la usa
//! el pipeline `wgpu` del compositor para saber que rect pedirle al shader en
//! cada frame de salida (ver ARQUITECTURA.md 3.3, tabla de modulos:
//! "Compositor... interpola la 'camara' (rect de crop/zoom) para cada frame").

use project::{Easing, Rect, ZoomKeyframe};

/// Calcula el rect de crop vigente en `t_ms`, interpolando entre keyframes.
///
/// Se asume `keyframes` ordenado por `start_ms` (asi los genera
/// `zoom_engine::generate_keyframes`; si vienen de una edicion manual del
/// usuario en la Fase 2, hay que ordenarlos antes de llamar esta funcion).
///
/// Antes del primer keyframe, o si no hay ninguno, el rect vigente es
/// `Rect::FULL_FRAME`. Durante la ventana `[start_ms, start_ms+duration_ms]`
/// de un keyframe se interpola desde el rect previo hacia `target_rect` con
/// el easing del keyframe; despues de esa ventana el rect queda fijo en
/// `target_rect` hasta el proximo keyframe.
#[must_use]
pub fn camera_rect_at(keyframes: &[ZoomKeyframe], t_ms: u64) -> Rect {
    let mut current = Rect::FULL_FRAME;

    for kf in keyframes {
        if t_ms < kf.start_ms {
            break;
        }

        let end_ms = kf.start_ms + kf.duration_ms;
        if t_ms >= end_ms {
            current = kf.target_rect;
            continue;
        }

        let duration = kf.duration_ms.max(1) as f32;
        let progress = (t_ms - kf.start_ms) as f32 / duration;
        let eased = ease(kf.easing, progress);
        return lerp_rect(current, kf.target_rect, eased);
    }

    current
}

/// Aplica la curva de easing a un progreso normalizado 0..1.
fn ease(easing: Easing, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => t,
        Easing::EaseInOutCubic => {
            if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
        }
        // Aproximacion tipo "overshoot" (no es una simulacion fisica
        // resorte-masa real) — alcanza para el selector de easing de la
        // Fase 2; cambiarla por algo mas fiel es un swap local si hace falta.
        Easing::Spring => {
            if t <= 0.0 {
                0.0
            } else if t >= 1.0 {
                1.0
            } else {
                let c4 = (2.0 * std::f32::consts::PI) / 3.0;
                2f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
            }
        }
    }
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect { x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t), w: lerp(a.w, b.w, t), h: lerp(a.h, b.h, t) }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::KeyframeSource;

    fn kf(start_ms: u64, duration_ms: u64, target_rect: Rect, easing: Easing) -> ZoomKeyframe {
        ZoomKeyframe {
            id: "kf_test".to_string(),
            start_ms,
            duration_ms,
            target_rect,
            easing,
            source: KeyframeSource::Auto,
        }
    }

    const ZOOM: Rect = Rect { x: 0.2, y: 0.2, w: 0.4, h: 0.4 };

    #[test]
    fn no_keyframes_stays_full_frame() {
        assert_eq!(camera_rect_at(&[], 0), Rect::FULL_FRAME);
        assert_eq!(camera_rect_at(&[], 999_999), Rect::FULL_FRAME);
    }

    #[test]
    fn before_first_keyframe_stays_full_frame() {
        let kfs = [kf(1_000, 600, ZOOM, Easing::Linear)];
        assert_eq!(camera_rect_at(&kfs, 0), Rect::FULL_FRAME);
        assert_eq!(camera_rect_at(&kfs, 999), Rect::FULL_FRAME);
    }

    #[test]
    fn at_transition_start_rect_equals_previous_state() {
        let kfs = [kf(1_000, 600, ZOOM, Easing::Linear)];
        // eased(0) == 0, entonces en t == start_ms el rect es el previo (full frame).
        assert_eq!(camera_rect_at(&kfs, 1_000), Rect::FULL_FRAME);
    }

    #[test]
    fn at_transition_end_rect_equals_target_exactly() {
        let kfs = [kf(1_000, 600, ZOOM, Easing::Linear)];
        assert_eq!(camera_rect_at(&kfs, 1_600), ZOOM);
    }

    #[test]
    fn holds_target_rect_after_transition_ends() {
        let kfs = [kf(1_000, 600, ZOOM, Easing::Linear)];
        assert_eq!(camera_rect_at(&kfs, 50_000), ZOOM);
    }

    #[test]
    fn linear_easing_interpolates_proportionally_at_midpoint() {
        let kfs = [kf(0, 1_000, ZOOM, Easing::Linear)];
        let rect = camera_rect_at(&kfs, 500);
        // A mitad de camino en linear: promedio exacto entre full frame y ZOOM.
        assert!((rect.x - 0.1).abs() < 1e-5, "x={}", rect.x);
        assert!((rect.w - 0.7).abs() < 1e-5, "w={}", rect.w);
    }

    #[test]
    fn ease_in_out_cubic_is_symmetric_at_midpoint() {
        let kfs = [kf(0, 1_000, ZOOM, Easing::EaseInOutCubic)];
        let rect = camera_rect_at(&kfs, 500);
        // ease-in-out-cubic(0.5) == 0.5 exacto por simetria de la curva.
        let expected = lerp_rect(Rect::FULL_FRAME, ZOOM, 0.5);
        assert!((rect.x - expected.x).abs() < 1e-5);
        assert!((rect.w - expected.w).abs() < 1e-5);
    }

    #[test]
    fn second_keyframe_interpolates_from_first_targets_rect_not_full_frame() {
        let zoom_a = Rect { x: 0.0, y: 0.0, w: 0.3, h: 0.3 };
        let zoom_b = Rect { x: 0.5, y: 0.5, w: 0.3, h: 0.3 };
        let kfs = [kf(0, 200, zoom_a, Easing::Linear), kf(1_000, 200, zoom_b, Easing::Linear)];

        // En plena transicion del segundo keyframe, a mitad de camino: debe
        // partir de zoom_a (el estado alcanzado por el primero), no de full frame.
        let rect = camera_rect_at(&kfs, 1_100);
        let expected = lerp_rect(zoom_a, zoom_b, 0.5);
        assert!((rect.x - expected.x).abs() < 1e-5, "deberia partir de zoom_a: {rect:?} vs {expected:?}");
    }

    #[test]
    fn spring_easing_reaches_exact_endpoints() {
        let kfs = [kf(0, 1_000, ZOOM, Easing::Spring)];
        assert_eq!(camera_rect_at(&kfs, 0), Rect::FULL_FRAME);
        assert_eq!(camera_rect_at(&kfs, 1_000), ZOOM);
    }
}
