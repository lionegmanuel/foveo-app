//! Segunda etapa de composicion (CPU, no wgpu): toma el frame ya
//! recortado/escalado por `Compositor::composite_frame` y lo compone sobre
//! un canvas del mismo tamanio con background, padding, esquinas
//! redondeadas y sombra segun `project::Style`. CPU en vez de un segundo
//! shader WGSL: resultado pixel-exacto y testeable sin device de GPU (ver
//! docs/superpowers/specs/2026-08-21-fase2-editor-design.md).

use project::Style;

const SHADOW_BLUR_PX: f32 = 28.0;
const SHADOW_OFFSET_Y_PX: f32 = 12.0;
const SHADOW_OPACITY: f32 = 0.35;

/// Cantidad de muestras del blur radial (Fase 3, ver `radial_zoom_blur`). Mas
/// muestras = mas suave pero mas caro; 8 alcanza para que no se note "banding"
/// en la mancha de blur a resoluciones de export tipicas.
const MOTION_BLUR_TAPS: u32 = 8;
/// Cuanto se "arrastran" hacia el centro las muestras mas lejanas del blur a
/// `motion_blur_strength` maxima (1.0). Subir esto hace el efecto mas
/// dramatico; con 0.05 ya se ve un streak radial creible sin destruir el
/// detalle de la imagen en las esquinas.
const MOTION_BLUR_MAX_PULL: f32 = 0.05;

/// Radio del cursor reconstruido, en pixeles, a 1080p de salida — se escala
/// proporcionalmente a la resolucion real de export (ver `apply_style_ex`).
const CURSOR_RADIUS_PX_AT_1080P: f32 = 11.0;
const CURSOR_RING_EXTRA_PX_AT_1080P: f32 = 6.0;

pub struct StyledFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Cursor reconstruido a dibujar sobre el canvas final, ya resuelto a
/// coordenadas de pixel del canvas de salida (ver `content_uv_to_canvas_px`)
/// — `apply_style_ex` no sabe nada de `Rect`/crop, solo dibuja en el punto
/// que le dan.
#[derive(Debug, Clone, Copy)]
pub struct CursorMarker {
    pub x_px: f32,
    pub y_px: f32,
    pub pressed: bool,
}

struct CanvasGeometry {
    inner_w: f32,
    inner_h: f32,
    half_w: f32,
    half_h: f32,
    center_x: f32,
    center_y: f32,
}

fn canvas_geometry(w: u32, h: u32, style: &Style) -> CanvasGeometry {
    let inset = (style.padding.clamp(0.0, 0.4) * (w.min(h) as f32)).round();
    let inner_w = ((w as f32) - 2.0 * inset).max(1.0);
    let inner_h = ((h as f32) - 2.0 * inset).max(1.0);
    let half_w = inner_w / 2.0;
    let half_h = inner_h / 2.0;
    CanvasGeometry { inner_w, inner_h, half_w, half_h, center_x: inset + half_w, center_y: inset + half_h }
}

/// Convierte una posicion normalizada (0..1) **dentro del contenido ya
/// recortado/escalado** (no del frame crudo completo — quien llama es
/// responsable de pasar el cursor por el `crop_rect` de la camara primero,
/// ver `exporter::render_and_export`) a coordenadas de pixel del canvas final
/// de `w`x`h`, considerando el padding de `style`.
#[must_use]
pub fn content_uv_to_canvas_px(u: f32, v: f32, w: u32, h: u32, style: &Style) -> (f32, f32) {
    let g = canvas_geometry(w, h, style);
    (g.center_x - g.half_w + u * g.inner_w, g.center_y - g.half_h + v * g.inner_h)
}

