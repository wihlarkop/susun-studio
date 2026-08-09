-- Phase 16b: local first-run UX state. This is deliberately separate from
-- runtime policy and security/audit state: it records no commands or runtime
-- connection data.
CREATE TABLE runtime_onboarding (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    state TEXT NOT NULL CHECK (state IN ('pending', 'completed', 'dismissed')),
    choice TEXT CHECK (choice IN ('built_in', 'existing')),
    completed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,

    CHECK (
        (state = 'completed' AND choice IS NOT NULL AND completed_at_ms IS NOT NULL)
        OR
        (state IN ('pending', 'dismissed') AND choice IS NULL AND completed_at_ms IS NULL)
    )
);

INSERT INTO runtime_onboarding (
    singleton, state, choice, completed_at_ms, updated_at_ms
) VALUES (
    1, 'pending', NULL, NULL, CAST(unixepoch('subsec') * 1000 AS INTEGER)
);
