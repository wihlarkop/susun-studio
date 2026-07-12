//! Focused persistence/reconciliation tests for Runtime Data 1. They use a
//! file-backed turso database (not `:memory:`, which is per-connection) so the
//! per-call `db.connect()` pattern the daemon uses is exercised faithfully.

use turso::{Connection, Database, params};

use super::command::{CommandKind, ProcessElevation, TrustedProgram};
use super::provider::{ObservedProfile, RESERVED_BUILT_IN_MACHINE, RuntimeClass, profile_id};
use super::windows_docker_desktop::WindowsDockerDesktopProvider;
use super::windows_podman::WindowsPodmanProvider;
use super::{AdoptOutcome, ForgetOutcome, RuntimeProfile, dimension, now_ms};
use crate::db;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn unique_db_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "studio-rt-test-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn fresh_db() -> TestResult<(Database, std::path::PathBuf)> {
    let path = unique_db_path();
    let db = db::open_database(path.clone()).await?;
    Ok((db, path))
}

fn observed(
    provider_id: &str,
    key: &str,
    name: &str,
    class: RuntimeClass,
    process_state: &str,
    provider_default: bool,
) -> ObservedProfile {
    let running = process_state == "running";
    ObservedProfile {
        id: profile_id(provider_id, key),
        provider_id: provider_id.to_owned(),
        provider_runtime_key: key.to_owned(),
        display_name: name.to_owned(),
        product: "podman".to_owned(),
        platform: "windows".to_owned(),
        runtime_class: class,
        installation: dimension("installed", Some("v1")),
        process: dimension(process_state, None),
        connection: dimension(
            if running {
                "summarized"
            } else {
                "not_applicable"
            },
            None,
        ),
        endpoint_summary: None,
        provider_default,
        observed_at_ms: now_ms(),
    }
}

async fn by_key(db: &Database, key: &str) -> TestResult<RuntimeProfile> {
    super::list_all_profiles(db)
        .await?
        .into_iter()
        .find(|profile| profile.provider_runtime_key == key)
        .ok_or_else(|| std::io::Error::other(format!("profile `{key}` is missing")).into())
}

async fn nullable_scalar(conn: &Connection, sql: &str) -> TestResult<Option<String>> {
    let mut rows = conn.query(sql, ()).await?;
    Ok(match rows.next().await? {
        Some(row) => row.get::<Option<String>>(0)?,
        None => None,
    })
}

async fn strings(conn: &Connection, sql: &str) -> TestResult<Vec<String>> {
    let mut rows = conn.query(sql, ()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(row.get::<String>(0)?);
    }
    Ok(out)
}

#[tokio::test]
async fn upgrade_preserves_selection_binding_and_repairs_multiselect() -> TestResult {
    let path = unique_db_path();
    let db = turso::Builder::new_local(path.to_string_lossy().as_ref())
        .build()
        .await?;
    let conn = db.connect()?;
    db::apply_migrations_upto(&conn, 10).await?;

    // Two 0010-shaped profiles, both marked selected (a latent multi-select
    // state the new invariant must repair), with p2 updated more recently.
    conn.execute(
        "INSERT INTO runtime_profiles (id, provider_id, provider_runtime_key, display_name,
            product, platform, installation_state, process_state, connection_state,
            is_selected, observed_at_ms, created_at_ms, updated_at_ms)
         VALUES ('p1','windows-podman','machine/a','A','podman','windows',
            'installed','running','summarized', 1, 100, 100, 200)",
        (),
    )
    .await?;
    conn.execute(
        "INSERT INTO runtime_profiles (id, provider_id, provider_runtime_key, display_name,
            product, platform, installation_state, process_state, connection_state,
            is_selected, observed_at_ms, created_at_ms, updated_at_ms)
         VALUES ('p2','windows-podman','machine/b','B','podman','windows',
            'installed','running','summarized', 1, 100, 100, 300)",
        (),
    )
    .await?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms, runtime_profile_id)
         VALUES ('proj1','P','/p',1,'p2')",
        (),
    )
    .await?;

    db::apply_pending_migrations(&conn).await?;

    // Exactly one selection survives, and it is the most recently updated one.
    assert_eq!(
        strings(
            &conn,
            "SELECT id FROM runtime_profiles WHERE is_selected = 1"
        )
        .await?,
        vec!["p2".to_owned()]
    );
    // The project binding is untouched.
    assert_eq!(
        nullable_scalar(
            &conn,
            "SELECT runtime_profile_id FROM projects WHERE id = 'proj1'"
        )
        .await?,
        Some("p2".to_owned())
    );
    // New identity/ownership columns carry the documented defaults.
    assert_eq!(
        strings(
            &conn,
            "SELECT runtime_class || '|' || ownership_state || '|' || source || '|' ||
                    availability_state FROM runtime_profiles WHERE id = 'p2'"
        )
        .await?,
        vec!["external_local|external|provider_discovery|available".to_owned()]
    );
    // The jobs attribution columns exist and accept values.
    conn.execute(
        "INSERT INTO jobs (id, kind, status, project_id, engine_id, request_json,
            created_at_ms, updated_at_ms, runtime_profile_id, runtime_class)
         VALUES ('j1','up','running','proj1','engine-docker-local','{}',1,1,'p2','external_local')",
        (),
    )
    .await?;

    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[tokio::test]