/// Blur radial ("zoom blur"): arrastra cada pixel hacia el centro de la
/// imagen en varias muestras y promedia, simulando el streak que deja una
/// camara real al hacer zoom rapido. `strength` en 0..1 (0 = sin efecto).
/// Se aplica sobre `content` (ya recortado/escalado), antes de componerlo
/// sobre el canvas — ver ARQUITECTURA.md seccion 6 ("Motion blur en las
/// transiciones de zoom").
fn radial_zoom_blur(content: &[u8], w: u32, h: u32, strength: f32) -> Vec<u8> {
    let strength = strength.clamp(0.0, 1.0);
    // Centro en coordenadas de INDICE de pixel (no de "centro de pixel" +0.5):
    // asi, a `scale == 1.0` (tap `t == 0`, siempre sin pull), `sx`/`sy` dan
    // exactamente `x`/`y` de vuelta — sin este cuidado, redondear con el
    // offset de +0.5 desalinea el muestreo en 1px incluso sin blur.
    let cx = (w as f32 - 1.0) / 2.0;
    let cy = (h as f32 - 1.0) / 2.0;
    let mut out = vec![0u8; content.len()];

    for y in 0..h {
        let dy = y as f32 - cy;
        for x in 0..w {
            let dx = x as f32 - cx;
            let mut acc = [0f32; 4];

            for i in 0..MOTION_BLUR_TAPS {
                let t = i as f32 / (MOTION_BLUR_TAPS - 1) as f32;
                let scale = 1.0 - strength * MOTION_BLUR_MAX_PULL * t;
                let sx = ((cx + dx * scale).round() as i64).clamp(0, w as i64 - 1) as u32;
                let sy = ((cy + dy * scale).round() as i64).clamp(0, h as i64 - 1) as u32;
                let idx = ((sy * w + sx) * 4) as usize;
                for c in 0..4 {
                    acc[c] += f32::from(content[idx + c]);
                }
            }

            let idx = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                out[idx + c] = (acc[c] / MOTION_BLUR_TAPS as f32).round() as u8;
            }
        }
    }

    out
}

/// Dibuja el cursor reconstruido sobre `canvas` (BGRA8, `w`x`h`, ya compuesto)
/// en `marker.x_px`/`y_px`. Un circulo blanco con borde oscuro suave; si
/// `pressed`, se agrega un anillo de acento alrededor (feedback de click).
/// Recorre solo un cuadrado acotado alrededor del cursor, no el frame entero.
fn draw_cursor(canvas: &mut [u8], w: u32, h: u32, marker: CursorMarker) {
    let scale = h as f32 / 1080.0;
    let radius = CURSOR_RADIUS_PX_AT_1080P * scale;
    let ring_extra = CURSOR_RING_EXTRA_PX_AT_1080P * scale;
    let outer_radius = radius + if marker.pressed { ring_extra } else { 0.0 };

    let min_x = ((marker.x_px - outer_radius - 2.0).floor().max(0.0)) as u32;
    let max_x = ((marker.x_px + outer_radius + 2.0).ceil().min(w as f32 - 1.0)) as u32;
    let min_y = ((marker.y_px - outer_radius - 2.0).floor().max(0.0)) as u32;
    let max_y = ((marker.y_px + outer_radius + 2.0).ceil().min(h as f32 - 1.0)) as u32;
    if min_x > max_x || min_y > max_y {
        return; // el cursor cae fuera del canvas visible
    }

    for y in min_y..=max_y {
        let py = (y as f32 + 0.5) - marker.y_px;
        for x in min_x..=max_x {
            let px = (x as f32 + 0.5) - marker.x_px;
            let dist = px.hypot(py);
            let idx = ((y * w + x) * 4) as usize;
            let bg = [canvas[idx], canvas[idx + 1], canvas[idx + 2], canvas[idx + 3]];

            if marker.pressed {
                let ring_sdf = dist - outer_radius;
                let ring_alpha = (1.0 - ring_sdf).clamp(0.0, 1.0) * 0.55;
                if ring_sdf < 1.0 {
                    let blended = blend(bg, [80, 170, 250], ring_alpha);
                    canvas[idx] = blended[0];
                    canvas[idx + 1] = blended[1];
                    canvas[idx + 2] = blended[2];
                }
            }

            let body_sdf = dist - radius;
            let body_alpha = (1.0 - body_sdf).clamp(0.0, 1.0);
            if body_alpha > 0.0 {
                let outline_alpha = (1.0 - (dist - (radius - 1.5)).abs()).clamp(0.0, 1.0) * 0.5;
                let fill = blend([bg[0], bg[1], bg[2], 255], [255, 255, 255], body_alpha);
                let outlined = blend(fill, [20, 20, 20], outline_alpha.min(body_alpha));
                canvas[idx] = outlined[0];
                canvas[idx + 1] = outlined[1];
                canvas[idx + 2] = outlined[2];
            }
        }
    }
}

