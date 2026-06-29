# ADR 0001: Initial Susun Studio Tech Stack

**Status:** Accepted

## Context

Susun Studio is a desktop product built on top of the Susun Rust crates. The app needs a polished cross-platform UI, native system integration, local state, and a path toward later engine/runtime management.

The product should be fast and low-memory. It should not start by building a custom managed runtime.

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
- Turso/libSQL for local state.
- Rust Tauri commands for backend logic.
- Susun crates for Compose analysis, planning, runtime, compatibility, and security workflows.

Do not use TanStack Table or TanStack Virtual as the default table/list layer.

## Consequences

- The frontend remains lightweight and easy to iterate.
- Rust owns the system-heavy work.
- The database can support future sync use cases.
- The UI component layer stays copy-owned and customizable.
- Heavy spreadsheet-like grids can be evaluated later only if product surfaces prove they need it.

## Deferred

- Managed runtime and VM strategy.
- Code signing and updater strategy.
- Hosted/team sync.
- GitHub PR review productization.