async fn recheck_is_observation_only_and_respects_user_selection() -> TestResult {
    let (db, path) = fresh_db().await?;

    // Import machine A as the provider default -> selected + external.
    super::persist_observed(
        &db,
        &[observed(
            "windows-podman",
            "machine/a",
            "A",
            RuntimeClass::ExternalLocal,
            "running",
            true,
        )],
    )
    .await?;
    // Import machine B (not default) -> external, not selected.
    super::persist_observed(
        &db,
        &[observed(
            "windows-podman",
            "machine/b",
            "B",
            RuntimeClass::ExternalLocal,
            "running",
            false,
        )],
    )
    .await?;

    // The user deliberately switches the selection to B.
    let b_id = profile_id("windows-podman", "machine/b");
    assert!(matches!(
        super::select_profile(&db, &b_id).await?,
        super::SelectOutcome::Selected
    ));

    // A rescan still reports A as the provider default and now sees it stopped.
    super::persist_observed(
        &db,
        &[observed(
            "windows-podman",
            "machine/a",
            "A",
            RuntimeClass::ExternalLocal,
            "stopped",
            true,
        )],
    )
    .await?;

    let a = by_key(&db, "machine/a").await?;
    let b = by_key(&db, "machine/b").await?;
    // Discovery did not steal the selection back to the provider default.
    assert!(b.is_selected);
    assert!(!a.is_selected);
    // Ownership stayed put; observation advanced.
    assert_eq!(a.ownership_state, "external");
    assert_eq!(a.process.state, "stopped");
    assert!(a.observation_revision >= 1);

    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[tokio::test]
async fn missing_only_after_authoritative_scan() -> TestResult {
    let (db, path) = fresh_db().await?;
    super::persist_observed(
        &db,
        &[observed(
            "windows-podman",
            "machine/a",
            "A",
            RuntimeClass::ExternalLocal,
            "running",
            true,
        )],
    )
    .await?;

    // Provider unavailable / scan failed -> availability must not change.
    super::reconcile_provider(&db, "windows-podman", &[], &None).await?;
    let a = by_key(&db, "machine/a").await?;
    assert_eq!(a.availability_state, "available");

    // Authoritative empty inventory -> genuinely missing, but not deleted and
    // still selected/visible.
    super::reconcile_provider(&db, "windows-podman", &[], &Some(Vec::new())).await?;
    let a = by_key(&db, "machine/a").await?;
    assert_eq!(a.availability_state, "missing");
    assert!(a.missing_since_ms.is_some());
    assert!(a.is_selected);

    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[tokio::test]
async fn reserved_name_conflicts_and_adoption_recovers() -> TestResult {
    let (db, path) = fresh_db().await?;
    let key = format!("machine/{RESERVED_BUILT_IN_MACHINE}");
    super::persist_observed(
        &db,
        &[observed(
            "windows-podman",
            &key,
            "Built-in",
            RuntimeClass::BuiltIn,
            "running",
            true,
        )],
    )
    .await?;

    let profile = by_key(&db, &key).await?;
    assert_eq!(profile.runtime_class, "built_in");
    assert_eq!(profile.ownership_state, "ownership_conflict");
    // A conflict is never silently adopted as the active runtime, and lifecycle
    // actions against it are blocked.
    assert!(!profile.is_selected);
    assert!(!profile.management.can_select);
    assert!(!profile.management.can_forget);
    assert!(profile.management.blocks_destructive_actions);
    assert!(profile.management.can_adopt);
    assert!(matches!(
        super::select_profile(&db, &profile.id).await?,
        super::SelectOutcome::Unavailable
    ));
    assert!(matches!(
        super::forget_profile(&db, &profile.id).await?,
        ForgetOutcome::NotExternal
    ));

    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms, runtime_profile_id)
         VALUES ('conflict-project','P','/p',1,?1)",
        params![profile.id.clone()],
    )
    .await?;
    assert!(matches!(
        super::engine_endpoint_for(&db, Some("conflict-project"))
            .await
            ?,
        super::EngineEndpointResolution::Unavailable { profile_id }
            if profile_id == profile.id
    ));
    assert_eq!(
        super::attribution_for(&db, Some("conflict-project")).await?,
        (Some(profile.id.clone()), Some("built_in".to_owned()))
    );

    // Deliberate recovery/adoption assigns Studio ownership + an owner token.
    assert!(matches!(
        super::adopt_profile(&db, &profile.id).await?,
        AdoptOutcome::Adopted
    ));
    let adopted = by_key(&db, &key).await?;
    assert_eq!(adopted.ownership_state, "studio_managed");
    assert_eq!(adopted.source, "studio_setup");
    assert!(!adopted.management.blocks_destructive_actions);

    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[tokio::test]
async fn forget_removes_metadata_but_keeps_binding() -> TestResult {
    let (db, path) = fresh_db().await?;
    super::persist_observed(
        &db,
        &[observed(
            "windows-podman",
            "machine/a",
            "A",
            RuntimeClass::ExternalLocal,
            "running",
            false,
        )],
    )
    .await?;
    let id = profile_id("windows-podman", "machine/a");

    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms, runtime_profile_id)
         VALUES ('proj','P','/p',1,?1)",
        params![id.clone()],
    )
    .await?;

    assert!(matches!(
        super::forget_profile(&db, &id).await?,
        ForgetOutcome::Forgotten
    ));
    // Studio metadata is gone.
    assert!(super::list_all_profiles(&db).await?.is_empty());
    // The loose project binding is intentionally preserved.
    assert_eq!(
        nullable_scalar(
            &conn,
            "SELECT runtime_profile_id FROM projects WHERE id = 'proj'"
        )
        .await?,
        Some(id),
    );

    let _ = std::fs::remove_file(&path);
    Ok(())
}

