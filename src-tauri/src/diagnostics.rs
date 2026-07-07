use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::daemon::{DaemonConnection, DaemonSupervisor};

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsExportError {
    #[error("daemon connection is not available yet")]
    NoConnection,
    #[error("failed to fetch daemon diagnostics: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("failed to resolve app log directory: {0}")]
    PathResolution(#[from] tauri::Error),
    #[error("failed to build diagnostics bundle: {0}")]
    Io(#[from] std::io::Error),
    #[error("export was cancelled")]
    Cancelled,
}

pub async fn export_bundle(app: &AppHandle) -> Result<(), DiagnosticsExportError> {
    let connection = app
        .state::<DaemonSupervisor>()
        .connection()
        .ok_or(DiagnosticsExportError::NoConnection)?;

    let report_bytes = fetch_diagnostics_json(&connection).await?;

    let log_dir = app.path().app_log_dir()?;
    let app_log = read_tail(&log_dir.join("susun-studio.log"), 200);
    let daemon_log = read_tail(&log_dir.join("daemon.log"), 200);

    let Some(target) = app
        .dialog()
        .file()
        .set_file_name("susun-studio-diagnostics.tar")
        .blocking_save_file()
    else {
        return Err(DiagnosticsExportError::Cancelled);
    };
    let target_path = target
        .into_path()
        .map_err(|_| DiagnosticsExportError::Cancelled)?;

    let mut archive = tar::Builder::new(Vec::new());
    append_bytes(&mut archive, "diagnostics.json", &report_bytes)?;
    append_bytes(&mut archive, "app.log", app_log.as_bytes())?;
    append_bytes(&mut archive, "daemon.log", daemon_log.as_bytes())?;
    let bytes = archive.into_inner()?;

    std::fs::write(&target_path, bytes)?;
    Ok(())
}

async fn fetch_diagnostics_json(connection: &DaemonConnection) -> Result<Vec<u8>, reqwest::Error> {
    // `error_for_status()` matters here: without it, a 401/500 JSON error
    // body from the daemon would get silently bundled as if it were the
    // real diagnostics report.
    let bytes = reqwest::Client::new()
        .get(format!("{}/v1/diagnostics", connection.base_url))
        .bearer_auth(&connection.token)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(bytes.to_vec())
}

fn read_tail(path: &std::path::Path, max_lines: usize) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn append_bytes(
    archive: &mut tar::Builder<Vec<u8>>,
    name: &str,
    contents: &[u8],
) -> std::io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive.append_data(&mut header, name, contents)
}
