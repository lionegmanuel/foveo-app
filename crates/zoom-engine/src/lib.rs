//! Deteccion de puntos de interes + generacion de keyframes de zoom
//! automaticos (ARQUITECTURA.md 3.3, Fase 1 de PROMPT_AGENTE_DEV.md).
//!
//! Logica pura: no toca disco, no conoce Tauri, wgpu ni ffmpeg. Solo conoce
//! `project::ZoomKeyframe` (el tipo que termina persistido en el `.szproj`).
//!
//! Contrato de entrada: los `InputEvent` que recibe `generate_keyframes` usan
//! `t_ms` **relativo al inicio de la grabacion** (0 = primer frame), no el
//! epoch-ms de reloj de pared que loguea el input-tracker — convertir de uno
//! a otro (restando `recording_started_wall_ms`) es responsabilidad de quien
//! arma el `Project` a partir del log crudo, no de este crate. Las posiciones
//! x/y van normalizadas 0..1, para que este crate sea agnostico de la
//! resolucion de captura (igual que `target_rect` en el `.szproj`).

use project::{Easing, KeyframeSource, Rect, ZoomKeyframe};

/// Un click cae en el mismo "cluster" de interes que el anterior si no pasaron
/// mas de esto entre ambos. Bien chico: agrupa doble-clicks y clicks
/// consecutivos rapidos sin fusionar dos interacciones separadas del usuario.
///
/// Ajustable: si el zoom in/out parpadea demasiado seguido en uso real, subir
/// este valor agrupa mas agresivamente.
pub const CLICK_CLUSTER_WINDOW_MS: u64 = 1_200;

/// Cuanto tiempo sin clicks tiene que pasar (desde el ultimo click de un
/// cluster) antes de generar un keyframe de zoom-out de vuelta al frame
/// completo.
///
/// Ajustable: mas alto = la camara se queda mas tiempo pegada al ultimo click
/// antes de alejarse.
pub const INACTIVITY_ZOOM_OUT_MS: u64 = 3_000;

/// Tamanio (normalizado, mismo valor para ancho y alto) del rectangulo de
/// zoom-in alrededor del centroide de un cluster de clicks.
///
/// Ajustable: mas chico = zoom mas cerrado.
pub const ZOOM_TARGET_SIZE: f32 = 0.42;

/// Duracion de la transicion de cada keyframe (tanto zoom-in como zoom-out).
/// Coincide con el ejemplo documentado en ARQUITECTURA.md 3.5.
pub const TRANSITION_DURATION_MS: u64 = 600;

