# Beta Troubleshooting Guide

Use this guide for common Susun Studio beta failures.

## Daemon Cannot Start

Symptoms:
- Studio shows disconnected.
- `/v1/health` is unreachable.
- The packaged sidecar or `bun run daemon` does not become healthy.

Checks:
- The daemon address must be loopback only, for example `127.0.0.1:7377`.
- In development, run `bun run daemon` from the repository root.
- If port `7377` is busy in manual dev mode, use another loopback port and point the frontend at the same URL.
- Check the daemon log or terminal output for the first startup error.

Recovery:
- Stop stale daemon processes.
- Restart Studio or rerun `bun run daemon`.
- If local state appears corrupt, back up and remove `.susun-studio/studio.db`, then re-import projects.

## Engine Unavailable

Symptoms:
- Engine card shows unreachable.
- Runtime actions fail with an engine unavailable error.

Checks:
- Start Docker Desktop, Docker Engine, or another Docker-compatible local engine.
- Confirm `docker ps` works from the same user account.
- Recheck engine health in Studio.

Recovery:
- Restart the engine.
- Restart the Studio daemon if the engine socket changed.
- Export diagnostics if the engine remains unreachable.

## Project Import Diagnostics

Symptoms:
- Import completes with diagnostics.
- Import returns no project.

Checks:
- Confirm every Compose file path exists and is readable.
- Confirm overlay file order is intentional.
- Confirm the env file path exists if provided.
- Review diagnostics in the project detail panel.

Recovery:
- Fix YAML or Compose schema issues.
- Enable required profiles.
- Re-import the same project. Studio updates the existing record.

## Runtime Action Failed

Symptoms:
- Job status is failed.
- Action report shows failed steps.

Checks:
- Open the job report and inspect the first failed action.
- Check logs and engine events if containers started.
- Recheck engine reachability.

Recovery:
- Run a fresh plan.
- Fix image, port, volume, or dependency issues.
- Retry the action.
- Export diagnostics if the error is unclear.

## Watch Session Failed

Symptoms:
- Watch card shows failed.
- Last action error is set.

Checks:
- Confirm watched paths still exist.
- Confirm sync source paths are files and below the 64 MiB copy limit.
- Confirm target service has a running container for sync/restart workflows.

Recovery:
- Stop the failed session.
- Adjust paths or service names.
- Start a new watch session.

## Export Diagnostics

Use **Export diagnostics** when reporting beta issues. The bundle includes:
- `diagnostics.json`
- app log tail
- daemon log tail

The bundle redacts known sensitive values, truncates recent job errors, and reports only the database filename and size instead of the full path.
