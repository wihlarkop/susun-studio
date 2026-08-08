use std::path::PathBuf;

use turso::{Connection, Database, params};

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "project_sources",
        sql: include_str!("../migrations/0002_project_sources.sql"),
    },
    Migration {
        version: 3,
        name: "plans",
        sql: include_str!("../migrations/0003_plans.sql"),
    },
    Migration {
        version: 4,
        name: "plans_drop_fk",
        sql: include_str!("../migrations/0004_plans_drop_fk.sql"),
    },
    Migration {
        version: 5,
        name: "engine_providers",
        sql: include_str!("../migrations/0005_engine_providers.sql"),
    },
    Migration {
        version: 6,
        name: "jobs",
        sql: include_str!("../migrations/0006_jobs.sql"),
    },
    Migration {
        version: 7,
        name: "job_error_codes",
        sql: include_str!("../migrations/0007_job_error_codes.sql"),
    },
    Migration {
        version: 8,
        name: "job_manifest",
        sql: include_str!("../migrations/0008_job_manifest.sql"),
    },
    Migration {
        version: 9,
        name: "watch",
        sql: include_str!("../migrations/0009_watch.sql"),
    },
    Migration {
        version: 10,
        name: "runtime_profiles",
        sql: include_str!("../migrations/0010_runtime_profiles.sql"),
    },
    Migration {
        version: 11,
        name: "runtime_ownership",
        sql: include_str!("../migrations/0011_runtime_ownership.sql"),
    },
    Migration {
        version: 12,
        name: "runtime_transitions",
        sql: include_str!("../migrations/0012_runtime_transitions.sql"),
    },
    Migration {
        version: 13,
        name: "runtime_action_audit",
        sql: include_str!("../migrations/0013_runtime_action_audit.sql"),
    },
    Migration {
        version: 14,
        name: "runtime_action_audit_artifact_actions",
        sql: include_str!("../migrations/0014_runtime_action_audit_artifact_actions.sql"),
    },
    Migration {
        version: 15,
        name: "build_job_progress",
        sql: include_str!("../migrations/0015_build_job_progress.sql"),
    },
    Migration {
        version: 16,
        name: "registry_credentials",
        sql: include_str!("../migrations/0016_registry_credentials.sql"),
    },
    Migration {
        version: 17,
        name: "artifact_transfer_progress",
        sql: include_str!("../migrations/0017_artifact_transfer_progress.sql"),
    },
    Migration {
        version: 18,
        name: "runtime_action_audit_image_push",
        sql: include_str!("../migrations/0018_runtime_action_audit_image_push.sql"),
    },
    Migration {
        version: 19,
        name: "runtime_policy",
        sql: include_str!("../migrations/0019_runtime_policy.sql"),
    },
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("failed to create database directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to open database {path}: {source}")]
    Open { path: PathBuf, source: turso::Error },

    #[error("failed to connect database: {0}")]
    Connect(turso::Error),

    #[error("failed to run database migration {version} ({name}): {source}")]
    Migration {
        version: i64,
        name: &'static str,
        source: turso::Error,
    },
}

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

/// The highest migration version this build knows how to apply. A backup
/// records this so restore can tell whether an archive is older (migrate it
/// forward) or from a newer, incompatible app (refuse).
pub fn latest_migration_version() -> i64 {
    MIGRATIONS
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0)
}

pub async fn open_database(path: PathBuf) -> Result<Database, DbError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DbError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let db = turso::Builder::new_local(&path.to_string_lossy())
        .build()
        .await
        .map_err(|source| DbError::Open {
            path: path.clone(),
            source,
        })?;
    let conn = db.connect().map_err(DbError::Connect)?;
    apply_migrations(&conn).await?;

    Ok(db)
}

async fn apply_migrations(conn: &Connection) -> Result<(), DbError> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _studio_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000)
        )",
        (),
    )
    .await
    .map_err(|source| DbError::Migration {
        version: 0,
        name: "migration_table",
        source,
    })?;

    for migration in MIGRATIONS {
        let applied = migration_applied(conn, migration).await?;
        if applied {
            continue;
        }

        conn.execute_batch(migration.sql)
            .await
            .map_err(|source| DbError::Migration {
                version: migration.version,
                name: migration.name,
                source,
            })?;
        conn.execute(
            "INSERT INTO _studio_migrations (version, name) VALUES (?1, ?2)",
            params![migration.version, migration.name],
        )
        .await
        .map_err(|source| DbError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;
    }

    Ok(())
}

