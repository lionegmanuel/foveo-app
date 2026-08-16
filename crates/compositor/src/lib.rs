//! Motor de composicion: decodifica la toma cruda, interpola la camara
//! (rect de crop/zoom) para cada frame de salida segun los keyframes del
//! proyecto, y renderiza el resultado en GPU (`wgpu`/WGSL). Pipeline minima
//! de Fase 1: solo crop + scale, sin motion blur ni cursor reconstruido
//! (eso es Fase 3, ver ARQUITECTURA.md seccion 6).
//!
//! Este crate no importa `tauri`.

pub mod camera_path;
pub mod decode;
pub mod render;

pub use camera_path::camera_rect_at;
pub use decode::{DecodeError, RawFrameReader};
pub use render::{Compositor, CompositorError, PIXEL_FORMAT};
