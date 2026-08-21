# Fase 2 — Editor post-grabación Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dar al usuario control manual sobre el zoom automático de Fase 1 (timeline editable de keyframes) y hacer real el panel de estilo (background/padding/esquinas/sombra), que hoy existe en el schema pero no se renderiza en ningún lado.

**Architecture:** El compositor gana un segundo paso de composición en CPU (`style_pass`, sin GPU/shader nuevo — más simple de testear pixel-exacto) que corre después de `Compositor::composite_frame`. El backend expone 5 comandos Tauri nuevos (leer proyecto, renderizar preview de baja resolución en un timestamp dado, y CRUD de keyframes). El frontend agrega un overlay de arrastre sobre el preview + una pista de timeline + un panel de estilo, todo en React puro sin librerías nuevas.

**Tech Stack:** Rust (wgpu ya existente + CPU puro para `style_pass`), `png`/`base64` (nuevos, solo en `screenzoom_desktop`), React 19 + TypeScript + Tailwind (ya existentes, sin deps nuevas).

**Spec:** `docs/superpowers/specs/2026-08-21-fase2-editor-design.md`

## Global Constraints

- Ningún crate en `crates/` puede depender de `tauri` (CLAUDE.md regla 2).
- Ninguna llamada bloqueante (decode/GPU/ffmpeg) puede correr en el hilo de eventos de Tauri — todo comando pesado usa `spawn_blocking`/`std::thread::spawn` (CLAUDE.md regla 1).
- `cargo clippy --workspace --all-targets -- -D warnings` y `cargo test --workspace` deben quedar limpios antes de cada commit.
- `pnpm exec tsc -b --noEmit` limpio antes de commitear cambios de frontend.
- Sin librerías de canvas/drag nuevas en el frontend (no hay Konva/Fabric en `package.json`, no hace falta para un solo rect arrastrable).
- Coordenadas de `Rect` siempre normalizadas 0..1, clampeadas antes de persistir.

---

### Task 1: `Rect::clamp_into_unit_square`

**Files:**
- Modify: `crates/project/src/lib.rs` (agregar método a `impl Rect`, cerca de `FULL_FRAME`, línea ~107)

**Interfaces:**
- Produces: `Rect::clamp_into_unit_square(self) -> Rect` — usado por Task 6 para validar ediciones manuales de keyframes antes de persistir.

- [ ] **Step 1: Escribir los tests que fallan**

Agregar al módulo `#[cfg(test)] mod tests` existente de `crates/project/src/lib.rs`:

```rust
#[test]
fn clamp_into_unit_square_leaves_a_rect_already_inside_untouched() {
    let r = Rect { x: 0.2, y: 0.3, w: 0.4, h: 0.5 };
    assert_eq!(r.clamp_into_unit_square(), r);
}

#[test]
fn clamp_into_unit_square_slides_a_rect_that_overflows_the_right_edge() {
    let r = Rect { x: 0.8, y: 0.1, w: 0.5, h: 0.2 };
    let clamped = r.clamp_into_unit_square();
    assert_eq!(clamped.w, 0.5);
    assert!((clamped.x - 0.5).abs() < 1e-6, "x debe deslizarse a 1.0 - w = 0.5, fue {}", clamped.x);
}

#[test]
fn clamp_into_unit_square_shrinks_a_rect_wider_than_the_frame() {
    let r = Rect { x: 0.5, y: 0.0, w: 1.5, h: 0.3 };
    let clamped = r.clamp_into_unit_square();
    assert_eq!(clamped.w, 1.0);
    assert_eq!(clamped.x, 0.0);
}

#[test]
fn clamp_into_unit_square_clamps_negative_origin_to_zero() {
    let r = Rect { x: -0.3, y: -0.1, w: 0.2, h: 0.2 };
    let clamped = r.clamp_into_unit_square();
    assert_eq!(clamped.x, 0.0);
    assert_eq!(clamped.y, 0.0);
}
```

- [ ] **Step 2: Correr y confirmar que falla**

Run: `cargo test -p project clamp_into_unit_square`
Expected: FAIL (`no method named clamp_into_unit_square`)

- [ ] **Step 3: Implementar**

En `impl Rect` (junto a `FULL_FRAME`):

```rust
impl Rect {
    pub const FULL_FRAME: Rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    /// Clampea el rect para que quede completamente dentro de [0,1]x[0,1]:
    /// primero acota w/h a como mucho 1.0, despues desliza x/y para que
    /// x+w <= 1.0 y y+h <= 1.0 sin cambiar el tamanio ya clampeado.
    #[must_use]
    pub fn clamp_into_unit_square(self) -> Rect {
        let w = self.w.clamp(0.0, 1.0);
        let h = self.h.clamp(0.0, 1.0);
        let x = self.x.clamp(0.0, 1.0 - w);
        let y = self.y.clamp(0.0, 1.0 - h);
        Rect { x, y, w, h }
    }
}
```

- [ ] **Step 4: Correr y confirmar que pasa**

Run: `cargo test -p project clamp_into_unit_square`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add crates/project/src/lib.rs
git commit -m "project: agrega Rect::clamp_into_unit_square para validar ediciones manuales"
```

---

### Task 2: `RawFrameReader::spawn_seeked`

**Files:**
- Modify: `crates/compositor/src/decode.rs`

**Interfaces:**
- Consumes: nada nuevo (mismo `Command`/`Stdio` que `RawFrameReader::spawn`).
- Produces: `RawFrameReader::spawn_seeked(input_path: &Path, width: u32, height: u32, seek_ms: u64) -> Result<Self, DecodeError>` — usado por Task 5 (`render_preview_frame`) para decodificar un solo frame cerca de `t_ms` sin decodificar toda la toma desde el principio.

- [ ] **Step 1: Escribir el test que falla**

Agregar `#[cfg(test)] mod tests` a `crates/compositor/src/decode.rs` (no existe todavía):

```rust
#[cfg(test)]
mod tests {
    use super::seek_arg;

    #[test]
    fn seek_arg_formats_milliseconds_as_seconds_with_millisecond_precision() {
        assert_eq!(seek_arg(0), "0.000");
        assert_eq!(seek_arg(1_500), "1.500");
        assert_eq!(seek_arg(12_345), "12.345");
    }
}
```

- [ ] **Step 2: Correr y confirmar que falla**

Run: `cargo test -p compositor seek_arg`
Expected: FAIL (`unresolved import` / `seek_arg` no existe)

- [ ] **Step 3: Implementar**

Agregar a `crates/compositor/src/decode.rs`, antes de `impl RawFrameReader`:

```rust
/// Formatea `ms` como segundos con precision de milisegundos para el flag
/// `-ss` de ffmpeg (ej. `1500` -> `"1.500"`).
fn seek_arg(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}
```