/// Run any pending migrations on an already-open connection. Used to migrate a
/// staged restore database forward to the current schema before it is swapped in.
pub async fn run_migrations(conn: &Connection) -> Result<(), DbError> {
    apply_migrations(conn).await
}

/// Apply only the migrations up to and including `max_version`, leaving later
/// ones pending. Used by tests to reconstruct a pre-upgrade database and then
/// exercise the newer migration against it.
#[cfg(test)]
pub async fn apply_migrations_upto(conn: &Connection, max_version: i64) -> Result<(), DbError> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _studio_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_ms INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000)
        )",
        (),
    )
    .await
    .map_err(|source| DbError::Migration {
        version: 0,
        name: "migration_table",
        source,
    })?;

    for migration in MIGRATIONS.iter().filter(|m| m.version <= max_version) {
        if migration_applied(conn, migration).await? {
            continue;
        }
        conn.execute_batch(migration.sql)
            .await
            .map_err(|source| DbError::Migration {
                version: migration.version,
                name: migration.name,
                source,
            })?;
        conn.execute(
            "INSERT INTO _studio_migrations (version, name) VALUES (?1, ?2)",
            params![migration.version, migration.name],
        )
        .await
        .map_err(|source| DbError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;
    }
    Ok(())
}

/// Run any pending migrations on an already-open connection (tests only).
#[cfg(test)]
pub async fn apply_pending_migrations(conn: &Connection) -> Result<(), DbError> {
    apply_migrations(conn).await
}

