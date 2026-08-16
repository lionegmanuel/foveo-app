# PROMPT_AGENTE_DEV.md

## Prompt autocontenido para Claude Code (o desarrollador humano) que va a implementar el proyecto

> Copiar y pegar este documento completo como instrucción inicial. No requiere contexto adicional del proyecto — todo lo necesario está acá o en `ARQUITECTURA.md`, que debe leerse primero.

---

## TU ROL

Sos un ingeniero senior de sistemas encargado de implementar, desde cero, una app de escritorio de screen recording con zoom automático estilo Screen Studio, para **Windows primero, macOS después**. El dueño del producto (quien te da estas instrucciones) ya tomó todas las decisiones de arquitectura. Tu trabajo es **implementar, no rediseñar**. Si encontrás un problema real con una decisión tomada, señalalo explícitamente y proponé una alternativa concreta — pero no la implementes sin confirmación explícita.

Antes de escribir una sola línea de código, leé completo `ARQUITECTURA.md`. Ese documento es la fuente de verdad de todas las decisiones técnicas. Este prompt te da el **orden de implementación** y las **reglas operativas**; `ARQUITECTURA.md` te da el **qué y el por qué**.

---

## STACK — YA DECIDIDO, NO LO VUELVAS A EVALUAR

- **Framework de escritorio**: Tauri v2 (no Electron, no nativo puro — no propongas alternativas).
- **Core**: Rust. Todo lo que toque captura, tracking de input, zoom/easing, composición GPU y encoding va en Rust, en crates dentro de `crates/`, sin dependencia de Tauri.
- **UI**: React + TypeScript + Tailwind, dentro de `apps/desktop/src/`.
- **Captura de pantalla (Windows)**: crate `windows-capture` (Windows.Graphics.Capture).
- **Tracking de input**: crate `rdev`.
- **Compositing/render**: `wgpu` + shaders WGSL.
- **Encoding final**: `ffmpeg` como sidecar de Tauri, con encoder de hardware cuando esté disponible.
- **Auto-update**: `tauri-plugin-updater`.
- **Sin backend propio, sin auth, sin licensing** en esta etapa. Es una app 100% local.

Si en algún punto te parece que "sería más fácil" resolver algo con Electron, con procesamiento en Node, o subiendo algo a un servidor — no lo hagas. Esas opciones fueron evaluadas y descartadas explícitamente (ver sección 2 de `ARQUITECTURA.md`).

---

## ESTRUCTURA DE CARPETAS — YA DEFINIDA

Usá exactamente la estructura descripta en la sección 5 de `ARQUITECTURA.md`. No la reorganices, no muevas crates entre carpetas, no agregues carpetas de nivel superior nuevas sin que se te pida explícitamente. Si te falta una carpeta que la estructura no contempla (por ejemplo, un lugar para tests de integración), agregala siguiendo el mismo criterio de nombres (`kebab-case`) y avisá que la agregaste, en vez de asumir en silencio.

Regla dura: los crates dentro de `crates/` **no pueden importar `tauri` como dependencia**. Si un crate necesita algo de Tauri, es señal de que esa lógica pertenece a `apps/desktop/src-tauri/src/commands/`, no al crate.

---

## ORDEN DE IMPLEMENTACIÓN POR FASES

No saltees fases ni las hagas en paralelo. Cada fase tiene un criterio de "listo" explícito — no avances a la siguiente sin cumplirlo.

### Fase 0 — Spike de riesgo técnico

**Objetivo**: probar que la captura nativa y el tracking de input funcionan ANTES de construir nada más.

1. Scaffold del repo: workspace de Cargo + Tauri v2 + Vite/React, siguiendo la estructura de carpetas.
2. Programa mínimo (puede ser un binario de test, no necesita UI) que use `windows-capture` para grabar la pantalla completa 60 segundos a 1080p60 y guardarlo como mp4 reproducible.
3. Programa mínimo en paralelo que use `rdev` para loguear eventos de mouse (posición + clicks) con timestamps a un archivo JSONL.

**Criterio de "listo"**: un mp4 de 60s reproducible en cualquier player + un JSONL con eventos de mouse timestampeados que se puedan correlacionar manualmente contra el video (por ejemplo, abriendo el video y confirmando que el timestamp de un click coincide con el frame donde se ve el click).

**No sigas a la Fase 1 si esto no funciona de forma confiable.** Es el riesgo técnico más grande del proyecto.

### Fase 1 — MVP de grabación + zoom automático (sin editor manual)

1. Crear el crate `crates/capture` con el trait `ScreenCapturer` y la implementación Windows.
2. Crear el crate `crates/input-tracker` con el trait `InputSource` y la implementación con `rdev`.
3. Crear el crate `crates/project` con el schema del archivo `.szproj` (ver sección 3.5 de `ARQUITECTURA.md`) y su serialización con `serde`.
4. Crear el crate `crates/zoom-engine` con la lógica de detección de puntos de interés (clusters de clicks, inactividad) y generación de keyframes con una curva de easing por defecto (`ease-in-out-cubic`). **Esta lógica debe ser 100% testeable con `cargo test`, sin GPU ni UI** — es lógica pura sobre datos.
5. Crear el crate `crates/compositor` con una pipeline `wgpu` mínima: decodificar la toma cruda, interpolar el rect de zoom por frame según los keyframes, aplicar crop+scale. Sin motion blur, sin cursor reconstruido todavía.
6. Crear el crate `crates/exporter` que orquesta el sidecar `ffmpeg`: recibe frames compuestos y los pipea para encoding + mux a MP4.
7. Conectar todo desde `apps/desktop/src-tauri/src/commands/`, exponiendo comandos: `start_recording`, `stop_recording`, `export_project`.
8. UI mínima en React: botón grabar/detener, selector de monitor (si hay más de uno), botón exportar con barra de progreso. Sin timeline editable todavía.

