# ADR 0001: Initial Susun Studio Tech Stack

**Status:** Accepted

## Context

Susun Studio is a desktop product built on top of the Susun Rust crates. The app needs a polished cross-platform UI, native system integration, local state, and a path toward engine, VM, and runtime management.

The product should be fast and low-memory. It should not start by building a custom managed runtime, but it should be architected from day one as a Docker Desktop replacement candidate rather than a UI-only wrapper.

## Decision

Use:

- Tauri for the desktop shell.
- Bun for frontend package management and scripts.
- SvelteKit for the frontend framework.
- Tailwind CSS for styling.
- shadcn-svelte and Bits UI for UI components and primitives.
- lucide-svelte for icons.
- Studio-owned table helpers for normal tables.
- Custom virtualized log/event views for high-volume streams.
- Turso Database through the `turso` Rust crate for local state.
- Thin Rust Tauri integration hooks for client-side shell integration and daemon process coordination.
- A separate user-level Rust daemon, `susun-studio-daemon`, as the primary backend boundary.
- Loopback HTTP and WebSocket as the Phase 1 daemon transport.
- A per-install local auth token for daemon requests and WebSocket upgrades.
- A split daemon security model: user daemon first, optional privileged helper later.
- Susun crates for Compose analysis, planning, runtime, compatibility, and security workflows.

Do not use TanStack Table or TanStack Virtual as the default table/list layer.

## Consequences

- The frontend remains lightweight and easy to iterate.
- Rust owns the system-heavy work.
- The Tauri app is a client of the Studio daemon, not the owner of long-running backend tasks.
- UI restarts do not need to kill daemon tasks, watch jobs, log streams, or future engine operations.
- CLI, tray, browser, and future automation clients can reuse the same local daemon API.
- The database can support future sync use cases.
- The UI component layer stays copy-owned and customizable.
- Heavy spreadsheet-like grids can be evaluated later only if product surfaces prove they need it.
- Phase 1 has more backend surface area than a Tauri-command-only app, but the daemon boundary prevents a rewrite when engine/VM management arrives.

## Deferred

- Managed runtime and VM strategy.
- Privileged helper implementation.
- Code signing and updater strategy.
- Hosted/team sync.
- GitHub PR review productization.
