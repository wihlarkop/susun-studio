# Contributing

Susun Studio is currently in planning/bootstrap mode.

## Workflow

- Keep product planning docs local under `docs/`.
- Track public project decisions outside `docs/`.
- Use focused branches and pull requests.
- Prefer small commits that map to the phase plans.

## Tooling

- Use Bun for frontend package management and scripts.
- Use Rust for Tauri backend code and the Studio daemon.
- Use Turso Database through the `turso` Rust crate for local daemon state.
- Use Tauri for desktop packaging.

## Quality Gates

Expected local checks are:

```powershell
bun run check
cargo fmt --all --check
cargo check --workspace
```

Runtime and packaging checks should stay lightweight until the feature code is ready for full Tauri builds.
