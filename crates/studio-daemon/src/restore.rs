//! Restore-apply preparation and daemon lifecycle coordination.
//!
//! Restore is a process-boundary operation: the daemon validates an archive,
//! stages a migrated copy of the restored database beside the active one,
//! writes an automatic pre-restore backup, and hands the Tauri supervisor an
//! opaque `restore_id` plus the on-disk paths. The supervisor then stops this
//! daemon, atomically swaps the files, and restarts. This module never lets a
//! prepared restore keep a live database handle open on the staged file.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::Notify;
use turso::Database;

use crate::backup::{self, RestoreManifestSummary};

/// How long a prepared restore stays valid before its staged artifacts are
/// considered abandoned.
const PREPARED_TTL_SECONDS: u64 = 15 * 60;

const STAGED_PREFIX: &str = ".studio-restore-staged-";
const PRE_RESTORE_PREFIX: &str = ".studio-pre-restore-";
const ROLLBACK_INFIX: &str = ".rollback-";

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error(transparent)]
    Archive(#[from] backup::RestoreError),
    #[error("this backup is not compatible with this app: {0}")]
    Incompatible(String),
    #[error("failed to build the backup for staging: {0}")]
    Backup(#[from] backup::BackupError),
    #[error("filesystem error while preparing restore: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error while preparing restore: {0}")]
    Database(#[from] turso::Error),
    #[error("failed to migrate the staged database: {0}")]
    Migration(String),
    #[error("the staged database failed validation: {0}")]
    Validation(String),
}

/// Daemon lifecycle state exposed so mutating requests can be refused once a
/// restore swap is imminent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonAvailability {
    Ready,
    RestorePreparing,
    RestoreShutdownPending,
}

/// The prepared-restore handoff returned to the Tauri supervisor. Real paths go
/// only to the trusted Tauri process, never the webview, which sees just the
/// final outcome. Nothing here keeps a live handle on the staged file.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedRestore {
    pub restore_id: String,
    pub active_database_path: String,
    pub staged_database_path: String,
    pub rollback_database_path: String,
    pub pre_restore_backup_path: String,
    pub manifest: RestoreManifestSummary,
    pub expires_at_epoch_seconds: u64,
}

/// Coordinates daemon availability and the graceful-shutdown trigger the
/// supervisor uses to reach the restore swap boundary.
pub struct RestoreCoordinator {
    availability: Mutex<DaemonAvailability>,
    shutdown: Notify,
}

impl Default for RestoreCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl RestoreCoordinator {
    pub fn new() -> Self {
        Self {
            availability: Mutex::new(DaemonAvailability::Ready),
            shutdown: Notify::new(),
        }
    }