async fn migration_applied(conn: &Connection, migration: &Migration) -> Result<bool, DbError> {
    let mut rows = conn
        .query(
            "SELECT version FROM _studio_migrations WHERE version = ?1 LIMIT 1",
            params![migration.version],
        )
        .await
        .map_err(|source| DbError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;

    rows.next()
        .await
        .map(|row| row.is_some())
        .map_err(|source| DbError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn unique_db_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "studio-db-test-{}.db",
            uuid::Uuid::new_v4().simple()
        ))
    }

    async fn version_eighteen_database() -> TestResult<(Database, Connection, PathBuf)> {
        let path = unique_db_path();
        let db = turso::Builder::new_local(path.to_string_lossy().as_ref())
            .build()
            .await?;
        let conn = db.connect()?;
        apply_migrations_upto(&conn, 18).await?;
        Ok((db, conn, path))
    }

    async fn strings(conn: &Connection, sql: &str) -> TestResult<Vec<String>> {
        let mut rows = conn.query(sql, ()).await?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().await? {
            values.push(row.get::<String>(0)?);
        }
        Ok(values)
    }

    async fn profile_snapshots(conn: &Connection) -> TestResult<Vec<String>> {
        let mut rows = conn
            .query(
                "SELECT id, provider_id, provider_runtime_key, display_name, product, platform,
                    runtime_class, ownership_state, source, owner_token,
                    installation_state, installation_detail, process_state, process_detail,
                    connection_state, connection_detail, endpoint_summary,
                    availability_state, last_seen_at_ms, missing_since_ms,
                    last_error_code, last_error_detail, last_error_at_ms,
                    observation_revision, observed_at_ms, created_at_ms, updated_at_ms
                 FROM runtime_profiles ORDER BY id",
                (),
            )
            .await?;
        let mut snapshots = Vec::new();
        while let Some(row) = rows.next().await? {
            let values = vec![
                row.get::<String>(0)?,
                row.get::<String>(1)?,
                row.get::<String>(2)?,
                row.get::<String>(3)?,
                row.get::<String>(4)?,
                row.get::<String>(5)?,
                row.get::<String>(6)?,
                row.get::<String>(7)?,
                row.get::<String>(8)?,
                row.get::<Option<String>>(9)?.unwrap_or_default(),
                row.get::<String>(10)?,
                row.get::<Option<String>>(11)?.unwrap_or_default(),
                row.get::<String>(12)?,
                row.get::<Option<String>>(13)?.unwrap_or_default(),
                row.get::<String>(14)?,
                row.get::<Option<String>>(15)?.unwrap_or_default(),
                row.get::<Option<String>>(16)?.unwrap_or_default(),
                row.get::<String>(17)?,
                row.get::<Option<i64>>(18)?
                    .map_or_else(String::new, |value| value.to_string()),
                row.get::<Option<i64>>(19)?
                    .map_or_else(String::new, |value| value.to_string()),
                row.get::<Option<String>>(20)?.unwrap_or_default(),
                row.get::<Option<String>>(21)?.unwrap_or_default(),
                row.get::<Option<i64>>(22)?
                    .map_or_else(String::new, |value| value.to_string()),
                row.get::<i64>(23)?.to_string(),
                row.get::<i64>(24)?.to_string(),
                row.get::<i64>(25)?.to_string(),
                row.get::<i64>(26)?.to_string(),
            ];
            snapshots.push(values.join("|"));
        }
        Ok(snapshots)
    }

    #[tokio::test]
    async fn runtime_policy_migration_copies_latest_selection_and_preserves_profiles() -> TestResult
    {
        let (_db, conn, path) = version_eighteen_database().await?;

        // Reproduce a legacy multi-selection state so the migration's ordering
        // rule is exercised even though version 18 normally enforces one row.
        conn.execute("DROP INDEX runtime_profiles_one_selected", ())
            .await?;
        for (id, class, ownership, source, owner_token, selected, updated_at_ms) in [
            (
                "external",
                "external_local",
                "external",
                "provider_discovery",
                None,
                1,
                200,
            ),
            (
                "built-in",
                "built_in",
                "studio_managed",
                "studio_setup",
                Some("owner-proof"),
                1,
                300,
            ),
        ] {
            conn.execute(
                "INSERT INTO runtime_profiles (
                    id, provider_id, provider_runtime_key, display_name, product, platform,
                    runtime_class, ownership_state, source, owner_token,
                    installation_state, installation_detail, process_state, process_detail,
                    connection_state, connection_detail, endpoint_summary,
                    availability_state, last_seen_at_ms, missing_since_ms,
                    last_error_code, last_error_detail, last_error_at_ms,
                    is_selected, observation_revision, observed_at_ms, created_at_ms, updated_at_ms
                ) VALUES (
                    ?1, 'provider', ?1, ?1, 'engine', 'windows',
                    ?2, ?3, ?4, ?5,
                    'installed', 'installation-detail', 'running', 'process-detail',
                    'summarized', 'connection-detail', 'redacted-endpoint',
                    'available', 10, NULL,
                    'runtime-error', 'error-detail', 11,
                    ?6, 12, 13, 14, ?7
                )",
                params![
                    id,
                    class,
                    ownership,
                    source,
                    owner_token,
                    selected,
                    updated_at_ms
                ],
            )
            .await?;
        }
        conn.execute(
            "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json,
                created_at_ms, updated_at_ms)
             VALUES ('historical', 'up', 'complete', 'project', 'engine', '{}', 1, 1)",
            (),
        )
        .await?;

        apply_pending_migrations(&conn).await?;

        assert_eq!(
            strings(
                &conn,
                "SELECT preferred_profile_id FROM runtime_policy WHERE singleton = 1",
            )
            .await?,
            vec!["built-in".to_owned()]
        );
        assert_eq!(
            strings(&conn, "SELECT CAST(COUNT(*) AS TEXT) FROM runtime_policy").await?,
            vec!["1".to_owned()]
        );
        assert!(
            conn.execute(
                "INSERT INTO runtime_policy (singleton, preferred_profile_id, updated_at_ms)
                 VALUES (2, 'other', 1)",
                (),
            )
            .await
            .is_err()
        );

        assert_eq!(
            profile_snapshots(&conn).await?,
            vec![
                "built-in|provider|built-in|built-in|engine|windows|built_in|studio_managed|studio_setup|owner-proof|installed|installation-detail|running|process-detail|summarized|connection-detail|redacted-endpoint|available|10||runtime-error|error-detail|11|12|13|14|300".to_owned(),
                "external|provider|external|external|engine|windows|external_local|external|provider_discovery||installed|installation-detail|running|process-detail|summarized|connection-detail|redacted-endpoint|available|10||runtime-error|error-detail|11|12|13|14|200".to_owned(),
            ]
        );
        assert_eq!(
            strings(
                &conn,
                "SELECT CAST(COUNT(*) AS TEXT) FROM pragma_table_info('runtime_profiles')
                 WHERE name = 'is_selected'",
            )
            .await?,
            vec!["0".to_owned()]
        );
        assert_eq!(
            strings(
                &conn,
                "SELECT CAST(COUNT(*) AS TEXT) FROM sqlite_master
                 WHERE type = 'index' AND name = 'runtime_profiles_one_selected'",
            )
            .await?,
            vec!["0".to_owned()]
        );

        conn.execute("DELETE FROM runtime_profiles WHERE id = 'built-in'", ())
            .await?;
        assert_eq!(
            strings(
                &conn,
                "SELECT preferred_profile_id FROM runtime_policy WHERE singleton = 1",
            )
            .await?,
            vec!["built-in".to_owned()]
        );

        assert_eq!(
            strings(
                &conn,
                "SELECT CAST(COUNT(*) AS TEXT) FROM jobs
                 WHERE id = 'historical' AND runtime_binding_source IS NULL",
            )
            .await?,
            vec!["1".to_owned()]
        );
        for source in ["project_pin", "global_preference", "platform_default"] {
            conn.execute(
                "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json,
                    created_at_ms, updated_at_ms, runtime_binding_source)
                 VALUES (?1, 'up', 'complete', 'project', 'engine', '{}', 1, 1, ?2)",
                params![format!("job-{source}"), source],
            )
            .await?;
        }
        assert!(
            conn.execute(
                "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json,
                    created_at_ms, updated_at_ms, runtime_binding_source)
                 VALUES ('invalid-source', 'up', 'complete', 'project', 'engine', '{}', 1, 1,
                    'fallback')",
                (),
            )
            .await
            .is_err()
        );

        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_policy_migration_breaks_equal_timestamp_selection_ties_by_id() -> TestResult {
        let (_db, conn, path) = version_eighteen_database().await?;

        conn.execute("DROP INDEX runtime_profiles_one_selected", ())
            .await?;
        for id in ["zeta", "alpha"] {
            conn.execute(
                "INSERT INTO runtime_profiles (
                    id, provider_id, provider_runtime_key, display_name, product, platform,
                    installation_state, process_state, connection_state, is_selected,
                    observed_at_ms, created_at_ms, updated_at_ms
                ) VALUES (
                    ?1, 'provider', ?1, ?1, 'engine', 'windows',
                    'installed', 'running', 'summarized', 1, 1, 1, 500
                )",
                params![id],
            )
            .await?;
        }

        apply_pending_migrations(&conn).await?;

        assert_eq!(
            strings(
                &conn,
                "SELECT preferred_profile_id FROM runtime_policy WHERE singleton = 1",
            )
            .await?,
            vec!["alpha".to_owned()]
        );

        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_policy_migration_preserves_null_without_a_selection() -> TestResult {
        let (_db, conn, path) = version_eighteen_database().await?;
        conn.execute(
            "INSERT INTO runtime_profiles (
                id, provider_id, provider_runtime_key, display_name, product, platform,
                installation_state, process_state, connection_state, is_selected,
                observed_at_ms, created_at_ms, updated_at_ms
            ) VALUES (
                'unselected', 'provider', 'runtime', 'Unselected', 'engine', 'windows',
                'installed', 'running', 'summarized', 0, 1, 1, 1
            )",
            (),
        )
        .await?;

        apply_pending_migrations(&conn).await?;

        let mut rows = conn
            .query(
                "SELECT preferred_profile_id FROM runtime_policy WHERE singleton = 1",
                (),
            )
            .await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| std::io::Error::other("singleton runtime policy row"))?;
        assert!(row.get::<Option<String>>(0)?.is_none());

        let _ = std::fs::remove_file(path);
        Ok(())
    }
}
