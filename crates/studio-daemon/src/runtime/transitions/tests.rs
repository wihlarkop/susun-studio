use turso::{Database, params};

use super::*;
use crate::action_plans::ActionPlanStore;
use crate::db;
use crate::runtime::{
    dimension,
    provider::{ObservedProfile, RuntimeClass, profile_id},
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const OWNER: &str = "owner-a";

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

/// Insert a Studio-managed built-in runtime directly so destructive commit paths
/// can be exercised without a live provider.
async fn insert_built_in(db: &Database, id: &str) -> TestResult {
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO runtime_profiles
            (id, provider_id, provider_runtime_key, display_name, product, platform,
             runtime_class, ownership_state, source, owner_token,
             installation_state, process_state, connection_state,
             availability_state, is_selected, observation_revision,
             observed_at_ms, created_at_ms, updated_at_ms)
         VALUES (?1, 'windows-podman', 'machine/built', 'Built', 'podman', 'windows',
             'built_in', 'studio_managed', 'studio_setup', 'own_tok',
             'installed', 'running', 'summarized',
             'available', 0, 0, 1, 1, 1)",
        params![id.to_owned()],
    )
    .await?;
    Ok(())
}

async fn audit_status(conn: &turso::Connection) -> TestResult<String> {
    let mut rows = conn
        .query(
            "SELECT terminal_status FROM runtime_action_audit ORDER BY started_at_ms DESC, id DESC LIMIT 1",
            (),
        )
        .await?;
    Ok(rows.next().await?.ok_or("audit")?.get(0)?)
}

