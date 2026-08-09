# External Runtime Migration And Recovery

Susun Studio supports a built-in **Susun Runtime**, powered by Podman, alongside existing external runtimes. Existing Podman installations and Docker Desktop remain **External**. Studio can use them, but it does not take ownership of them, start or stop them as Studio-managed resources, or transfer that ownership during migration.

## Connect To An External Runtime

Use **Use an external runtime** to change Studio's global preferred runtime. Studio first shows a daemon-owned impact preview and requires confirmation.

This changes the default context for unpinned projects only. It does not create project pins, does not make a runtime Studio-managed, and does not create a migration record. Existing explicit project pins continue to win over the global preference.

If the selected runtime is missing or unavailable, Studio blocks the change. It never silently falls back to another runtime.

## Move Explicit Project Bindings

Use **Move explicit project bindings** when projects already have explicit pins and need to move from one runtime profile to another.

Studio previews the exact selected bindings before any change. The target must be present, selectable, reachable, and compatible. Active jobs or watch sessions can block a move. The preview expires, and Studio repeats validation when you confirm it; if state changed, prepare a fresh preview.

An unavailable or missing source can still be used for recovery when Studio's local metadata proves the affected explicit pins. The target is never allowed to be unavailable.

Only the selected `projects.runtime_profile_id` metadata is moved. Unpinned projects remain unpinned, the global preference does not change, and historical job attribution remains unchanged.

## What Migration Does Not Copy

Migration does not copy or transfer:

- images
- containers
- volumes
- networks
- registry credentials
- project files
- runtime settings
- runtime ownership
- engine data or runtime configuration

Prepare or rebuild the needed engine data on the target runtime using the normal Studio workflows.

## Rollback Limits

Rollback is metadata-only and available only when the recorded migration remains safe to reverse. It always requires a new rollback preview followed by a separate explicit confirmation. Studio revalidates the recorded pins and active work at commit time.

Rollback restores only the exact explicit project pins recorded by that migration. It does not restore engine data, credentials, runtime configuration, ownership, or the global preference.

## Compatibility And Current Limits

Compatibility is daemon-probed for the exact runtime profile. Studio does not infer compatibility from names, profile identifiers, endpoints, or version strings, and it does not apply an invented minimum engine-version floor.

Endpoint-pinned image builds remain unsupported. Remote arbitrary runtime registration and dynamic runtime plugins are also unsupported in this beta. Studio is local-only.

Use the Runtime page to inspect compatibility before choosing a runtime. Use the diagnostics bundle when a target remains unavailable after recheck.