Y dentro de `impl RawFrameReader`, junto a `spawn`:

```rust
/// Como `spawn`, pero arranca la decodificacion desde `seek_ms` en vez del
/// principio del archivo — usa el seek rapido de ffmpeg (`-ss` antes de
/// `-i`, por keyframes, no frame-exacto) para no tener que leer toda la
/// toma cruda solo para renderizar un frame de preview cerca del final.
pub fn spawn_seeked(input_path: &Path, width: u32, height: u32, seek_ms: u64) -> Result<Self, DecodeError> {
    let mut child = Command::new(find_ffmpeg_binary())
        .args(["-v", "error", "-ss", &seek_arg(seek_ms), "-i"])
        .arg(input_path)
        .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout fue pedido como piped");
    let frame_size = (width as usize) * (height as usize) * 4;

    Ok(Self { child, stdout, frame_size })
}
```

- [ ] **Step 4: Correr y confirmar que pasa**

Run: `cargo test -p compositor seek_arg`
Expected: PASS (3 tests)

Run también (regresión, sin GPU/ffmpeg real): `cargo clippy -p compositor --all-targets -- -D warnings`
Expected: limpio

- [ ] **Step 5: Commit**

```bash
git add crates/compositor/src/decode.rs
git commit -m "compositor: agrega RawFrameReader::spawn_seeked para decodificar preview por timestamp"
```

---

### Task 3: `style_pass` — background/padding/esquinas/sombra en CPU

**Files:**
- Create: `crates/compositor/src/style_pass.rs`
- Modify: `crates/compositor/src/lib.rs` (agregar `pub mod style_pass;` + re-export)

**Interfaces:**
- Consumes: `project::Style` (ya existe: `background: String, padding: f32, corner_radius: f32, shadow: bool, cursor_smoothing: bool, motion_blur: bool`).
- Produces: `pub fn apply_style(content: &[u8], w: u32, h: u32, style: &Style) -> StyledFrame` y `pub struct StyledFrame { pub width: u32, pub height: u32, pub bgra: Vec<u8> }` — usados por Task 4 (`exporter::render_and_export`) y Task 5 (`render_preview_frame`). `content` es BGRA8 `w*h*4` bytes, la salida de `Compositor::composite_frame`.

Nota de diseño (se aparta ligeramente del spec original en un detalle no observable desde afuera): en vez de un segundo shader WGSL, `apply_style` es CPU puro — mismo resultado visual, pero determinista y testeable sin device de GPU. `corner_radius` se trata como píxeles literales en la resolución de salida (sin escalado por "resolución de referencia" — no lo pedía el spec, YAGNI).

- [ ] **Step 1: Escribir los tests que fallan**

Crear `crates/compositor/src/style_pass.rs`:

```rust
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
        let style = Style { background: "gradient-01".into(), padding: 0.0, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);
        assert_eq!(pixel_at(&out, 50, 50), [200, 150, 100, 255]);
    }

    #[test]
    fn padding_shows_background_at_the_corner_and_content_at_the_center() {
        let content = solid_content([10, 10, 10]);
        let style = Style { background: "gradient-04".into(), padding: 0.2, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);

        let expected_corner_bg = background_bgra_at("gradient-04", 0, SIZE);
        assert_eq!(pixel_at(&out, 0, 0), expected_corner_bg, "la esquina debe ser background, no contenido");
        assert_eq!(pixel_at(&out, 50, 50), [10, 10, 10, 255], "el centro debe seguir siendo contenido");
    }

    #[test]
    fn large_corner_radius_cuts_the_extreme_corner_to_background() {
        let content = solid_content([255, 255, 255]);
        let style = Style { background: "gradient-04".into(), padding: 0.0, corner_radius: 20.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);

        let expected_corner_bg = background_bgra_at("gradient-04", 0, SIZE);
        assert_eq!(pixel_at(&out, 0, 0), expected_corner_bg, "la esquina extrema debe quedar cortada por el radio");
        assert_eq!(pixel_at(&out, 50, 50), [255, 255, 255, 255], "el centro no debe verse afectado por el radio");
    }

    #[test]
    fn shadow_disabled_leaves_pure_background_outside_the_content() {
        let content = solid_content([255, 255, 255]);
        let style = Style { background: "gradient-04".into(), padding: 0.3, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let out = apply_style(&content, SIZE, SIZE, &style);
        assert_eq!(pixel_at(&out, 0, 0), background_bgra_at("gradient-04", 0, SIZE));
    }

    #[test]
    fn shadow_enabled_darkens_the_background_right_next_to_the_content() {
        let content = solid_content([255, 255, 255]);
        let style_no_shadow = Style { background: "gradient-04".into(), padding: 0.2, corner_radius: 0.0, shadow: false, cursor_smoothing: false, motion_blur: false };
        let style_shadow = Style { shadow: true, ..style_no_shadow.clone() };

        let without = apply_style(&content, SIZE, SIZE, &style_no_shadow);
        let with = apply_style(&content, SIZE, SIZE, &style_shadow);

        // Un pixel justo debajo del borde inferior del contenido (donde cae
        // la sombra, offset hacia abajo) debe oscurecerse vs. sin sombra.
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
```

Nota: `Style` necesita `Clone` para el test `shadow_enabled_darkens...` (`..style_no_shadow.clone()`) — ya deriva `Clone` en `crates/project/src/lib.rs:129` (`#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] pub struct Style`), no hace falta tocar el schema.

- [ ] **Step 2: Correr y confirmar que falla**

Run: `cargo test -p compositor style_pass`
Expected: FAIL (el módulo no está declarado todavía en `lib.rs`)

- [ ] **Step 3: Declarar el módulo y re-exportar**

En `crates/compositor/src/lib.rs`:

```rust
pub mod camera_path;
pub mod decode;
pub mod render;
pub mod style_pass;

pub use camera_path::camera_rect_at;
pub use decode::{DecodeError, RawFrameReader};
pub use render::{Compositor, CompositorError, PIXEL_FORMAT};
pub use style_pass::{apply_style, StyledFrame};
```

- [ ] **Step 4: Correr y confirmar que pasa**

Run: `cargo test -p compositor style_pass`
Expected: PASS (6 tests)

Run también: `cargo clippy -p compositor --all-targets -- -D warnings`
Expected: limpio

- [ ] **Step 5: Commit**

```bash
git add crates/compositor/src/style_pass.rs crates/compositor/src/lib.rs
git commit -m "compositor: agrega style_pass (background/padding/esquinas/sombra) en CPU"
```

---

### Task 4: Wire `apply_style` en `render_and_export`

**Files:**
- Modify: `crates/exporter/src/lib.rs:129-138` (loop principal de `render_and_export`)

**Interfaces:**
- Consumes: `compositor::apply_style(content: &[u8], w: u32, h: u32, style: &Style) -> StyledFrame` (Task 3).

