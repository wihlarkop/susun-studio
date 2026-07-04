-- No FOREIGN KEYs (turso silently discards inserts into FK tables). Integrity is
-- enforced in the daemon: actions look up the project first.
CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    project_id TEXT NOT NULL,
    engine_id TEXT NOT NULL,
    request_json TEXT NOT NULL,
    result_json TEXT,
    error TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS jobs_project_id_idx ON jobs (project_id, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS job_events (
    job_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    event_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (job_id, sequence)
);
