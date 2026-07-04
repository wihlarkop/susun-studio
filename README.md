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

Phase 1 (Foundation) and Phase 2 (Susun Project Import) are complete. Phase 3 (Planning Workspace) is next.

The Tauri/SvelteKit client is scaffolded and talks to a separate `susun-studio-daemon` Rust crate over a local, token-authenticated HTTP API. The daemon embeds SQL migrations that run automatically at startup and persists local state in a Turso database.

A user can import a real Docker Compose project through the Susun SDK, without Docker installed, and inspect the result: services, ports, volumes, networks, configs, secrets, active profiles, and parse diagnostics. Re-importing the same project updates its existing record instead of creating a duplicate.

Current API surface:

- `GET /v1/health`
- `GET /v1/projects` / `POST /v1/projects`
- `POST /v1/projects/import`
- `GET /v1/settings` / `PUT /v1/settings`

All routes other than `/v1/health` require the bearer token configured via `SUSUN_STUDIO_DAEMON_TOKEN`. The daemon only accepts requests from local dev origins (`localhost`/`127.0.0.1` on ports 1420/5173, plus `tauri://localhost`).

## Prerequisites

The daemon depends on the [Susun](https://github.com/wihlarkop/susun) core crates via a local path dependency (`crates/studio-daemon/Cargo.toml`), not a published crate. Clone `susun` as a sibling directory before building:

```
Project/
├── susun/
└── susun-studio/
```

This layout is required for local builds and is why CI checks out `susun` into a subdirectory and symlinks it into place before running any Rust step.

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

## Checks

Frontend:

```powershell
bun run check       # svelte-check
bun run lint        # oxlint
bun run fmt:check   # oxfmt
bun run test        # vitest
```

Rust:

```powershell
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI (`.github/workflows/ci.yml`) runs all of the above on every push, checking out `susun` as a sibling directory first.

## Database Migrations

Daemon database migrations live in `crates/studio-daemon/migrations/` as plain SQL files and are embedded into the daemon binary at compile time.

Migrations run automatically on daemon startup before the HTTP API is served. Applied versions are recorded in `_studio_migrations`, so a packaged desktop install can upgrade its local database without requiring a separate migration command.

## Repository Notes

Private planning documents live under `docs/` and are intentionally ignored by Git, matching the Susun core workflow.

Generated artifacts such as `node_modules/`, `.svelte-kit/`, frontend build output, Tauri build output, Cargo `target/` directories, and `.susun-studio/` local daemon state should not be committed.
