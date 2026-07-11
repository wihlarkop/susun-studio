use log::info;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::daemon::{DaemonConnection, DaemonSupervisor};
use tauri::Manager;

#[derive(Debug, thiserror::Error)]
pub enum BackupCommandError {
    #[error("daemon connection is not available yet")]
    NoConnection,
    #[error("failed to talk to the daemon: {0}")]
    Request(#[from] reqwest::Error),
    #[error("failed to write backup file: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Daemon(String),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupOutcome {
    Saved,
    Cancelled,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum RestorePreviewOutcome {
    Cancelled,
    /// A validated, non-mutating preview passed straight through from the daemon.
    Previewed {
        preview: serde_json::Value,
    },
}

/// Fetches a freshly-built backup archive from the daemon and writes it to a
/// user-chosen path atomically (temp file in the same directory, then rename).
/// A cancelled save dialog is a normal, non-error outcome.
pub async fn backup_studio_data(app: &AppHandle) -> Result<BackupOutcome, BackupCommandError> {
    info!("event=backup_started");
    let connection = connection(app)?;
    let archive = fetch_backup(&connection).await?;

    let Some(target) = app
        .dialog()
        .file()
        .set_file_name("susun-studio-backup.tar")
        .blocking_save_file()
    else {
        info!("event=backup_cancelled");
        return Ok(BackupOutcome::Cancelled);
    };
    let Ok(target_path) = target.into_path() else {
        info!("event=backup_cancelled");
        return Ok(BackupOutcome::Cancelled);
    };

    write_atomically(&target_path, &archive)?;
    info!(
        "event=backup_finished bytes={} file={}",
        archive.len(),
        target_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<unknown>".to_owned())
    );
    Ok(BackupOutcome::Saved)
}

/// Lets the user pick a backup archive and asks the daemon to validate it,
/// returning a safe preview. Nothing is restored here — this is read-only.
pub async fn preview_restore(app: &AppHandle) -> Result<RestorePreviewOutcome, BackupCommandError> {
    info!("event=restore_preview_started");
    let connection = connection(app)?;

    let Some(picked) = app
        .dialog()
        .file()
        .add_filter("Studio backup", &["tar"])
        .blocking_pick_file()
    else {
        info!("event=restore_preview_cancelled");
        return Ok(RestorePreviewOutcome::Cancelled);
    };
    let Ok(archive_path) = picked.into_path() else {
        info!("event=restore_preview_cancelled");
        return Ok(RestorePreviewOutcome::Cancelled);
    };

    let archive = std::fs::read(&archive_path)?;
    let preview = request_preview(&connection, archive).await?;
    info!("event=restore_preview_finished");
    Ok(RestorePreviewOutcome::Previewed { preview })
}

fn connection(app: &AppHandle) -> Result<DaemonConnection, BackupCommandError> {
    app.state::<DaemonSupervisor>()
        .connection()
        .ok_or(BackupCommandError::NoConnection)
}

async fn fetch_backup(connection: &DaemonConnection) -> Result<Vec<u8>, BackupCommandError> {
    let bytes = reqwest::Client::new()
        .get(format!("{}/v1/backup", connection.base_url))
        .bearer_auth(&connection.token)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(bytes.to_vec())
}

async fn request_preview(
    connection: &DaemonConnection,
    archive: Vec<u8>,
) -> Result<serde_json::Value, BackupCommandError> {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/restore/preview", connection.base_url))
        .bearer_auth(&connection.token)
        .header("content-type", "application/octet-stream")
        .body(archive)
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        // Surface the daemon's actionable validation message (e.g. checksum
        // mismatch, incompatible schema) rather than a generic HTTP error.
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|e| e.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("daemon returned {status}"));
        return Err(BackupCommandError::Daemon(message));
    }

    serde_json::from_str(&body).map_err(|error| BackupCommandError::Daemon(error.to_string()))
}

/// Writes `bytes` to `target` via a sibling temp file and a final rename, so a
/// partial write never leaves a corrupt backup at the destination. Rust's
/// `rename` replaces an existing file on both Windows and Unix.
fn write_atomically(target: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = target.parent().unwrap_or_else(|| std::path::Path::new("."));
    let temp = directory.join(format!(
        ".susun-backup-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&temp, bytes)?;
    match std::fs::rename(&temp, target) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}
