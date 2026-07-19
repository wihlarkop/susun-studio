-- Extends the runtime_action_audit vocabulary for Phase 15c local image
-- mutations (tag, remove). SQLite CHECK constraints cannot be altered in
-- place, so the table is rebuilt with the same columns and indexes; existing
-- rows carry over unchanged.

CREATE TABLE runtime_action_audit_new (
    id TEXT PRIMARY KEY,
    action_kind TEXT NOT NULL CHECK (action_kind IN (
        'migration_commit',
        'migration_rollback',
        'destructive_repair',
        'destructive_reset_engine_data',
        'destructive_remove_built_in_runtime',
        'engine_prune',
        'metadata_restore',
        'image_tag',
        'image_remove'
    )),
    domain TEXT NOT NULL CHECK (domain IN ('migration', 'destructive', 'prune', 'restore', 'artifact')),
    profile_id TEXT,
    runtime_class TEXT,
    ownership_result TEXT NOT NULL,
    command_kind TEXT,
    elevation_mode TEXT,
    terminal_status TEXT NOT NULL,
    affected_counts_json TEXT NOT NULL DEFAULT '[]',
    app_version TEXT NOT NULL,
    failure_code TEXT,
    correlation_token TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER
);

INSERT INTO runtime_action_audit_new SELECT * FROM runtime_action_audit;

DROP TABLE runtime_action_audit;
ALTER TABLE runtime_action_audit_new RENAME TO runtime_action_audit;

CREATE UNIQUE INDEX runtime_action_audit_correlation_idx
    ON runtime_action_audit (correlation_token)
    WHERE correlation_token IS NOT NULL;

CREATE INDEX runtime_action_audit_started_idx
    ON runtime_action_audit (started_at_ms DESC);
