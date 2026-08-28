//! Comandos Tauri de lectura/edicion del `.szproj` para el editor de
//! timeline (Fase 2). Toda edicion manual clampea el rect (`Rect::clamp_into_unit_square`
//! en `crates/project`) y marca `KeyframeSource::Manual` para que un futuro
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
            cursor_path: Vec::new(),
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
