# Beta Compatibility Matrix

This page describes the current Susun Studio beta surface. It is intentionally conservative: if a workflow is not listed as supported, treat it as partial or not yet available.

## Supported

| Area               | Status                                                                                                                                   |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Compose import     | Multiple Compose files, optional env file, project name override, and profiles.                                                          |
| Project inspection | Services, ports, volumes, networks, configs, secrets, dependencies, active profiles, and diagnostics. Secret contents are not displayed. |
| Planning           | `up` and `down` dry-run plans without requiring Docker.                                                                                  |
| Runtime choice     | A persisted preferred runtime, explicit project pins, fail-closed unavailable bindings, and daemon-owned impact previews.                |
| Susun Runtime      | Studio-managed built-in runtime, powered by Podman. Its lifecycle actions require a trusted preview and confirmation.                    |
| External runtimes  | Existing Podman and Docker Desktop profiles remain external. Studio does not adopt or take ownership of them.                            |
| Runtime actions    | `up`, `down`, `clean`, service start/stop/restart/wait/ports, exec, run, and copy.                                                       |
| Jobs               | Durable job history, cancellation, basic recovery marking, and recent results.                                                           |
| Logs and events    | Ticketed event streams for project logs, engine events, job events, exec, run, and watch.                                                |
| Watch              | Rebuild, restart, sync, and sync-restart sessions owned by the daemon.                                                                   |
| Diagnostics bundle | Redacted local `.tar` export with diagnostics JSON and app/daemon log tails.                                                             |
| System prune       | Containers, networks, volumes, and images, with explicit confirmation.                                                                   |
| Artifacts          | Read-only container/image/build-cache/registry views plus guarded image tag/remove and scoped prune actions where supported.             |
| Registry transfers | Guarded image pull/push jobs and local OS-managed registry credential metadata where the selected runtime supports them.                 |
| Runtime migration  | Guarded metadata-only movement of selected explicit project pins, recovery from a missing source, bounded history, and rollback preview. |

## Partial Or Limited

| Area                   | Limit                                                                                                                    |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Image build            | Durable local Buildx jobs are supported only for the platform-default engine. Endpoint-pinned builds remain unsupported. |
| Exec                   | Non-interactive only. Interactive TTY is not available in this beta.                                                     |
| Run                    | One-off containers use service env, volumes, and networks, but no published ports and no config/secret mounts.           |
| Watch sync             | File copy is capped at 64 MiB. Removed-file sync is intentionally non-destructive.                                       |
| Diagnostics redaction  | Key-based heuristic. Common secret-bearing keys are redacted, but arbitrary value-only secrets cannot be guaranteed.     |
| Local database privacy | Data is local to the user profile. Anyone with filesystem access to that profile can read the local database.            |
| Runtime compatibility  | Compatibility is daemon-probed and profile-specific. There is no invented minimum engine-version floor.                  |
| Runtime migration      | Migration moves metadata bindings only. It does not copy engine data or transfer runtime ownership.                      |

## Not Supported In This Beta

| Area                        | Notes                                                                 |
| --------------------------- | --------------------------------------------------------------------- |
| Privileged helper           | Designed, not implemented.                                            |
| Remote daemon access        | Explicit non-goal. Studio is local-only.                              |
| Remote runtime registration | Explicit non-goal. Studio does not accept arbitrary remote endpoints. |
| Dynamic runtime plugins     | Explicit non-goal for this beta.                                      |
| Automatic telemetry upload  | Beta policy is local-only diagnostics.                                |