- [ ] **Step 1: Actualizar el import**

En `crates/exporter/src/lib.rs:13`, cambiar:

```rust
use compositor::{Compositor, RawFrameReader, camera_rect_at};
```

por:

```rust
use compositor::{Compositor, RawFrameReader, apply_style, camera_rect_at};
```

- [ ] **Step 2: Aplicar el style pass en el loop de render**

En `render_and_export`, reemplazar:

```rust
        let composed = compositor.composite_frame(&frame, crop_rect)?;
        exporter.write_frame(&composed)?;
```

por:

```rust
        let composed = compositor.composite_frame(&frame, crop_rect)?;
        let styled = apply_style(&composed, out_width, out_height, &project.style);
        exporter.write_frame(&styled.bgra)?;
```

- [ ] **Step 3: Correr los tests existentes (regresión, no agrega tests nuevos)**

Run: `cargo test -p exporter`
Expected: PASS (sin tests unitarios propios hoy; confirma que compila)

Run: `cargo clippy -p exporter --all-targets -- -D warnings`
Expected: limpio

Run (opcional, si `spikes-output/fase0_capture.mp4` existe de la Fase 0 — end-to-end real con GPU+ffmpeg reales):
`cargo test -p exporter --test render_and_export -- --ignored --nocapture`
Expected: PASS — el mp4 exportado ahora tiene padding/esquinas/sombra por default (`Style::default()`), sigue decodificando limpio.

- [ ] **Step 4: Commit**

```bash
git add crates/exporter/src/lib.rs
git commit -m "exporter: aplica style_pass (Task 3) en render_and_export"
```

---

### Task 5: Comando Tauri `render_preview_frame`

**Files:**
- Modify: `apps/desktop/src-tauri/Cargo.toml` (agregar deps `compositor`, `png`, `base64`)
- Create: `apps/desktop/src-tauri/src/commands/preview.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`

**Interfaces:**
- Consumes: `project::Project::load`, `compositor::{RawFrameReader, Compositor, camera_rect_at, apply_style}` (Tasks 2-4).
- Produces: comando Tauri `render_preview_frame(project_path: String, t_ms: u64, max_width: u32) -> Result<String, String>` (PNG codificado en base64, sin el prefijo `data:image/png;base64,`) — usado por Task 12 (frontend).

- [ ] **Step 1: Agregar dependencias**

Run: `cargo add compositor --path ../../../crates/compositor -p screenzoom_desktop`
Run: `cargo add png -p screenzoom_desktop`
Run: `cargo add base64 -p screenzoom_desktop`

Verificar que `apps/desktop/src-tauri/Cargo.toml` quedó con las 3 líneas nuevas en `[dependencies]`.

- [ ] **Step 2: Implementar el comando**

Crear `apps/desktop/src-tauri/src/commands/preview.rs`:

```rust
//! Comando Tauri de preview: renderiza un solo frame (decode -> camara ->
//! composite -> style) a baja resolucion para el editor de timeline (Fase
//! 2). Corre en un thread dedicado via `spawn_blocking` (CLAUDE.md regla
//! 1) — decodificar+GPU+PNG-encode no debe bloquear el hilo de eventos.

use std::io::Cursor;

use base64::Engine;
use compositor::{Compositor, RawFrameReader, apply_style, camera_rect_at};
use project::Project;

fn render_preview_frame_sync(project_path: &str, t_ms: u64, max_width: u32) -> Result<String, String> {
    let project = Project::load(project_path).map_err(|e| e.to_string())?;

    let mut keyframes = project.zoom_keyframes.clone();
    keyframes.sort_by_key(|k| k.start_ms);

    let in_width = project.raw_take.resolution.width;
    let in_height = project.raw_take.resolution.height;
    let aspect = in_height as f32 / in_width as f32;
    let out_width = max_width.max(2);
    let out_height = ((out_width as f32) * aspect).round().max(2.0) as u32;

    let mut reader = RawFrameReader::spawn_seeked(&project.raw_take.path, in_width, in_height, t_ms)
        .map_err(|e| e.to_string())?;
    let frame = reader
        .next_frame()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no se pudo leer ningun frame en ese timestamp".to_string())?;

    let mut compositor = Compositor::new(in_width, in_height, out_width, out_height).map_err(|e| e.to_string())?;
    let crop_rect = camera_rect_at(&keyframes, t_ms);
    let composed = compositor.composite_frame(&frame, crop_rect).map_err(|e| e.to_string())?;
    let styled = apply_style(&composed, out_width, out_height, &project.style);

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut png_bytes), out_width, out_height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;

        // png espera RGBA; el pipeline entero es BGRA — convertir en el
        // borde de salida, no en la pipeline compartida (ver
        // crates/compositor/src/render.rs: BGRA se eligio para no convertir
        // entre windows-capture/ffmpeg, esto es solo para mostrar en UI).
        let mut rgba = styled.bgra;
        for px in rgba.chunks_mut(4) {
            px.swap(0, 2);
        }
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }

    Ok(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
}

#[tauri::command]
pub fn render_preview_frame(project_path: String, t_ms: u64, max_width: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || render_preview_frame_sync(&project_path, t_ms, max_width))
        .join()
        .map_err(|_| "el thread de render de preview panickeo".to_string())?
}
```

- [ ] **Step 3: Registrar el módulo**

En `apps/desktop/src-tauri/src/commands/mod.rs`:

```rust
pub mod export;
pub mod keyframes;
pub mod preview;
pub mod recording;
```

(la línea `pub mod keyframes;` la agrega Task 6 — si Task 6 todavía no corrió, dejar solo `pub mod preview;` agregada y ajustar en Task 6).

- [ ] **Step 4: Compilar y lintear**

Run: `cargo build -p screenzoom_desktop`
Expected: compila (el comando no está registrado en `main.rs` todavía — eso es Task 7 — así que no hace falta que funcione end-to-end aún, solo que compile)

Run: `cargo clippy -p screenzoom_desktop --all-targets -- -D warnings`
Expected: limpio

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/src/commands/preview.rs apps/desktop/src-tauri/src/commands/mod.rs
git commit -m "desktop: agrega comando render_preview_frame"
```

---

### Task 6: Comandos Tauri de lectura/edición de proyecto

**Files:**
- Create: `apps/desktop/src-tauri/src/commands/keyframes.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs` (agregar `pub mod keyframes;` si Task 5 no lo agregó ya)

**Interfaces:**
- Consumes: `project::{Project, ZoomKeyframe, Rect, Style}`, `Rect::clamp_into_unit_square` (Task 1).
- Produces: comandos Tauri `get_project(project_path: String) -> Result<Project, String>`, `update_keyframe(project_path: String, keyframe: ZoomKeyframe) -> Result<(), String>`, `add_keyframe(project_path: String, keyframe: ZoomKeyframe) -> Result<(), String>`, `delete_keyframe(project_path: String, keyframe_id: String) -> Result<(), String>`, `update_style(project_path: String, style: Style) -> Result<(), String>` — usados por Task 12 (frontend).

- [ ] **Step 1: Escribir los tests que fallan**

Crear `apps/desktop/src-tauri/src/commands/keyframes.rs`:

```rust
//! Comandos Tauri de lectura/edicion del `.szproj` para el editor de
//! timeline (Fase 2). Toda edicion manual clampea el rect (Task 1 de
//! crates/project) y marca `KeyframeSource::Manual` para que un futuro
//! recalculo automatico del Zoom Engine no la pise.

