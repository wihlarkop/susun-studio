//! Restore-apply orchestration: the process-boundary swap the daemon cannot do
//! to itself. The daemon prepares a staged, migrated database; here we stop it,
//! atomically swap the file, and restart it — rolling back if the restored
//! database fails to come up. See `crates/studio-daemon/src/restore.rs`.

use std::path::Path;
use std::time::Duration;

use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::daemon::{self, DaemonConnection, DaemonSupervisor};

const SHUTDOWN_WAIT: Duration = Duration::from_secs(15);
const ROLLBACK_WAIT: Duration = Duration::from_secs(10);
const HANDLE_SETTLE: Duration = Duration::from_millis(300);
const RENAME_RETRIES: usize = 15;
const RENAME_RETRY_DELAY: Duration = Duration::from_millis(200);

#[derive(Debug, thiserror::Error)]
pub enum RestoreCommandError {
    #[error("daemon connection is not available yet")]
    NoConnection,
    #[error("restore requires the packaged desktop app (the daemon must be managed by Studio)")]
    Unsupported,
    #[error("failed to talk to the daemon: {0}")]
    Request(#[from] reqwest::Error),
    #[error("failed to read the backup archive: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Daemon(String),
    #[error("the daemon could not be restarted after the restore: {0}")]
    DaemonDown(String),
}

#[derive(Debug, Deserialize)]
struct PreparedRestore {
    restore_id: String,
    active_database_path: String,
    staged_database_path: String,
    rollback_database_path: String,
    // The pre-restore safety backup is kept on disk; the daemon's startup sweep
    // clears it. Deserialized so the field is documented, not used directly.
    #[allow(dead_code)]
    pre_restore_backup_path: String,
    manifest: RestoreSummary,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RestoreSummary {
    pub app_version: String,
    pub schema_migration_version: i64,
    pub current_schema_migration_version: i64,
    pub project_count: i64,
    pub runtime_profile_count: i64,
    pub job_count: i64,
}

#[derive(Debug, Serialize)]
pub struct DaemonConnectionPayload {
    pub base_url: String,
    pub token: String,
}

impl From<DaemonConnection> for DaemonConnectionPayload {
    fn from(connection: DaemonConnection) -> Self {
        Self {
            base_url: connection.base_url,
            token: connection.token,
        }
    }
}

/// The result the webview receives. It carries the *new* daemon connection so
/// the frontend can re-point its client after the restart (the respawned daemon
/// binds a fresh port and token).
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum RestoreOutcome {
    Restored {
        summary: RestoreSummary,
        connection: DaemonConnectionPayload,
    },
    RolledBack {
        reason: String,
        connection: DaemonConnectionPayload,
    },
}

pub async fn apply_restore(
    app: &AppHandle,
    archive_path: &str,
) -> Result<RestoreOutcome, RestoreCommandError> {
    let supervisor = app.state::<DaemonSupervisor>();
    if !supervisor.manages_child() {
        return Err(RestoreCommandError::Unsupported);
    }
    let connection = supervisor
        .connection()
        .ok_or(RestoreCommandError::NoConnection)?;

    // Reuse the archive the user picked at preview time — no second dialog.
    let archive = std::fs::read(archive_path)?;

    // 1. Daemon validates + stages a migrated copy + writes a pre-restore backup.
    let prepared = request_prepare(&connection, archive).await?;
    info!("event=restore_prepared restore_id={}", prepared.restore_id);

    // 2. Ask the daemon to release the database and exit gracefully.
    request_shutdown(&connection).await?;
    if !daemon::wait_until_unreachable(&connection.base_url, SHUTDOWN_WAIT).await {
        warn!("event=restore_daemon_graceful_timeout action=hard_kill");
        supervisor.shutdown();
        daemon::wait_until_unreachable(&connection.base_url, ROLLBACK_WAIT).await;
    }
    // Give the OS a moment to release the database file handle.
    tokio::time::sleep(HANDLE_SETTLE).await;

    // 3. Atomically swap the staged database into place.
    if let Err(error) = swap_in_restored(&prepared).await {
        warn!("event=restore_swap_failed error={error}");
        restore_previous(&prepared).await;
        let connection = respawn_or_down(app).await?;
        return Ok(RestoreOutcome::RolledBack {
            reason: format!("could not swap the restored database: {error}"),
            connection: connection.into(),
        });
    }

    // 4. Bring the daemon back on the restored database.
    match daemon::respawn(app).await {
        Ok(connection) => {
            info!("event=restore_finished restore_id={}", prepared.restore_id);
            // The restore succeeded; the rollback copy is no longer needed.
            let _ = std::fs::remove_file(&prepared.rollback_database_path);
            Ok(RestoreOutcome::Restored {
                summary: prepared.manifest,
                connection: connection.into(),
            })
        }
        Err(error) => {
            warn!("event=restore_restart_failed error={error} action=rollback");
            supervisor.shutdown();
            daemon::wait_until_unreachable(&connection.base_url, ROLLBACK_WAIT).await;
            tokio::time::sleep(HANDLE_SETTLE).await;
            restore_previous(&prepared).await;
            let connection = respawn_or_down(app).await?;
            Ok(RestoreOutcome::RolledBack {
                reason: format!("the restored database did not start: {error}"),
                connection: connection.into(),
            })
        }
    }
}

async fn respawn_or_down(app: &AppHandle) -> Result<DaemonConnection, RestoreCommandError> {
    daemon::respawn(app)
        .await
        .map_err(|error| RestoreCommandError::DaemonDown(error.to_string()))
}

/// active -> rollback, then staged -> active. If the second rename fails the
/// caller rolls back by renaming the rollback copy back to active.
async fn swap_in_restored(prepared: &PreparedRestore) -> Result<(), std::io::Error> {
    rename_with_retry(
        Path::new(&prepared.active_database_path),
        Path::new(&prepared.rollback_database_path),
    )
    .await?;
    rename_with_retry(
        Path::new(&prepared.staged_database_path),
        Path::new(&prepared.active_database_path),
    )
    .await
}

/// Put the previous database back: discard whatever is at the active path and
/// rename the rollback copy into place. Best-effort.
async fn restore_previous(prepared: &PreparedRestore) {
    let active = Path::new(&prepared.active_database_path);
    let rollback = Path::new(&prepared.rollback_database_path);
    if !rollback.exists() {
        return;
    }
    let _ = std::fs::remove_file(active);
    let _ = rename_with_retry(rollback, active).await;
}

/// Rename with retries — on Windows the previous daemon process may briefly hold
/// the database file open after it stops responding.
async fn rename_with_retry(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    let mut last_error = None;
    for attempt in 0..RENAME_RETRIES {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < RENAME_RETRIES {
                    tokio::time::sleep(RENAME_RETRY_DELAY).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("rename failed")))
}

async fn request_prepare(
    connection: &DaemonConnection,
    archive: Vec<u8>,
) -> Result<PreparedRestore, RestoreCommandError> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/restore/prepare", connection.base_url))
        .bearer_auth(&connection.token)
        .header("content-type", "application/octet-stream")
        .body(archive)
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(RestoreCommandError::Daemon(daemon_error_message(
            &body, status,
        )));
    }
    serde_json::from_str(&body)
        .map_err(|error| RestoreCommandError::Daemon(format!("invalid prepare response: {error}")))
}

async fn request_shutdown(connection: &DaemonConnection) -> Result<(), RestoreCommandError> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/restore/shutdown", connection.base_url))
        .bearer_auth(&connection.token)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(RestoreCommandError::Daemon(daemon_error_message(
            &body, status,
        )));
    }
    Ok(())
}

fn daemon_error_message(body: &str, status: reqwest::StatusCode) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|e| e.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("daemon returned {status}"))
}
