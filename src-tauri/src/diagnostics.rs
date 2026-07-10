use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::daemon::{DaemonConnection, DaemonSupervisor};

const REDACTED: &str = "<redacted>";

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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticsExportOutcome {
    Exported,
    Cancelled,
}

pub async fn export_bundle(
    app: &AppHandle,
) -> Result<DiagnosticsExportOutcome, DiagnosticsExportError> {
    info!("event=diagnostics_export_started");
    let connection = app
        .state::<DaemonSupervisor>()
        .connection()
        .ok_or(DiagnosticsExportError::NoConnection)?;

    let report_bytes = fetch_diagnostics_json(&connection).await?;

    let log_dir = app.path().app_log_dir()?;
    let app_log = redact_sensitive_text(&read_tail(&log_dir.join("susun-studio.log"), 200));
    let daemon_log = redact_sensitive_text(&read_tail(&log_dir.join("daemon.log"), 200));

    let Some(target) = app
        .dialog()
        .file()
        .set_file_name("susun-studio-diagnostics.tar")
        .blocking_save_file()
    else {
        info!("event=diagnostics_export_cancelled");
        return Ok(DiagnosticsExportOutcome::Cancelled);
    };
    let Ok(target_path) = target.into_path() else {
        info!("event=diagnostics_export_cancelled");
        return Ok(DiagnosticsExportOutcome::Cancelled);
    };

    let mut archive = tar::Builder::new(Vec::new());
    append_bytes(&mut archive, "diagnostics.json", &report_bytes)?;
    append_bytes(&mut archive, "app.log", app_log.as_bytes())?;
    append_bytes(&mut archive, "daemon.log", daemon_log.as_bytes())?;
    let bytes = archive.into_inner()?;

    std::fs::write(&target_path, bytes)?;
    let target_file = target_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unknown>".to_owned());
    info!(
        "event=diagnostics_export_finished target_file={} bytes={}",
        target_file,
        std::fs::metadata(&target_path)
            .ok()
            .map(|meta| meta.len())
            .unwrap_or_default()
    );
    Ok(DiagnosticsExportOutcome::Exported)
}

async fn fetch_diagnostics_json(connection: &DaemonConnection) -> Result<Vec<u8>, reqwest::Error> {
    info!(
        "event=diagnostics_fetch_started base_url={}",
        connection.base_url
    );
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
    info!("event=diagnostics_fetch_finished bytes={}", bytes.len());
    Ok(bytes.to_vec())
}

fn read_tail(path: &std::path::Path, max_lines: usize) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<unknown>".to_owned());
        warn!("event=diagnostics_log_tail_missing file={file_name}");
        return String::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn redact_sensitive_text(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(redact_token_segment)
        .collect::<String>()
}

fn redact_token_segment(segment: &str) -> String {
    let token = segment.trim_end_matches(char::is_whitespace);
    let whitespace = &segment[token.len()..];
    format!("{}{}", redact_token(token), whitespace)
}

fn redact_token(token: &str) -> String {
    let key = token
        .split_once('=')
        .or_else(|| token.split_once(':'))
        .map(|(key, _)| key)
        .unwrap_or(token);

    if contains_sensitive_marker(key) {
        if let Some((key, _)) = token.split_once('=') {
            return format!("{key}={REDACTED}");
        }
        if let Some((key, _)) = token.split_once(':') {
            return format!("{key}:{REDACTED}");
        }
        return REDACTED.to_owned();
    }

    token.to_owned()
}

fn contains_sensitive_marker(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    const SUBSTRING_MARKERS: &[&str] = &[
        "authorization",
        "credential",
        "passwd",
        "password",
        "private_key",
        "secret",
        "token",
        "connection_string",
        "conn_str",
        "database_url",
        "db_url",
    ];
    const TOKEN_MARKERS: &[&str] = &[
        "auth", "bearer", "cert", "cookie", "dsn", "jwt", "key", "session",
    ];

    SUBSTRING_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
        || lower
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|token| TOKEN_MARKERS.contains(&token))
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

#[cfg(test)]
mod tests {
    use super::{REDACTED, redact_sensitive_text};

    #[test]
    fn redacts_sensitive_log_tokens() {
        let redacted = redact_sensitive_text(
            "request failed Authorization=Bearer-123 DATABASE_URL:postgres://user:pass@host PORT=7377",
        );

        assert!(redacted.contains(&format!("Authorization={REDACTED}")));
        assert!(redacted.contains(&format!("DATABASE_URL:{REDACTED}")));
        assert!(redacted.contains("PORT=7377"));
        assert!(!redacted.contains("Bearer-123"));
        assert!(!redacted.contains("postgres://user:pass@host"));
    }
}