    pub fn availability(&self) -> DaemonAvailability {
        *self
            .availability
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn set_availability(&self, availability: DaemonAvailability) {
        *self
            .availability
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = availability;
    }

    /// Awaited by the server's graceful-shutdown future. Completes when a
    /// restore swap has been requested.
    pub async fn shutdown_requested(&self) {
        self.shutdown.notified().await;
    }

    /// Enter `RestoreShutdownPending` and trigger graceful shutdown so the
    /// supervisor can swap files with no live handle on the active database.
    pub fn begin_restore_shutdown(&self) {
        self.set_availability(DaemonAvailability::RestoreShutdownPending);
        self.shutdown.notify_waiters();
    }
}

/// Prepare a restore: validate, stage a migrated copy, write a pre-restore
/// backup, and register the handoff. Does not mutate the active database.
pub async fn prepare_restore(
    coordinator: &RestoreCoordinator,
    db: &Database,
    active_db_path: &Path,
    archive: &[u8],
) -> Result<PreparedRestore, RestoreError> {
    coordinator.set_availability(DaemonAvailability::RestorePreparing);
    let result = prepare_inner(db, active_db_path, archive).await;
    // Whatever happens, drop back to Ready — the shutdown state is only entered
    // later, deliberately, by the supervisor via `begin_restore_shutdown`.
    coordinator.set_availability(DaemonAvailability::Ready);
    result
}

async fn prepare_inner(
    db: &Database,
    active_db_path: &Path,
    archive: &[u8],
) -> Result<PreparedRestore, RestoreError> {
    let current_schema = crate::db::latest_migration_version();
    let (preview, database_bytes) = backup::validated_database(archive, current_schema)?;
    if !preview.compatible {
        return Err(RestoreError::Incompatible(preview.reason.unwrap_or_else(
            || "the backup schema is newer than this app".to_owned(),
        )));
    }

    let active = std::path::absolute(active_db_path)?;
    let directory = active
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let restore_id = format!("rst_{}", uuid::Uuid::new_v4().simple());

    // Stage the restored database on the same filesystem as the active one so
    // the final rename can be atomic.
    let staged_database_path = directory.join(format!("{STAGED_PREFIX}{restore_id}.db"));
    std::fs::write(&staged_database_path, &database_bytes)?;

    // Migrate + validate the staged copy through an independent handle, then
    // close it so no live handle remains on the file the supervisor will swap.
    migrate_and_validate_staged(&staged_database_path).await?;

    // Automatic pre-restore backup of the *current* data, as a safety net.
    let pre_restore_backup_path = directory.join(format!("{PRE_RESTORE_PREFIX}{restore_id}.tar"));
    let pre_restore_bytes = backup::create_backup_archive(db, active_db_path).await?;
    std::fs::write(&pre_restore_backup_path, &pre_restore_bytes)?;

    let rollback_database_path = rollback_path(&active, &restore_id);
    let expires_at_epoch_seconds = now_epoch_seconds() + PREPARED_TTL_SECONDS;

    Ok(PreparedRestore {
        restore_id,
        active_database_path: path_string(&active),
        staged_database_path: path_string(&staged_database_path),
        rollback_database_path: path_string(&rollback_database_path),
        pre_restore_backup_path: path_string(&pre_restore_backup_path),
        manifest: preview.manifest,
        expires_at_epoch_seconds,
    })
}

async fn migrate_and_validate_staged(staged_path: &Path) -> Result<(), RestoreError> {
    let staged = turso::Builder::new_local(staged_path.to_string_lossy().as_ref())
        .build()
        .await?;
    let conn = staged.connect()?;
    crate::db::run_migrations(&conn)
        .await
        .map_err(|error| RestoreError::Migration(error.to_string()))?;

    // App-level validation: the migrated database must report the current
    // schema version and expose the core tables.
    let applied = max_migration_version(&conn).await?;
    let expected = crate::db::latest_migration_version();
    if applied != expected {
        return Err(RestoreError::Validation(format!(
            "staged database is at schema v{applied}, expected v{expected}"
        )));
    }
    for table in ["projects", "runtime_profiles", "jobs", "settings"] {
        ensure_table_readable(&conn, table).await?;
    }

    // Restored ownership is untrusted: a backup could carry studio_managed
    // profiles and owner tokens from another install. Reset ownership to unknown
    // and mark profiles as needing fresh detection, so the daemon re-probes and
    // re-adopts through the normal flow rather than inheriting trust from a file.
    sanitize_restored_ownership(&conn).await?;

    // Drop the handle before returning so nothing keeps the staged file open.
    drop(conn);
    drop(staged);
    Ok(())
}

/// Strip trusted ownership evidence from a restored database so nothing is
/// treated as Studio-managed until live detection confirms it.
async fn sanitize_restored_ownership(conn: &turso::Connection) -> Result<(), RestoreError> {
    conn.execute(
        "UPDATE runtime_profiles
         SET ownership_state = 'ownership_unknown',
             owner_token = NULL,
             source = 'restored_metadata',
             availability_state = 'unknown',
             missing_since_ms = NULL",
        (),
    )
    .await?;
    // The ownership event log is history from a prior install; its tokens carry
    // no authority here, so clear them while keeping the audit trail.
    conn.execute(
        "UPDATE runtime_ownership_events SET owner_token = NULL WHERE owner_token IS NOT NULL",
        (),
    )
    .await?;
    Ok(())
}

async fn max_migration_version(conn: &turso::Connection) -> Result<i64, RestoreError> {
    let mut rows = conn
        .query(
            "SELECT COALESCE(MAX(version), 0) FROM _studio_migrations",
            (),
        )
        .await?;
    Ok(match rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    })
}

async fn ensure_table_readable(conn: &turso::Connection, table: &str) -> Result<(), RestoreError> {
    conn.query(&format!("SELECT 1 FROM {table} LIMIT 1"), ())
        .await
        .map_err(|error| {
            RestoreError::Validation(format!("core table `{table}` is unreadable: {error}"))
        })?;
    Ok(())
}

fn rollback_path(active: &Path, restore_id: &str) -> PathBuf {
    let file_name = active
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "studio.db".to_owned());
    let directory = active
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    directory.join(format!("{file_name}{ROLLBACK_INFIX}{restore_id}"))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Recover from a crash *during* the file swap: if the active database file is
/// missing but a rollback copy of it survives, the swap was interrupted after
/// the active database was renamed aside but before the staged copy took its
/// place. Rename the newest rollback back so no data is lost. Best-effort;
/// called at startup before the database is opened.
pub fn recover_incomplete_swap(db_path: &Path) {
    if db_path.exists() {
        return;
    }
    let Some(directory) = db_path.parent() else {
        return;
    };
    let Some(file_name) = db_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return;
    };
    let rollback_prefix = format!("{file_name}{ROLLBACK_INFIX}");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let newest_rollback = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&rollback_prefix)
        })
        .max_by_key(|entry| {
            entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH)
        });
    if let Some(entry) = newest_rollback {
        let _ = std::fs::rename(entry.path(), db_path);
    }
}

/// Delete abandoned staged databases and pre-restore backups from a previous
/// run so restore artifacts do not accumulate. Rollback copies are intentionally
/// left alone — only the restore orchestration removes them, so a failed restart
/// can still roll back. Best-effort; called at startup.
pub fn sweep_stale_artifacts(db_path: &Path) {
    let Some(directory) = db_path.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(STAGED_PREFIX) || name.starts_with(PRE_RESTORE_PREFIX) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
