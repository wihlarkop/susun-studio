-- Runtime Data 1: profile ownership and persistence.
--
-- Adds stable identity/ownership/source, availability (missing) tracking, an
-- observation revision separate from user-metadata timestamps, an opaque owner
-- token, and last-error history to runtime_profiles. Discovery may rewrite the
-- observed columns freely, but the daemon never touches the ownership/selection
-- columns on a recheck (enforced in Rust), so built-in and external profiles
-- stay truthful through restarts and provider disappearance.
--
-- turso 0.7.0-pre.14 accepts column + table CHECK constraints, partial unique
-- indexes, and ALTER TABLE ... RENAME TO (verified before writing this), so the
-- class/ownership/source vocabularies and the "at most one selected profile"
-- rule are validated at the database level via a table rebuild.

-- 1. Rebuild runtime_profiles with the new columns and validation.
CREATE TABLE runtime_profiles_v2 (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    provider_runtime_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    product TEXT NOT NULL,
    platform TEXT NOT NULL,

    -- Stable identity + ownership (user intent; discovery must never rewrite).
    runtime_class TEXT NOT NULL DEFAULT 'external_local'
        CHECK (runtime_class IN ('built_in', 'external_local', 'external_remote')),
    ownership_state TEXT NOT NULL DEFAULT 'external'
        CHECK (ownership_state IN ('studio_managed', 'external', 'ownership_conflict', 'ownership_unknown')),
    source TEXT NOT NULL DEFAULT 'provider_discovery'
        CHECK (source IN ('studio_setup', 'provider_discovery', 'user_remote', 'restored_metadata')),
    -- Opaque proof that Studio created/adopted this runtime. Never stores raw
    -- endpoints or credentials — only a random Studio-minted token.
    owner_token TEXT,

    -- Observed health (discovery rewrites these on every recheck).
    installation_state TEXT NOT NULL,
    installation_detail TEXT,
    process_state TEXT NOT NULL,
    process_detail TEXT,
    connection_state TEXT NOT NULL,
    connection_detail TEXT,
    endpoint_summary TEXT,

    -- Presence/availability, kept separate from process/connection health so a
    -- provider that is unavailable/failed does not read as "profile removed".
    availability_state TEXT NOT NULL DEFAULT 'available'
        CHECK (availability_state IN ('available', 'missing', 'unknown')),
    last_seen_at_ms INTEGER,
    missing_since_ms INTEGER,

    -- Last-error history (most recent only; append-only detail lives in the
    -- ownership event log below).
    last_error_code TEXT,
    last_error_detail TEXT,
    last_error_at_ms INTEGER,

    -- Global selection (user intent).
    is_selected INTEGER NOT NULL DEFAULT 0,

    -- Observation timeline (observation_revision + observed_at_ms) is tracked
    -- separately from the user-metadata timeline (updated_at_ms) so a recheck
    -- bumps the former without disturbing the latter.
    observation_revision INTEGER NOT NULL DEFAULT 0,
    observed_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,

    UNIQUE(provider_id, provider_runtime_key),

    -- Class/ownership/source combination validity.
    CHECK (ownership_state <> 'studio_managed' OR runtime_class = 'built_in'),
    CHECK (source <> 'studio_setup' OR runtime_class = 'built_in'),
    CHECK (source <> 'user_remote' OR runtime_class = 'external_remote'),
    CHECK (owner_token IS NULL OR runtime_class = 'built_in'),
    CHECK (availability_state <> 'missing' OR missing_since_ms IS NOT NULL)
);

-- Carry existing rows forward. Every 0010-era profile was produced by provider
-- discovery of a local engine, so it maps to external_local / external /
-- provider_discovery with no owner token. Selection and observation timestamps
-- are preserved exactly.
INSERT INTO runtime_profiles_v2 (
    id, provider_id, provider_runtime_key, display_name, product, platform,
    runtime_class, ownership_state, source, owner_token,
    installation_state, installation_detail, process_state, process_detail,
    connection_state, connection_detail, endpoint_summary,
    availability_state, last_seen_at_ms, missing_since_ms,
    last_error_code, last_error_detail, last_error_at_ms,
    is_selected, observation_revision, observed_at_ms, created_at_ms, updated_at_ms
)
SELECT
    id, provider_id, provider_runtime_key, display_name, product, platform,
    'external_local', 'external', 'provider_discovery', NULL,
    installation_state, installation_detail, process_state, process_detail,
    connection_state, connection_detail, endpoint_summary,
    'available', observed_at_ms, NULL,
    NULL, NULL, NULL,
    is_selected, 0, observed_at_ms, created_at_ms, updated_at_ms
FROM runtime_profiles;

DROP TABLE runtime_profiles;
ALTER TABLE runtime_profiles_v2 RENAME TO runtime_profiles;

-- Repair any latent "multiple globally selected profiles" state before the
-- partial unique index goes on, keeping the most recently updated selection.
UPDATE runtime_profiles SET is_selected = 0
WHERE is_selected = 1
  AND id <> (
      SELECT id FROM runtime_profiles WHERE is_selected = 1
      ORDER BY updated_at_ms DESC, id ASC LIMIT 1
  );

-- Enforce at most one globally selected profile from here on.
CREATE UNIQUE INDEX runtime_profiles_one_selected
    ON runtime_profiles(is_selected) WHERE is_selected = 1;

-- 2. Record which runtime the job ran against, so reports keep attribution.
--    Nullable so every historical job row is preserved untouched.
ALTER TABLE jobs ADD COLUMN runtime_profile_id TEXT;
ALTER TABLE jobs ADD COLUMN runtime_class TEXT;

-- 3. Append-only ownership event log. Included because destructive built-in
--    actions must be able to prove the target is Studio-managed and recovery
--    needs the history of how ownership was assigned; the single-row columns
--    above cannot show transitions (import -> conflict -> adopt/forget).
CREATE TABLE IF NOT EXISTS runtime_ownership_events (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    provider_runtime_key TEXT NOT NULL,
    event_kind TEXT NOT NULL,
    from_ownership_state TEXT,
    to_ownership_state TEXT,
    owner_token TEXT,
    detail TEXT,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS runtime_ownership_events_profile_idx
    ON runtime_ownership_events (profile_id, created_at_ms DESC);
