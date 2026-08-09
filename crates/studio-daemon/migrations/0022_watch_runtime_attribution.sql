ALTER TABLE watch_sessions ADD COLUMN runtime_profile_id TEXT;
ALTER TABLE watch_sessions ADD COLUMN runtime_class TEXT
    CHECK (runtime_class IS NULL OR runtime_class IN ('built_in', 'external_local', 'external_remote'));
ALTER TABLE watch_sessions ADD COLUMN runtime_binding_source TEXT
    CHECK (runtime_binding_source IS NULL OR runtime_binding_source IN ('project_pin', 'global_preference', 'platform_default'));

CREATE INDEX idx_watch_sessions_active_runtime
    ON watch_sessions(status, runtime_profile_id, runtime_binding_source);
