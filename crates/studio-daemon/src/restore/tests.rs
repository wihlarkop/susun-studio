//! Tests for restore-apply preparation and lifecycle coordination. The actual
//! file swap + daemon restart is orchestrated by the Tauri supervisor and is
//! covered by manual desktop verification, not here.

use std::sync::Arc;
use std::time::Duration;

use super::{
    DaemonAvailability, RestoreCoordinator, RestoreError, prepare_restore, recover_incomplete_swap,
    sweep_stale_artifacts,
};
use crate::{backup, db};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn unique_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "studio-restore-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Sets a file's modified time to a fixed epoch offset so ordering by mtime is
/// deterministic in tests. Opens with write access — Windows needs it to touch
/// file attributes.
fn set_mtime_secs(path: &std::path::Path, secs: u64) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs))
}

async fn seeded_db(path: &std::path::Path) -> TestResult<turso::Database> {
    let db = db::open_database(path.to_path_buf()).await?;
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms) VALUES ('p','Proj','/proj',1)",
        (),
    )
    .await?;
    Ok(db)
}

#[tokio::test]
async fn prepare_stages_migrated_copy_and_pre_restore_backup() -> TestResult {
    let dir = unique_dir();
    let active = dir.join("studio.db");
    let db = seeded_db(&active).await?;
    let archive = backup::create_backup_archive(&db, &active).await?;

    let coordinator = RestoreCoordinator::new();
    let prepared = prepare_restore(&coordinator, &db, &active, &archive).await?;

    // Handoff points at real, existing artifacts beside the active database.
    assert!(std::path::Path::new(&prepared.staged_database_path).exists());
    assert!(std::path::Path::new(&prepared.pre_restore_backup_path).exists());
    assert!(prepared.restore_id.starts_with("rst_"));
    assert_eq!(prepared.manifest.project_count, 1);
    // Preparing never leaves the daemon stuck out of the Ready state.
    assert_eq!(coordinator.availability(), DaemonAvailability::Ready);

    // The staged copy is a real, migrated database carrying the seeded data.
    let staged = turso::Builder::new_local(&prepared.staged_database_path)
        .build()
        .await?;
    let conn = staged.connect()?;
    let mut rows = conn.query("SELECT COUNT(*) FROM projects", ()).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| std::io::Error::other("expected a count row"))?;
    let count: i64 = row.get(0)?;
    assert_eq!(count, 1);
    let mut audit_rows = conn
        .query(
            "SELECT terminal_status, correlation_token FROM runtime_action_audit
             WHERE action_kind = 'metadata_restore'",
            (),
        )
        .await?;
    let audit = audit_rows
        .next()
        .await?
        .ok_or_else(|| std::io::Error::other("expected staged restore audit"))?;
    assert_eq!(audit.get::<String>(0)?, "staged");
    assert_eq!(audit.get::<String>(1)?, prepared.restore_id);

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn prepare_sanitizes_restored_ownership() -> TestResult {
    let dir = unique_dir();
    let active = dir.join("studio.db");
    let db = seeded_db(&active).await?;
    // A backup that claims a Studio-managed built-in with an owner token.
    db.connect()?
        .execute(
            "INSERT INTO runtime_profiles (id, provider_id, provider_runtime_key, display_name,
                product, platform, runtime_class, ownership_state, source, owner_token,
                installation_state, process_state, connection_state, availability_state,
                observed_at_ms, created_at_ms, updated_at_ms)
             VALUES ('b','windows-podman','machine/susun-runtime-default','Built-in','podman',
                'windows','built_in','studio_managed','studio_setup','tok-123',
                'installed','running','summarized','available',1,1,1)",
            (),
        )
        .await?;
    let archive = backup::create_backup_archive(&db, &active).await?;

    let coordinator = RestoreCoordinator::new();
    let prepared = prepare_restore(&coordinator, &db, &active, &archive).await?;

    // The staged database must not carry trusted ownership forward.
    let staged = turso::Builder::new_local(&prepared.staged_database_path)
        .build()
        .await?;
    let conn = staged.connect()?;
    let mut rows = conn
        .query(
            "SELECT ownership_state, owner_token, source, availability_state
             FROM runtime_profiles WHERE id = 'b'",
            (),
        )
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| std::io::Error::other("expected the built-in profile"))?;
    let ownership_state: String = row.get(0)?;
    let owner_token: Option<String> = row.get(1)?;
    let source: String = row.get(2)?;
    let availability_state: String = row.get(3)?;
    assert_eq!(ownership_state, "ownership_unknown");
    assert!(owner_token.is_none());
    assert_eq!(source, "restored_metadata");
    assert_eq!(availability_state, "unknown");

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn prepare_rejects_a_non_archive() -> TestResult {
    let dir = unique_dir();
    let active = dir.join("studio.db");
    let db = seeded_db(&active).await?;

    let coordinator = RestoreCoordinator::new();
    assert!(matches!(
        prepare_restore(&coordinator, &db, &active, b"not a tar archive").await,
        Err(RestoreError::Archive(_))
    ));
    // A failed prepare still returns the daemon to Ready.
    assert_eq!(coordinator.availability(), DaemonAvailability::Ready);

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn begin_restore_shutdown_flips_state_and_wakes_waiter() -> TestResult {
    let coordinator = Arc::new(RestoreCoordinator::new());
    let waiter = coordinator.clone();
    let handle = tokio::spawn(async move { waiter.shutdown_requested().await });
    // Let the spawned task register as a waiter before we notify.
    tokio::time::sleep(Duration::from_millis(20)).await;

    coordinator.begin_restore_shutdown();

    tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .map_err(|_| std::io::Error::other("shutdown signal did not fire"))?
        .map_err(|_| std::io::Error::other("waiter task did not complete"))?;
    assert_eq!(
        coordinator.availability(),
        DaemonAvailability::RestoreShutdownPending
    );
    Ok(())
}

#[test]
fn prepared_restore_token_is_single_use() {
    let coordinator = RestoreCoordinator::new();
    coordinator.arm_restore("rst_expected");
    assert!(!coordinator.consume_armed_restore("rst_other"));
    assert!(coordinator.consume_armed_restore("rst_expected"));
    assert!(!coordinator.consume_armed_restore("rst_expected"));
}

#[test]
fn sweep_removes_staged_keeps_rollback_and_newest_pre_restore() -> TestResult {
    let dir = unique_dir();
    let active = dir.join("studio.db");
    std::fs::write(&active, b"db")?;
    std::fs::write(dir.join(".studio-restore-staged-abc.db"), b"staged")?;
    std::fs::write(dir.join("studio.db.rollback-abc"), b"rollback")?;

    // Two pre-restore backups; the newer one must survive as the "undo the last
    // restore" net, the older one is swept. Set explicit, distinct mtimes so
    // newest selection is deterministic regardless of write timing.
    let older = dir.join(".studio-pre-restore-old.tar");
    let newer = dir.join(".studio-pre-restore-new.tar");
    std::fs::write(&older, b"old-backup")?;
    std::fs::write(&newer, b"new-backup")?;
    set_mtime_secs(&older, 1_000)?;
    set_mtime_secs(&newer, 2_000)?;

    sweep_stale_artifacts(&active);

    assert!(active.exists());
    assert!(!dir.join(".studio-restore-staged-abc.db").exists());
    // Rollback copies survive so a failed restart can still roll back.
    assert!(dir.join("studio.db.rollback-abc").exists());
    // Newest pre-restore backup kept; older one swept.
    assert!(newer.exists());
    assert!(!older.exists());

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn recover_restores_active_from_rollback_when_swap_was_interrupted() -> TestResult {
    let dir = unique_dir();
    let active = dir.join("studio.db");
    // Active is missing (crash between the two swap renames); rollback survives.
    std::fs::write(dir.join("studio.db.rollback-xyz"), b"previous-data")?;

    recover_incomplete_swap(&active);

    assert!(active.exists());
    assert_eq!(std::fs::read(&active)?, b"previous-data");
    assert!(!dir.join("studio.db.rollback-xyz").exists());

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn recover_leaves_a_present_active_untouched() -> TestResult {
    let dir = unique_dir();
    let active = dir.join("studio.db");
    std::fs::write(&active, b"current")?;
    std::fs::write(dir.join("studio.db.rollback-xyz"), b"previous")?;

    recover_incomplete_swap(&active);

    // A present active database must never be overwritten from a rollback.
    assert_eq!(std::fs::read(&active)?, b"current");

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
