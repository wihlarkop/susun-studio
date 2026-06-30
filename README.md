# Susun Studio

Susun Studio is a cross-platform Compose-native desktop workspace built on top of Susun.

The product is designed as a daemon-first platform spine that can grow into a Docker Desktop replacement. The first versions target existing Docker-compatible engines, while the architecture starts with a separate local Studio daemon so engine, VM, task, and UI concerns can evolve independently.

## Product Direction

- Desktop shell: Tauri.
- Frontend tooling: Bun.
- Frontend framework: SvelteKit and TypeScript.
- UI foundation: Tailwind CSS, shadcn-svelte, Bits UI, lucide-svelte.
- Backend: separate user-level Rust Studio daemon plus thin Tauri client.
- Local API: loopback HTTP and WebSocket with local auth.
- Local state: Turso/libSQL owned by the daemon.
- Core Compose logic: Susun crates.
- Runtime strategy: bring your own Docker-compatible engine first; optional privileged helper and managed runtime providers later.

## Current Status

This repository is in Phase 1 foundation work.

The Tauri/SvelteKit client has been scaffolded. The next implementation step is to replace the starter UI with the Susun Studio app shell and add the separate `susun-studio-daemon` crate.

## Development

Install frontend dependencies:

```powershell
bun install
```

Run frontend checks:

```powershell
bun run check
```

Run the Tauri app during development:

```powershell
bun run tauri dev
```

Run the Tauri Rust checks directly:

```powershell
cd src-tauri
cargo check
```

## Repository Notes

Private planning documents live under `docs/` and are intentionally ignored by Git, matching the Susun core workflow.

Generated artifacts such as `node_modules/`, `.svelte-kit/`, frontend build output, Tauri build output, and Cargo `target/` directories should not be committed.