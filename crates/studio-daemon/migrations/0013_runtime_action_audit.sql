-- Runtime Security 4: bounded, redacted audit trail for trusted runtime plans
-- and destructive data operations (migration, reset/remove/repair, engine prune,
-- metadata restore).
--
-- These rows are user-visible. They deliberately store ONLY enumerated codes and
-- integer counts — never argv, environment, credentials, endpoint values, private
-- paths, container output, or registry tokens. Ownership evidence lives elsewhere
-- (runtime_profiles.owner_token, runtime_ownership_events); the clear-history
-- action empties this table without touching that evidence.

CREATE TABLE runtime_action_audit (
    id TEXT PRIMARY KEY,
    -- Stable action vocabulary (audited even when rejected before execution).
    action_kind TEXT NOT NULL CHECK (action_kind IN (
        'migration_commit',
        'migration_rollback',
        'destructive_repair',
        'destructive_reset_engine_data',
        'destructive_remove_built_in_runtime',
        'engine_prune',
        'metadata_restore'
    )),
    domain TEXT NOT NULL CHECK (domain IN ('migration', 'destructive', 'prune', 'restore')),
    -- Runtime/engine identity + class for attribution. Nullable: prune/restore
    -- are not always tied to a single runtime profile.
    profile_id TEXT,
    runtime_class TEXT,
    -- Result of the ownership / authorization revalidation gate.
    ownership_result TEXT NOT NULL,
    -- The safe command kind actually authorized (e.g. 'metadata_only',
    -- 'provider_prune', 'deferred_provider_reset'), never a raw command line.
    command_kind TEXT,
    -- Elevation posture: 'none' | 'current_user' | 'os_mediated_consent'.
    elevation_mode TEXT,
    -- Terminal status: completed | failed | cancelled | rejected | deferred_to_phase_14b.
    terminal_status TEXT NOT NULL,
    -- Redacted affected-resource counts only: [{"category":"...","count":N}].
    affected_counts_json TEXT NOT NULL DEFAULT '[]',
    app_version TEXT NOT NULL,
    -- Short redacted failure code (e.g. 'stale_preview', 'provider_unreachable').
    -- Never a raw error string, path, or secret.
    failure_code TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER
);

CREATE INDEX runtime_action_audit_started_idx
    ON runtime_action_audit (started_at_ms DESC);