// --- Trusted command model (Runtime Security 1) ---------------------------
//
// These exercise `build_command` directly (not `command_for_action`, which is
// platform-gated) so command content is verified on any host, including the
// ubuntu CI runner where the Windows providers report `supported() == false`.

#[test]
fn podman_install_is_a_one_shot_elevated_package_manager_command() -> TestResult {
    let command = WindowsPodmanProvider
        .build_command("install", &[])
        .ok_or("podman install command should exist")?;
    assert_eq!(command.program, TrustedProgram::Winget);
    assert_eq!(command.kind, CommandKind::PackageManager);
    assert_eq!(command.elevation, ProcessElevation::OneShotOsMediated);
    assert!(
        command
            .args
            .iter()
            .any(|arg| arg.to_str() == Some("RedHat.Podman"))
    );
    Ok(())
}

#[test]
fn podman_lifecycle_without_a_selected_profile_has_no_command() {
    // start/stop/restart require a selected machine profile; with none, the
    // provider produces no command rather than guessing a target.
    assert!(WindowsPodmanProvider.build_command("start", &[]).is_none());
    assert!(WindowsPodmanProvider.build_command("stop", &[]).is_none());
    assert!(
        WindowsPodmanProvider
            .build_command("restart", &[])
            .is_none()
    );
}

#[test]
fn podman_init_runs_as_current_user_runtime_cli() -> TestResult {
    let command = WindowsPodmanProvider
        .build_command("init", &[])
        .ok_or("podman init command should exist")?;
    assert_eq!(command.program, TrustedProgram::Podman);
    assert_eq!(command.kind, CommandKind::RuntimeCli);
    assert_eq!(command.elevation, ProcessElevation::CurrentUser);
    Ok(())
}

