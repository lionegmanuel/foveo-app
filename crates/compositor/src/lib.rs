//! Motor de composicion: decodifica la toma cruda, interpola la camara
//! (rect de crop/zoom) para cada frame de salida segun los keyframes del
//! proyecto, renderiza el crop/scale en GPU (`wgpu`/WGSL, `render.rs`) y
//! aplica estilo/motion blur/cursor reconstruido en CPU (`style_pass.rs`,
//! `cursor_path.rs` — ver ARQUITECTURA.md seccion 6, Fase 3).
//!
//! Este crate no importa `tauri`.

pub mod camera_path;
pub mod cursor_path;
pub mod decode;
pub mod render;
pub mod style_pass;

pub use camera_path::camera_rect_at;
pub use cursor_path::{CursorAt, CursorPath};
pub use decode::{DecodeError, RawFrameReader};
pub use render::{Compositor, CompositorError, PIXEL_FORMAT};
pub use style_pass::{CursorMarker, StyledFrame, apply_style, apply_style_ex, content_uv_to_canvas_px};
