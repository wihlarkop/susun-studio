use turso::{Database, params};

const MAX_ROWS_PER_JOB: i64 = 500;
const MAX_MESSAGE_CHARS: usize = 2_000;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct TransferProgressEntry {
    pub sequence: i64,
    pub operation: String,
    pub stage: String,
    pub current: Option<i64>,
    pub total: Option<i64>,
    pub message: Option<String>,
    pub created_at_ms: i64,
}

pub async fn persist_transfer_progress(
    db: &Database,
    job_id: &str,
    sequence: i64,
    progress: susun::ActionProgress,
) -> Result<(), turso::Error> {
    let conn = db.connect()?;
    let id = format!("tp_{}", uuid::Uuid::new_v4().simple());
    let operation = match progress.operation {
        susun::EngineProgressOperation::PullImage => "pull_image",
        susun::EngineProgressOperation::PushImage => "push_image",
    };
    let stage = bound_text(&progress.stage);
    let message = progress.message.as_deref().map(bound_text);
    conn.execute(
        "INSERT INTO artifact_transfer_progress (
            id, job_id, sequence, operation, stage, current_units, total_units, message, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            job_id.to_owned(),
            sequence,
            operation,
            stage,
            progress.current.map(clamp_u64),
            progress.total.map(clamp_u64),
            message,
            crate::runtime::now_ms(),
        ],
    )
    .await?;
    conn.execute(
        "DELETE FROM artifact_transfer_progress WHERE job_id = ?1 AND id NOT IN (
            SELECT id FROM artifact_transfer_progress WHERE job_id = ?1
            ORDER BY sequence DESC LIMIT ?2
         )",
        params![job_id.to_owned(), MAX_ROWS_PER_JOB],
    )
    .await?;
    Ok(())
}

pub async fn read_transfer_progress(
    db: &Database,
    job_id: &str,
) -> Result<Vec<TransferProgressEntry>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT sequence, operation, stage, current_units, total_units, message, created_at_ms
             FROM artifact_transfer_progress WHERE job_id = ?1 ORDER BY sequence ASC",
            params![job_id.to_owned()],
        )
        .await?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next().await? {
        entries.push(TransferProgressEntry {
            sequence: row.get(0)?,
            operation: row.get(1)?,
            stage: row.get(2)?,
            current: row.get(3)?,
            total: row.get(4)?,
            message: row.get(5)?,
            created_at_ms: row.get(6)?,
        });
    }
    Ok(entries)
}

fn clamp_u64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn bound_text(value: &str) -> String {
    let redacted = crate::logging::redact_sensitive_text(value);
    if redacted.chars().count() <= MAX_MESSAGE_CHARS {
        redacted
    } else {
        let truncated: String = redacted.chars().take(MAX_MESSAGE_CHARS).collect();
        format!("{truncated}... [truncated]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn progress_is_ordered_bounded_and_message_limited()
    -> Result<(), Box<dyn std::error::Error>> {
        let db = crate::test_support::fresh_db("transfer-progress").await?;
        for sequence in 0..=500 {
            persist_transfer_progress(
                &db,
                "job-1",
                sequence,
                susun::ActionProgress {
                    operation: susun::EngineProgressOperation::PullImage,
                    stage: "download".to_owned(),
                    current: Some(sequence as u64),
                    total: Some(501),
                    message: Some("x".repeat(3_000)),
                },
            )
            .await?;
        }
        let rows = read_transfer_progress(&db, "job-1").await?;
        assert_eq!(rows.len(), 500);
        assert_eq!(rows[0].sequence, 1);
        assert!(
            rows[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .chars()
                .count()
                <= 2_020
        );
        Ok(())
    }
}
