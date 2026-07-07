use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;

use crate::{auth::authorize, error::ApiError, state::AppState};

const MAX_ERROR_MESSAGE_CHARS: usize = 200;

#[derive(Debug, Serialize)]
pub struct DiagnosticsJobError {
    pub id: String,
    pub kind: String,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticsEngineStatus {
    pub id: String,
    pub display_name: String,
    pub reachable: bool,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticsReport {
    pub daemon_version: &'static str,
    pub api_version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    /// Filename only (e.g. `studio.db`) — the full path can reveal the host
    /// username or directory layout, so it's redacted by default.
    pub db_file_name: String,
    pub db_size_bytes: Option<u64>,
    pub project_count: i64,
    pub recent_job_errors: Vec<DiagnosticsJobError>,
    pub engines: Vec<DiagnosticsEngineStatus>,
}

pub async fn diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DiagnosticsReport>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;

    let mut project_rows = conn.query("SELECT COUNT(*) FROM projects", ()).await?;
    let project_count: i64 = match project_rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    };

    let mut job_rows = conn
        .query(
            "SELECT id, kind, error, error_code, created_at_ms
             FROM jobs WHERE status = 'failed'
             ORDER BY created_at_ms DESC LIMIT 10",
            (),
        )
        .await?;
    let mut recent_job_errors = Vec::new();
    while let Some(row) = job_rows.next().await? {
        let error: Option<String> = row.get(2)?;
        recent_job_errors.push(DiagnosticsJobError {
            id: row.get(0)?,
            kind: row.get(1)?,
            error: error.map(|text| truncate_error(&text)),
            error_code: row.get(3)?,
            created_at_ms: row.get(4)?,
        });
    }

    let mut engine_rows = conn
        .query("SELECT id, display_name, last_health_json FROM engines", ())
        .await?;
    let mut engines = Vec::new();
    while let Some(row) = engine_rows.next().await? {
        let last_health_json: Option<String> = row.get(2)?;
        let reachable = last_health_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|value| value.get("reachable").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        engines.push(DiagnosticsEngineStatus {
            id: row.get(0)?,
            display_name: row.get(1)?,
            reachable,
        });
    }

    let db_file_name = state
        .db_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unknown>".to_owned());
    let db_size_bytes = std::fs::metadata(&state.db_path)
        .ok()
        .map(|meta| meta.len());

    Ok(Json(DiagnosticsReport {
        daemon_version: env!("CARGO_PKG_VERSION"),
        api_version: "1",
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        db_file_name,
        db_size_bytes,
        project_count,
        recent_job_errors,
        engines,
    }))
}

fn truncate_error(text: &str) -> String {
    if text.chars().count() <= MAX_ERROR_MESSAGE_CHARS {
        return text.to_owned();
    }
    let truncated: String = text.chars().take(MAX_ERROR_MESSAGE_CHARS).collect();
    format!("{truncated}… (truncated)")
}
