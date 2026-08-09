use turso::{Database, params};

use super::{
    ContextChangeCommit, ContextChangeRejection, RuntimeBindingSource, RuntimeBindingState,
    SetPreferredOutcome, commit_preference_change, commit_project_binding_change,
    preview_preference_change, preview_project_binding_change, read_preference, resolve_global,
    resolve_project, set_preferred, summarize_global, summarize_projects,
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

#[tokio::test]
async fn global_context_preview_separates_pins_and_blocks_only_redirected_work() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "global", "Global", "available", "external").await?;
    insert_profile(&db, "next", "Next", "available", "external").await?;
    insert_profile(&db, "pinned", "Pinned", "available", "external").await?;
    assert_eq!(
        set_preferred(&db, Some("global")).await?,
        SetPreferredOutcome::Updated
    );
    pin_project(&db, "inherits", None).await?;
    pin_project(&db, "pinned-project", Some("pinned")).await?;

    let preview = preview_preference_change(&db, Some("next")).await?;
    assert_eq!(preview.inheriting_project_count, 1);
    assert_eq!(preview.explicitly_pinned_project_count, 1);
    assert!(preview.change_allowed);
    assert_eq!(preview.affected_active_jobs, 0);
    assert_eq!(preview.affected_active_watch_sessions, 0);

    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO jobs (
            id, kind, status, project_id, engine_id, request_json,
            runtime_profile_id, runtime_binding_source, created_at_ms, updated_at_ms
         ) VALUES ('pinned-job', 'up', 'running', 'pinned-project', 'engine', '{}',
                   'pinned', 'project_pin', 1, 1)",
        (),
    )
    .await?;
    assert!(
        preview_preference_change(&db, Some("next"))
            .await?
            .change_allowed
    );

    conn.execute(
        "INSERT INTO jobs (
            id, kind, status, project_id, engine_id, request_json,
            runtime_profile_id, runtime_binding_source, created_at_ms, updated_at_ms
         ) VALUES ('inherited-job', 'up', 'running', 'inherits', 'engine', '{}',
                   'global', 'global_preference', 1, 1)",
        (),
    )
    .await?;
    let blocked = preview_preference_change(&db, Some("next")).await?;
    assert!(!blocked.change_allowed);
    assert_eq!(blocked.reason_code.as_deref(), Some("active_work"));
    assert_eq!(blocked.affected_active_jobs, 1);

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn clearing_a_project_pin_previews_global_and_fails_closed_when_unavailable() -> TestResult {
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "missing", "Missing", "missing", "external").await?;
    insert_profile(&db, "pinned", "Pinned", "available", "external").await?;
    pin_project(&db, "project", Some("pinned")).await?;
    let conn = db.connect()?;
    conn.execute(
        "UPDATE runtime_policy SET preferred_profile_id = 'missing' WHERE singleton = 1",
        (),
    )
    .await?;

    let preview = preview_project_binding_change(&db, "project", None)
        .await?
        .ok_or("project preview")?;
    assert_eq!(preview.current.source, RuntimeBindingSource::ProjectPin);
    assert_eq!(
        preview.target.source,
        RuntimeBindingSource::GlobalPreference
    );
    assert_eq!(preview.target.state, RuntimeBindingState::Unavailable);
    assert!(!preview.change_allowed);
    assert_eq!(preview.reason_code.as_deref(), Some("target_unavailable"));

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn context_commits_recompute_fingerprints_and_leave_policy_and_pins_unchanged() -> TestResult
{
    let (db, path) = fresh_db().await?;
    insert_profile(&db, "global", "Global", "available", "external").await?;
    insert_profile(&db, "next", "Next", "available", "external").await?;
    insert_profile(&db, "pinned", "Pinned", "available", "external").await?;
    assert_eq!(
        set_preferred(&db, Some("global")).await?,
        SetPreferredOutcome::Updated
    );
    pin_project(&db, "project", Some("pinned")).await?;

    let global_preview = preview_preference_change(&db, Some("next")).await?;
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO jobs (
            id, kind, status, project_id, engine_id, request_json,
            runtime_profile_id, runtime_binding_source, created_at_ms, updated_at_ms
         ) VALUES ('running-global', 'up', 'running', 'project', 'engine', '{}',
                   'global', 'global_preference', 1, 1)",
        (),
    )
    .await?;
    assert_eq!(
        commit_preference_change(&db, Some("next"), &global_preview.impact_fingerprint).await?,
        ContextChangeCommit::Rejected(ContextChangeRejection::StalePreview)
    );
    assert_eq!(
        read_preference(&db).await?.preferred_profile_id.as_deref(),
        Some("global")
    );

    conn.execute("DELETE FROM jobs WHERE id = 'running-global'", ())
        .await?;
    let pin_preview = preview_project_binding_change(&db, "project", Some("next"))
        .await?
        .ok_or("project preview")?;
    conn.execute(
        "UPDATE projects SET runtime_profile_id = 'global' WHERE id = 'project'",
        (),
    )
    .await?;
    assert_eq!(
        commit_project_binding_change(
            &db,
            "project",
            Some("next"),
            &pin_preview.impact_fingerprint
        )
        .await?,
        Some(ContextChangeCommit::Rejected(
            ContextChangeRejection::StalePreview
        ))
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}
