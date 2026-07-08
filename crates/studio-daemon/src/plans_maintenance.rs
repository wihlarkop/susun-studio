//! Startup maintenance: re-serialize every stored plan through the current
//! (possibly newly-widened) secret-redaction rules, so a widened keyword
//! list in `susun-secret` also repairs plans that were already persisted
//! under an older, narrower list — not just plans created from now on.

use turso::Database;

/// Re-serializes every row in `plans` whose `plan_json` is non-empty,
/// applying the current redaction rules, and writes back only the rows
/// whose serialized form actually changed. Returns the number of rows
/// rewritten. Blocked plans store an empty `plan_json` and are skipped.
pub async fn redact_stored_plans(db: &Database) -> Result<usize, turso::Error> {
    let conn = db.connect()?;

    // Collect first, write after: reading a row and writing on the same
    // connection while a cursor from that read is still open silently
    // drops the write on this turso version (see the Phase 3 investigation
    // note in project memory) — so the read is fully materialized into an
    // owned Vec inside this block before any `execute` call below.
    let rows: Vec<(String, String)> = {
        let mut rows = conn
            .query("SELECT id, plan_json FROM plans WHERE plan_json != ''", ())
            .await?;
        let mut collected = Vec::new();
        while let Some(row) = rows.next().await? {
            collected.push((row.get(0)?, row.get(1)?));
        }
        collected
    };

    let mut rewritten = 0usize;
    for (id, stored_json) in rows {
        let Ok(plan) = serde_json::from_str::<susun::ExecutionPlan>(&stored_json) else {
            // A row we can't parse under the current susun::ExecutionPlan
            // shape is not this pass's job to fix — skip it rather than
            // fail the whole startup sweep over one bad row.
            continue;
        };
        let Ok(redacted_json) = serde_json::to_string(&plan) else {
            continue;
        };
        if redacted_json != stored_json {
            conn.execute(
                "UPDATE plans SET plan_json = ?1 WHERE id = ?2",
                turso::params![redacted_json, id],
            )
            .await?;
            rewritten += 1;
        }
    }

    Ok(rewritten)
}
