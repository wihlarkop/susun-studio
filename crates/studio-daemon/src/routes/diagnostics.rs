use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;

use crate::{action_audit, auth::authorize, error::ApiError, logging, state::AppState};

const MAX_ERROR_MESSAGE_CHARS: usize = 200;
const REDACTED: &str = "<redacted>";
/// How many recent destructive-action audit rows to include in diagnostics.
const MAX_AUDIT_ENTRIES: usize = 25;

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
pub struct DiagnosticsActionAudit {
    pub action_kind: String,
    pub domain: String,
    pub runtime_class: Option<String>,
    pub ownership_result: String,
    pub command_kind: Option<String>,
    pub elevation_mode: Option<String>,
    pub terminal_status: String,
    pub affected: Vec<action_audit::AffectedCount>,
    pub app_version: String,
    pub failure_code: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
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
    pub recent_action_audit: Vec<DiagnosticsActionAudit>,
}

pub async fn diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DiagnosticsReport>, ApiError> {
    authorize(&state, &headers)?;
    logging::info("diagnostics_requested", &[]);

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
            error: error.map(|text| redact_and_truncate_error(&text)),
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
    let recent_action_audit = action_audit::list(&state.db, MAX_AUDIT_ENTRIES as i64)
        .await?
        .into_iter()
        .map(|row| DiagnosticsActionAudit {
            action_kind: diagnostic_code(&row.action_kind),
            domain: diagnostic_code(&row.domain),
            runtime_class: row.runtime_class.as_deref().map(diagnostic_code),
            ownership_result: diagnostic_code(&row.ownership_result),
            command_kind: row.command_kind.as_deref().map(diagnostic_code),
            elevation_mode: row.elevation_mode.as_deref().map(diagnostic_code),
            terminal_status: diagnostic_code(&row.terminal_status),
            affected: row
                .affected
                .into_iter()
                .take(16)
                .map(|affected| action_audit::AffectedCount {
                    category: diagnostic_code(&affected.category),
                    count: affected.count.max(0),
                })
                .collect(),
            app_version: diagnostic_code(&row.app_version),
            failure_code: row.failure_code.as_deref().map(diagnostic_code),
            started_at_ms: row.started_at_ms,
            completed_at_ms: row.completed_at_ms,
        })
        .collect();

    let report = DiagnosticsReport {
        daemon_version: env!("CARGO_PKG_VERSION"),
        api_version: "1",
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        db_file_name,
        db_size_bytes,
        project_count,
        recent_job_errors,
        engines,
        recent_action_audit,
    };
    logging::info(
        "diagnostics_finished",
        &[
            ("project_count", report.project_count.to_string()),
            (
                "recent_job_error_count",
                report.recent_job_errors.len().to_string(),
            ),
            ("engine_count", report.engines.len().to_string()),
            (
                "action_audit_count",
                report.recent_action_audit.len().to_string(),
            ),
        ],
    );
    Ok(Json(report))
}

fn diagnostic_code(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(48));
    for character in value.chars().take(48) {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-') {
            output.push(character.to_ascii_lowercase());
        } else {
            output.push('_');
        }
    }
    if output.is_empty() {
        "unknown".to_owned()
    } else {
        output
    }
}

fn redact_and_truncate_error(text: &str) -> String {
    let redacted = redact_sensitive_error_text(text);
    if redacted.chars().count() <= MAX_ERROR_MESSAGE_CHARS {
        return redacted;
    }
    let truncated: String = redacted.chars().take(MAX_ERROR_MESSAGE_CHARS).collect();
    format!("{truncated}… (truncated)")
}

fn redact_sensitive_error_text(text: &str) -> String {
    text.split_whitespace()
        .map(redact_error_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_error_token(token: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::{REDACTED, redact_and_truncate_error};

    #[test]
    fn diagnostics_error_redaction_masks_sensitive_key_values() {
        let redacted = redact_and_truncate_error(
            "engine failed with API_KEY=super-secret DATABASE_URL:postgres://user:pass@host PORT=8080",
        );

        assert!(redacted.contains(&format!("API_KEY={REDACTED}")));
        assert!(redacted.contains(&format!("DATABASE_URL:{REDACTED}")));
        assert!(redacted.contains("PORT=8080"));
        assert!(!redacted.contains("super-secret"));
        assert!(!redacted.contains("postgres://user:pass@host"));
    }
}