use project::{KeyframeSource, Project, Style, ZoomKeyframe};

fn load_project(project_path: &str) -> Result<Project, String> {
    Project::load(project_path).map_err(|e| e.to_string())
}

fn save_project(project_path: &str, project: &Project) -> Result<(), String> {
    project.save(project_path).map_err(|e| e.to_string())
}

fn upsert_keyframe(mut keyframe: ZoomKeyframe, mut project: Project) -> Project {
    keyframe.target_rect = keyframe.target_rect.clamp_into_unit_square();
    keyframe.source = KeyframeSource::Manual;

    if let Some(existing) = project.zoom_keyframes.iter_mut().find(|k| k.id == keyframe.id) {
        *existing = keyframe;
    } else {
        project.zoom_keyframes.push(keyframe);
    }
    project
}

#[tauri::command]
pub fn get_project(project_path: String) -> Result<Project, String> {
    load_project(&project_path)
}

#[tauri::command]
pub fn update_keyframe(project_path: String, keyframe: ZoomKeyframe) -> Result<(), String> {
    let project = load_project(&project_path)?;
    let project = upsert_keyframe(keyframe, project);
    save_project(&project_path, &project)
}

#[tauri::command]
pub fn add_keyframe(project_path: String, keyframe: ZoomKeyframe) -> Result<(), String> {
    update_keyframe(project_path, keyframe)
}

#[tauri::command]
pub fn delete_keyframe(project_path: String, keyframe_id: String) -> Result<(), String> {
    let mut project = load_project(&project_path)?;
    project.zoom_keyframes.retain(|k| k.id != keyframe_id);
    save_project(&project_path, &project)
}

#[tauri::command]
pub fn update_style(project_path: String, style: Style) -> Result<(), String> {
    let mut project = load_project(&project_path)?;
    project.style = style;
    save_project(&project_path, &project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::{Easing, ExportSettings, InputLog, RawTake, Rect, Resolution};

    fn sample_project() -> Project {
        Project {
            version: project::CURRENT_VERSION,
            raw_take: RawTake { path: "take.mp4".into(), fps: 60, resolution: Resolution { width: 1920, height: 1080 }, duration_ms: 5_000 },
            input_log: InputLog { path: "log.jsonl".into() },
            zoom_keyframes: vec![ZoomKeyframe {
                id: "kf_001".into(),
                start_ms: 0,
                duration_ms: 500,
                target_rect: Rect { x: 0.1, y: 0.1, w: 0.3, h: 0.3 },
                easing: Easing::EaseInOutCubic,
                source: KeyframeSource::Auto,
            }],
            style: Style::default(),
            export_settings: ExportSettings::default(),
        }
    }

    #[test]
    fn upsert_keyframe_replaces_an_existing_id() {
        let project = sample_project();
        let edited = ZoomKeyframe {
            id: "kf_001".into(),
            start_ms: 100,
            duration_ms: 400,
            target_rect: Rect { x: 0.2, y: 0.2, w: 0.4, h: 0.4 },
            easing: Easing::Spring,
            source: KeyframeSource::Auto, // se debe pisar a Manual
        };

        let updated = upsert_keyframe(edited, project);
        assert_eq!(updated.zoom_keyframes.len(), 1);
        assert_eq!(updated.zoom_keyframes[0].start_ms, 100);
        assert_eq!(updated.zoom_keyframes[0].source, KeyframeSource::Manual);
    }

    #[test]
    fn upsert_keyframe_appends_a_new_id() {
        let project = sample_project();
        let new_kf = ZoomKeyframe {
            id: "kf_002".into(),
            start_ms: 1000,
            duration_ms: 300,
            target_rect: Rect { x: 0.0, y: 0.0, w: 0.2, h: 0.2 },
            easing: Easing::Linear,
            source: KeyframeSource::Auto,
        };

        let updated = upsert_keyframe(new_kf, project);
        assert_eq!(updated.zoom_keyframes.len(), 2);
        assert_eq!(updated.zoom_keyframes[1].id, "kf_002");
    }

    #[test]
    fn upsert_keyframe_clamps_a_rect_that_overflows_the_frame() {
        let project = sample_project();
        let overflowing = ZoomKeyframe {
            id: "kf_001".into(),
            start_ms: 0,
            duration_ms: 500,
            target_rect: Rect { x: 0.9, y: 0.9, w: 0.5, h: 0.5 },
            easing: Easing::EaseInOutCubic,
            source: KeyframeSource::Auto,
        };

        let updated = upsert_keyframe(overflowing, project);
        let rect = updated.zoom_keyframes[0].target_rect;
        assert!(rect.x + rect.w <= 1.0 + 1e-6);
        assert!(rect.y + rect.h <= 1.0 + 1e-6);
    }
}
```

- [ ] **Step 2: Correr y confirmar que pasa**

Run: `cargo test -p screenzoom_desktop keyframes`
Expected: PASS (3 tests) — no hace falta un paso "falla primero" separado acá porque el archivo se crea completo con implementación + tests juntos (mismo patrón que `commands/recording.rs` de Fase 1); confirmar igual corriendo antes de continuar.

- [ ] **Step 3: Registrar el módulo (si Task 5 no lo dejó ya declarado)**

En `apps/desktop/src-tauri/src/commands/mod.rs`, confirmar que quede:

```rust
pub mod export;
pub mod keyframes;
pub mod preview;
pub mod recording;
```

- [ ] **Step 4: Compilar y lintear**

Run: `cargo build -p screenzoom_desktop`
Run: `cargo clippy -p screenzoom_desktop --all-targets -- -D warnings`
Expected: ambos limpios

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/commands/keyframes.rs apps/desktop/src-tauri/src/commands/mod.rs
git commit -m "desktop: agrega comandos get_project/update_keyframe/add_keyframe/delete_keyframe/update_style"
```

---

### Task 7: Registrar los comandos nuevos en `main.rs`

**Files:**
- Modify: `apps/desktop/src-tauri/src/main.rs`

**Interfaces:**
- Consumes: los 5 comandos de Task 5 y Task 6.

- [ ] **Step 1: Agregar los comandos al `invoke_handler`**

En `main.rs`, cambiar:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::recording::list_monitors,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::export::export_project,
        ])
