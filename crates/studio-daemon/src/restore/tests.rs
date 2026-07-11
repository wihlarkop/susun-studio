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
fn sweep_removes_staged_and_pre_restore_but_keeps_rollback() -> TestResult {
    let dir = unique_dir();
    let active = dir.join("studio.db");
    std::fs::write(&active, b"db")?;
    std::fs::write(dir.join(".studio-restore-staged-abc.db"), b"staged")?;
    std::fs::write(dir.join(".studio-pre-restore-abc.tar"), b"backup")?;
    std::fs::write(dir.join("studio.db.rollback-abc"), b"rollback")?;

    sweep_stale_artifacts(&active);

    assert!(active.exists());
    assert!(!dir.join(".studio-restore-staged-abc.db").exists());
    assert!(!dir.join(".studio-pre-restore-abc.tar").exists());
    // Rollback copies survive so a failed restart can still roll back.
    assert!(dir.join("studio.db.rollback-abc").exists());

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
