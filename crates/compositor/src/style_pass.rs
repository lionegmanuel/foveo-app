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

pub struct StyledFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
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

/// Compone `content` (BGRA8, `w`x`h`, ya recortado/escalado por
/// `Compositor::composite_frame`) sobre un canvas del mismo tamanio segun
/// `style`: el contenido se achica hacia adentro segun `style.padding`
/// (fraccion del lado menor), con esquinas redondeadas (`style.corner_radius`,
/// pixeles literales en la resolucion de salida) y una sombra suave si
/// `style.shadow` esta activo. `style.motion_blur`/`cursor_smoothing` no se
/// tocan aca (Fase 3).
pub fn apply_style(content: &[u8], w: u32, h: u32, style: &Style) -> StyledFrame {
    assert_eq!(content.len(), (w as usize) * (h as usize) * 4, "content debe ser BGRA8 de w*h*4 bytes");

    let inset = (style.padding.clamp(0.0, 0.4) * (w.min(h) as f32)).round();
    let inner_w = ((w as f32) - 2.0 * inset).max(1.0);
    let inner_h = ((h as f32) - 2.0 * inset).max(1.0);
    let half_w = inner_w / 2.0;
    let half_h = inner_h / 2.0;
    let center_x = inset + half_w;
    let center_y = inset + half_h;
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

    StyledFrame { width: w, height: h, bgra: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: u32 = 100;

    fn solid_content(color: [u8; 3]) -> Vec<u8> {
        let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];
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
}
