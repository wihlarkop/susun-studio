use turso::{Database, params};

use super::{
    RuntimeBindingSource, RuntimeBindingState, SetPreferredOutcome, read_preference,
    resolve_global, resolve_project, set_preferred, summarize_global, summarize_projects,
};
use crate::{db, runtime};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn unique_db_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "studio-runtime-policy-test-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn fresh_db() -> TestResult<(Database, std::path::PathBuf)> {
    let path = unique_db_path();
    Ok((db::open_database(path.clone()).await?, path))
}

async fn insert_profile(
    db: &Database,
    id: &str,
    name: &str,
    availability: &str,
    ownership: &str,
) -> TestResult {
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO runtime_profiles (
            id, provider_id, provider_runtime_key, display_name, product, platform,
            runtime_class, ownership_state, source,
            installation_state, process_state, connection_state,
            availability_state, missing_since_ms, observation_revision, observed_at_ms, created_at_ms, updated_at_ms
        ) VALUES (
            ?1, 'windows-podman', ?5, ?2, 'podman', 'windows',
            'external_local', ?3, 'provider_discovery',
            'installed', 'running', 'summarized',
            ?4, ?6, 0, 1, 1, 1
        )",
        params![
            id.to_owned(),
            name.to_owned(),
            ownership.to_owned(),
            availability.to_owned(),
            format!("machine/{id}"),
            (availability == "missing").then_some(1_i64),
        ],
    )
    .await?;
    Ok(())
}

