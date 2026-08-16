# CLAUDE.md

Este archivo le da contexto a Claude Code (u otro agente de IA) cuando trabaja en este repositorio. Léelo completo antes de tocar código. Para el _por qué_ de las decisiones de arquitectura, ver `docs/ARQUITECTURA.md`. Para el orden de implementación, ver `docs/PROMPT_AGENTE_DEV.md`.

## Qué es este proyecto

App de escritorio de screen recording con zoom automático (estilo Screen Studio), Windows primero, macOS después. Uso personal por ahora, sin backend, sin licensing, 100% local.

## Stack y versiones

| Componente            | Versión / detalle                                                                                                                       |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Tauri                 | v2 (2.10+)                                                                                                                              |
| Rust                  | edición 2024 (o la estable más reciente al momento de iniciar el proyecto — fijar en `Cargo.toml` del workspace y no cambiar sin razón) |
| Node                  | LTS activo al momento de iniciar (fijar en `.nvmrc`)                                                                                    |
| Frontend              | React + TypeScript + Vite + Tailwind CSS                                                                                                |
| Gestor de paquetes JS | pnpm (workspaces)                                                                                                                       |
| Captura de pantalla   | `windows-capture` (Windows.Graphics.Capture)                                                                                            |
| Tracking de input     | `rdev`                                                                                                                                  |
| Compositing/render    | `wgpu` + WGSL                                                                                                                           |
| Encoding              | `ffmpeg` (sidecar de Tauri)                                                                                                             |
| Auto-update           | `tauri-plugin-updater`                                                                                                                  |

No actualices ninguna de estas dependencias a una versión mayor sin verificar que no rompe la arquitectura descripta en `docs/ARQUITECTURA.md`.

## Estructura de carpetas

```
screenzoom/
├── apps/desktop/          # App Tauri: frontend (src/) + backend (src-tauri/)
├── crates/                # Lógica Rust pura, sin dependencia de Tauri
│   ├── capture/
│   ├── input-tracker/
│   ├── zoom-engine/
│   ├── compositor/
│   ├── exporter/
│   └── project/
├── sidecars/ffmpeg/       # Binarios ffmpeg por plataforma (no versionados en git)
├── docs/                  # Este archivo, ARQUITECTURA.md, PROMPT_AGENTE_DEV.md
└── .env.example
```

Ver `docs/ARQUITECTURA.md` sección 5 para el detalle completo con subcarpetas.

## Comandos

```bash
# Instalar dependencias (JS + Rust)
pnpm install

# Desarrollo (levanta Vite + Tauri en modo dev, hot reload de UI y de Rust vía cargo-watch si está configurado)
pnpm tauri dev

# Build de producción (genera instalador para la plataforma actual)
pnpm tauri build

# Tests de Rust (todos los crates del workspace, incluyendo apps/desktop/src-tauri)
cargo test --workspace

# Tests de un crate específico
cargo test -p zoom-engine

# Lint/format Rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# Lint/format frontend
pnpm lint
pnpm format
```

Si alguno de estos comandos no existe todavía porque el proyecto está en etapa temprana, configuralo como parte del scaffold de la Fase 0 — no asumas que ya existe.

## Convenciones de código

- **Rust**: `cargo fmt` con configuración default, `clippy` sin warnings permitidos (`-D warnings`) antes de cualquier commit a una rama compartida. Nombres de crates y módulos en `snake_case`, tipos en `PascalCase`.
- **TypeScript/React**: componentes en `PascalCase.tsx`, hooks propios prefijados `use*`, un componente por archivo salvo componentes triviales de soporte.
- **Comandos Tauri**: `snake_case` en Rust (`start_recording`), se invocan igual desde TS (`invoke("start_recording")`), sin traducir el nombre.
- **Comentarios**: explicar el _por qué_, no el _qué_ — el código ya dice qué hace. Especial atención a comentar los parámetros ajustables del `zoom-engine` (umbrales de detección de clicks/inactividad, duración default de easing).

