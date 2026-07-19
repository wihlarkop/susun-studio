//! Startup maintenance: reconcile jobs left "running" by a crash/kill, and
//! sweep old job history. Both run once at daemon startup, before serving —
//! the in-memory `JobRegistry` is always empty on a fresh process, so there
//! is nothing to reconcile there, only these persisted rows.

use turso::{Database, params};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Marks any job still `running` or `queued` at startup as interrupted — the
/// daemon restarted mid-job, so its true final state is unknown. `queued`
/// covers image-build jobs specifically: they sit in that status during the
/// (potentially slow) build-context preparation phase, before the actual
/// provider call starts, so a crash there must be reconciled the same way a
/// crash mid-`running` is. Returns the number of rows fixed.
pub async fn reconcile_interrupted_jobs(db: &Database) -> Result<u64, turso::Error> {
    let conn = db.connect()?;
    conn.execute(
        "UPDATE jobs
         SET status = 'failed',
             error = 'Daemon restarted while this job was running; its final state is unknown.',
             error_code = 'interrupted',
             updated_at_ms = ?1
         WHERE status IN ('running', 'queued')",
        params![now_ms()],
    )
    .await
}

/// Hardcoded for now — exposing these as user-editable settings is a
/// separate, larger piece of work (the Settings UI doesn't exist yet) and
/// isn't required to make jobs durable.
const RETENTION_COUNT_PER_PROJECT: usize = 200;
const RETENTION_DAYS: i64 = 30;

/// Deletes jobs (and their events) that are both beyond the per-project
/// count cap AND older than the age cap — either cap alone keeps a job.
/// Ranks jobs per project in application code rather than SQL: this
/// turso/Limbo version's window-function support is unverified, and a
/// plain `ORDER BY` is unambiguously supported. Returns the number of jobs
/// deleted.
pub async fn sweep_old_jobs(db: &Database) -> Result<usize, turso::Error> {
    let conn = db.connect()?;
    let cutoff_ms = now_ms() - RETENTION_DAYS * 24 * 60 * 60 * 1000;

    let ids_by_recency: Vec<(String, String, i64)> = {
        let mut rows = conn
            .query(
                "SELECT id, project_id, created_at_ms FROM jobs ORDER BY project_id, created_at_ms DESC",
                (),
            )
            .await?;
        let mut collected = Vec::new();
        while let Some(row) = rows.next().await? {
            collected.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        collected
    };

    let mut per_project_rank: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut prunable_ids = Vec::new();
    for (id, project_id, created_at_ms) in ids_by_recency {
        let rank = per_project_rank.entry(project_id).or_insert(0);
        let beyond_count_cap = *rank >= RETENTION_COUNT_PER_PROJECT;
        *rank += 1;
        if beyond_count_cap && created_at_ms < cutoff_ms {
            prunable_ids.push(id);
        }
    }

    for job_id in &prunable_ids {
        conn.execute(
            "DELETE FROM job_events WHERE job_id = ?1",
            params![job_id.clone()],
        )
        .await?;
        conn.execute(
            "DELETE FROM build_job_progress WHERE job_id = ?1",
            params![job_id.clone()],
        )
        .await?;
        conn.execute("DELETE FROM jobs WHERE id = ?1", params![job_id.clone()])
            .await?;
    }

    Ok(prunable_ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    async fn insert_job(db: &Database, id: &str, status: &str, created_at_ms: i64) -> TestResult {
        let conn = db.connect()?;
        conn.execute(
            "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json, created_at_ms, updated_at_ms)
             VALUES (?1, 'image_build', ?2, 'p1', 'engine-docker-local', '{}', ?3, ?3)",
            params![id.to_owned(), status.to_owned(), created_at_ms],
        )
        .await?;
        Ok(())
    }

    /// The whole reason `queued` was added to this query's `WHERE` clause:
    /// an image-build job crashing during its (potentially slow)
    /// preparation phase — before it ever reaches `running` — must not be
    /// left looking active forever, the same as a crash mid-`running`.
    #[tokio::test]
    async fn reconcile_interrupted_jobs_covers_both_running_and_queued() -> TestResult {
        let db = crate::test_support::fresh_db("maintenance-reconcile-queued").await?;
        insert_job(&db, "j-running", "running", 1).await?;
        insert_job(&db, "j-queued", "queued", 1).await?;
        insert_job(&db, "j-succeeded", "succeeded", 1).await?;

        let fixed = reconcile_interrupted_jobs(&db).await?;
        assert_eq!(fixed, 2);

        let conn = db.connect()?;
        let mut rows = conn
            .query("SELECT id, status, error_code FROM jobs ORDER BY id", ())
            .await?;
        let mut seen = Vec::new();
        while let Some(row) = rows.next().await? {
            let id: String = row.get(0)?;
            let status: String = row.get(1)?;
            let error_code: Option<String> = row.get(2)?;
            seen.push((id, status, error_code));
        }
        assert_eq!(
            seen,
            vec![
                (
                    "j-queued".to_owned(),
                    "failed".to_owned(),
                    Some("interrupted".to_owned())
                ),
                (
                    "j-running".to_owned(),
                    "failed".to_owned(),
                    Some("interrupted".to_owned())
                ),
                ("j-succeeded".to_owned(), "succeeded".to_owned(), None),
            ]
        );
        Ok(())
    }

    /// `sweep_old_jobs` must also clean up `build_job_progress` rows for any
    /// job it deletes — an orphaned progress row for a job that no longer
    /// exists would be a small but real leak the up/down-only sweep didn't
    /// need to worry about before build progress existed.
    #[tokio::test]
    async fn sweep_old_jobs_also_deletes_build_job_progress() -> TestResult {
        let db = crate::test_support::fresh_db("maintenance-sweep-build-progress").await?;
        let ancient = -(RETENTION_DAYS + 1) * 24 * 60 * 60 * 1000;
        // `j-0` gets the oldest timestamp of the batch (by a wide margin), so
        // `ORDER BY created_at_ms DESC` ranks it last and it is deterministically
        // the one job beyond the per-project count cap — ties among the rest
        // would otherwise make "which job gets pruned" unspecified.
        insert_job(&db, "j-0", "succeeded", ancient - 1_000_000).await?;
        for index in 1..=RETENTION_COUNT_PER_PROJECT {
            insert_job(
                &db,
                &format!("j-{index}"),
                "succeeded",
                ancient + index as i64,
            )
            .await?;
        }
        let conn = db.connect()?;
        conn.execute(
            "INSERT INTO build_job_progress (id, job_id, sequence, kind, created_at_ms)
             VALUES ('bp1', 'j-0', 0, 'started', 1)",
            (),
        )
        .await?;

        let removed = sweep_old_jobs(&db).await?;
        assert_eq!(removed, 1);

        let mut rows = conn
            .query("SELECT COUNT(*) FROM build_job_progress", ())
            .await?;
        let remaining: i64 = rows
            .next()
            .await?
            .map(|row| row.get(0))
            .transpose()?
            .unwrap_or(-1);
        assert_eq!(remaining, 0);
        Ok(())
    }
}
