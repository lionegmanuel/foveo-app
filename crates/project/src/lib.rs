//! Schema del archivo de proyecto `.szproj` — el corazon del modelo no
//! destructivo (ver ARQUITECTURA.md seccion 3.5). Este crate solo define
//! datos + (de)serializacion; no toca disco fuera de `Project::load`/`save`,
//! no conoce Tauri, wgpu ni ffmpeg.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Version del formato del archivo de proyecto. Subir este numero (y migrar
/// datos viejos si hace falta) es la unica forma permitida de cambiar el
/// schema de forma incompatible.
pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub raw_take: RawTake,
    pub input_log: InputLog,
    #[serde(default)]
    pub zoom_keyframes: Vec<ZoomKeyframe>,
    #[serde(default)]
    pub style: Style,
    #[serde(default)]
    pub export_settings: ExportSettings,
}

impl Project {
    /// Crea un proyecto nuevo a partir de una toma cruda recien grabada, sin
    /// keyframes todavia (el Zoom Engine los agrega despues) y con estilo /
    /// export settings por defecto.
    #[must_use]
    pub fn new(raw_take: RawTake, input_log: InputLog) -> Self {
        Self {
            version: CURRENT_VERSION,
            raw_take,
            input_log,
            zoom_keyframes: Vec::new(),
            style: Style::default(),
            export_settings: ExportSettings::default(),
        }
    }

    /// Lee y parsea un `.szproj` desde disco.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// Serializa (pretty-printed, para que sea diffable en git/manualmente
    /// editable) y escribe el `.szproj` a disco.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ProjectError> {
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(path, raw)?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("error de IO leyendo/escribiendo el proyecto: {0}")]
    Io(#[from] io::Error),
    #[error("error de formato en el .szproj: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTake {
    pub path: PathBuf,
    pub fps: u32,
    pub resolution: Resolution,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputLog {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomKeyframe {
    pub id: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub target_rect: Rect,
    pub easing: Easing,
    pub source: KeyframeSource,
}

/// Rectangulo de zoom en coordenadas normalizadas (0-1), independiente de la
/// resolucion de captura vs. la de export (ver ARQUITECTURA.md 3.5).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// El frame completo, sin zoom aplicado.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Easing {
    Linear,
    EaseInOutCubic,
    Spring,
}

/// Marca si un keyframe fue generado por el Zoom Engine o editado/creado a
/// mano, para que un recalculo automatico no pise ediciones manuales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyframeSource {
    Auto,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub background: String,
    pub padding: f32,
    pub corner_radius: f32,
    pub shadow: bool,
    pub cursor_smoothing: bool,
    pub motion_blur: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: "gradient-01".to_string(),
            padding: 0.06,
            corner_radius: 18.0,
            shadow: true,
            cursor_smoothing: true,
            motion_blur: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportResolution {
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "4k")]
    P4k,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    H265,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub resolution: ExportResolution,
    pub fps: u32,
    pub codec: Codec,
    /// "auto" deja que el Exporter elija el encoder de hardware disponible
    /// (NVENC/QSV/AMF/Media Foundation); cualquier otro valor es un override
    /// manual resuelto en la capa de exporter (crates/exporter, Fase 1).
    pub hw_accel: String,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self { resolution: ExportResolution::P1080, fps: 60, codec: Codec::H264, hw_accel: "auto".to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_project() -> Project {
        Project {
            version: CURRENT_VERSION,
            raw_take: RawTake {
                path: PathBuf::from("takes/take_2026-08-14T10-00-00.mp4"),
                fps: 60,
                resolution: Resolution { width: 3840, height: 2160 },
                duration_ms: 125_000,
            },
            input_log: InputLog { path: PathBuf::from("takes/take_2026-08-14T10-00-00.input.jsonl") },
            zoom_keyframes: vec![ZoomKeyframe {
                id: "kf_001".to_string(),
                start_ms: 3_200,
                duration_ms: 600,
                target_rect: Rect { x: 0.42, y: 0.31, w: 0.35, h: 0.35 },
                easing: Easing::EaseInOutCubic,
                source: KeyframeSource::Auto,
            }],
            style: Style::default(),
            export_settings: ExportSettings::default(),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let project = sample_project();
        let json = serde_json::to_string_pretty(&project).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(project, parsed);
    }

    #[test]
    fn matches_arquitectura_example_field_names() {
        // Ancla el schema contra el ejemplo documentado en ARQUITECTURA.md 3.5:
        // si alguien renombra un campo sin querer, este test lo detecta.
        let project = sample_project();
        let json = serde_json::to_value(&project).unwrap();

        assert_eq!(json["raw_take"]["fps"], 60);
        assert_eq!(json["raw_take"]["resolution"]["width"], 3840);
        // Comparar via as_f64() en vez de contra el literal 0.42 directamente:
        // el campo es f32, y json! promueve numeros a f64, asi que el literal
        // f64 0.42 y el f32 0.42 ensanchado no bitmatchean exactamente.
        assert_eq!(json["zoom_keyframes"][0]["target_rect"]["x"].as_f64().unwrap(), 0.42_f32 as f64);
        assert_eq!(json["zoom_keyframes"][0]["easing"], "ease-in-out-cubic");
        assert_eq!(json["zoom_keyframes"][0]["source"], "auto");
        assert_eq!(json["export_settings"]["resolution"], "1080p");
    }

    #[test]
    fn defaults_are_stable_when_style_and_export_settings_are_omitted() {
        // Un .szproj viejo (o escrito a mano) sin "style"/"export_settings"
        // debe seguir cargando con los defaults, no fallar.
        let minimal = serde_json::json!({
            "version": 1,
            "raw_take": {
                "path": "takes/x.mp4",
                "fps": 60,
                "resolution": { "width": 1920, "height": 1080 },
                "duration_ms": 1000
            },
            "input_log": { "path": "takes/x.input.jsonl" }
        });
        let project: Project = serde_json::from_value(minimal).unwrap();
        assert!(project.zoom_keyframes.is_empty());
        assert_eq!(project.export_settings.resolution, ExportResolution::P1080);
    }

    #[test]
    fn save_then_load_round_trips_on_disk() {
        let project = sample_project();
        let dir = std::env::temp_dir().join(format!("screenzoom-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.szproj");

        project.save(&path).unwrap();
        let loaded = Project::load(&path).unwrap();

        assert_eq!(project, loaded);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let err = Project::load("this/path/does/not/exist.szproj").unwrap_err();
        assert!(matches!(err, ProjectError::Io(_)));
    }

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
}