type Rgb = (u8, u8, u8);

fn preset_stops(preset: &str) -> (Rgb, Rgb) {
    const DEFAULT: (Rgb, Rgb) = ((60, 20, 90), (20, 60, 140));
    match preset {
        "gradient-01" => DEFAULT,
        "gradient-02" => ((10, 60, 55), (30, 140, 110)),
        "gradient-03" => ((230, 90, 40), (210, 40, 120)),
        "gradient-04" => ((30, 30, 35), (10, 10, 12)),
        "gradient-05" => ((235, 110, 40), (100, 30, 120)),
        "gradient-06" => ((40, 110, 200), (30, 200, 220)),
        _ => DEFAULT,
    }
}

/// Color BGRA de fondo para la fila `y` de `height` (gradiente vertical de
/// 2 stops, interpolacion lineal). Preset desconocido cae en `gradient-01`.
fn background_bgra_at(preset: &str, y: u32, height: u32) -> [u8; 4] {
    let (top, bottom) = preset_stops(preset);
    let t = if height <= 1 { 0.0 } else { y as f32 / (height - 1) as f32 };
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    [lerp(top.2, bottom.2), lerp(top.1, bottom.1), lerp(top.0, bottom.0), 255]
}

/// Signed distance function de un rectangulo redondeado centrado en el
/// origen (Inigo Quilez, tecnica estandar): negativo adentro, 0 en el
/// borde, positivo afuera.
fn rounded_rect_sdf(px: f32, py: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let r = r.min(half_w).min(half_h).max(0.0);
    let qx = px.abs() - half_w + r;
    let qy = py.abs() - half_h + r;
    qx.max(qy).min(0.0) + qx.max(0.0).hypot(qy.max(0.0)) - r
}

fn blend(bg: [u8; 4], fg: [u8; 3], alpha: f32) -> [u8; 4] {
    let a = alpha.clamp(0.0, 1.0);
    let mix = |b: u8, f: u8| (b as f32 * (1.0 - a) + f as f32 * a).round() as u8;
    [mix(bg[0], fg[0]), mix(bg[1], fg[1]), mix(bg[2], fg[2]), 255]
}

/// Como `apply_style_ex`, pero sin motion blur ni cursor (compatibilidad con
/// los call sites/tests de la Fase 2 que no necesitan esos efectos).
pub fn apply_style(content: &[u8], w: u32, h: u32, style: &Style) -> StyledFrame {
    apply_style_ex(content, w, h, style, 0.0, None)
}

