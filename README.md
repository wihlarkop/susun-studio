# Susun Studio

Susun Studio is a planned cross-platform Compose-native desktop workspace built on top of Susun.

The product is designed as a daemon-first platform spine that can grow into a Docker Desktop replacement. The first version still targets existing Docker-compatible engines, but the architecture starts with a separate local Studio daemon so engine, VM, task, and UI concerns can evolve independently.

## Product Direction

- Desktop shell: Tauri.
- Frontend tooling: Bun.
- Frontend framework: SvelteKit.
- UI foundation: Tailwind CSS, shadcn-svelte, Bits UI, lucide-svelte.
- Backend: separate user-level Rust Studio daemon plus thin Tauri client.
- Local API: loopback HTTP and WebSocket with local auth.
- Local state: Turso/libSQL owned by the daemon.
- Core Compose logic: Susun crates.
- Runtime strategy: bring your own Docker-compatible engine first; optional privileged helper and managed runtime providers later.

## Current Status

This repository is in planning/bootstrap mode.

Implementation should begin after the Susun Phase 5 stack is merged and stable enough for Studio to depend on it.

Phase 1 should prove the daemon boundary with real workspace and project persistence before runtime operations are added.

## Development

The desktop app has not been scaffolded yet. Once Phase 1 starts, use Bun for frontend commands:

```powershell
bun install
bun run tauri dev
```

## Private Planning Docs

Local planning documents live under `docs/` and are intentionally ignored by Git, matching the workflow used in the Susun core repository.