```

por:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::recording::list_monitors,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::export::export_project,
            commands::preview::render_preview_frame,
            commands::keyframes::get_project,
            commands::keyframes::update_keyframe,
            commands::keyframes::add_keyframe,
            commands::keyframes::delete_keyframe,
            commands::keyframes::update_style,
        ])
```

- [ ] **Step 2: Compilar**

Run: `cargo build -p screenzoom_desktop`
Expected: compila limpio

- [ ] **Step 3: Correr toda la suite del workspace (regresión completa)**

Run: `cargo test --workspace`
Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: ambos limpios — esto cierra el trabajo de Rust de Fase 2

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src-tauri/src/main.rs
git commit -m "desktop: registra los comandos del editor de Fase 2 en el invoke_handler"
```

---

### Task 8: Tipos y wrappers TS en `commands.ts`

**Files:**
- Modify: `apps/desktop/src/lib/commands.ts`

**Interfaces:**
- Produces: tipos `Rect`, `Easing`, `KeyframeSource`, `ZoomKeyframe`, `Style`, `Resolution`, `RawTake`, `InputLog`, `ExportResolution`, `Codec`, `ExportSettings`, `Project`; funciones `getProject`, `renderPreviewFrame`, `updateKeyframe`, `addKeyframe`, `deleteKeyframe`, `updateStyle` — usados por Tasks 9-12.

- [ ] **Step 1: Agregar los tipos y funciones**

Agregar a `apps/desktop/src/lib/commands.ts` (después de los imports existentes, antes de `MonitorInfo` o después — no importa el orden, se agrega al final del archivo):

```typescript
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type Easing = "linear" | "ease-in-out-cubic" | "spring";
export type KeyframeSource = "auto" | "manual";

export interface ZoomKeyframe {
  id: string;
  start_ms: number;
  duration_ms: number;
  target_rect: Rect;
  easing: Easing;
  source: KeyframeSource;
}

export interface Style {
  background: string;
  padding: number;
  corner_radius: number;
  shadow: boolean;
  cursor_smoothing: boolean;
  motion_blur: boolean;
}

export interface Resolution {
  width: number;
  height: number;
}

export interface RawTake {
  path: string;
  fps: number;
  resolution: Resolution;
  duration_ms: number;
}

export interface InputLog {
  path: string;
}

export type ExportResolution = "1080p" | "1440p" | "4k";
export type Codec = "h264" | "h265";

export interface ExportSettings {
  resolution: ExportResolution;
  fps: number;
  codec: Codec;
  hw_accel: string;
}

export interface Project {
  version: number;
  raw_take: RawTake;
  input_log: InputLog;
  zoom_keyframes: ZoomKeyframe[];
  style: Style;
  export_settings: ExportSettings;
}

export function getProject(projectPath: string): Promise<Project> {
  return invoke("get_project", { projectPath });
}

/** Devuelve un PNG en base64 (sin el prefijo `data:image/png;base64,`). */
export function renderPreviewFrame(projectPath: string, tMs: number, maxWidth: number): Promise<string> {
  return invoke("render_preview_frame", { projectPath, tMs, maxWidth });
}

export function updateKeyframe(projectPath: string, keyframe: ZoomKeyframe): Promise<void> {
  return invoke("update_keyframe", { projectPath, keyframe });
}

export function addKeyframe(projectPath: string, keyframe: ZoomKeyframe): Promise<void> {
  return invoke("add_keyframe", { projectPath, keyframe });
}

export function deleteKeyframe(projectPath: string, keyframeId: string): Promise<void> {
  return invoke("delete_keyframe", { projectPath, keyframeId });
}

export function updateStyle(projectPath: string, style: Style): Promise<void> {
  return invoke("update_style", { projectPath, style });
}
```

- [ ] **Step 2: Verificar tipos**

Run: `pnpm exec tsc -b --noEmit`
Expected: limpio

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/src/lib/commands.ts
git commit -m "desktop-ui: agrega tipos y wrappers TS de los comandos del editor"
```

---

### Task 9: `TimelineOverlay.tsx` — arrastre del rect de zoom sobre el preview

**Files:**
- Create: `apps/desktop/src/components/TimelineOverlay.tsx`

**Interfaces:**
- Consumes: `Rect` (Task 8).
- Produces: componente `TimelineOverlay({ previewSrc, rect, onRectChange })` — usado por Task 12.

- [ ] **Step 1: Implementar**

Crear `apps/desktop/src/components/TimelineOverlay.tsx`:

```tsx
import { useCallback, useRef, useState } from "react";
import type { Rect } from "../lib/commands";

interface TimelineOverlayProps {
  previewSrc: string | null;
  rect: Rect;
  onRectChange: (rect: Rect) => void;
}

type DragMode = "move" | "nw" | "ne" | "sw" | "se" | null;

const MIN_SIZE = 0.05;

export function TimelineOverlay({ previewSrc, rect, onRectChange }: TimelineOverlayProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dragMode, setDragMode] = useState<DragMode>(null);
  const dragStart = useRef<{ pointerX: number; pointerY: number; rect: Rect } | null>(null);

  const handlePointerDown = useCallback(
    (mode: DragMode) => (event: React.PointerEvent) => {
      event.stopPropagation();
      (event.currentTarget as Element).setPointerCapture(event.pointerId);
      setDragMode(mode);
      dragStart.current = { pointerX: event.clientX, pointerY: event.clientY, rect };
    },
    [rect],
  );

  const handlePointerMove = useCallback(
    (event: React.PointerEvent) => {
      if (!dragMode || !dragStart.current || !containerRef.current) {
        return;
      }
      const bounds = containerRef.current.getBoundingClientRect();
      const dx = (event.clientX - dragStart.current.pointerX) / bounds.width;
      const dy = (event.clientY - dragStart.current.pointerY) / bounds.height;
      const start = dragStart.current.rect;

      let next: Rect = start;
      if (dragMode === "move") {
        next = { ...start, x: start.x + dx, y: start.y + dy };
      } else if (dragMode === "se") {
        next = { ...start, w: start.w + dx, h: start.h + dy };
      } else if (dragMode === "nw") {
        next = { x: start.x + dx, y: start.y + dy, w: start.w - dx, h: start.h - dy };
      } else if (dragMode === "ne") {
        next = { ...start, y: start.y + dy, w: start.w + dx, h: start.h - dy };
      } else if (dragMode === "sw") {
        next = { ...start, x: start.x + dx, w: start.w - dx, h: start.h + dy };
      }

      const w = Math.min(Math.max(next.w, MIN_SIZE), 1);
      const h = Math.min(Math.max(next.h, MIN_SIZE), 1);
      const x = Math.min(Math.max(next.x, 0), 1 - w);
      const y = Math.min(Math.max(next.y, 0), 1 - h);
      onRectChange({ x, y, w, h });
    },
    [dragMode, onRectChange],
  );

  const handlePointerUp = useCallback(() => {
    setDragMode(null);
    dragStart.current = null;
  }, []);

  const handleClasses: Record<"nw" | "ne" | "sw" | "se", string> = {
    nw: "-left-1.5 -top-1.5 cursor-nwse-resize",
    se: "-bottom-1.5 -right-1.5 cursor-nwse-resize",
    ne: "-right-1.5 -top-1.5 cursor-nesw-resize",
    sw: "-bottom-1.5 -left-1.5 cursor-nesw-resize",
  };

  return (
    <div ref={containerRef} className="relative aspect-video w-full select-none overflow-hidden rounded bg-black">
      {previewSrc && (
        <img src={`data:image/png;base64,${previewSrc}`} alt="preview" className="pointer-events-none h-full w-full object-contain" />
      )}
      <div
        className="absolute cursor-move border-2 border-blue-500 bg-blue-500/10"
        style={{ left: `${rect.x * 100}%`, top: `${rect.y * 100}%`, width: `${rect.w * 100}%`, height: `${rect.h * 100}%` }}
        onPointerDown={handlePointerDown("move")}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
      >
        {(Object.keys(handleClasses) as Array<keyof typeof handleClasses>).map((corner) => (
          <div
            key={corner}
            className={`absolute h-3 w-3 rounded-full border border-blue-500 bg-white ${handleClasses[corner]}`}
            onPointerDown={handlePointerDown(corner)}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerUp}
          />
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verificar tipos**

Run: `pnpm exec tsc -b --noEmit`
Expected: limpio

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/src/components/TimelineOverlay.tsx
git commit -m "desktop-ui: agrega TimelineOverlay (rect de zoom arrastrable sobre el preview)"
```