#[tokio::test]
async fn migration_commit_moves_only_the_planned_bindings() -> TestResult {
    let (db, path, source, target) = fixture().await?;
    let store = ActionPlanStore::default();
    let request = MigrationRequest {
        source_profile_id: source.clone(),
        target_profile_id: target.clone(),
        project_ids: vec!["p1".to_owned()],
    };
    let preview = preview_migration(&db, &store, OWNER, &request)
        .await?
        .ok_or("preview")?;
    assert!(preview.can_migrate);
    let plan_id = preview.plan_id.clone().ok_or("plan_id")?;

    // Wrong owner cannot claim; the plan survives for the real owner.
    assert!(
        commit_migration(&db, &store, "owner-b", &plan_id)
            .await
            .is_err()
    );

    let Ok(result) = commit_migration(&db, &store, OWNER, &plan_id).await else {
        return Err("commit rejected".into());
    };
    assert_eq!(result.status, "completed");
    assert_eq!(result.project_count, 1);

    // Replay of a consumed plan is rejected.
    assert!(
        commit_migration(&db, &store, OWNER, &plan_id)
            .await
            .is_err()
    );

    let conn = db.connect()?;
    assert_eq!(binding(&conn, "p1").await?, target);
    assert_eq!(binding(&conn, "p2").await?, source);
    assert_eq!(migration_status(&conn).await?, "completed");
    assert_eq!(audit_status(&conn).await?, "completed");
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn migration_commit_rejects_stale_preview() -> TestResult {
    let (db, path, source, target) = fixture().await?;
    let store = ActionPlanStore::default();
    let request = MigrationRequest {
        source_profile_id: source.clone(),
        target_profile_id: target.clone(),
        project_ids: vec!["p1".to_owned()],
    };
    let preview = preview_migration(&db, &store, OWNER, &request)
        .await?
        .ok_or("preview")?;
    let plan_id = preview.plan_id.clone().ok_or("plan_id")?;

    // Change the inventory after preview: rebind p1 away from the source.
    let conn = db.connect()?;
    conn.execute(
        "UPDATE projects SET runtime_profile_id = ?1 WHERE id = 'p1'",
        params![target.clone()],
    )
    .await?;

    // Commit must refuse the stale plan rather than mutate.
    assert!(
        commit_migration(&db, &store, OWNER, &plan_id)
            .await
            .is_err()
    );
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn migration_commit_rejects_when_a_job_starts_after_preview() -> TestResult {
    let (db, path, source, target) = fixture().await?;
    let store = ActionPlanStore::default();
    let request = MigrationRequest {
        source_profile_id: source.clone(),
        target_profile_id: target.clone(),
        project_ids: vec!["p1".to_owned()],
    };
    let preview = preview_migration(&db, &store, OWNER, &request)
        .await?
        .ok_or("preview")?;
    let plan_id = preview.plan_id.clone().ok_or("plan_id")?;

    // A running job appears on the source runtime after preview.
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json,
            created_at_ms, updated_at_ms, runtime_profile_id, runtime_class)
         VALUES ('j1','up','running','p1','engine-docker-local','{}',1,1,?1,'built_in')",
        params![source.clone()],
    )
    .await?;

    assert!(
        commit_migration(&db, &store, OWNER, &plan_id)
            .await
            .is_err()
    );
    // Binding stays on the source.
    assert_eq!(binding(&conn, "p1").await?, source);
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn rollback_prepare_and_commit_restore_bindings() -> TestResult {
    let (db, path, source, target) = fixture().await?;
    let store = ActionPlanStore::default();
    let request = MigrationRequest {
        source_profile_id: source.clone(),
        target_profile_id: target.clone(),
        project_ids: vec!["p1".to_owned()],
    };
    let preview = preview_migration(&db, &store, OWNER, &request)
        .await?
        .ok_or("preview")?;
    let plan_id = preview.plan_id.clone().ok_or("plan_id")?;
    let Ok(result) = commit_migration(&db, &store, OWNER, &plan_id).await else {
        return Err("commit rejected".into());
    };

    let rollback_preview = preview_migration_rollback(&db, &store, OWNER, &result.migration_id)
        .await?
        .ok_or("rollback preview")?;
    assert!(rollback_preview.restorable);
    let rollback_plan = rollback_preview.plan_id.clone().ok_or("rollback plan")?;

    let Ok(rollback) = commit_migration_rollback(&db, &store, OWNER, &rollback_plan).await else {
        return Err("rollback rejected".into());
    };
    assert_eq!(rollback.status, "rolled_back");
    assert_eq!(rollback.restored_project_count, 1);

    let conn = db.connect()?;
    assert_eq!(binding(&conn, "p1").await?, source);
    assert_eq!(migration_status(&conn).await?, "rolled_back");
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn destructive_preview_never_allows_external_runtime() -> TestResult {
    let (db, path, source, _) = fixture().await?;
    let store = ActionPlanStore::default();
    let preview = preview_destructive_operation(
        &db,
        &store,
        OWNER,
        &source,
        &DestructivePreviewRequest {
            action: DestructiveAction::ResetEngineData,
        },
    )
    .await?
    .ok_or("preview")?;
    assert!(!preview.allowed);
    assert!(preview.blocker.is_some());
    assert!(preview.plan_id.is_none());
    assert_eq!(preview.affected.last().and_then(|item| item.count), Some(2));
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn destructive_commit_gates_then_defers_to_phase_14b() -> TestResult {
    let (db, path, _, _) = fixture().await?;
    let store = ActionPlanStore::default();
    insert_built_in(&db, "built-1").await?;

    let preview = preview_destructive_operation(
        &db,
        &store,
        OWNER,
        "built-1",
        &DestructivePreviewRequest {
            action: DestructiveAction::ResetEngineData,
        },
    )
    .await?
    .ok_or("preview")?;
    assert!(preview.allowed);
    let plan_id = preview.plan_id.clone().ok_or("plan_id")?;

    let Ok(result) = commit_destructive_operation(&db, &store, OWNER, &plan_id).await else {
        return Err("commit rejected".into());
    };
    assert_eq!(result.status, "deferred_to_phase_14b");
    assert!(result.revalidated);

    // No engine mutation is claimed; audit records the deferral, not success.
    let conn = db.connect()?;
    assert_eq!(audit_status(&conn).await?, "deferred_to_phase_14b");

    // The single-use plan is spent.
    assert!(
        commit_destructive_operation(&db, &store, OWNER, &plan_id)
            .await
            .is_err()
    );
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn destructive_commit_rejects_when_ownership_changes() -> TestResult {
    let (db, path, _, _) = fixture().await?;
    let store = ActionPlanStore::default();
    insert_built_in(&db, "built-1").await?;

    let preview = preview_destructive_operation(
        &db,
        &store,
        OWNER,
        "built-1",
        &DestructivePreviewRequest {
            action: DestructiveAction::RemoveBuiltInRuntime,
        },
    )
    .await?
    .ok_or("preview")?;
    let plan_id = preview.plan_id.clone().ok_or("plan_id")?;

    // Ownership drops to a conflict after preview → stale fingerprint.
    let conn = db.connect()?;
    conn.execute(
        "UPDATE runtime_profiles SET ownership_state = 'ownership_conflict', owner_token = NULL
         WHERE id = 'built-1'",
        (),
    )
    .await?;

    assert!(
        commit_destructive_operation(&db, &store, OWNER, &plan_id)
            .await
            .is_err()
    );
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
