-- Phase 15d: durable image-build jobs reuse the existing `jobs` table (a new
-- `kind = 'image_build'` row, no schema change needed there since `kind` is
-- free text) but need their own ordered, bounded progress history — build
-- progress events are a different shape from `job_events`' up/down
-- RuntimeEvent payloads, and explicitly need a per-job retention bound
-- `job_events` does not have.
--
-- No FOREIGN KEY to `jobs`, matching this project's established convention
-- (turso silently discards inserts into FK tables); integrity is enforced in
-- the daemon.
CREATE TABLE build_job_progress (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    -- 'started' | 'vertex_started' | 'vertex_progress' | 'vertex_log' | 'vertex_finished' | 'finished'
    kind TEXT NOT NULL,
    vertex_id TEXT,
    -- 'stdout' | 'stderr', only set for kind = 'vertex_log'
    log_stream TEXT,
    -- Redacted (susun_build already redacts) and length-bounded text, only
    -- set for kind = 'vertex_log'.
    text TEXT,
    -- 'succeeded' | 'failed' | 'cancelled', only set for kind = 'vertex_finished'
    status TEXT,
    current_units INTEGER,
    total_units INTEGER,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX build_job_progress_job_id_idx
    ON build_job_progress (job_id, sequence ASC);