#[test]
fn docker_desktop_install_is_a_one_shot_elevated_package_manager_command() -> TestResult {
    let command = WindowsDockerDesktopProvider
        .build_command("install", &[])
        .ok_or("docker desktop install command should exist")?;
    assert_eq!(command.program, TrustedProgram::Winget);
    assert_eq!(command.kind, CommandKind::PackageManager);
    assert_eq!(command.elevation, ProcessElevation::OneShotOsMediated);
    assert!(
        command
            .args
            .iter()
            .any(|arg| arg.to_str() == Some("Docker.DockerDesktop"))
    );
    Ok(())
}

#[test]
fn docker_desktop_lifecycle_is_a_current_user_os_config_command() -> TestResult {
    // Fixed daemon-owned PowerShell scripts, no interpolated user data, no
    // elevation: modelled as a current-user OS-config command.
    for action in ["start", "stop", "restart"] {
        let command = WindowsDockerDesktopProvider
            .build_command(action, &[])
            .ok_or("docker desktop lifecycle command should exist")?;
        assert_eq!(command.program, TrustedProgram::PowerShell);
        assert_eq!(command.kind, CommandKind::OsConfigTool);
        assert_eq!(command.elevation, ProcessElevation::CurrentUser);
    }
    Ok(())
}

#[test]
fn unknown_action_has_no_command() {
    assert!(
        WindowsPodmanProvider
            .build_command("frobnicate", &[])
            .is_none()
    );
    assert!(
        WindowsDockerDesktopProvider
            .build_command("frobnicate", &[])
            .is_none()
    );
}

#[test]
fn provider_built_arguments_preserve_order() -> TestResult {
    // The exact argv order is part of the command's meaning; a reorder would
    // change what winget installs. Assert the full, ordered vector.
    let command = WindowsPodmanProvider
        .build_command("install", &[])
        .ok_or("podman install command should exist")?;
    let args: Vec<&str> = command.args.iter().filter_map(|arg| arg.to_str()).collect();
    assert_eq!(
        args,
        vec![
            "install",
            "--id",
            "RedHat.Podman",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn native_non_utf8_arguments_are_preserved() {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    // A lone UTF-16 surrogate (0xD800) is a valid OS string but not valid
    // UTF-8/UTF-16 — exactly the case `String` would lose. The trusted command
    // model must carry it through unchanged.
    let native = OsString::from_wide(&[0x0075, 0xD800, 0x0076]); // 'u' <surrogate> 'v'
    assert!(native.to_str().is_none(), "fixture must not be valid UTF-8");

    let command = super::command::ExecutableCommand {
        program: super::command::TrustedProgram::Podman,
        args: vec![native.clone()],
        env_allowlist: Vec::new(),
        working_dir: None,
        timeout: std::time::Duration::from_secs(1),
        kind: super::command::CommandKind::RuntimeCli,
        elevation: super::command::ProcessElevation::CurrentUser,
        success_message: String::new(),
    };

    // Preserved byte-for-byte (compare the raw UTF-16 units).
    let round_tripped: Vec<u16> = command.args[0].encode_wide().collect();
    assert_eq!(round_tripped, vec![0x0075, 0xD800, 0x0076]);
}

#[test]
fn captured_output_is_redacted_and_truthfully_marked_truncated() {
    let output = super::CapturedOutput {
        bytes: b"token=private-value completed".to_vec(),
        truncated: true,
    };
    let displayed = super::redact_runtime_output(&output);
    assert!(!displayed.contains("private-value"));
    assert!(displayed.contains("token=<redacted>"));
    assert!(displayed.ends_with("... (truncated)"));
}

#[test]
fn provider_commands_inherit_no_environment_by_default() -> TestResult {
    for command in [
        WindowsPodmanProvider.build_command("install", &[]),
        WindowsDockerDesktopProvider.build_command("install", &[]),
    ] {
        let command = command.ok_or("expected provider install command")?;
        assert!(command.env_allowlist.is_empty());
    }
    Ok(())
}

// --- Endpoint redaction (Runtime Security 2) ------------------------------

#[test]
fn engine_endpoint_summary_is_redacted_not_a_raw_pipe_path() {
    // The endpoint summary is the only endpoint form that reaches DTOs, the DB,
    // logs, backups, and diagnostics. It must never carry a real pipe path.
    let summary = super::provider::EndpointSummary::windows_named_pipe();
    assert!(!summary.redacted.contains('\\'));
    assert!(summary.redacted.contains("<local-pipe>"));
    let json = summary.to_json_string().unwrap_or_default();
    assert!(!json.contains('\\'));
    assert!(json.contains("<local-pipe>"));
}
