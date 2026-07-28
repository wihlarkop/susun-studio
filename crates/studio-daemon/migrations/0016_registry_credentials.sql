-- Secret values live only in the platform credential store. This table keeps
-- the opaque lookup key and display-safe lifecycle metadata.
CREATE TABLE IF NOT EXISTS registry_credentials (
    id TEXT PRIMARY KEY,
    registry_identity TEXT NOT NULL UNIQUE,
    username_label TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    last_success_at_ms INTEGER
);

CREATE INDEX IF NOT EXISTS registry_credentials_registry_identity_idx
    ON registry_credentials (registry_identity);