/// Compone `content` (BGRA8, `w`x`h`, ya recortado/escalado por
/// `Compositor::composite_frame`) sobre un canvas del mismo tamanio segun
/// `style`: el contenido se achica hacia adentro segun `style.padding`
/// (fraccion del lado menor), con esquinas redondeadas (`style.corner_radius`,
/// pixeles literales en la resolucion de salida) y una sombra suave si
/// `style.shadow` esta activo.
///
/// `motion_blur_strength` (0..1) aplica un blur radial sobre `content` antes
/// de componerlo — quien llama (`exporter::render_and_export`) lo calcula a
/// partir de la velocidad de la camara y solo si `style.motion_blur` esta
/// activo (aca no se vuelve a chequear el flag, para que este modulo se
/// pueda testear sin necesidad de armar un `Style` completo por caso).
/// `cursor` dibuja el cursor reconstruido si esta presente; quien llama solo
/// deberia pasarlo si `style.cursor_smoothing` esta activo.
pub fn apply_style_ex(
    content: &[u8],
    w: u32,
    h: u32,
    style: &Style,
    motion_blur_strength: f32,
    cursor: Option<CursorMarker>,
) -> StyledFrame {
    assert_eq!(content.len(), (w as usize) * (h as usize) * 4, "content debe ser BGRA8 de w*h*4 bytes");

    let blurred;
    let content = if motion_blur_strength > 0.001 {
        blurred = radial_zoom_blur(content, w, h, motion_blur_strength);
        &blurred[..]
    } else {
        content
    };

    let g = canvas_geometry(w, h, style);
    let (inner_w, inner_h, half_w, half_h, center_x, center_y) =
        (g.inner_w, g.inner_h, g.half_w, g.half_h, g.center_x, g.center_y);
    let radius = style.corner_radius.max(0.0);

    let mut out = vec![0u8; content.len()];

    for y in 0..h {
        let bg = background_bgra_at(&style.background, y, h);
        let py = (y as f32 + 0.5) - center_y;

        for x in 0..w {
            let idx = ((y * w + x) * 4) as usize;
            let px = (x as f32 + 0.5) - center_x;
            let content_sdf = rounded_rect_sdf(px, py, half_w, half_h, radius);

            let pixel = if content_sdf < 0.5 {
                let u = ((px + half_w) / inner_w).clamp(0.0, 1.0);
                let v = ((py + half_h) / inner_h).clamp(0.0, 1.0);
                let src_x = ((u * (w as f32 - 1.0)).round() as u32).min(w - 1);
                let src_y = ((v * (h as f32 - 1.0)).round() as u32).min(h - 1);
                let src_idx = ((src_y * w + src_x) * 4) as usize;
                let content_px = [content[src_idx], content[src_idx + 1], content[src_idx + 2]];

                if content_sdf <= -0.5 {
                    [content_px[0], content_px[1], content_px[2], 255]
                } else {
                    let edge_alpha = 1.0 - (content_sdf + 0.5);
                    blend(bg, content_px, edge_alpha)
                }
            } else if style.shadow {
                let shadow_sdf = rounded_rect_sdf(px, py - SHADOW_OFFSET_Y_PX, half_w, half_h, radius);
                let shadow_alpha = (1.0 - shadow_sdf / SHADOW_BLUR_PX).clamp(0.0, 1.0) * SHADOW_OPACITY;
                blend(bg, [0, 0, 0], shadow_alpha)
            } else {
                bg
            };

            out[idx] = pixel[0];
            out[idx + 1] = pixel[1];
            out[idx + 2] = pixel[2];
            out[idx + 3] = 255;
        }
    }

    if let Some(marker) = cursor {
        draw_cursor(&mut out, w, h, marker);
    }

    StyledFrame { width: w, height: h, bgra: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: u32 = 100;

    fn solid_content(color: [u8; 3]) -> Vec<u8> {
        solid_content_sized(color, SIZE)
    }

    fn solid_content_sized(color: [u8; 3], size: u32) -> Vec<u8> {
        let mut buf = vec![0u8; (size * size * 4) as usize];
        for px in buf.chunks_mut(4) {
            px[0] = color[0]; // B
            px[1] = color[1]; // G
            px[2] = color[2]; // R
            px[3] = 255;
        }
        buf
    }

    fn pixel_at(frame: &StyledFrame, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * frame.width + x) * 4) as usize;
        [frame.bgra[idx], frame.bgra[idx + 1], frame.bgra[idx + 2], frame.bgra[idx + 3]]
    }

    #[test]
    fn no_padding_no_radius_no_shadow_reproduces_content_at_the_center() {
        let content = solid_content([200, 150, 100]);
        let style =
            Style { background: "gradient-01".into(), padding: 0.0, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);
        assert_eq!(pixel_at(&out, 50, 50), [200, 150, 100, 255]);
    }

    #[test]
    fn padding_shows_background_at_the_corner_and_content_at_the_center() {
        let content = solid_content([10, 10, 10]);
        let style =
            Style { background: "gradient-04".into(), padding: 0.2, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);

        let expected_corner_bg = background_bgra_at("gradient-04", 0, SIZE);
        assert_eq!(pixel_at(&out, 0, 0), expected_corner_bg, "la esquina debe ser background, no contenido");
        assert_eq!(pixel_at(&out, 50, 50), [10, 10, 10, 255], "el centro debe seguir siendo contenido");
    }

    #[test]
    fn large_corner_radius_cuts_the_extreme_corner_to_background() {
        let content = solid_content([255, 255, 255]);
        let style =
            Style { background: "gradient-04".into(), padding: 0.0, corner_radius: 20.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);

        let expected_corner_bg = background_bgra_at("gradient-04", 0, SIZE);
        assert_eq!(pixel_at(&out, 0, 0), expected_corner_bg, "la esquina extrema debe quedar cortada por el radio");
        assert_eq!(pixel_at(&out, 50, 50), [255, 255, 255, 255], "el centro no debe verse afectado por el radio");
    }

    #[test]
    fn shadow_disabled_leaves_pure_background_outside_the_content() {
        let content = solid_content([255, 255, 255]);
        let style =
            Style { background: "gradient-04".into(), padding: 0.3, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);
        assert_eq!(pixel_at(&out, 0, 0), background_bgra_at("gradient-04", 0, SIZE));
    }

    #[test]
    fn shadow_enabled_darkens_the_background_right_next_to_the_content() {
        let content = solid_content([255, 255, 255]);
        let style_no_shadow =
            Style { background: "gradient-04".into(), padding: 0.2, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let style_shadow = Style { shadow: true, ..style_no_shadow.clone() };

        let without = apply_style(&content, SIZE, SIZE, &style_no_shadow);
        let with = apply_style(&content, SIZE, SIZE, &style_shadow);

        let inset = (0.2_f32 * SIZE as f32).round() as u32;
        let below_content_y = (SIZE - inset + 4).min(SIZE - 1);
        let x = SIZE / 2;

        let px_without = pixel_at(&without, x, below_content_y);
        let px_with = pixel_at(&with, x, below_content_y);
        assert!(
            px_with[0] < px_without[0] && px_with[1] < px_without[1] && px_with[2] < px_without[2],
            "con sombra activa el pixel deberia ser mas oscuro: sin={:?} con={:?}",
            px_without,
            px_with
        );
    }

    #[test]
    fn unknown_background_preset_falls_back_to_gradient_01() {
        assert_eq!(background_bgra_at("no-existe", 0, 100), background_bgra_at("gradient-01", 0, 100));
    }

    #[test]
    fn zero_motion_blur_strength_reproduces_apply_style_exactly() {
        let content = solid_content([200, 150, 100]);
        let style = Style { motion_blur: true, ..Style::default() };
        let with_zero = apply_style_ex(&content, SIZE, SIZE, &style, 0.0, None);
        let without = apply_style(&content, SIZE, SIZE, &style);
        assert_eq!(with_zero.bgra, without.bgra);
    }

    #[test]
    fn motion_blur_leaves_the_exact_center_pixel_unchanged() {
        // El centro de la imagen es el foco del blur radial: la muestra en
        // dx=dy=0 nunca se mueve, sea cual sea `strength`. Se usa un lado
        // impar (101) para que el centro geometrico caiga justo en el centro
        // de un pixel (con un lado par, cae entre dos pixeles y ninguno tiene
        // dx/dy exactamente 0).
        const ODD_SIZE: u32 = 101;
        let mut content = vec![0u8; (ODD_SIZE * ODD_SIZE * 4) as usize];
        for px in content.chunks_mut(4) {
            px[3] = 255;
        }
        let center = (ODD_SIZE / 2) as usize;
        let idx = (center * ODD_SIZE as usize + center) * 4;
        content[idx] = 255;
        content[idx + 1] = 255;
        content[idx + 2] = 255;

        let blurred = radial_zoom_blur(&content, ODD_SIZE, ODD_SIZE, 1.0);
        assert_eq!(&blurred[idx..idx + 3], &content[idx..idx + 3]);
    }

    #[test]
    fn motion_blur_dims_a_corner_pixel_by_averaging_it_with_darker_samples_pulled_toward_the_center() {
        let mut content = solid_content([0, 0, 0]);
        // Un pixel brillante en la esquina (lejos del centro): el blur radial
        // promedia su propia muestra (tap sin pull) con muestras traidas
        // hacia el centro, que caen en vecinos oscuros — el resultado deberia
        // ser mas oscuro que el original, no seguir en blanco puro.
        let bright_idx = 0usize;
        content[bright_idx] = 255;
        content[bright_idx + 1] = 255;
        content[bright_idx + 2] = 255;

        let blurred = radial_zoom_blur(&content, SIZE, SIZE, 1.0);
        assert!(blurred[bright_idx] < 255, "la esquina brillante deberia atenuarse, fue {}", blurred[bright_idx]);
    }

    #[test]
    fn content_uv_to_canvas_px_maps_the_center_to_the_canvas_center_regardless_of_padding() {
        let style = Style { padding: 0.1, ..Style::default() };
        let (x, y) = content_uv_to_canvas_px(0.5, 0.5, 200, 100, &style);
        assert!((x - 100.0).abs() < 1.0);
        assert!((y - 50.0).abs() < 1.0);
    }

    // Los tests de cursor usan un canvas a 1080p (no `SIZE`): las constantes
    // de radio del cursor estan definidas en pixeles "a 1080p de salida" y se
    // reescalan por `h/1080.0`, asi que a `SIZE` (100px) el cursor mediria
    // ~1px de radio y estos tests serian invisibles/fragiles.
    const CURSOR_TEST_SIZE: u32 = 1080;

    #[test]
    fn draw_cursor_paints_white_at_its_center_over_a_dark_background() {
        let content = solid_content_sized([10, 10, 10], CURSOR_TEST_SIZE);
        let style = Style { shadow: false, ..Style::default() };
        let marker = CursorMarker { x_px: 540.0, y_px: 540.0, pressed: false };
        let out = apply_style_ex(&content, CURSOR_TEST_SIZE, CURSOR_TEST_SIZE, &style, 0.0, Some(marker));
        let px = pixel_at(&out, 540, 540);
        assert!(px[0] > 200 && px[1] > 200 && px[2] > 200, "el centro del cursor deberia ser blanco, fue {px:?}");
    }

    #[test]
    fn draw_cursor_pressed_adds_a_ring_further_out_than_the_unpressed_cursor() {
        let content = solid_content_sized([10, 10, 10], CURSOR_TEST_SIZE);
        let style = Style { shadow: false, ..Style::default() };
        let unpressed = CursorMarker { x_px: 540.0, y_px: 540.0, pressed: false };
        let pressed = CursorMarker { x_px: 540.0, y_px: 540.0, pressed: true };

        let out_unpressed = apply_style_ex(&content, CURSOR_TEST_SIZE, CURSOR_TEST_SIZE, &style, 0.0, Some(unpressed));
        let out_pressed = apply_style_ex(&content, CURSOR_TEST_SIZE, CURSOR_TEST_SIZE, &style, 0.0, Some(pressed));

        // A una distancia entre el radio del cuerpo (~11px) y el radio del
        // anillo (~17px), "pressed" deberia pintar el anillo de acento
        // mientras que "unpressed" deja ese pixel intacto (fondo plano).
        let ring_only_x = 540 + CURSOR_RADIUS_PX_AT_1080P as u32 + 2;
        let unpressed_px = pixel_at(&out_unpressed, ring_only_x, 540);
        let pressed_px = pixel_at(&out_pressed, ring_only_x, 540);
        assert_ne!(unpressed_px, pressed_px, "el anillo de click deberia pintar algo que 'unpressed' no pinta");
    }
}
