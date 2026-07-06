//! Startup maintenance: a watch session is entirely in-memory (a native OS
//! watcher thread), so a daemon crash/restart silently orphans any row still
//! `status = 'running'`. Reconciled once at startup, mirroring
//! `jobs::maintenance::reconcile_interrupted_jobs`.

use turso::{Database, params};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Marks any watch session still `running` at startup as stopped. Returns
/// the number of rows fixed.
pub async fn reconcile_interrupted_watch_sessions(db: &Database) -> Result<u64, turso::Error> {
    let conn = db.connect()?;
    conn.execute(
        "UPDATE watch_sessions
         SET status = 'stopped',
             error = 'Daemon restarted while this watch session was running.',
             updated_at_ms = ?1
         WHERE status = 'running'",
        params![now_ms()],
    )
    .await
}
