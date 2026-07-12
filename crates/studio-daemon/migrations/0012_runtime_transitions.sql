-- Runtime Data 3: durable runtime migration and destructive-operation records.

CREATE TABLE runtime_migrations (
    id TEXT PRIMARY KEY,
    source_profile_id TEXT NOT NULL,
    target_profile_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('completed', 'failed', 'rolled_back')),
    project_count INTEGER NOT NULL,
    project_ids_json TEXT NOT NULL,
    skipped_items_json TEXT NOT NULL,
    failures_json TEXT NOT NULL,
    rollback_available INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER NOT NULL,
    rolled_back_at_ms INTEGER
);

CREATE INDEX runtime_migrations_created_idx
    ON runtime_migrations (created_at_ms DESC);

CREATE TABLE runtime_destructive_operations (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('repair', 'reset_engine_data', 'remove_built_in_runtime')),
    status TEXT NOT NULL CHECK (status IN ('prepared', 'completed', 'partial_failure', 'failed')),
    scope_json TEXT NOT NULL,
    result_json TEXT,
    recovery_guidance TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER
);

CREATE INDEX runtime_destructive_operations_profile_idx
    ON runtime_destructive_operations (profile_id, created_at_ms DESC);
