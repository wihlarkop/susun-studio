-- The first cut of 0003 declared a FOREIGN KEY on plans. We dropped it to stay
-- consistent with the rest of the schema (projects/settings have none). Databases
-- that already applied that first 0003 keep the old table, so recreate it here
-- without the constraint. The table only ever held plan rows, so this loses
-- nothing. (Note: the FK was never the persistence bug — that was an open read
-- cursor held across the INSERT in create_plan, fixed in the daemon.)
DROP TABLE IF EXISTS plans;

CREATE TABLE plans (
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