---

### Task 10: `KeyframeTrack.tsx` — pista horizontal de keyframes

**Files:**
- Create: `apps/desktop/src/components/KeyframeTrack.tsx`

**Interfaces:**
- Consumes: `ZoomKeyframe` (Task 8).
- Produces: componente `KeyframeTrack({ keyframes, durationMs, selectedId, onSelect, onChange, onDelete })` — usado por Task 12.

- [ ] **Step 1: Implementar**

Crear `apps/desktop/src/components/KeyframeTrack.tsx`:

```tsx
import { useCallback, useRef } from "react";
import type { ZoomKeyframe } from "../lib/commands";

interface KeyframeTrackProps {
  keyframes: ZoomKeyframe[];
  durationMs: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onChange: (keyframe: ZoomKeyframe) => void;
  onDelete: (id: string) => void;
}

export function KeyframeTrack({ keyframes, durationMs, selectedId, onSelect, onChange, onDelete }: KeyframeTrackProps) {
  const trackRef = useRef<HTMLDivElement>(null);

  const handleDrag = useCallback(
    (keyframe: ZoomKeyframe, mode: "move" | "resize") => (event: React.PointerEvent) => {
      event.stopPropagation();
      const track = trackRef.current;
      if (!track || durationMs <= 0) {
        return;
      }
      const bounds = track.getBoundingClientRect();
      const msPerPixel = durationMs / Math.max(bounds.width, 1);
      const startClientX = event.clientX;
      const startMs = keyframe.start_ms;
      const startDuration = keyframe.duration_ms;

      const onMove = (moveEvent: PointerEvent) => {
        const deltaMs = (moveEvent.clientX - startClientX) * msPerPixel;
        if (mode === "move") {
          const nextStart = Math.max(0, Math.min(startMs + deltaMs, durationMs - startDuration));
          onChange({ ...keyframe, start_ms: Math.round(nextStart) });
        } else {
          const nextDuration = Math.max(100, Math.min(startDuration + deltaMs, durationMs - startMs));
          onChange({ ...keyframe, duration_ms: Math.round(nextDuration) });
        }
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [durationMs, onChange],
  );

  return (
    <div ref={trackRef} className="relative h-12 w-full rounded bg-neutral-900">
      {keyframes.map((keyframe) => (
        <div
          key={keyframe.id}
          className={`absolute top-1 h-10 cursor-move rounded border px-1 text-xs text-white ${
            keyframe.id === selectedId ? "border-blue-400 bg-blue-600/70" : "border-neutral-600 bg-neutral-700/70"
          }`}
          style={{
            left: `${(keyframe.start_ms / durationMs) * 100}%`,
            width: `${(keyframe.duration_ms / durationMs) * 100}%`,
          }}
          onClick={() => onSelect(keyframe.id)}
          onPointerDown={handleDrag(keyframe, "move")}
          onDoubleClick={() => onDelete(keyframe.id)}
          title="Arrastrar para mover, doble click para borrar"
        >
          <div className="absolute right-0 top-0 h-full w-2 cursor-ew-resize" onPointerDown={handleDrag(keyframe, "resize")} />
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: Verificar tipos**

Run: `pnpm exec tsc -b --noEmit`
Expected: limpio

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/src/components/KeyframeTrack.tsx
git commit -m "desktop-ui: agrega KeyframeTrack (pista de timeline con drag/resize/delete)"
```

---

### Task 11: `StylePanel.tsx` — panel de estilo + easing

**Files:**
- Create: `apps/desktop/src/components/StylePanel.tsx`

**Interfaces:**
- Consumes: `Style`, `Easing`, `ZoomKeyframe` (Task 8).
- Produces: componente `StylePanel({ style, onStyleChange, selectedKeyframe, onEasingChange })` — usado por Task 12.

- [ ] **Step 1: Implementar**

Crear `apps/desktop/src/components/StylePanel.tsx`:

