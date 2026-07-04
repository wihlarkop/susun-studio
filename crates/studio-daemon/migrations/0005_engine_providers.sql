CREATE TABLE IF NOT EXISTS engines (
    id TEXT PRIMARY KEY,
    provider_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    connection_config_json TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    is_default INTEGER NOT NULL DEFAULT 0,
    last_health_json TEXT,
    last_health_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL
);

INSERT OR IGNORE INTO engines (id, provider_kind, display_name, enabled, is_default, created_at_ms)
VALUES ('engine-docker-local', 'docker_local', 'Local Docker', 1, 1, unixepoch('subsec') * 1000);
