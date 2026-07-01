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
- Local state: Turso Database owned by the daemon.
- Core Compose logic: Susun crates.
- Runtime strategy: bring your own Docker-compatible engine first; optional privileged helper and managed runtime providers later.

## Current Status

This repository is in Phase 1 foundation work.

The Tauri/SvelteKit client has been scaffolded. A separate `susun-studio-daemon` crate exposes `GET /v1/health`, authenticated project and settings endpoints, embedded startup SQL migrations, and Turso-backed local persistence. The desktop shell reads daemon health, project, and settings state from the local API.

## Development

Install frontend dependencies:

```powershell
bun install
```

Run frontend checks:

```powershell
bun run check
```

Run the local daemon:

```powershell
bun run daemon
```

By default the daemon listens on `127.0.0.1:7377`, uses the development token `susun-studio-dev-token`, and stores local state at `.susun-studio/studio.db`. Override these when needed:

```powershell
$env:SUSUN_STUDIO_DAEMON_ADDR = "127.0.0.1:7477"
$env:SUSUN_STUDIO_DAEMON_TOKEN = "dev-secret"
$env:SUSUN_STUDIO_DB_PATH = ".tmp/studio.db"
bun run daemon
```

Point the frontend at non-default daemon settings:

```powershell
$env:PUBLIC_SUSUN_STUDIO_DAEMON_URL = "http://127.0.0.1:7477"
$env:PUBLIC_SUSUN_STUDIO_DAEMON_TOKEN = "dev-secret"
bun run dev
```

Run the Tauri app during development:

```powershell
bun run tauri dev
```

Run Rust checks directly:

```powershell
cargo check --workspace
```

## Database Migrations

Daemon database migrations live in `crates/studio-daemon/migrations/` as plain SQL files and are embedded into the daemon binary at compile time.

Migrations run automatically on daemon startup before the HTTP API is served. Applied versions are recorded in `_studio_migrations`, so a packaged desktop install can upgrade its local database without requiring a separate migration command.

## Repository Notes

Private planning documents live under `docs/` and are intentionally ignored by Git, matching the Susun core workflow.

Generated artifacts such as `node_modules/`, `.svelte-kit/`, frontend build output, Tauri build output, Cargo `target/` directories, and `.susun-studio/` local daemon state should not be committed.
