CREATE TABLE artifact_transfer_progress (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    operation TEXT NOT NULL,
    stage TEXT NOT NULL,
    current_units INTEGER,
    total_units INTEGER,
    message TEXT,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX artifact_transfer_progress_job_id_idx
    ON artifact_transfer_progress (job_id, sequence ASC);
