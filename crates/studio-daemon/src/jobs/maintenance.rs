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

/// Marks any job still `running` at startup as interrupted — the daemon
/// restarted mid-job, so its true final state is unknown. Returns the
/// number of rows fixed.
pub async fn reconcile_interrupted_jobs(db: &Database) -> Result<u64, turso::Error> {
    let conn = db.connect()?;
    conn.execute(
        "UPDATE jobs
         SET status = 'failed',
             error = 'Daemon restarted while this job was running; its final state is unknown.',
             error_code = 'interrupted',
             updated_at_ms = ?1
         WHERE status = 'running'",
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
        conn.execute("DELETE FROM jobs WHERE id = ?1", params![job_id.clone()])
            .await?;
    }

    Ok(prunable_ids.len())
}
