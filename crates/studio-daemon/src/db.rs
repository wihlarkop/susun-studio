use std::path::PathBuf;

use turso::{Connection, Database, params};

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial",
    sql: include_str!("../migrations/0001_initial.sql"),
}];

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
