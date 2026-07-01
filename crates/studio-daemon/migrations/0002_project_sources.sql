ALTER TABLE projects ADD COLUMN compose_files TEXT;
ALTER TABLE projects ADD COLUMN env_file TEXT;
ALTER TABLE projects ADD COLUMN project_name_override TEXT;
ALTER TABLE projects ADD COLUMN profiles TEXT;
ALTER TABLE projects ADD COLUMN last_analyzed_at_ms INTEGER;
ALTER TABLE projects ADD COLUMN summary_json TEXT;
ALTER TABLE projects ADD COLUMN diagnostics_json TEXT;
ALTER TABLE projects ADD COLUMN has_errors INTEGER;
