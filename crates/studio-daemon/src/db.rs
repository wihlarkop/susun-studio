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
