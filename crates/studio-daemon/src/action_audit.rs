//! Bounded, redacted audit trail for destructive runtime actions.
//!
//! Every row is user-visible and secret-free *by construction*: callers pass only
//! enumerated codes and integer counts, and [`sanitize_code`] strips anything
//! that is not a short lowercase token before it reaches the database. Nothing
//! here ever stores argv, environment, credentials, endpoint values, private
//! paths, container output, or registry tokens.
//!
//! Ownership evidence (`runtime_profiles.owner_token`, `runtime_ownership_events`)
//! lives in separate tables; [`clear`] empties this audit table only, so a user
//! clearing their action history never destroys the evidence that proves which
//! built-in runtime Studio owns.

use serde::{Deserialize, Serialize};
use turso::{Database, params};

use crate::action_plans::ActionKind;
use crate::runtime::now_ms;

/// Keep at most this many audit rows. Older rows are swept on each write so the
/// table cannot grow without bound.
const MAX_AUDIT_ROWS: i64 = 500;
/// Codes are short tokens; anything longer is truncated (defensive — callers
/// already pass fixed enumerations).
const MAX_CODE_CHARS: usize = 48;

/// A redacted affected-resource count. Counts only — never identifiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedCount {
    pub category: String,
    pub count: i64,
}

/// Terminal outcomes recorded for an audited action.
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_REJECTED: &str = "rejected";
pub const STATUS_DEFERRED_14B: &str = "deferred_to_phase_14b";

/// The safe, redacted description of one destructive action attempt.
pub struct AuditEntry {
    pub kind: ActionKind,
    pub profile_id: Option<String>,
    pub runtime_class: Option<String>,
    /// Enumerated ownership/authorization result, e.g. "authorized",
    /// "rejected_external", "stale_preview".
    pub ownership_result: String,
    /// Safe command kind actually authorized, e.g. "metadata_only",
    /// "provider_prune", "deferred_provider_reset".
    pub command_kind: Option<String>,
    /// "none" | "current_user" | "os_mediated_consent".
    pub elevation_mode: Option<String>,
    pub terminal_status: String,
    pub affected: Vec<AffectedCount>,
    /// Short redacted failure code, never a raw error string.
    pub failure_code: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

/// A user-visible audit row returned to the UI and diagnostics export.
#[derive(Debug, Serialize)]
pub struct AuditRow {
    pub id: String,
    pub action_kind: String,
    pub domain: String,
    pub profile_id: Option<String>,
    pub runtime_class: Option<String>,
    pub ownership_result: String,
    pub command_kind: Option<String>,
    pub elevation_mode: Option<String>,
    pub terminal_status: String,
    pub affected: Vec<AffectedCount>,
    pub app_version: String,
    pub failure_code: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

/// Persist one redacted audit row and enforce the retention cap. Returns the new
/// row id. Best-effort affected-count serialization (never fails the action).
pub async fn record(db: &Database, entry: AuditEntry) -> Result<String, turso::Error> {
    let id = format!("aud_{}", uuid::Uuid::new_v4().simple());
    let affected_json = serde_json::to_string(&entry.affected).unwrap_or_else(|_| "[]".to_owned());
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO runtime_action_audit (
            id, action_kind, domain, profile_id, runtime_class, ownership_result,
            command_kind, elevation_mode, terminal_status, affected_counts_json,
            app_version, failure_code, started_at_ms, completed_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            id.clone(),
            entry.kind.as_str().to_owned(),
            entry.kind.domain().to_owned(),
            entry.profile_id,
            entry.runtime_class,
            sanitize_code(&entry.ownership_result),
            entry.command_kind.as_deref().map(sanitize_code),
            entry.elevation_mode.as_deref().map(sanitize_code),
            sanitize_code(&entry.terminal_status),
            affected_json,
            env!("CARGO_PKG_VERSION").to_owned(),
            entry.failure_code.as_deref().map(sanitize_code),
            entry.started_at_ms,
            entry.completed_at_ms,
        ],
    )
    .await?;