/// Easing por defecto de la Fase 1 (una sola curva, sin selector todavia —
/// eso es Fase 2). Ver ARQUITECTURA.md seccion 6.
pub const DEFAULT_EASING: Easing = Easing::EaseInOutCubic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventKind {
    Move,
    ButtonPress,
    ButtonRelease,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputEvent {
    /// Milisegundos desde el inicio de la grabacion (ver doc del modulo).
    pub t_ms: u64,
    /// Posicion normalizada 0..1 dentro del frame.
    pub x: f32,
    pub y: f32,
    pub kind: InputEventKind,
}

/// Genera los keyframes de zoom automaticos para una grabacion completa.
///
/// `recording_duration_ms` acota el final: no se generan keyframes despues
/// de este punto (ni siquiera el zoom-out final si cae justo en el borde).
/// Devuelve `Vec::new()` si no hay ningun click (caso borde explicito, ver
/// tests) — la grabacion queda a frame completo todo el tiempo, sin panics.
#[must_use]
pub fn generate_keyframes(events: &[InputEvent], recording_duration_ms: u64) -> Vec<ZoomKeyframe> {
    let mut clicks: Vec<InputEvent> =
        events.iter().copied().filter(|e| e.kind == InputEventKind::ButtonPress).collect();
    clicks.sort_by_key(|e| e.t_ms);

    if clicks.is_empty() {
        return Vec::new();
    }

    let clusters = cluster_clicks(&clicks);

    let mut keyframes = Vec::with_capacity(clusters.len() * 2);
    let mut next_id = 1u32;

    for (i, cluster) in clusters.iter().enumerate() {
        let start_ms = cluster.first().expect("cluster nunca esta vacio").t_ms;
        let last_click_ms = cluster.last().expect("cluster nunca esta vacio").t_ms;

        let (centroid_x, centroid_y) = centroid(cluster);
        keyframes.push(zoom_in_keyframe(&mut next_id, start_ms, centroid_x, centroid_y));

        let next_cluster_start = clusters.get(i + 1).map(|c| c[0].t_ms);
        let window_end = next_cluster_start.unwrap_or(recording_duration_ms);
        let idle_gap = window_end.saturating_sub(last_click_ms);

        if idle_gap >= INACTIVITY_ZOOM_OUT_MS {
            let zoom_out_start = last_click_ms + INACTIVITY_ZOOM_OUT_MS;
            if zoom_out_start < recording_duration_ms {
                keyframes.push(zoom_out_keyframe(&mut next_id, zoom_out_start));
            }
        }
    }

    keyframes
}

/// Agrupa clicks ya ordenados por tiempo en clusters: cada click se une al
/// cluster anterior si esta a `CLICK_CLUSTER_WINDOW_MS` o menos del click
/// previo (encadenado, no ancla fija — asi una racha de clicks separados por
/// poquito cada uno no se corta aunque el primero y el ultimo esten lejos).
fn cluster_clicks(sorted_clicks: &[InputEvent]) -> Vec<Vec<InputEvent>> {
    let mut clusters: Vec<Vec<InputEvent>> = Vec::new();
    let mut current: Vec<InputEvent> = vec![sorted_clicks[0]];

    for &click in &sorted_clicks[1..] {
        let prev_t = current.last().expect("current nunca esta vacio").t_ms;
        if click.t_ms.saturating_sub(prev_t) <= CLICK_CLUSTER_WINDOW_MS {
            current.push(click);
        } else {
            clusters.push(std::mem::take(&mut current));
            current.push(click);
        }
    }
    clusters.push(current);

    clusters
}

fn centroid(cluster: &[InputEvent]) -> (f32, f32) {
    let n = cluster.len() as f32;
    let sum_x: f32 = cluster.iter().map(|e| e.x).sum();
    let sum_y: f32 = cluster.iter().map(|e| e.y).sum();
    (sum_x / n, sum_y / n)
}

fn zoom_in_keyframe(next_id: &mut u32, start_ms: u64, centroid_x: f32, centroid_y: f32) -> ZoomKeyframe {
    let kf = ZoomKeyframe {
        id: format!("kf_{:03}", *next_id),
        start_ms,
        duration_ms: TRANSITION_DURATION_MS,
        target_rect: target_rect_around(centroid_x, centroid_y),
        easing: DEFAULT_EASING,
        source: KeyframeSource::Auto,
    };
    *next_id += 1;
    kf
}

fn zoom_out_keyframe(next_id: &mut u32, start_ms: u64) -> ZoomKeyframe {
    let kf = ZoomKeyframe {
        id: format!("kf_{:03}", *next_id),
        start_ms,
        duration_ms: TRANSITION_DURATION_MS,
        target_rect: Rect::FULL_FRAME,
        easing: DEFAULT_EASING,
        source: KeyframeSource::Auto,
    };
    *next_id += 1;
    kf
}

/// Rectangulo de `ZOOM_TARGET_SIZE` centrado en (cx, cy), clampeado para que
/// nunca se salga del frame (0..1) aunque el click este pegado a un borde.
fn target_rect_around(cx: f32, cy: f32) -> Rect {
    let half = ZOOM_TARGET_SIZE / 2.0;
    let max_origin = 1.0 - ZOOM_TARGET_SIZE;
    let x = (cx - half).clamp(0.0, max_origin);
    let y = (cy - half).clamp(0.0, max_origin);
    Rect { x, y, w: ZOOM_TARGET_SIZE, h: ZOOM_TARGET_SIZE }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(t_ms: u64, x: f32, y: f32) -> InputEvent {
        InputEvent { t_ms, x, y, kind: InputEventKind::ButtonPress }
    }

    fn mv(t_ms: u64, x: f32, y: f32) -> InputEvent {
        InputEvent { t_ms, x, y, kind: InputEventKind::Move }
    }

    // --- caso borde: sin ningun evento de input ---

    #[test]
    fn no_events_produces_no_keyframes() {
        assert_eq!(generate_keyframes(&[], 60_000), Vec::new());
    }

    #[test]
    fn only_move_events_no_clicks_produces_no_keyframes() {
        let events = [mv(0, 0.1, 0.1), mv(500, 0.5, 0.5), mv(1_000, 0.9, 0.9)];
        assert_eq!(generate_keyframes(&events, 60_000), Vec::new());
    }

    // --- clicks aislados ---

    #[test]
    fn isolated_click_produces_zoom_in_and_zoom_out() {
        let events = [click(1_000, 0.5, 0.5)];
        let kfs = generate_keyframes(&events, 60_000);

        assert_eq!(kfs.len(), 2, "un click aislado deberia generar zoom-in + zoom-out: {kfs:?}");

        assert_eq!(kfs[0].start_ms, 1_000);
        assert_eq!(kfs[0].source, KeyframeSource::Auto);
        assert_eq!(kfs[0].easing, DEFAULT_EASING);
        assert_ne!(kfs[0].target_rect, Rect::FULL_FRAME);

        assert_eq!(kfs[1].start_ms, 1_000 + INACTIVITY_ZOOM_OUT_MS);
        assert_eq!(kfs[1].target_rect, Rect::FULL_FRAME);
    }

    #[test]
    fn two_isolated_clicks_far_apart_each_get_their_own_keyframes() {
        // Separados por mucho mas que CLICK_CLUSTER_WINDOW_MS y que
        // INACTIVITY_ZOOM_OUT_MS: dos interacciones totalmente independientes.
        let events = [click(1_000, 0.2, 0.2), click(20_000, 0.8, 0.8)];
        let kfs = generate_keyframes(&events, 60_000);

        // zoom-in #1, zoom-out #1, zoom-in #2, zoom-out #2
        assert_eq!(kfs.len(), 4);
        assert_eq!(kfs[0].start_ms, 1_000);
        assert_eq!(kfs[1].start_ms, 1_000 + INACTIVITY_ZOOM_OUT_MS);
        assert_eq!(kfs[2].start_ms, 20_000);
        assert_eq!(kfs[3].start_ms, 20_000 + INACTIVITY_ZOOM_OUT_MS);
    }

    // --- clusters de clicks cercanos en tiempo ---

    #[test]
    fn clicks_close_in_time_collapse_into_a_single_zoom_in_keyframe() {
        let events = [click(1_000, 0.2, 0.2), click(1_300, 0.3, 0.3), click(1_600, 0.4, 0.4)];
        let kfs = generate_keyframes(&events, 60_000);

        // Un solo zoom-in (mas su zoom-out final), no uno por click.
        assert_eq!(kfs.len(), 2, "clicks a 300ms deberian fusionarse en un cluster: {kfs:?}");
        assert_eq!(kfs[0].start_ms, 1_000, "el zoom-in arranca en el primer click del cluster");

        // El target_rect esta centrado en el centroide (0.3, 0.3), no en el
        // primer ni el ultimo click.
        let expected_x = (0.3 - ZOOM_TARGET_SIZE / 2.0).clamp(0.0, 1.0 - ZOOM_TARGET_SIZE);
        assert!((kfs[0].target_rect.x - expected_x).abs() < 1e-6);
    }

    #[test]
    fn chained_clicks_stay_in_one_cluster_even_if_first_and_last_are_far_apart() {
        // Cada click esta a <= CLICK_CLUSTER_WINDOW_MS del anterior, aunque el
        // primero y el ultimo (0ms y 3000ms) esten lejos entre si.
        let events = [click(0, 0.0, 0.0), click(1_000, 0.2, 0.2), click(2_000, 0.4, 0.4), click(3_000, 0.6, 0.6)];
        let kfs = generate_keyframes(&events, 60_000);

        assert_eq!(kfs.len(), 2, "clicks encadenados deberian seguir siendo un solo cluster: {kfs:?}");
        assert_eq!(kfs[0].start_ms, 0);
        assert_eq!(kfs[1].start_ms, 3_000 + INACTIVITY_ZOOM_OUT_MS);
    }

    // --- periodos de inactividad ---

    #[test]
    fn short_gap_between_clusters_does_not_trigger_zoom_out() {
        // Segundo cluster arranca antes de que se cumpla INACTIVITY_ZOOM_OUT_MS
        // desde el ultimo click del primero: la camara pasa derecho de un
        // target al otro, sin volver a frame completo en el medio.
        let events = [
            click(1_000, 0.2, 0.2),
            click(1_100, 0.2, 0.2),
            // gap de 2000ms (< INACTIVITY_ZOOM_OUT_MS de 3000ms) hasta el siguiente cluster
            click(3_100, 0.8, 0.8),
        ];
        // Grabacion termina 100ms despues del ultimo click (bien por debajo de
        // INACTIVITY_ZOOM_OUT_MS) para que este test aisle unicamente el
        // comportamiento "entre clusters", sin el zoom-out final de cierre.
        let kfs = generate_keyframes(&events, 3_200);

        assert_eq!(kfs.len(), 2, "no deberia haber zoom-out entre clusters cercanos: {kfs:?}");
        assert_eq!(kfs[0].start_ms, 1_000);
        assert_eq!(kfs[1].start_ms, 3_100);
    }

    #[test]
    fn long_gap_between_clusters_inserts_zoom_out_before_the_next_zoom_in() {
        let events = [
            click(1_000, 0.2, 0.2),
            // gap de 5000ms (> INACTIVITY_ZOOM_OUT_MS): vuelve a frame completo
            // antes de que arranque el segundo cluster.
            click(6_000, 0.8, 0.8),
        ];
        // Grabacion termina poco despues del ultimo click, para que este test
        // se quede enfocado en el zoom-out "entre clusters" y no sume ademas
        // el zoom-out final de cierre (eso ya lo cubre otro test).
        let kfs = generate_keyframes(&events, 6_500);

        assert_eq!(kfs.len(), 3, "deberia haber zoom-out entre clusters lejanos: {kfs:?}");
        assert_eq!(kfs[0].start_ms, 1_000); // zoom-in cluster 1
        assert_eq!(kfs[1].start_ms, 1_000 + INACTIVITY_ZOOM_OUT_MS); // zoom-out
        assert_eq!(kfs[1].target_rect, Rect::FULL_FRAME);
        assert_eq!(kfs[2].start_ms, 6_000); // zoom-in cluster 2
    }

    #[test]
    fn trailing_inactivity_after_the_last_cluster_also_zooms_out() {
        // Si despues del ultimo cluster la grabacion sigue mucho mas tiempo
        // (idle hasta el final), tambien debe volver a frame completo — no
        // solo entre clusters intermedios.
        let events = [click(1_000, 0.2, 0.2), click(6_000, 0.8, 0.8)];
        let kfs = generate_keyframes(&events, 60_000);

        assert_eq!(kfs.len(), 4, "deberia zoomear afuera despues del ultimo cluster tambien: {kfs:?}");
        assert_eq!(kfs[3].start_ms, 6_000 + INACTIVITY_ZOOM_OUT_MS);
        assert_eq!(kfs[3].target_rect, Rect::FULL_FRAME);
    }

    #[test]
    fn no_trailing_zoom_out_if_it_would_land_past_the_recording_end() {
        // Click a 59_900ms con grabacion de 60_000ms: el zoom-out caeria en
        // 59_900 + 3_000 = 62_900, despues del final. No se genera.
        let events = [click(59_900, 0.5, 0.5)];
        let kfs = generate_keyframes(&events, 60_000);

        assert_eq!(kfs.len(), 1, "no deberia haber zoom-out despues del final de la grabacion: {kfs:?}");
        assert_eq!(kfs[0].start_ms, 59_900);
    }

    #[test]
    fn target_rect_is_clamped_to_stay_inside_the_frame_near_edges() {
        let events = [click(1_000, 0.0, 0.0)]; // click pegado a la esquina superior izquierda
        let kfs = generate_keyframes(&events, 60_000);

        let rect = kfs[0].target_rect;
        assert!(rect.x >= 0.0 && rect.x + rect.w <= 1.0, "rect fuera de frame: {rect:?}");
        assert!(rect.y >= 0.0 && rect.y + rect.h <= 1.0, "rect fuera de frame: {rect:?}");
    }

    #[test]
    fn keyframe_ids_are_unique_and_sequential() {
        let events = [click(1_000, 0.2, 0.2), click(10_000, 0.8, 0.8)];
        let kfs = generate_keyframes(&events, 60_000);
        let ids: Vec<&str> = kfs.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids, vec!["kf_001", "kf_002", "kf_003", "kf_004"]);
    }
}
