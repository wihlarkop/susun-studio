use susun::{
    ContainerId, ContainerState, EngineContainerSummary, EngineImageSummary, ImageId, ImageRef,
    ObservedImageRef, ProjectInstanceId, ProjectName, ResourceName,
};
use turso::{Database, params};

use super::*;
use crate::db;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn unique_db_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "studio-artifact-test-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn fresh_db() -> TestResult<Database> {
    let path = unique_db_path();
    let db = db::open_database(path).await?;
    Ok(db)
}

async fn insert_project(db: &Database, id: &str, name: &str, path: &str) -> TestResult {
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms) VALUES (?1, ?2, ?3, 1)",
        params![id.to_owned(), name.to_owned(), path.to_owned()],
    )
    .await?;
    Ok(())
}

async fn insert_runtime_profile(
    db: &Database,
    id: &str,
    runtime_class: &str,
    is_selected: bool,
) -> TestResult {
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO runtime_profiles (
            id, provider_id, provider_runtime_key, display_name, product, platform,
            runtime_class, ownership_state, source,
            installation_state, process_state, connection_state,
            is_selected, observed_at_ms, created_at_ms, updated_at_ms
        ) VALUES (?1, ?2, ?2, ?3, 'podman', 'windows', ?4, 'external', 'provider_discovery',
            'installed', 'running', 'summarized', ?5, 1, 1, 1)",
        params![
            id.to_owned(),
            format!("key-{id}"),
            format!("Runtime {id}"),
            runtime_class.to_owned(),
            i64::from(is_selected),
        ],
    )
    .await?;
    Ok(())
}

fn sample_container(
    id: &str,
    name: &str,
    project_identity: Option<&str>,
) -> TestResult<EngineContainerSummary> {
    Ok(EngineContainerSummary {
        id: ContainerId::new(id)?,
        name: ResourceName::new(name)?,
        state: ContainerState::Running,
        health: None,
        image: ObservedImageRef::Reference(ImageRef::new("nginx:latest")),
        // The SDK's own `EngineContainerSummary` type structurally excludes
        // label values (only keys are ever present); an empty vec here is
        // enough to exercise the mapping without needing to name the
        // internal `LabelKey` type, which isn't re-exported by the facade.
        label_keys: Vec::new(),
        project_identity: project_identity.map(ProjectInstanceId::new).transpose()?,
        created_at_epoch_seconds: Some(1_700_000_000),
        writable_size_bytes: Some(1024),
        root_filesystem_size_bytes: Some(2048),
    })
}

#[test]
fn container_summary_row_excludes_label_values_and_carries_display_safe_fields() -> TestResult {
    let container = sample_container("c1", "web-1", None)?;
    let row = container_summary_row(&container, &[]);

    assert_eq!(row.id, "c1");
    assert_eq!(row.name, "web-1");
    assert_eq!(row.state, "running");
    assert_eq!(row.health, None);
    assert_eq!(row.image_reference.as_deref(), Some("nginx:latest"));
    assert!(row.label_keys.is_empty());
    assert_eq!(row.known_project_id, None);
    assert_eq!(row.writable_size_bytes, Some(1024));
    Ok(())
}

#[tokio::test]
async fn container_summary_row_associates_a_known_studio_project() -> TestResult {
    let db = fresh_db().await?;
    let name = ProjectName::new("demo");
    let path = "C:/projects/demo";
    let instance_id = ProjectInstanceId::derive(&name, path);
    insert_project(&db, "proj-1", "demo", path).await?;

    let known = known_projects(&db).await?;
    let container = sample_container("c1", "web-1", Some(instance_id.as_str()))?;
    let row = container_summary_row(&container, &known);

    assert_eq!(row.known_project_id.as_deref(), Some("proj-1"));
    Ok(())
}

#[tokio::test]
async fn container_summary_row_leaves_unmatched_project_identity_unassociated() -> TestResult {
    let db = fresh_db().await?;
    insert_project(&db, "proj-1", "demo", "C:/projects/demo").await?;

    let known = known_projects(&db).await?;
    // Some other engine-side project the daemon never imported.
    let container = sample_container("c1", "web-1", Some("deadbeefdeadbeef"))?;
    let row = container_summary_row(&container, &known);

    assert_eq!(row.known_project_id, None);
    Ok(())
}

#[test]
fn image_summary_row_excludes_label_values_and_carries_display_safe_fields() -> TestResult {
    let image = EngineImageSummary {
        id: ImageId::new("img1")?,
        references: vec![ImageRef::new("nginx:latest")],
        digests: vec!["sha256:deadbeef".to_owned()],
        label_keys: Vec::new(),
        created_at_epoch_seconds: Some(1_700_000_000),
        size_bytes: Some(4096),
        shared_size_bytes: Some(512),
        container_count: Some(2),
    };

    let row = image_summary_row(&image);

    assert_eq!(row.id, "img1");
    assert_eq!(row.references, vec!["nginx:latest".to_owned()]);
    assert_eq!(row.digests, vec!["sha256:deadbeef".to_owned()]);
    assert!(row.label_keys.is_empty());
    assert_eq!(row.size_bytes, Some(4096));
    assert_eq!(row.container_count, Some(2));
    Ok(())
}

#[tokio::test]
async fn runtime_context_passes_through_built_in_classification_unchanged() -> TestResult {
    let db = fresh_db().await?;
    insert_runtime_profile(&db, "profile-built-in", "built_in", true).await?;

    let context = runtime_context(&db, Some("profile-built-in")).await?;

    assert_eq!(
        context.runtime_profile_id.as_deref(),
        Some("profile-built-in")
    );
    assert_eq!(context.runtime_class.as_deref(), Some("built_in"));
    assert_eq!(context.is_selected, Some(true));
    Ok(())
}

/// External runtimes must never be presented as Studio-owned: the response
/// carries whatever classification Studio's own ownership model assigned,
/// never a fabricated "built_in".
#[tokio::test]
async fn runtime_context_never_upgrades_an_external_profile_to_built_in() -> TestResult {
    let db = fresh_db().await?;
    insert_runtime_profile(&db, "profile-external", "external_local", false).await?;

    let context = runtime_context(&db, Some("profile-external")).await?;

    assert_eq!(context.runtime_class.as_deref(), Some("external_local"));
    assert_ne!(context.runtime_class.as_deref(), Some("built_in"));
    Ok(())
}

#[tokio::test]
async fn runtime_context_reports_platform_default_when_no_profile_selected() -> TestResult {
    let db = fresh_db().await?;

    let context = runtime_context(&db, None).await?;

    assert_eq!(context.runtime_profile_id, None);
    assert_eq!(context.runtime_class, None);
    assert_eq!(context.display_name, None);
    Ok(())
}

/// A database fault must never look identical to "no such profile" — the
/// caller needs to tell a daemon fault apart from a normal missing-profile
/// state.
#[tokio::test]
async fn runtime_context_propagates_database_errors_instead_of_hiding_them() -> TestResult {
    let db = fresh_db().await?;
    let conn = db.connect()?;
    conn.execute("DROP TABLE runtime_profiles", ()).await?;

    let result = runtime_context(&db, Some("profile-1")).await;

    assert!(result.is_err());
    Ok(())
}
