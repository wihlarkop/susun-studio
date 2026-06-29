# Contributing

Susun Studio is currently in planning/bootstrap mode.

## Workflow

- Keep product planning docs local under `docs/`.
- Track public project decisions outside `docs/`.
- Use focused branches and pull requests.
- Prefer small commits that map to the phase plans.

## Tooling

- Use Bun for frontend package management and scripts.
- Use Rust for Tauri backend code.
- Use Turso/libSQL for local state.
- Use Tauri for desktop packaging.

## Quality Gates

When implementation starts, expected local checks are:

```powershell
bun run check
cd src-tauri
cargo fmt --all --check
cargo check
cargo clippy --all-targets -- -D warnings
```

Runtime and packaging checks will be added after the app is scaffolded.
