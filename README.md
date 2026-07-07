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

Phases 1-6 are complete: Foundation, Susun Project Import, Planning Workspace, Engine Connections, Runtime Actions, and Project Operations UX (service lifecycle, logs/events streaming, exec/run/cp).

The Tauri/SvelteKit client is scaffolded and talks to a separate `susun-studio-daemon` Rust crate over a local, token-authenticated HTTP API. The daemon embeds SQL migrations that run automatically at startup and persists local state in a Turso database.

A user can import a real Docker Compose project through the Susun SDK, without Docker installed, and inspect the result: services, ports, volumes, networks, configs, secrets, active profiles, and parse diagnostics. Re-importing the same project updates its existing record instead of creating a duplicate.

Once an engine is connected, a project can be planned (dry-run diff of what `up`/`down` would do), executed as a cancellable background job with live progress over SSE, and operated day-to-day from a tabbed project workspace: live service state, logs (bounded viewer with pause/filter/export), engine events, and per-service lifecycle actions (start/stop/restart/wait/port lookup/exec/run/copy).

Current API surface:

- `GET /v1/health`
- `GET /v1/projects` / `POST /v1/projects`
- `POST /v1/projects/import`
- `POST /v1/projects/{id}/plans/{up,down}`, `GET /v1/projects/{id}/plans`, `GET /v1/plans/{id}`
- `GET /v1/engines`, `GET /v1/engines/{id}/health`, `GET /v1/engines/{id}/capabilities`
- `POST /v1/projects/{id}/actions/{up,down,build}`, `GET /v1/jobs`, `GET /v1/jobs/{id}`, `POST /v1/jobs/{id}/cancel`, `GET /v1/jobs/{id}/events` (ticketed SSE)
- `GET /v1/projects/{id}/snapshot`, `GET /v1/projects/{id}/streams/{logs,events}` (ticketed SSE)
- `POST /v1/projects/{id}/services/{service}/{start,stop,restart,wait,cp}`, `GET /v1/projects/{id}/services/{service}/ports`
- `GET /v1/projects/{id}/services/{service}/streams/{exec,run}` (ticketed SSE)
- `GET /v1/settings` / `PUT /v1/settings`

All routes other than `/v1/health` require the bearer token configured via `SUSUN_STUDIO_DAEMON_TOKEN`. The daemon only accepts requests from local dev origins (`localhost`/`127.0.0.1` on ports 1420/5173, plus `tauri://localhost`). SSE streams authenticate via short-lived, single-use, scope-bound tickets (issued by an authenticated POST) rather than a token in the URL.

**Known v1 limitations:** `exec` is non-interactive only (interactive TTY exec needs a bidirectional transport, deferred); `run` starts a disposable one-off container with the service's env/volumes/networks but no published ports and no config/secret mounts; image build is unsupported by the underlying `BollardEngine`.

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

## Packaging & Releases (Phase 10)

The app now spawns and supervises `susun-studio-daemon` itself as a bundled Tauri sidecar (with a fresh per-launch auth token and a freshly reserved loopback port) instead of requiring `bun run daemon` to be started by hand — that manual dev workflow still works too and is auto-detected in debug builds.

Two things are still placeholders and must be filled in with real values before a packaged build will run or the updater will work:

1. **Sidecar binary.** `tauri.conf.json`'s `bundle.externalBin` requires `src-tauri/binaries/susun-studio-daemon-<target-triple>[.exe]` to actually exist — Tauri's build script fails without it. Build it first:

   ```powershell
   bun run build:sidecar
   ```

   Run this before `bun run tauri dev` or `bun run tauri build` (it's also wired into `beforeBuildCommand`, so a full `tauri build` does it automatically — but `cargo check`/`clippy`/`build` on `src-tauri` directly will fail until you've run it at least once).

2. **Updater signing keypair.** `src-tauri/tauri.conf.json`'s `plugins.updater.pubkey` is currently the literal placeholder string `REPLACE_WITH_PUBKEY_FROM_bun_run_tauri_signer_generate` — `src-tauri` will not compile until this is a real key. Generate one (one-time, not committed):

   ```powershell
   bun run tauri signer generate -w "$HOME/.tauri/susun-studio-updater.key"
   ```

   Paste the printed public key into `tauri.conf.json`'s `plugins.updater.pubkey`. Save the private key file and its password somewhere secure — they become the `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` GitHub Actions secrets needed by `.github/workflows/release.yml` (tag-triggered, builds Windows/macOS x2/Linux via `tauri-action`, publishes a draft GitHub Release with an unsigned installer plus a signed `latest.json` updater manifest). This key signs _update artifacts_, not the app binary itself — OS code signing (Windows/macOS/Linux) is separately deferred; see the "signing plan" note below.

Two more top-bar features shipped alongside packaging: **Check for updates** (two-step — shows an "Install update" state and waits for a second click before downloading/installing/relaunching, never auto-installs) and **Export diagnostics** (saves a redacted `.tar` with app/daemon log tails, DB filename-only + size, recent failed-job errors truncated to 200 chars, and engine reachability).

Full design notes (signing plan, per-release checklist) live in gitignored `docs/` — re-derive them from `docs/superpowers/specs/2026-07-07-packaging-signing-plan.md` and `docs/superpowers/plans/studio/release-checklist.md` on a machine that still has that working tree, or from the implementation plan at `docs/superpowers/plans/2026-07-07-phase-10-packaging-and-updates.md`.

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