```tsx
import type { Easing, Style, ZoomKeyframe } from "../lib/commands";

const BACKGROUND_PRESETS = ["gradient-01", "gradient-02", "gradient-03", "gradient-04", "gradient-05", "gradient-06"];
const EASING_OPTIONS: Easing[] = ["linear", "ease-in-out-cubic", "spring"];

interface StylePanelProps {
  style: Style;
  onStyleChange: (style: Style) => void;
  selectedKeyframe: ZoomKeyframe | null;
  onEasingChange: (easing: Easing) => void;
}

export function StylePanel({ style, onStyleChange, selectedKeyframe, onEasingChange }: StylePanelProps) {
  return (
    <div className="flex flex-col gap-3 rounded border border-neutral-700 bg-neutral-900 p-4 text-sm text-neutral-200">
      <h2 className="font-medium">Estilo</h2>

      <label className="flex flex-col gap-1">
        Fondo
        <select
          className="rounded border border-neutral-700 bg-neutral-800 px-2 py-1"
          value={style.background}
          onChange={(event) => onStyleChange({ ...style, background: event.target.value })}
        >
          {BACKGROUND_PRESETS.map((preset) => (
            <option key={preset} value={preset}>
              {preset}
            </option>
          ))}
        </select>
      </label>

      <label className="flex flex-col gap-1">
        Padding ({Math.round(style.padding * 100)}%)
        <input
          type="range"
          min={0}
          max={0.4}
          step={0.01}
          value={style.padding}
          onChange={(event) => onStyleChange({ ...style, padding: Number(event.target.value) })}
        />
      </label>

      <label className="flex flex-col gap-1">
        Esquinas ({style.corner_radius}px)
        <input
          type="range"
          min={0}
          max={80}
          step={1}
          value={style.corner_radius}
          onChange={(event) => onStyleChange({ ...style, corner_radius: Number(event.target.value) })}
        />
      </label>

      <label className="flex items-center gap-2">
        <input type="checkbox" checked={style.shadow} onChange={(event) => onStyleChange({ ...style, shadow: event.target.checked })} />
        Sombra
      </label>

      {selectedKeyframe && (
        <label className="flex flex-col gap-1 border-t border-neutral-700 pt-3">
          Easing del keyframe seleccionado
          <select
            className="rounded border border-neutral-700 bg-neutral-800 px-2 py-1"
            value={selectedKeyframe.easing}
            onChange={(event) => onEasingChange(event.target.value as Easing)}
          >
            {EASING_OPTIONS.map((easing) => (
              <option key={easing} value={easing}>
                {easing}
              </option>
            ))}
          </select>
        </label>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Verificar tipos**

Run: `pnpm exec tsc -b --noEmit`
Expected: limpio

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/src/components/StylePanel.tsx
git commit -m "desktop-ui: agrega StylePanel (fondo/padding/esquinas/sombra/easing)"
```

---

### Task 12: Integrar el editor en `App.tsx`

**Files:**
- Modify: `apps/desktop/src/App.tsx`

**Interfaces:**
- Consumes: `getProject`, `renderPreviewFrame`, `updateKeyframe`, `deleteKeyframe`, `updateStyle` (Task 8); `TimelineOverlay` (Task 9); `KeyframeTrack` (Task 10); `StylePanel` (Task 11).

- [ ] **Step 1: Reemplazar el contenido de `App.tsx`**

El estado de edición (proyecto cargado, keyframe seleccionado, posición del scrubber, preview actual) se agrega como estado nuevo separado del `Status` existente, poblado cuando `status.kind === "stopped"`. El fetch de preview se debounce 100ms para no saturar de renders mientras se arrastra.