    // Retention: keep only the newest MAX_AUDIT_ROWS rows.
    conn.execute(
        "DELETE FROM runtime_action_audit
         WHERE id NOT IN (
             SELECT id FROM runtime_action_audit
             ORDER BY started_at_ms DESC, id DESC
             LIMIT ?1
         )",
        params![MAX_AUDIT_ROWS],
    )
    .await?;
    Ok(id)
}

/// Convenience for the common "rejected before execution" audit: the plan/gate
/// refused the action, so nothing was executed.
pub async fn record_rejection(
    db: &Database,
    kind: ActionKind,
    profile_id: Option<String>,
    ownership_result: &str,
    failure_code: &str,
) -> Result<String, turso::Error> {
    let now = now_ms();
    record(
        db,
        AuditEntry {
            kind,
            profile_id,
            runtime_class: None,
            ownership_result: ownership_result.to_owned(),
            command_kind: None,
            elevation_mode: None,
            terminal_status: STATUS_REJECTED.to_owned(),
            affected: Vec::new(),
            failure_code: Some(failure_code.to_owned()),
            started_at_ms: now,
            completed_at_ms: Some(now),
        },
    )
    .await
}

/// List the most recent audit rows, newest first.
pub async fn list(db: &Database, limit: i64) -> Result<Vec<AuditRow>, turso::Error> {
    let limit = limit.clamp(1, MAX_AUDIT_ROWS);
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, action_kind, domain, profile_id, runtime_class, ownership_result,
                    command_kind, elevation_mode, terminal_status, affected_counts_json,
                    app_version, failure_code, started_at_ms, completed_at_ms
             FROM runtime_action_audit
             ORDER BY started_at_ms DESC, id DESC
             LIMIT ?1",
            params![limit],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let affected_json: String = row.get(9)?;
        out.push(AuditRow {
            id: row.get(0)?,
            action_kind: row.get(1)?,
            domain: row.get(2)?,
            profile_id: row.get(3)?,
            runtime_class: row.get(4)?,
            ownership_result: row.get(5)?,
            command_kind: row.get(6)?,
            elevation_mode: row.get(7)?,
            terminal_status: row.get(8)?,
            affected: serde_json::from_str(&affected_json).unwrap_or_default(),
            app_version: row.get(10)?,
            failure_code: row.get(11)?,
            started_at_ms: row.get(12)?,
            completed_at_ms: row.get(13)?,
        });
    }
    Ok(out)
}

/// Clear the entire action-history audit table. Deliberately touches ONLY
/// `runtime_action_audit`; ownership evidence in `runtime_profiles.owner_token`
/// and `runtime_ownership_events` is preserved. Returns the number of rows
/// removed.
pub async fn clear(db: &Database) -> Result<u64, turso::Error> {
    let conn = db.connect()?;
    conn.execute("DELETE FROM runtime_action_audit", ()).await
}

/// Reduce an arbitrary code string to a short, lowercase, secret-free token.
/// Keeps `[a-z0-9_]`, collapses everything else to `_`, and truncates. This is a
/// defensive backstop: callers already pass fixed enumerations, but this
/// guarantees a stray path or message fragment can never land in an audit row.
fn sanitize_code(code: &str) -> String {
    let mut out = String::with_capacity(code.len().min(MAX_CODE_CHARS));
    for ch in code.chars() {
        if out.chars().count() >= MAX_CODE_CHARS {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".to_owned()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    async fn fixture() -> TestResult<(Database, std::path::PathBuf)> {
        let path = std::env::temp_dir().join(format!(
            "studio-audit-test-{}.db",
            uuid::Uuid::new_v4().simple()
        ));
        let db = db::open_database(path.clone()).await?;
        Ok((db, path))
    }

    #[test]
    fn sanitize_strips_paths_and_secrets() {
        assert_eq!(
            sanitize_code(r"C:\Users\me\secret token=abc"),
            "c__users_me_secret_token_abc"
        );
        assert_eq!(sanitize_code("stale_preview"), "stale_preview");
        assert_eq!(sanitize_code(""), "unknown");
    }

    #[tokio::test]
    async fn record_list_roundtrip_is_redacted() -> TestResult {
        let (db, path) = fixture().await?;
        record(
            &db,
            AuditEntry {
                kind: ActionKind::MigrationCommit,
                profile_id: Some("p2".to_owned()),
                runtime_class: Some("external_local".to_owned()),
                ownership_result: "authorized".to_owned(),
                command_kind: Some("metadata_only".to_owned()),
                elevation_mode: Some("none".to_owned()),
                terminal_status: STATUS_COMPLETED.to_owned(),
                affected: vec![AffectedCount {
                    category: "project_bindings".to_owned(),
                    count: 3,
                }],
                failure_code: None,
                started_at_ms: 10,
                completed_at_ms: Some(20),
            },
        )
        .await?;
        let rows = list(&db, 50).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action_kind, "migration_commit");
        assert_eq!(rows[0].domain, "migration");
        assert_eq!(rows[0].affected[0].count, 3);
        assert!(rows[0].failure_code.is_none());
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[tokio::test]
    async fn clear_empties_audit_but_preserves_ownership_events() -> TestResult {
        let (db, path) = fixture().await?;
        record_rejection(
            &db,
            ActionKind::DestructiveResetEngineData,
            Some("p1".to_owned()),
            "rejected_external",
            "external_runtime",
        )
        .await?;
        let conn = db.connect()?;
        conn.execute(
            "INSERT INTO runtime_ownership_events
                (id, profile_id, provider_id, provider_runtime_key, event_kind, created_at_ms)
             VALUES ('e1', 'p1', 'windows-podman', 'machine/x', 'setup_created', 1)",
            (),
        )
        .await?;

        let removed = clear(&db).await?;
        assert!(removed >= 1);
        assert_eq!(list(&db, 50).await?.len(), 0);

        // Ownership evidence must survive a history clear.
        let mut rows = conn
            .query("SELECT COUNT(*) FROM runtime_ownership_events", ())
            .await?;
        let remaining: i64 = rows
            .next()
            .await?
            .map(|r| r.get(0))
            .transpose()?
            .unwrap_or(0);
        assert_eq!(remaining, 1);
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}