## Reglas específicas de este proyecto (NO negociables)

### 1. Nunca bloquear el hilo principal con procesamiento de video

Captura, tracking de input, composición GPU y encoding **siempre** corren en threads dedicados (`std::thread::spawn`) o tasks async de tokio marcadas explícitamente como `blocking` cuando corresponda (`tokio::task::spawn_blocking`). El hilo que maneja el loop de eventos de Tauri/UI no debe, bajo ninguna circunstancia, ejecutar código de captura, decodificación de video, render GPU o llamadas bloqueantes a `ffmpeg`. La comunicación de progreso hacia la UI se hace por eventos (`app.emit`), nunca haciendo que la UI espere sincrónicamente un resultado largo.

### 2. Separación estricta: `crates/` no conoce Tauri

Ningún crate dentro de `crates/` puede tener `tauri` en su `Cargo.toml`. Si sentís la necesidad de importar algo de Tauri dentro de un crate, es señal de que ese código pertenece a `apps/desktop/src-tauri/src/commands/`, que actúa como capa de adaptación entre la UI y la lógica pura.

### 3. Modelo de datos no-destructivo

Nunca se sobrescribe la "toma cruda" (`raw_take`) grabada. Todo ajuste de zoom, estilo o edición se guarda en el archivo `.szproj` como metadata separada. El render final siempre se genera a partir de: toma cruda + `.szproj`, nunca modificando el video fuente. Esto permite re-exportar con cambios sin volver a grabar.

### 4. Manejo de permisos de captura de pantalla

**Windows**: Windows.Graphics.Capture no requiere un diálogo de permisos del sistema como macOS, pero sí requiere que el proceso tenga privilegios normales de usuario y que el picker de fuente (monitor/ventana) se presente correctamente vía la API. Manejar explícitamente el caso en que el usuario cancela el picker o no hay fuentes disponibles, sin crashear.

**macOS (cuando se implemente en Fase 4)**: ScreenCaptureKit requiere permiso explícito de "Screen Recording" en Preferencias del Sistema. La app debe: (a) detectar si el permiso no está otorgado, (b) mostrar una pantalla explicando qué hacer (no solo un error genérico), (c) idealmente, abrir directamente el panel de Preferencias del Sistema correspondiente. Nunca asumir que el permiso está otorgado sin chequearlo primero.

### 5. Gestión del binario de ffmpeg

El binario de `ffmpeg` no se versiona en git (pesa demasiado). Se descarga en el proceso de build/setup desde una fuente confiable, con la variante de licencia LGPL-only (ver `.env.example` y sección 7 de `ARQUITECTURA.md` sobre implicancias de licencia GPL de `libx264`). El script de setup debe verificar el hash del binario descargado antes de usarlo.

### 6. Testing de `zoom-engine`

Es el módulo con más lógica de negocio pura y el diferencial del producto. Todo cambio a la lógica de detección de puntos de interés o generación de keyframes debe venir acompañado de tests unitarios que cubran al menos: clicks aislados, clusters de clicks cercanos en tiempo, períodos de inactividad, y el caso borde de una grabación sin ningún evento de input.

### 7. Errores y crashes durante grabación

La escritura de la toma cruda a disco debe ser resiliente: si la app crashea a mitad de una grabación, el archivo parcial ya escrito debe seguir siendo un video válido y reproducible hasta el último frame flusheado (no un archivo corrupto). Preferir formatos/contenedores que toleren esto (fragmented mp4 o similar) sobre contenedores que solo son válidos si se cierran correctamente.

## Qué NO hacer (ver también `docs/PROMPT_AGENTE_DEV.md`)

No implementar soporte macOS, autenticación, licensing, telemetría, o un editor de video genérico tipo NLE sin que se pida explícitamente. No cambiar de stack. No agregar abstracciones genéricas sin una necesidad concreta actual.
