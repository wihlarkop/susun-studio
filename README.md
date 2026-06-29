# Susun Studio

Susun Studio is a planned cross-platform Compose-native desktop workspace built on top of Susun.

The first version will be a fast, low-memory control plane for existing Docker-compatible engines. It will focus on understanding, planning, and operating Compose projects rather than replacing the container runtime.

## Product Direction

- Desktop shell: Tauri.
- Frontend tooling: Bun.
- Frontend framework: SvelteKit.
- UI foundation: Tailwind CSS, shadcn-svelte, Bits UI, lucide-svelte.
- Backend: Rust Tauri commands and task services.
- Local state: Turso/libSQL.
- Core Compose logic: Susun crates.
- Runtime strategy: bring your own Docker-compatible engine first; managed runtime research later.

## Current Status

This repository is in planning/bootstrap mode.

Implementation should begin after the Susun Phase 5 stack is merged and stable enough for Studio to depend on it.

## Development

The desktop app has not been scaffolded yet. Once Phase 1 starts, use Bun for frontend commands:

```powershell
bun install
bun run tauri dev
```

## Private Planning Docs

Local planning documents live under `docs/` and are intentionally ignored by Git, matching the workflow used in the Susun core repository.