**Criterio de "listo"**: grabar una demo real de 2-3 minutos usando la app (no un script de test) y obtener un MP4 exportado con zooms automáticos siguiendo los clicks, sin ningún ajuste manual.

### Fase 2 — Editor post-grabación

1. UI de timeline: lista/línea de tiempo de los keyframes de zoom generados en la Fase 1, con controles para arrastrar el punto de inicio/fin, redimensionar el área de zoom, borrar y agregar keyframes manuales.
2. Al menos 3 presets de easing seleccionables por keyframe (linear, ease-in-out, spring).
3. Preview en la UI: renderizado de baja resolución (no hace falta 4K en preview) que se actualiza al mover el timeline, para no depender de exportar para ver el resultado.
4. Panel de configuración de estilo: fondo, padding, esquinas redondeadas — persistido en `.szproj`.

**Criterio de "listo"**: poder tomar cualquier proyecto de la Fase 1, editar manualmente los keyframes generados, y exportar viendo reflejados esos cambios en el video final.

### Fase 3 — Pulido de calidad ("nivel DIOS")

1. Motion blur en las transiciones de zoom dentro del shader WGSL del compositor.
2. Reconstrucción de cursor suavizado: en vez de usar el cursor crudo capturado, generar un cursor vectorial con movimiento interpolado/suavizado entre posiciones registradas.
3. Selección de ventana específica como fuente de captura (no solo pantalla completa).
4. Integrar `tauri-plugin-updater` con un manifest estático (puede alojarse en GitHub Releases).
5. Manejo robusto de errores: si la app crashea durante una grabación, la toma cruda ya escrita a disco debe seguir siendo recuperable (no perder el archivo aunque se pierda el estado de la sesión).

**Criterio de "listo"**: uso diario real de la app sin miedo a perder grabaciones, con resultado visual comparable a un primer pase de Screen Studio.

### Fase 4 — Expansión (NO implementar sin pedido explícito)

Puerto a macOS, subtítulos, transiciones, webcam overlay, multi-track de audio, export a GIF, licensing. Estas features están **fuera de scope hasta nuevo aviso**, incluso si te parecen "fáciles de agregar ya que estás". Ver la sección "QUÉ NO HACER" más abajo.

---

## CRITERIOS DE "LISTO" — REGLA GENERAL

Una fase no está "lista" porque el código compila. Está lista cuando cumple el criterio explícito de esa fase usando la app real (grabando algo de verdad), no un test sintético aislado. Si algo compila pero no cumple el criterio de la fase, la fase no está terminada.

---

## QUÉ NO HACER (scope creep, abstracciones prematuras, features no pedidas)

Esta sección es tan importante como el roadmap. Un agente de IA sin este límite tiende a "mejorar" el alcance sin que se lo pidan, lo cual retrasa el MVP real.

1. **No implementes soporte para macOS todavía.** El trait `ScreenCapturer` debe estar diseñado para ser extensible a futuro (eso sí es necesario), pero no escribas la implementación de `screencapturekit-rs` hasta que se pida explícitamente.
2. **No agregues autenticación, cuentas de usuario, ni licensing.** No hay backend en este proyecto todavía. Si necesitás guardar algo, es un archivo local, no una llamada a un servidor.
3. **No agregues telemetría o analytics** salvo que se pida explícitamente. Ver `.env.example` para dónde iría si algún día se activa — pero no la actives por defecto.
4. **No introduzcas abstracciones genéricas "por si acaso"** (por ejemplo, un sistema de plugins, un motor de efectos genérico, soporte multi-formato de proyecto) antes de que exista una necesidad concreta y actual de esa flexibilidad. Cada capa de abstracción que no resuelve un problema de HOY es deuda técnica, no inversión.
5. **No cambies de stack a mitad de camino** aunque encuentres una librería que "se ve mejor". Si genuinamente creés que hay un problema serio con una decisión de `ARQUITECTURA.md`, documentalo y preguntá — no lo resuelvas por tu cuenta cambiando de dirección.
6. **No bloquees el hilo principal/UI con trabajo pesado.** Captura, tracking, composición y export corren SIEMPRE en threads dedicados o tasks async, nunca en el mismo contexto que actualiza la UI. Ver `CLAUDE.md` para el detalle de esta regla.
7. **No implementes un editor de video genérico tipo NLE completo** (multi-pista, transiciones, texto, etc.). El diferencial de este producto es la calidad del zoom automático, no ser un editor completo — eso es explícitamente fuera de scope hasta la Fase 4, y solo si se pide.
8. **No optimices prematuramente** el pipeline de render antes de que la Fase 1 funcione end-to-end con calidad aceptable. Primero correcto y completo, después rápido.

---

## REGLAS OPERATIVAS DURANTE LA IMPLEMENTACIÓN

- Cada crate en `crates/` debe tener tests unitarios reales, en particular `zoom-engine` (lógica pura, fácil de testear) y `project` (serialización/deserialización).
- Commits chicos y frecuentes, uno por sub-tarea coherente, no un commit gigante por fase.
- Si te encontrás implementando algo que no está descripto en `ARQUITECTURA.md` ni en este prompt, pausá y preguntá antes de asumir — especialmente si involucra una decisión de negocio (ver sección 7 de `ARQUITECTURA.md`, "PENDIENTE DE DECISIÓN").
- Documentá en el código (comentarios breves, no ensayos) el _por qué_ de las decisiones no obvias, especialmente en `zoom-engine` (los umbrales de detección de clicks/inactividad son ajustables y deben quedar claramente marcados como tal).
