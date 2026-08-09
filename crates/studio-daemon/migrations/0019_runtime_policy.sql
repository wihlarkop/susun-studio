-- Phase 16a: one persisted global runtime preference, separate from observed
-- runtime profiles. The preferred id intentionally has no foreign key: a
-- forgotten/missing profile remains visible and recoverable as user intent.
CREATE TABLE runtime_policy (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    preferred_profile_id TEXT,
    updated_at_ms INTEGER NOT NULL
);

-- Preserve the legacy selection as the initial preference. A malformed older
-- database may have more than one selected profile, so retain the documented
-- deterministic winner.
INSERT INTO runtime_policy (singleton, preferred_profile_id, updated_at_ms)
SELECT
    1,
    (
        SELECT id FROM runtime_profiles
        WHERE is_selected = 1
        ORDER BY updated_at_ms DESC, id ASC
        LIMIT 1
    ),
    COALESCE(
        (
            SELECT updated_at_ms FROM runtime_profiles
            WHERE is_selected = 1
            ORDER BY updated_at_ms DESC, id ASC
            LIMIT 1
        ),
        CAST(unixepoch('subsec') * 1000 AS INTEGER)
    );

-- Rebuild the profile table without global selection. Keep the identity,
-- ownership, observation, and validation contract from migration 11 exactly.
CREATE TABLE runtime_profiles_v3 (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    provider_runtime_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    product TEXT NOT NULL,
    platform TEXT NOT NULL,

    runtime_class TEXT NOT NULL DEFAULT 'external_local'
        CHECK (runtime_class IN ('built_in', 'external_local', 'external_remote')),
    ownership_state TEXT NOT NULL DEFAULT 'external'
        CHECK (ownership_state IN ('studio_managed', 'external', 'ownership_conflict', 'ownership_unknown')),
    source TEXT NOT NULL DEFAULT 'provider_discovery'
        CHECK (source IN ('studio_setup', 'provider_discovery', 'user_remote', 'restored_metadata')),
    owner_token TEXT,

    installation_state TEXT NOT NULL,
    installation_detail TEXT,
    process_state TEXT NOT NULL,
    process_detail TEXT,
    connection_state TEXT NOT NULL,
    connection_detail TEXT,
    endpoint_summary TEXT,

    availability_state TEXT NOT NULL DEFAULT 'available'
        CHECK (availability_state IN ('available', 'missing', 'unknown')),
    last_seen_at_ms INTEGER,
    missing_since_ms INTEGER,

    last_error_code TEXT,
    last_error_detail TEXT,
    last_error_at_ms INTEGER,

    observation_revision INTEGER NOT NULL DEFAULT 0,
    observed_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,

    UNIQUE(provider_id, provider_runtime_key),

    CHECK (ownership_state <> 'studio_managed' OR runtime_class = 'built_in'),
    CHECK (source <> 'studio_setup' OR runtime_class = 'built_in'),
    CHECK (source <> 'user_remote' OR runtime_class = 'external_remote'),
    CHECK (owner_token IS NULL OR runtime_class = 'built_in'),
    CHECK (availability_state <> 'missing' OR missing_since_ms IS NOT NULL)
);

INSERT INTO runtime_profiles_v3 (
    id, provider_id, provider_runtime_key, display_name, product, platform,
    runtime_class, ownership_state, source, owner_token,
    installation_state, installation_detail, process_state, process_detail,
    connection_state, connection_detail, endpoint_summary,
    availability_state, last_seen_at_ms, missing_since_ms,
    last_error_code, last_error_detail, last_error_at_ms,
    observation_revision, observed_at_ms, created_at_ms, updated_at_ms
)
SELECT
    id, provider_id, provider_runtime_key, display_name, product, platform,
    runtime_class, ownership_state, source, owner_token,
    installation_state, installation_detail, process_state, process_detail,
    connection_state, connection_detail, endpoint_summary,
    availability_state, last_seen_at_ms, missing_since_ms,
    last_error_code, last_error_detail, last_error_at_ms,
    observation_revision, observed_at_ms, created_at_ms, updated_at_ms
FROM runtime_profiles;

DROP TABLE runtime_profiles;
ALTER TABLE runtime_profiles_v3 RENAME TO runtime_profiles;

-- Historical jobs intentionally remain unattributed. New values are bounded
-- to the three resolution sources so persisted reports cannot invent a state.
ALTER TABLE jobs ADD COLUMN runtime_binding_source TEXT
    CHECK (runtime_binding_source IN (
        'project_pin',
        'global_preference',
        'platform_default'
    ));
