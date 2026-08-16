# screenzoom

App de escritorio de screen recording con zoom automatico estilo Screen Studio.
Windows primero, macOS despues. Uso personal, sin backend, 100% local.

> Nombre del producto: placeholder `screenzoom`, PENDIENTE DE DECISION (ver `docs/ARQUITECTURA.md` seccion 7).

## Documentacion

- `docs/ARQUITECTURA.md` — decisiones de arquitectura, stack, research competitivo, modelo de datos.
- `docs/PROMPT_AGENTE_DEV.md` — orden de implementacion por fases y reglas operativas.

Leer ambos completos antes de tocar codigo.

## Stack

Tauri v2 + Rust (core) + React/TypeScript/Vite/Tailwind (UI). Ver tabla completa en `ARQUITECTURA.md` seccion 0.

## Requisitos de entorno

- Rust (toolchain estable, target `x86_64-pc-windows-msvc`) + Visual Studio Build Tools (C++ workload) + Windows 10/11 SDK.
- Node 24 LTS (ver `.nvmrc`) + pnpm.

## Comandos

\`\`\`bash
pnpm install
pnpm tauri dev        # desarrollo
pnpm tauri build       # build de produccion
cargo test --workspace # tests de Rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
pnpm lint
pnpm format
\`\`\`

## Estado

En desarrollo — Fase 0 (spike de riesgo tecnico: captura nativa + input tracking). Ver `docs/PROMPT_AGENTE_DEV.md` para el roadmap completo.
