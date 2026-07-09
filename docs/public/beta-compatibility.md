# Beta Compatibility Matrix

This page describes the current Susun Studio beta surface. It is intentionally conservative: if a workflow is not listed as supported, treat it as partial or not yet available.

## Supported

| Area | Status |
|---|---|
| Compose import | Multiple Compose files, optional env file, project name override, and profiles. |
| Project inspection | Services, ports, volumes, networks, configs, secrets, dependencies, active profiles, and diagnostics. Secret contents are not displayed. |
| Planning | `up` and `down` dry-run plans without requiring Docker. |
| Local engine | Docker-compatible local engine through the Susun runtime adapter. |
| Runtime actions | `up`, `down`, `clean`, service start/stop/restart/wait/ports, exec, run, and copy. |
| Jobs | Durable job history, cancellation, basic recovery marking, and recent results. |
| Logs and events | Ticketed event streams for project logs, engine events, job events, exec, run, and watch. |
| Watch | Rebuild, restart, sync, and sync-restart sessions owned by the daemon. |
| Diagnostics bundle | Redacted local `.tar` export with diagnostics JSON and app/daemon log tails. |
| System prune | Containers, networks, volumes, and images, with explicit confirmation. |

## Partial Or Limited

| Area | Limit |
|---|---|
| Image build | Build support depends on the current engine adapter and may be unavailable. |
| Exec | Non-interactive only. Interactive TTY is not available in this beta. |
| Run | One-off containers use service env, volumes, and networks, but no published ports and no config/secret mounts. |
| Watch sync | File copy is capped at 64 MiB. Removed-file sync is intentionally non-destructive. |
| Diagnostics redaction | Key-based heuristic. Common secret-bearing keys are redacted, but arbitrary value-only secrets cannot be guaranteed. |
| Local database privacy | Data is local to the user profile. Anyone with filesystem access to that profile can read the local database. |
| Engine management | The UI assumes the default local Docker-compatible engine. Managed runtime and provider switching are later work. |

## Not Supported In This Beta

| Area | Notes |
|---|---|
| Managed runtime | Planned after beta. |
| Privileged helper | Designed, not implemented. |
| Remote daemon access | Explicit non-goal. Studio is local-only. |
| Registry auth and push/pull UI | Later image/registry phase. |
| Image and build-cache detail views | Later image/build phase. |
| Docker Desktop migration tooling | Later parity phase. |
| Automatic telemetry upload | Beta policy is local-only diagnostics. |
