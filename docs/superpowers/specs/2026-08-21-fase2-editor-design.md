# Fase 2 — Editor post-grabación (diseño)

Sub-proyecto A de 2 identificados para llevar la app a "100% terminada" (ver `UPDATES.md` sesión 2026-08-21 para el B: Fase 3 — pulido). Cierra el criterio de "listo" de Fase 2 en `docs/PROMPT_AGENTE_DEV.md`: poder tomar cualquier proyecto de Fase 1, editar manualmente los keyframes generados, y exportar viendo esos cambios reflejados en el video final.

## Contexto

Fase 1 dejó el zoom 100% automático (`zoom-engine` genera keyframes desde clicks/inactividad), sin forma de ajustarlo. El schema `.szproj` (`crates/project`) ya define `Easing` (Linear/EaseInOutCubic/Spring, las 3 ya interpoladas en `crates/compositor/src/camera_path.rs`) y `Style` (background/padding/corner_radius/shadow/cursor_smoothing/motion_blur), pero `Style` no se aplica en ningún render — el compositor de Fase 1 solo hace crop+scale.

## Arquitectura

- **`crates/compositor`**: nuevo `style_pass` — segunda etapa de render (mismo pipeline `wgpu`, shader WGSL adicional) que compone el resultado del crop+scale sobre un background, aplicando padding, esquinas redondeadas (mask) y sombra simple. No se toca `camera_path` (ya soporta los 3 easings) ni el crop+scale existente.
- **Comandos Tauri nuevos** (`apps/desktop/src-tauri/src/commands/`):
  - `render_preview_frame(project_path, t_ms, max_width) -> Vec<u8> (PNG)`: decode + `camera_rect_at` + composite + `style_pass` a baja resolución (target 480p), corre en `spawn_blocking`.
  - `update_keyframe(project_path, keyframe)`, `add_keyframe(project_path, keyframe)`, `delete_keyframe(project_path, keyframe_id)`: mutan y persisten el `.szproj`. Toda edición manual marca `KeyframeSource::Manual`.
- **Frontend** (`apps/desktop/src/`):
  - `Timeline.tsx`: overlay con pointer events sobre el preview — rect de zoom arrastrable/redimensionable (8 handles), sin librerías nuevas (no hay Konva/Fabric en `package.json`, no hace falta para un solo rect).
  - `KeyframeTrack.tsx`: pista horizontal con una barra por keyframe — click para seleccionar, drag para `start_ms`, resize en los extremos para `duration_ms`.
  - Dropdown de easing por keyframe seleccionado.
  - Panel de estilo: background/padding/corner_radius/shadow, cada cambio dispara `invoke` + un nuevo `render_preview_frame` en la posición actual del scrubber (debounce ~100ms).

## Flujo de datos

1. Usuario arrastra el rect sobre el preview o mueve una barra en el track → frontend calcula `Rect`/`start_ms`/`duration_ms` normalizados.
2. `invoke("update_keyframe", …)` → backend clampea a los bordes del frame (reusa la lógica de clamping ya existente en `zoom-engine`) → persiste en `.szproj` → `KeyframeSource::Manual`.
3. Frontend pide `render_preview_frame` en la posición actual del scrubber → backend corre la pipeline real (compositor + `style_pass`) a 480p → devuelve PNG → frontend lo pinta en el canvas de preview.

## Manejo de errores

Coordenadas fuera de rango se clampean antes de persistir (mismo criterio de Fase 1). Un fallo de `render_preview_frame` no bloquea la UI: se conserva el último frame válido en pantalla + indicador de error, nunca una pantalla rota. Errores de IO al persistir el `.szproj` se propagan al frontend vía el mecanismo de error de comandos Tauri existente.

## Testing

- `crates/compositor`: unit tests del `style_pass` — padding correcto en píxeles, máscara de esquina redondeada en las 4 esquinas, background sólido reproducido exacto (mismo patrón que los tests de crop/color de Fase 1).
- Comandos Tauri de CRUD de keyframes: unit tests igual que `start_recording`/`stop_recording` de Fase 1.
- Sin test runner de frontend (no existe hoy, consistente con Fase 1) — validación manual con `pnpm tauri dev`.

## Criterio de "listo"

Tomar un `.szproj` generado por Fase 1, editar manualmente al menos un keyframe (mover/redimensionar/cambiar easing) y el panel de estilo, exportar, y confirmar que el MP4 resultante refleja esos cambios.

## Fuera de alcance (queda para Fase 3 / sub-proyecto B)

Motion blur real en el shader, cursor reconstruido/suavizado, selección de ventana como fuente, `tauri-plugin-updater`, resiliencia a crashes de la toma cruda (requiere decisión de diseño propia — `windows-capture` 2.0.1 no expone MP4 fragmentado, confirmado leyendo `encoder.rs` del crate).
