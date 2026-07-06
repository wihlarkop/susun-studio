CREATE TABLE watch_sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    status TEXT NOT NULL,
    action TEXT NOT NULL,
    services_json TEXT NOT NULL,
    sync_specs_json TEXT NOT NULL,
    watch_paths_json TEXT NOT NULL,
    debounce_ms INTEGER NOT NULL,
    track_restart_as_job INTEGER NOT NULL DEFAULT 0,
    last_action_status TEXT,
    last_action_error TEXT,
    error TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_watch_sessions_project ON watch_sessions(project_id);

CREATE TABLE watch_events (
    watch_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    event_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_watch_events_watch ON watch_events(watch_id, sequence);