async fn pin_project(db: &Database, project_id: &str, profile_id: Option<&str>) -> TestResult {
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms, runtime_profile_id)
         VALUES (?1, ?2, ?3, 1, ?4)",
        params![
            project_id.to_owned(),
            project_id.to_owned(),
            format!("/{project_id}"),
            profile_id.map(str::to_owned),
        ],
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn project_pin_wins_over_global_preference() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "global", "Global", "available", "external").await?;
    insert_profile(&db, "pinned", "Pinned", "available", "external").await?;
    assert_eq!(
        set_preferred(&db, Some("global")).await?,
        SetPreferredOutcome::Updated
    );
    pin_project(&db, "project", Some("pinned")).await?;

    let resolved = resolve_project(&db, "project").await?;
    assert_eq!(resolved.summary().source, RuntimeBindingSource::ProjectPin);
    assert_eq!(resolved.summary().state, RuntimeBindingState::Ready);
    assert_eq!(resolved.summary().profile_id.as_deref(), Some("pinned"));
    assert!(resolved.endpoint().is_some());

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn batched_project_summaries_preserve_pins_and_share_global_preference() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "global", "Global", "available", "external").await?;
    insert_profile(&db, "pinned", "Pinned", "available", "external").await?;
    assert_eq!(
        set_preferred(&db, Some("global")).await?,
        SetPreferredOutcome::Updated
    );
    pin_project(&db, "pinned-project", Some("pinned")).await?;
    pin_project(&db, "global-project", None).await?;

    let summaries = summarize_projects(
        &db,
        &["pinned-project".to_owned(), "global-project".to_owned()],
    )
    .await?;

    assert_eq!(
        summaries["pinned-project"].source,
        RuntimeBindingSource::ProjectPin
    );
    assert_eq!(
        summaries["pinned-project"].profile_id.as_deref(),
        Some("pinned")
    );
    assert_eq!(
        summaries["global-project"].source,
        RuntimeBindingSource::GlobalPreference
    );
    assert_eq!(
        summaries["global-project"].profile_id.as_deref(),
        Some("global")
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn missing_project_pin_is_preserved_and_blocks_resolution() -> TestResult {
    let (db, path) = fresh_db().await?;
    pin_project(&db, "project", Some("missing-profile")).await?;

    let resolved = resolve_project(&db, "project").await?;
    assert_eq!(resolved.summary().source, RuntimeBindingSource::ProjectPin);
    assert_eq!(resolved.summary().state, RuntimeBindingState::Missing);
    assert_eq!(
        resolved.summary().profile_id.as_deref(),
        Some("missing-profile")
    );
    assert!(resolved.endpoint().is_none());

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn unavailable_project_pin_has_no_endpoint() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "pinned", "Pinned", "missing", "external").await?;
    pin_project(&db, "project", Some("pinned")).await?;

    let resolved = resolve_project(&db, "project").await?;
    assert_eq!(resolved.summary().state, RuntimeBindingState::Unavailable);
    assert!(resolved.endpoint().is_none());

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn unavailable_preferred_profile_blocks_global_resolution() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "global", "Global", "missing", "external").await?;
    let conn = db.connect()?;
    conn.execute(
        "UPDATE runtime_policy SET preferred_profile_id = 'global' WHERE singleton = 1",
        (),
    )
    .await?;

    let resolved = resolve_global(&db).await?;
    assert_eq!(
        resolved.summary().source,
        RuntimeBindingSource::GlobalPreference
    );
    assert_eq!(resolved.summary().state, RuntimeBindingState::Unavailable);
    assert!(resolved.endpoint().is_none());

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn no_preference_uses_explicit_platform_default_compatibility_state() -> TestResult {
    let (db, path) = fresh_db().await?;

    let preference = read_preference(&db).await?;
    assert_eq!(preference.preferred_profile_id, None);
    assert_eq!(
        preference.binding.source,
        RuntimeBindingSource::PlatformDefault
    );
    assert_eq!(preference.binding.state, RuntimeBindingState::Unconfigured);
    assert_eq!(summarize_global(&db).await?, preference.binding);
    assert!(resolve_global(&db).await?.endpoint().is_none());

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn set_preferred_rejects_unknown_unavailable_and_ownership_conflicted_profiles() -> TestResult
{
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "unavailable", "Unavailable", "missing", "external").await?;
    insert_profile(
        &db,
        "conflicted",
        "Conflicted",
        "available",
        "ownership_conflict",
    )
    .await?;

    assert_eq!(
        set_preferred(&db, Some("unknown")).await?,
        SetPreferredOutcome::NotFound
    );
    assert_eq!(
        set_preferred(&db, Some("unavailable")).await?,
        SetPreferredOutcome::Unavailable
    );
    assert_eq!(
        set_preferred(&db, Some("conflicted")).await?,
        SetPreferredOutcome::Unavailable
    );
    assert_eq!(read_preference(&db).await?.preferred_profile_id, None);

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn forgetting_preferred_profile_keeps_the_policy_reference() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "preferred", "Preferred", "available", "external").await?;
    assert_eq!(
        set_preferred(&db, Some("preferred")).await?,
        SetPreferredOutcome::Updated
    );

    assert!(matches!(
        runtime::forget_profile(&db, "preferred").await?,
        runtime::ForgetOutcome::Forgotten
    ));
    assert_eq!(
        read_preference(&db).await?.preferred_profile_id.as_deref(),
        Some("preferred")
    );
    assert_eq!(
        summarize_global(&db).await?.state,
        RuntimeBindingState::Missing
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn recheck_does_not_change_preference() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "preferred", "Preferred", "available", "external").await?;
    assert_eq!(
        set_preferred(&db, Some("preferred")).await?,
        SetPreferredOutcome::Updated
    );

    runtime::persist_observed(
        &db,
        &[runtime::provider::ObservedProfile {
            id: "discovered".to_owned(),
            provider_id: "windows-podman".to_owned(),
            provider_runtime_key: "machine/discovered".to_owned(),
            display_name: "Discovered".to_owned(),
            product: "podman".to_owned(),
            platform: "windows".to_owned(),
            runtime_class: runtime::provider::RuntimeClass::ExternalLocal,
            installation: runtime::dimension("installed", None),
            process: runtime::dimension("running", None),
            connection: runtime::dimension("summarized", None),
            endpoint_summary: None,
            observed_at_ms: runtime::now_ms(),
        }],
    )
    .await?;

    assert_eq!(
        read_preference(&db).await?.preferred_profile_id.as_deref(),
        Some("preferred")
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn setup_claim_writes_ownership_and_preference_together_after_proof() -> TestResult {
    let (db, path) = fresh_db().await?;
    let key = format!("machine/{}", runtime::provider::RESERVED_BUILT_IN_MACHINE);
    let id = runtime::provider::profile_id("windows-podman", &key);
    runtime::persist_observed(
        &db,
        &[runtime::provider::ObservedProfile {
            id: id.clone(),
            provider_id: "windows-podman".to_owned(),
            provider_runtime_key: key.clone(),
            display_name: "Susun Runtime".to_owned(),
            product: "podman".to_owned(),
            platform: "windows".to_owned(),
            runtime_class: runtime::provider::RuntimeClass::BuiltIn,
            installation: runtime::dimension("installed", None),
            process: runtime::dimension("running", None),
            connection: runtime::dimension("summarized", None),
            endpoint_summary: None,
            observed_at_ms: runtime::now_ms(),
        }],
    )
    .await?;

    assert!(runtime::claim_setup_profile(&db, "windows-podman", &key, &id).await?);
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT ownership_state FROM runtime_profiles WHERE id = ?1",
            params![id.clone()],
        )
        .await?;
    assert_eq!(
        rows.next().await?.ok_or("profile")?.get::<String>(0)?,
        "studio_managed"
    );
    assert_eq!(
        read_preference(&db).await?.preferred_profile_id.as_deref(),
        Some(id.as_str())
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}