```tsx
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  addKeyframe,
  deleteKeyframe,
  exportProject,
  getProject,
  listMonitors,
  renderPreviewFrame,
  startRecording,
  stopRecording,
  updateKeyframe,
  updateStyle,
  type Easing,
  type MonitorInfo,
  type Project,
  type Rect,
  type ZoomKeyframe,
} from "./lib/commands";
import { TimelineOverlay } from "./components/TimelineOverlay";
import { KeyframeTrack } from "./components/KeyframeTrack";
import { StylePanel } from "./components/StylePanel";

type Status =
  | { kind: "idle" }
  | { kind: "recording" }
  | { kind: "stopped"; projectPath: string }
  | { kind: "exporting"; framesDone: number }
  | { kind: "exported"; outputPath: string }
  | { kind: "error"; message: string };

const PREVIEW_WIDTH = 854;
const PREVIEW_DEBOUNCE_MS = 100;

function App() {
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [selectedMonitor, setSelectedMonitor] = useState<number | undefined>(undefined);
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  const [project, setProject] = useState<Project | null>(null);
  const [selectedKeyframeId, setSelectedKeyframeId] = useState<string | null>(null);
  const [scrubMs, setScrubMs] = useState(0);
  const [previewSrc, setPreviewSrc] = useState<string | null>(null);
  const previewDebounce = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    listMonitors()
      .then((list) => {
        setMonitors(list);
        setSelectedMonitor((current) => current ?? list[0]?.index);
      })
      .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
  }, []);

  useEffect(() => {
    const unlistenProgress = listen<{ frames_done: number }>("export-progress", (event) => {
      setStatus({ kind: "exporting", framesDone: event.payload.frames_done });
    });
    const unlistenFinished = listen<string>("export-finished", (event) => {
      setStatus({ kind: "exported", outputPath: event.payload });
    });
    const unlistenError = listen<{ message: string }>("export-error", (event) => {
      setStatus({ kind: "error", message: event.payload.message });
    });

    return () => {
      void unlistenProgress.then((unlisten) => unlisten());
      void unlistenFinished.then((unlisten) => unlisten());
      void unlistenError.then((unlisten) => unlisten());
    };
  }, []);

  // Carga el proyecto recien grabado apenas hay un projectPath disponible.
  useEffect(() => {
    if (status.kind !== "stopped") {
      return;
    }
    getProject(status.projectPath)
      .then((loaded) => {
        setProject(loaded);
        setSelectedKeyframeId(loaded.zoom_keyframes[0]?.id ?? null);
        setScrubMs(0);
      })
      .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
  }, [status]);

  // Re-renderiza el preview (debounced) cada vez que cambia el proyecto o el scrubber.
  useEffect(() => {
    if (status.kind !== "stopped" || !project) {
      return;
    }
    if (previewDebounce.current) {
      clearTimeout(previewDebounce.current);
    }
    previewDebounce.current = setTimeout(() => {
      renderPreviewFrame(status.projectPath, scrubMs, PREVIEW_WIDTH)
        .then(setPreviewSrc)
        .catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
    }, PREVIEW_DEBOUNCE_MS);

    return () => {
      if (previewDebounce.current) {
        clearTimeout(previewDebounce.current);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project, scrubMs]);

  const handleRecordToggle = useCallback(async () => {
    try {
      if (status.kind === "recording") {
        const projectPath = await stopRecording();
        setStatus({ kind: "stopped", projectPath });
      } else {
        await startRecording(selectedMonitor);
        setStatus({ kind: "recording" });
      }
    } catch (err) {
      setStatus({ kind: "error", message: String(err) });
    }
  }, [status.kind, selectedMonitor]);

  const handleExport = useCallback(async () => {
    if (status.kind !== "stopped") {
      return;
    }
    const outputPath = status.projectPath.replace(/\.szproj$/, "_export.mp4");
    try {
      await exportProject(status.projectPath, outputPath);
      setStatus({ kind: "exporting", framesDone: 0 });
    } catch (err) {
      setStatus({ kind: "error", message: String(err) });
    }
  }, [status]);

  const selectedKeyframe = project?.zoom_keyframes.find((k) => k.id === selectedKeyframeId) ?? null;

  const persistKeyframe = useCallback(
    (keyframe: ZoomKeyframe) => {
      if (status.kind !== "stopped") {
        return;
      }
      setProject((current) => {
        if (!current) return current;
        const zoom_keyframes = current.zoom_keyframes.map((k) => (k.id === keyframe.id ? keyframe : k));
        return { ...current, zoom_keyframes };
      });
      void updateKeyframe(status.projectPath, keyframe).catch((err: unknown) =>
        setStatus({ kind: "error", message: String(err) }),
      );
    },
    [status],
  );

  const handleRectChange = useCallback(
    (rect: Rect) => {
      if (!selectedKeyframe) return;
      persistKeyframe({ ...selectedKeyframe, target_rect: rect });
    },
    [selectedKeyframe, persistKeyframe],
  );

  const handleEasingChange = useCallback(
    (easing: Easing) => {
      if (!selectedKeyframe) return;
      persistKeyframe({ ...selectedKeyframe, easing });
    },
    [selectedKeyframe, persistKeyframe],
  );

  const handleDeleteKeyframe = useCallback(
    (id: string) => {
      if (status.kind !== "stopped") return;
      setProject((current) => (current ? { ...current, zoom_keyframes: current.zoom_keyframes.filter((k) => k.id !== id) } : current));
      if (selectedKeyframeId === id) setSelectedKeyframeId(null);
      void deleteKeyframe(status.projectPath, id).catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
    },
    [status, selectedKeyframeId],
  );

  const handleStyleChange = useCallback(
    (style: Project["style"]) => {
      if (status.kind !== "stopped") return;
      setProject((current) => (current ? { ...current, style } : current));
      void updateStyle(status.projectPath, style).catch((err: unknown) => setStatus({ kind: "error", message: String(err) }));
    },
    [status],
  );

  const isRecording = status.kind === "recording";

  return (
    <main className="flex min-h-screen flex-col items-center gap-6 bg-neutral-950 p-8 text-neutral-100">
      <h1 className="text-xl font-semibold">screenzoom</h1>

      <label className="flex flex-col gap-1 text-sm text-neutral-400">
        Monitor
        <select
          className="rounded border border-neutral-700 bg-neutral-900 px-3 py-2 text-neutral-100"
          value={selectedMonitor ?? ""}
          onChange={(event) => setSelectedMonitor(Number(event.target.value))}
          disabled={isRecording || monitors.length === 0}
        >
          {monitors.map((monitor) => (
            <option key={monitor.index} value={monitor.index}>
              {monitor.name} ({monitor.width}x{monitor.height})
            </option>
          ))}
        </select>
      </label>

      <button
        type="button"
        className="rounded-full bg-red-600 px-6 py-3 font-medium text-white transition hover:bg-red-500 disabled:opacity-50"
        onClick={() => void handleRecordToggle()}
        disabled={status.kind === "exporting"}
      >
        {isRecording ? "Detener grabacion" : "Grabar"}
      </button>

      {status.kind === "stopped" && project && (
        <div className="flex w-full max-w-4xl flex-col gap-4">
          <TimelineOverlay
            previewSrc={previewSrc}
            rect={selectedKeyframe?.target_rect ?? { x: 0, y: 0, w: 1, h: 1 }}
            onRectChange={handleRectChange}
          />

          <input
            type="range"
            min={0}
            max={project.raw_take.duration_ms}
            step={16}
            value={scrubMs}
            onChange={(event) => setScrubMs(Number(event.target.value))}
            className="w-full"
          />

          <KeyframeTrack
            keyframes={project.zoom_keyframes}
            durationMs={project.raw_take.duration_ms}
            selectedId={selectedKeyframeId}
            onSelect={setSelectedKeyframeId}
            onChange={persistKeyframe}
            onDelete={handleDeleteKeyframe}
          />

          <StylePanel
            style={project.style}
            onStyleChange={handleStyleChange}
            selectedKeyframe={selectedKeyframe}
            onEasingChange={handleEasingChange}
          />

          <button
            type="button"
            className="rounded-full bg-blue-600 px-6 py-3 font-medium text-white transition hover:bg-blue-500"
            onClick={() => void handleExport()}
          >
            Exportar
          </button>
        </div>
      )}

      {status.kind === "exporting" && <p className="text-sm text-neutral-400">Exportando... {status.framesDone} frames procesados</p>}
      {status.kind === "exported" && <p className="text-sm text-green-400">Listo: {status.outputPath}</p>}
      {status.kind === "error" && <p className="text-sm text-red-400">Error: {status.message}</p>}
    </main>
  );
}

export default App;
```

- [ ] **Step 2: Verificar tipos y build**

Run: `pnpm exec tsc -b --noEmit`
Run: `pnpm exec vite build`
Expected: ambos limpios

- [ ] **Step 3: Verificación manual end-to-end (criterio de "listo" de Fase 2)**

Run: `pnpm tauri dev` (en background, no bloquear la sesión)

Con la app levantada: grabar unos segundos, detener, confirmar que aparece el editor con el preview + rect arrastrable + track de keyframes + panel de estilo; mover el rect de un keyframe, cambiar el padding/fondo, exportar; confirmar que el mp4 resultante refleja los cambios (padding/fondo visibles, rect de zoom en la posición ajustada). Esto requiere criterio visual — si algo no se ve bien, documentarlo como pendiente en `UPDATES.md` en vez de forzar que "pase".

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/App.tsx
git commit -m "desktop-ui: integra el editor de Fase 2 (timeline + estilo + preview) en App.tsx"
```

---

## Self-Review (completado antes de entregar el plan)

- **Cobertura del spec**: timeline editable (Tasks 9-10, 12), 3 presets de easing seleccionables (Task 11, ya soportados por el backend desde Fase 1), preview de baja resolución que se actualiza al mover el timeline (Tasks 5, 12), panel de estilo persistido en `.szproj` (Tasks 3, 4, 6, 11) — cubierto.
- **Placeholders**: ninguno — cada step tiene código completo, sin "TBD"/"similar a Task N".
- **Consistencia de tipos**: `Rect`, `ZoomKeyframe`, `Style` en TS (Task 8) mirrorean exactamente los campos `snake_case` de los structs de `project` (sin `rename_all`, así que el wire format es snake_case tal cual) — verificado contra `crates/project/src/lib.rs`. `apply_style`/`StyledFrame` (Task 3) se usan con la misma firma en Task 4 y Task 5.
- **Alcance**: enfocado en el criterio de "listo" de Fase 2 de `docs/PROMPT_AGENTE_DEV.md`; motion blur real, cursor reconstruido, selección de ventana, updater y crash-recovery quedan fuera (sub-proyecto B, Fase 3).
