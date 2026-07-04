-- No FOREIGN KEY, matching the rest of the schema (projects/settings have none).
-- Referential integrity is enforced in the daemon: create_plan looks up the
-- project first and returns ProjectNotFound when it is missing.
CREATE TABLE IF NOT EXISTS plans (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    plan_json TEXT NOT NULL,
    summary_json TEXT NOT NULL,
    blocked_diagnostics_json TEXT,
    susun_schema_version TEXT,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS plans_project_id_idx ON plans (project_id, created_at_ms DESC);
