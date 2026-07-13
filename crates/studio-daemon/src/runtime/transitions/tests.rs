use turso::{Database, params};

use super::*;
use crate::db;
use crate::runtime::{
    dimension,
    provider::{ObservedProfile, RuntimeClass, profile_id},
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

async fn fixture() -> TestResult<(Database, std::path::PathBuf, String, String)> {
    let path = std::env::temp_dir().join(format!(
        "studio-transition-test-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    let db = db::open_database(path.clone()).await?;
    let source_key = "machine/source";
    let target_key = "machine/target";
    let profiles = [
        observed(source_key, "Source"),
        observed(target_key, "Target"),
    ];
    super::super::persist_observed(&db, &profiles).await?;
    let source = profile_id("windows-podman", source_key);
    let target = profile_id("windows-podman", target_key);
    let conn = db.connect()?;
    for (id, name) in [("p1", "One"), ("p2", "Two")] {
        conn.execute(
            "INSERT INTO projects (id, name, path, created_at_ms, runtime_profile_id)
             VALUES (?1, ?2, ?3, 1, ?4)",
            params![
                id.to_owned(),
                name.to_owned(),
                format!("/{id}"),
                source.clone()
            ],
        )
        .await?;
    }
    Ok((db, path, source, target))
}

fn observed(key: &str, name: &str) -> ObservedProfile {
    ObservedProfile {
        id: profile_id("windows-podman", key),
        provider_id: "windows-podman".to_owned(),
        provider_runtime_key: key.to_owned(),
        display_name: name.to_owned(),
        product: "podman".to_owned(),
        platform: "windows".to_owned(),
        runtime_class: RuntimeClass::ExternalLocal,
        installation: dimension("installed", None),
        process: dimension("running", None),
        connection: dimension("summarized", None),
        endpoint_summary: None,
        provider_default: false,
        observed_at_ms: now_ms(),
    }
}

#[tokio::test]
async fn migration_preview_and_execution_move_selected_bindings_only() -> TestResult {
    let (db, path, source, target) = fixture().await?;
    let request = MigrationRequest {
        source_profile_id: source.clone(),
        target_profile_id: target.clone(),
        project_ids: vec!["p1".to_owned()],
    };
    let preview = preview_migration(&db, &request).await?.ok_or("preview")?;
    assert!(preview.can_migrate);
    assert_eq!(preview.projects.len(), 1);
    assert!(preview.rollback_available);

    let result = execute_migration(&db, &request).await?.ok_or("result")?;
    assert_eq!(result.status, "completed");
    assert_eq!(result.project_count, 1);

    let conn = db.connect()?;
    assert_eq!(binding(&conn, "p1").await?, target);
    assert_eq!(binding(&conn, "p2").await?, source);
    assert_eq!(migration_status(&conn).await?, "completed");

    let rollback = rollback_migration(&db, &result.migration_id)
        .await?
        .ok_or("rollback")?;
    assert_eq!(rollback.status, "rolled_back");
    assert_eq!(rollback.restored_project_count, 1);
    assert_eq!(binding(&conn, "p1").await?, source);
    assert_eq!(migration_status(&conn).await?, "rolled_back");
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn migration_rolls_back_every_binding_when_one_update_fails() -> TestResult {
    let (db, path, source, target) = fixture().await?;
    let request = MigrationRequest {
        source_profile_id: source.clone(),
        target_profile_id: target,
        project_ids: vec!["p1".to_owned(), "p1".to_owned()],
    };
    let result = execute_migration(&db, &request).await?.ok_or("result")?;
    assert_eq!(result.status, "failed");

    let conn = db.connect()?;
    assert_eq!(binding(&conn, "p1").await?, source);
    assert_eq!(migration_status(&conn).await?, "failed");
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn destructive_preview_never_allows_external_runtime() -> TestResult {
    let (db, path, source, _) = fixture().await?;
    let preview = preview_destructive_operation(
        &db,
        &source,
        &DestructivePreviewRequest {
            action: DestructiveAction::ResetEngineData,
        },
    )
    .await?
    .ok_or("preview")?;
    assert!(!preview.allowed);
    assert!(preview.blocker.is_some());
    assert_eq!(preview.affected.last().and_then(|item| item.count), Some(2));
    let _ = std::fs::remove_file(path);
    Ok(())
}

async fn binding(conn: &turso::Connection, project_id: &str) -> TestResult<String> {
    let mut rows = conn
        .query(
            "SELECT runtime_profile_id FROM projects WHERE id = ?1",
            params![project_id.to_owned()],
        )
        .await?;
    Ok(rows.next().await?.ok_or("project")?.get(0)?)
}

async fn migration_status(conn: &turso::Connection) -> TestResult<String> {
    let mut rows = conn
        .query(
            "SELECT status FROM runtime_migrations ORDER BY created_at_ms DESC LIMIT 1",
            (),
        )
        .await?;
    Ok(rows.next().await?.ok_or("migration")?.get(0)?)
}
