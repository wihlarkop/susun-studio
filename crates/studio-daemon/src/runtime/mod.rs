mod command;
mod endpoint_policy;
mod provider;
mod trusted_exec;
pub mod trusted_plans;
mod windows_docker_desktop;
mod windows_podman;

use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use command::TrustedProgram;
use command::{ExecutableCommand, ProcessElevation};
use provider::{ObservedProfile, RuntimeClass, RuntimeProvider};
use serde::Serialize;
use susun::EngineEndpoint;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::timeout,
};
use turso::{Connection, Database, params};
use windows_docker_desktop::WindowsDockerDesktopProvider;
use windows_podman::WindowsPodmanProvider;

pub use endpoint_policy::validate_engine_endpoint;
pub use provider::{
    ManagementCapabilities, RuntimeAction, RuntimeDimension, RuntimeError, RuntimeProfile,
};

/// Columns selected to hydrate a [`RuntimeProfile`]; the order matches
/// [`profile_from_row`]. Kept in one place so every read stays in sync.
const PROFILE_COLUMNS: &str =
    "id, provider_id, provider_runtime_key, display_name, product, platform,
    runtime_class, ownership_state, source,
    installation_state, installation_detail, process_state, process_detail,
    connection_state, connection_detail, endpoint_summary,
    availability_state, last_seen_at_ms, missing_since_ms,
    last_error_code, last_error_detail, last_error_at_ms,
    is_selected, observation_revision, observed_at_ms";

#[derive(Debug, Serialize)]
pub struct RuntimeStatus {
    pub providers: Vec<RuntimeProviderStatus>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeProviderStatus {
    pub provider_id: String,
    pub display_name: String,
    pub product: String,
    pub platform: String,
    pub supported: bool,
    pub installation: RuntimeDimension,
    pub process: RuntimeDimension,
    pub connection: RuntimeDimension,
    pub freshness: String,
    pub summary: String,
    pub remediation: Vec<String>,
    pub actions: Vec<RuntimeAction>,
    pub profiles: Vec<RuntimeProfile>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeActionResult {
    pub action: String,
    pub status: String,
    pub message: String,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeLogLine {
    pub level: String,
    pub message: String,
}

pub enum EngineEndpointResolution {
    Explicit(EngineEndpoint),
    PlatformDefault,
    Unavailable { profile_id: String },
}

/// Outcome of forgetting an external profile's Studio metadata.
pub enum ForgetOutcome {
    Forgotten,
    NotFound,
    /// Built-in runtime records require recovery or the dedicated teardown
    /// path; metadata-only forgetting is restricted to external profiles.
    NotExternal,
    /// Studio-managed built-ins are not forgettable through this path — they
    /// require the deliberate lifecycle/teardown flow, not metadata removal.
    StudioManaged,
}

pub enum SelectOutcome {
    Selected,
    NotFound,
    Unavailable,
}

/// Outcome of the guarded built-in adoption/recovery flow.
pub enum AdoptOutcome {
    Adopted,
    NotFound,
    NotBuiltIn,
    AlreadyManaged,
}

/// The full set of runtime providers Studio knows how to detect and manage.
/// Add a new provider implementation and register it here to extend Runtime
/// support to another platform or product.
fn registered_providers() -> Vec<Box<dyn RuntimeProvider>> {
    vec![
        Box::new(WindowsPodmanProvider),
        Box::new(WindowsDockerDesktopProvider),
    ]
}

fn find_provider(provider_id: &str) -> Option<Box<dyn RuntimeProvider>> {
    registered_providers()
        .into_iter()
        .find(|provider| provider.id() == provider_id)
}

pub async fn status(db: &Database) -> Result<RuntimeStatus, turso::Error> {
    let mut providers = Vec::new();
    for provider in registered_providers() {
        let observation = provider.detect();
        reconcile_provider(
            db,
            provider.id(),
            &observation.profiles,
            &observation.scanned_keys,
        )
        .await?;
        let profiles = list_profiles_for_provider(db, provider.id()).await?;
        let actions = provider.planned_actions(&observation, &profiles);
        providers.push(RuntimeProviderStatus {
            provider_id: provider.id().to_owned(),
            display_name: provider.display_name().to_owned(),
            product: provider.product().to_owned(),
            platform: provider.platform().to_owned(),
            supported: provider.supported(),
            installation: observation.installation,
            process: observation.process,
            connection: observation.connection,
            freshness: "fresh".to_owned(),
            summary: observation.summary,
            remediation: observation.remediation,
            actions,
            profiles,
        });
    }

    Ok(RuntimeStatus { providers })
}

pub async fn select_profile(
    db: &Database,
    profile_id: &str,
) -> Result<SelectOutcome, turso::Error> {
    let mut conn = db.connect()?;
    // Fully materialize the existence check before writing on the same
    // connection — turso silently drops a write issued while an earlier
    // read cursor is still open (see project memory on this quirk).
    let selectable = {
        let mut rows = conn
            .query(
                "SELECT runtime_class, ownership_state, availability_state
                 FROM runtime_profiles WHERE id = ?1 LIMIT 1",
                params![profile_id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let runtime_class: String = row.get(0)?;
                let ownership_state: String = row.get(1)?;
                let availability_state: String = row.get(2)?;
                ManagementCapabilities::derive(
                    &runtime_class,
                    &ownership_state,
                    &availability_state,
                )
                .can_select
            }
            None => return Ok(SelectOutcome::NotFound),
        }
    };
    if !selectable {
        return Ok(SelectOutcome::Unavailable);
    }

    let tx = conn.transaction().await?;
    tx.execute("UPDATE runtime_profiles SET is_selected = 0", ())
        .await?;
    // Selection is user metadata, so it advances updated_at_ms (not the
    // observation timeline). The single-selected partial unique index is
    // satisfied because every other row was just cleared above.
    tx.execute(
        "UPDATE runtime_profiles SET is_selected = 1, updated_at_ms = ?1 WHERE id = ?2",
        params![now_ms(), profile_id.to_owned()],
    )
    .await?;
    tx.commit().await?;
    Ok(SelectOutcome::Selected)
}

/// Forget an external profile: remove Studio's stored metadata for it. The row
/// is deleted (a still-present runtime is simply re-discovered as fresh and
/// external on the next scan), but project bindings are loose references and
/// are intentionally left pointing at the now-absent id so bound projects stay
/// visible rather than silently switching runtimes.
pub async fn forget_profile(
    db: &Database,
    profile_id: &str,
) -> Result<ForgetOutcome, turso::Error> {
    let mut conn = db.connect()?;
    let existing = {
        let mut rows = conn
            .query(
                "SELECT provider_id, provider_runtime_key, runtime_class, ownership_state
                 FROM runtime_profiles WHERE id = ?1 LIMIT 1",
                params![profile_id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let provider_id: String = row.get(0)?;
                let key: String = row.get(1)?;
                let runtime_class: String = row.get(2)?;
                let ownership_state: String = row.get(3)?;
                Some((provider_id, key, runtime_class, ownership_state))
            }
            None => None,
        }
    };
    let Some((provider_id, key, runtime_class, ownership_state)) = existing else {
        return Ok(ForgetOutcome::NotFound);
    };
    if runtime_class == "built_in" {
        return Ok(ForgetOutcome::NotExternal);
    }
    if ownership_state == "studio_managed" {
        return Ok(ForgetOutcome::StudioManaged);
    }

    let tx = conn.transaction().await?;
    tx.execute(
        "DELETE FROM runtime_profiles WHERE id = ?1",
        params![profile_id.to_owned()],
    )
    .await?;
    record_ownership_event(
        &tx,
        profile_id,
        &provider_id,
        &key,
        "forgotten",
        Some(&ownership_state),
        None,
        None,
        Some("external profile metadata forgotten by user"),
    )
    .await?;
    tx.commit().await?;
    Ok(ForgetOutcome::Forgotten)
}

/// Deliberately adopt a built-in runtime that Studio cannot yet prove it owns
/// (an ownership conflict or restored/unknown ownership after database loss).
/// This assigns `studio_managed` ownership, records a fresh opaque owner token,
/// and logs the transition — but never mutates the underlying machine.
pub async fn adopt_profile(db: &Database, profile_id: &str) -> Result<AdoptOutcome, turso::Error> {
    let mut conn = db.connect()?;
    let existing = {
        let mut rows = conn
            .query(
                "SELECT provider_id, provider_runtime_key, runtime_class, ownership_state
                 FROM runtime_profiles WHERE id = ?1 LIMIT 1",
                params![profile_id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let provider_id: String = row.get(0)?;
                let key: String = row.get(1)?;
                let runtime_class: String = row.get(2)?;
                let ownership_state: String = row.get(3)?;
                Some((provider_id, key, runtime_class, ownership_state))
            }
            None => None,
        }
    };
    let Some((provider_id, key, runtime_class, ownership_state)) = existing else {
        return Ok(AdoptOutcome::NotFound);
    };
    if runtime_class != "built_in" {
        return Ok(AdoptOutcome::NotBuiltIn);
    }
    if ownership_state == "studio_managed" {
        return Ok(AdoptOutcome::AlreadyManaged);
    }

    let owner_token = format!("own_{}", uuid::Uuid::new_v4().simple());
    let tx = conn.transaction().await?;
    tx.execute(
        "UPDATE runtime_profiles
         SET ownership_state = 'studio_managed', source = 'studio_setup',
             owner_token = ?1, updated_at_ms = ?2
         WHERE id = ?3",
        params![owner_token.clone(), now_ms(), profile_id.to_owned()],
    )
    .await?;
    record_ownership_event(
        &tx,
        profile_id,
        &provider_id,
        &key,
        "adopted",
        Some(&ownership_state),
        Some("studio_managed"),
        Some(&owner_token),
        Some("deliberate built-in adoption/recovery"),
    )
    .await?;
    tx.commit().await?;
    Ok(AdoptOutcome::Adopted)
}

/// The runtime profile Studio attributes a new job to: the project's own bound
/// profile when it is still present, otherwise the globally selected profile.
/// Returns `(profile_id, runtime_class)` so job records keep attribution even
/// after the profile later disappears.
pub async fn attribution_for(
    db: &Database,
    project_id: Option<&str>,
) -> Result<(Option<String>, Option<String>), turso::Error> {
    let conn = db.connect()?;

    if let Some(project_id) = project_id {
        let bound: Option<String> = {
            let mut rows = conn
                .query(
                    "SELECT runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
                    params![project_id.to_owned()],
                )
                .await?;
            match rows.next().await? {
                Some(row) => row.get(0)?,
                None => None,
            }
        };
        if let Some(profile_id) = bound {
            let class: Option<String> = {
                let mut rows = conn
                    .query(
                        "SELECT runtime_class FROM runtime_profiles
                         WHERE id = ?1 LIMIT 1",
                        params![profile_id.clone()],
                    )
                    .await?;
                match rows.next().await? {
                    Some(row) => Some(row.get(0)?),
                    None => None,
                }
            };
            return Ok((Some(profile_id), class));
        }
    }

    let mut rows = conn
        .query(
            "SELECT id, runtime_class FROM runtime_profiles
             WHERE is_selected = 1 LIMIT 1",
            (),
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok((Some(row.get(0)?), Some(row.get(1)?))),
        None => Ok((None, None)),
    }
}

/// Engine endpoint for a specific project: the project's own binding wins
/// (when the bound profile still exists, is present, and is connectable), then
/// the globally selected profile, then `None` (platform default).
pub async fn engine_endpoint_for(
    db: &Database,
    project_id: Option<&str>,
) -> Result<EngineEndpointResolution, turso::Error> {
    if let Some(project_id) = project_id {
        let conn = db.connect()?;
        let bound_profile_id: Option<String> = {
            let mut rows = conn
                .query(
                    "SELECT runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
                    params![project_id.to_owned()],
                )
                .await?;
            match rows.next().await? {
                Some(row) => row.get(0)?,
                None => None,
            }
        };
        if let Some(profile_id) = bound_profile_id {
            return Ok(match endpoint_for_profile(db, &profile_id).await? {
                Some(endpoint) => EngineEndpointResolution::Explicit(endpoint),
                None => EngineEndpointResolution::Unavailable { profile_id },
            });
        }
    }

    let conn = db.connect()?;
    let selected_profile_id: Option<String> = {
        let mut rows = conn
            .query(
                "SELECT id FROM runtime_profiles WHERE is_selected = 1 LIMIT 1",
                (),
            )
            .await?;
        match rows.next().await? {
            Some(row) => Some(row.get(0)?),
            None => None,
        }
    };
    let Some(profile_id) = selected_profile_id else {
        return Ok(EngineEndpointResolution::PlatformDefault);
    };
    Ok(match endpoint_for_profile(db, &profile_id).await? {
        Some(endpoint) => EngineEndpointResolution::Explicit(endpoint),
        None => EngineEndpointResolution::Unavailable { profile_id },
    })
}

async fn endpoint_for_profile(
    db: &Database,
    profile_id: &str,
) -> Result<Option<EngineEndpoint>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT provider_id, provider_runtime_key, runtime_class, ownership_state,
                    connection_state, availability_state
             FROM runtime_profiles WHERE id = ?1 LIMIT 1",
            params![profile_id.to_owned()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let provider_id: String = row.get(0)?;
    let provider_runtime_key: String = row.get(1)?;
    let runtime_class: String = row.get(2)?;
    let ownership_state: String = row.get(3)?;
    let connection_state: String = row.get(4)?;
    let availability_state: String = row.get(5)?;
    // A missing or unreachable bound profile falls through to the selected /
    // default endpoint. The binding row itself is left untouched so the project
    // still shows which runtime it prefers once that runtime returns.
    if connection_state != "summarized"
        || !ManagementCapabilities::derive(&runtime_class, &ownership_state, &availability_state)
            .can_select
    {
        return Ok(None);
    }
    Ok(find_provider(&provider_id)
        .and_then(|provider| provider.endpoint_for_runtime_key(&provider_runtime_key)))
}

pub fn logs() -> Vec<RuntimeLogLine> {
    let mut lines = Vec::new();
    for provider in registered_providers() {
        let observation = provider.detect();
        lines.push(RuntimeLogLine {
            level: "info".to_owned(),
            message: format!(
                "{} status: {}",
                provider.display_name(),
                observation.installation.state
            ),
        });
        lines.push(RuntimeLogLine {
            level: if provider.supported() { "info" } else { "warn" }.to_owned(),
            message: format!("{}: {}", provider.display_name(), observation.summary),
        });
        for step in observation.remediation {
            lines.push(RuntimeLogLine {
                level: "info".to_owned(),
                message: format!("{}: {step}", provider.display_name()),
            });
        }
    }

    lines
}

pub(crate) fn dimension(state: &str, detail: Option<&str>) -> provider::RuntimeDimension {
    provider::RuntimeDimension {
        state: state.to_owned(),
        detail: detail.map(str::to_owned),
    }
}

pub(crate) fn command_output(program: &str, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Runs exactly one trusted runtime command at its pre-approved elevation.
/// Current-user failures never retry with broader privileges. Commands marked
/// for one-shot OS-mediated elevation launch directly through the UAC path,
/// with no persistent privileged helper.
///
/// Before launching, the program is resolved to its trusted absolute path and
/// its identity re-verified (Authenticode or MSIX) against the daemon's policy —
/// so a swapped or untrusted binary is rejected at the last moment, and the
/// process is launched by verified absolute path, never a `PATH` lookup.
#[derive(Debug)]
enum CommandRunError {
    Cancelled,
    Failed(String),
}

pub async fn prepare_trusted_action(
    db: &Database,
    store: &trusted_plans::TrustedPlanStore,
    owner: &str,
    provider_id: &str,
    action: &str,
) -> Result<Result<trusted_plans::TrustedPlanPreview, RuntimeActionResult>, turso::Error> {
    let Some((action_state, command, _next_steps)) =
        resolve_trusted_action(db, provider_id, action).await?
    else {
        return Ok(Err(not_executed(
            action,
            "Runtime action is unavailable. Recheck provider state.",
        )));
    };
    if let Err(error) = trusted_exec::verify_trusted_program(command.program) {
        return Ok(Err(not_executed(action, &error.to_string())));
    }
    Ok(Ok(store.prepare(
        owner,
        trusted_plans::TrustedPlanMetadata {
            provider_id,
            action,
            label: &action_state.label,
            destructive: action_state.destructive,
            consequence: &action_state.reason,
        },
        command,
    )))
}

pub async fn execute_trusted_action(
    db: &Database,
    store: &trusted_plans::TrustedPlanStore,
    owner: &str,
    plan_id: &str,
) -> RuntimeActionResult {
    let claimed = match store.claim(plan_id, owner) {
        Ok(claimed) => claimed,
        Err(error) => return not_executed("trusted_plan", &error.to_string()),
    };
    let resolved = match resolve_trusted_action(db, &claimed.provider_id, &claimed.action).await {
        Ok(Some(value)) => value,
        Ok(None) => {
            store.finish(&claimed.plan_id, trusted_plans::TrustedPlanState::Failed);
            return not_executed(
                &claimed.action,
                "Provider state changed; prepare a new plan.",
            );
        }
        Err(_) => {
            store.finish(&claimed.plan_id, trusted_plans::TrustedPlanState::Failed);
            return not_executed(&claimed.action, "Provider revalidation failed.");
        }
    };
    if resolved.1 != claimed.command {
        store.finish(&claimed.plan_id, trusted_plans::TrustedPlanState::Failed);
        return not_executed(
            &claimed.action,
            "Approved command changed; prepare a new plan.",
        );
    }
    let mut next_steps = resolved.2;
    let result = match run_command(&claimed.command).await {
        Ok(output) => {
            next_steps.push("Recheck runtime status to refresh observed state.".to_owned());
            RuntimeActionResult {
                action: claimed.action,
                status: "executed".to_owned(),
                message: if output.trim().is_empty() {
                    claimed.command.success_message
                } else {
                    format!("{} {}", claimed.command.success_message, output.trim())
                },
                next_steps,
            }
        }
        Err(CommandRunError::Cancelled) => RuntimeActionResult {
            action: claimed.action,
            status: "cancelled".to_owned(),
            message: "The elevation request was cancelled. No fallback was attempted.".to_owned(),
            next_steps,
        },
        Err(CommandRunError::Failed(error)) => RuntimeActionResult {
            action: claimed.action,
            status: "failed".to_owned(),
            message: error,
            next_steps,
        },
    };
    let terminal_state = match result.status.as_str() {
        "executed" => trusted_plans::TrustedPlanState::Succeeded,
        "cancelled" => trusted_plans::TrustedPlanState::Cancelled,
        _ => trusted_plans::TrustedPlanState::Failed,
    };
    store.finish(&claimed.plan_id, terminal_state);
    result
}

pub fn cancel_trusted_action(
    store: &trusted_plans::TrustedPlanStore,
    owner: &str,
    plan_id: &str,
) -> RuntimeActionResult {
    match store.cancel(plan_id, owner) {
        Ok(()) => RuntimeActionResult {
            action: "trusted_plan".to_owned(),
            status: "cancelled".to_owned(),
            message: "The trusted runtime plan was cancelled.".to_owned(),
            next_steps: Vec::new(),
        },
        Err(error) => not_executed("trusted_plan", &error.to_string()),
    }
}

fn not_executed(action: &str, message: &str) -> RuntimeActionResult {
    RuntimeActionResult {
        action: action.to_owned(),
        status: "not_executed".to_owned(),
        message: message.to_owned(),
        next_steps: vec![
            "Prepare a fresh runtime plan after rechecking provider state.".to_owned(),
        ],
    }
}

async fn resolve_trusted_action(
    db: &Database,
    provider_id: &str,
    action: &str,
) -> Result<Option<(RuntimeAction, ExecutableCommand, Vec<String>)>, turso::Error> {
    let Some(provider) = find_provider(provider_id) else {
        return Ok(None);
    };
    let observation = provider.detect();
    reconcile_provider(
        db,
        provider_id,
        &observation.profiles,
        &observation.scanned_keys,
    )
    .await?;
    let profiles = list_profiles_for_provider(db, provider_id).await?;
    let actions = provider.planned_actions(&observation, &profiles);
    let Some(action_state) = actions.into_iter().find(|candidate| candidate.id == action) else {
        return Ok(None);
    };
    if !action_state.enabled {
        return Ok(None);
    }
    if profiles.iter().any(|profile| {
        profile.is_selected
            && profile.provider_id == provider_id
            && profile.management.blocks_destructive_actions
    }) {
        return Ok(None);
    }
    let Some(command) = provider.command_for_action(action, &profiles) else {
        return Ok(None);
    };
    let mut next_steps = observation.remediation;
    if next_steps.is_empty() {
        next_steps.push("Use the Runtime panel to review provider state.".to_owned());
    }
    Ok(Some((action_state, command, next_steps)))
}

async fn run_command(command: &ExecutableCommand) -> Result<String, CommandRunError> {
    let target = trusted_exec::verify_trusted_program(command.program)
        .map_err(|error| CommandRunError::Failed(error.to_string()))?;
    match command.elevation {
        ProcessElevation::CurrentUser => run_once(&target.path, command)
            .await
            .map_err(CommandRunError::Failed),
        ProcessElevation::OneShotOsMediated => run_elevated(&target.path, command).await,
    }
}

async fn run_once(
    program_path: &std::path::Path,
    command: &ExecutableCommand,
) -> Result<String, String> {
    let mut process = Command::new(program_path);
    process.args(&command.args);
    process.env_clear();
    forward_allowed_environment(&mut process, &command.env_allowlist);
    if let Some(dir) = &command.working_dir {
        process.current_dir(dir);
    }

    process
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = process
        .spawn()
        .map_err(|_| "trusted command could not start".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "trusted command stdout unavailable".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "trusted command stderr unavailable".to_owned())?;
    let stdout_task = tokio::spawn(read_bounded_output(stdout));
    let stderr_task = tokio::spawn(read_bounded_output(stderr));

    let status = match timeout(command.timeout, child.wait()).await {
        Ok(result) => result.map_err(|_| "trusted command wait failed".to_owned())?,
        Err(_) => {
            terminate_process_tree(child.id()).await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(
                "trusted command timed out; external installer state may be uncertain".to_owned(),
            );
        }
    };
    let stdout = stdout_task
        .await
        .map_err(|_| "trusted stdout capture failed".to_owned())??;
    let stderr = stderr_task
        .await
        .map_err(|_| "trusted stderr capture failed".to_owned())??;
    if !status.success() {
        let stderr = redact_runtime_output(&stderr);
        return Err(if stderr.is_empty() {
            format!("trusted command exited with {status}")
        } else {
            stderr
        });
    }
    Ok(redact_runtime_output(&stdout))
}

async fn terminate_process_tree(pid: Option<u32>) {
    #[cfg(windows)]
    if let Some(pid) = pid
        && let Ok(target) = trusted_exec::verify_trusted_program(TrustedProgram::Taskkill)
    {
        let mut command = Command::new(target.path);
        command
            .env_clear()
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let _ = timeout(std::time::Duration::from_secs(5), command.status()).await;
    }
    #[cfg(not(windows))]
    let _ = pid;
}

/// Forwards only explicitly allow-listed environment variables after the child
/// environment has been cleared. Runtime commands currently declare an empty
/// allowlist, so daemon credentials cannot be inherited.
fn forward_allowed_environment(process: &mut tokio::process::Command, allowlist: &[&str]) {
    for key in allowlist {
        if let Ok(value) = std::env::var(key) {
            process.env(key, value);
        }
    }
}

const MAX_CAPTURE_BYTES: usize = 64 * 1024;

struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_bounded_output(mut reader: impl AsyncRead + Unpin) -> Result<CapturedOutput, String> {
    let mut retained = Vec::new();
    let mut truncated = false;
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|_| "trusted output read failed".to_owned())?;
        if read == 0 {
            break;
        }
        let remaining = MAX_CAPTURE_BYTES.saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
        truncated |= read > remaining;
    }
    Ok(CapturedOutput {
        bytes: retained,
        truncated,
    })
}

fn redact_runtime_output(output: &CapturedOutput) -> String {
    let text = String::from_utf8_lossy(&output.bytes);
    let redacted = crate::logging::redact_sensitive_text(text.trim());
    if output.truncated {
        return format!("{redacted} ... (truncated)");
    }
    redacted
}

async fn run_elevated(
    program_path: &std::path::Path,
    command: &ExecutableCommand,
) -> Result<String, CommandRunError> {
    let path = program_path.to_path_buf();
    let args = command.args.clone();
    let working_dir = command.working_dir.clone();
    let command_timeout = command.timeout;
    let outcome = tokio::task::spawn_blocking(move || {
        studio_windows_trust::run_elevated_process(
            &path,
            &args,
            working_dir.as_deref(),
            command_timeout,
        )
    })
    .await
    .map_err(|_| CommandRunError::Failed("elevation worker failed".to_owned()))?
    .map_err(|error| CommandRunError::Failed(error.to_string()))?;
    match outcome {
        studio_windows_trust::ElevatedProcessOutcome::Exited(0) => Ok(String::new()),
        studio_windows_trust::ElevatedProcessOutcome::Exited(code) => Err(CommandRunError::Failed(
            format!("elevated command exited with code {code}"),
        )),
        studio_windows_trust::ElevatedProcessOutcome::Cancelled => Err(CommandRunError::Cancelled),
        studio_windows_trust::ElevatedProcessOutcome::TimedOut => Err(CommandRunError::Failed(
            "elevated command timed out; external installer state may be uncertain".to_owned(),
        )),
    }
}

/// Persist a provider's observation, then reconcile availability. Persisting
/// only ever rewrites observed health for known profiles — ownership, source,
/// owner token, selection, and the user-metadata timestamp are never touched by
/// a recheck. New profiles are imported with discovery-derived ownership.
async fn reconcile_provider(
    db: &Database,
    provider_id: &str,
    observed: &[ObservedProfile],
    scanned_keys: &Option<Vec<String>>,
) -> Result<(), turso::Error> {
    persist_observed(db, observed).await?;
    if let Some(present_keys) = scanned_keys {
        reconcile_missing(db, provider_id, present_keys).await?;
    }
    Ok(())
}

async fn persist_observed(db: &Database, observed: &[ObservedProfile]) -> Result<(), turso::Error> {
    let conn = db.connect()?;
    // Whether any profile is already globally selected — the initial-import
    // selection is only honoured when nothing is selected yet.
    let mut any_selected = {
        let mut rows = conn
            .query(
                "SELECT 1 FROM runtime_profiles WHERE is_selected = 1 LIMIT 1",
                (),
            )
            .await?;
        rows.next().await?.is_some()
    };

    for profile in observed {
        let already_exists = {
            let mut rows = conn
                .query(
                    "SELECT 1 FROM runtime_profiles
                     WHERE provider_id = ?1 AND provider_runtime_key = ?2 LIMIT 1",
                    params![
                        profile.provider_id.clone(),
                        profile.provider_runtime_key.clone()
                    ],
                )
                .await?;
            rows.next().await?.is_some()
        };
        // The observation timeline uses the provider's observed time; the
        // persistence timeline (created/updated) uses wall-clock now.
        let observed_at = profile.observed_at_ms;
        let now = now_ms();

        if already_exists {
            // Observation-only update: bump the observation timeline and clear
            // any missing flag, but leave every ownership/selection column and
            // updated_at_ms exactly as the user left them.
            conn.execute(
                "UPDATE runtime_profiles SET
                    display_name = ?1, product = ?2, platform = ?3,
                    installation_state = ?4, installation_detail = ?5,
                    process_state = ?6, process_detail = ?7,
                    connection_state = ?8, connection_detail = ?9,
                    endpoint_summary = ?10,
                    availability_state = 'available', missing_since_ms = NULL,
                    last_seen_at_ms = ?11, observed_at_ms = ?11,
                    observation_revision = observation_revision + 1
                 WHERE provider_id = ?12 AND provider_runtime_key = ?13",
                params![
                    profile.display_name.clone(),
                    profile.product.clone(),
                    profile.platform.clone(),
                    profile.installation.state.clone(),
                    profile.installation.detail.clone(),
                    profile.process.state.clone(),
                    profile.process.detail.clone(),
                    profile.connection.state.clone(),
                    profile.connection.detail.clone(),
                    profile.endpoint_summary.clone(),
                    observed_at,
                    profile.provider_id.clone(),
                    profile.provider_runtime_key.clone(),
                ],
            )
            .await?;
            continue;
        }

        // New profile. Discovery derives ownership from the runtime class: a
        // machine carrying the reserved built-in name that Studio did not
        // create is an ownership conflict, never an automatic adoption.
        let (ownership_state, event_kind) = match profile.runtime_class {
            RuntimeClass::BuiltIn => ("ownership_conflict", "conflict_detected"),
            _ => ("external", "imported"),
        };
        // Honour the provider's default only for the first import and never for
        // an ownership conflict — Studio must not silently adopt a reserved-name
        // machine it cannot prove it created by making it the active runtime.
        let select_now =
            profile.provider_default && !any_selected && ownership_state != "ownership_conflict";

        conn.execute(
            "INSERT INTO runtime_profiles (
                id, provider_id, provider_runtime_key, display_name, product, platform,
                runtime_class, ownership_state, source, owner_token,
                installation_state, installation_detail, process_state, process_detail,
                connection_state, connection_detail, endpoint_summary,
                availability_state, last_seen_at_ms, missing_since_ms,
                is_selected, observation_revision, observed_at_ms, created_at_ms, updated_at_ms
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, 'provider_discovery', NULL,
                ?9, ?10, ?11, ?12,
                ?13, ?14, ?15,
                'available', ?16, NULL,
                ?17, 0, ?16, ?18, ?18
            )",
            params![
                profile.id.clone(),
                profile.provider_id.clone(),
                profile.provider_runtime_key.clone(),
                profile.display_name.clone(),
                profile.product.clone(),
                profile.platform.clone(),
                profile.runtime_class.as_str().to_owned(),
                ownership_state.to_owned(),
                profile.installation.state.clone(),
                profile.installation.detail.clone(),
                profile.process.state.clone(),
                profile.process.detail.clone(),
                profile.connection.state.clone(),
                profile.connection.detail.clone(),
                profile.endpoint_summary.clone(),
                observed_at,
                i64::from(select_now),
                now,
            ],
        )
        .await?;
        if select_now {
            any_selected = true;
        }
        record_ownership_event(
            &conn,
            &profile.id,
            &profile.provider_id,
            &profile.provider_runtime_key,
            event_kind,
            None,
            Some(ownership_state),
            None,
            Some(if select_now {
                "initial import; selected as default"
            } else {
                "initial import"
            }),
        )
        .await?;
    }
    Ok(())
}

/// Mark profiles that a successful scan did not report as `missing` without
/// deleting them, so bound projects and the user's selection survive a runtime
/// disappearing. Runs only when the provider produced an authoritative
/// inventory; a failed/unavailable scan leaves availability untouched.
async fn reconcile_missing(
    db: &Database,
    provider_id: &str,
    present_keys: &[String],
) -> Result<(), turso::Error> {
    let conn = db.connect()?;
    let mut to_mark = Vec::new();
    {
        let mut rows = conn
            .query(
                "SELECT provider_runtime_key, availability_state
                 FROM runtime_profiles WHERE provider_id = ?1",
                params![provider_id.to_owned()],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let key: String = row.get(0)?;
            let availability_state: String = row.get(1)?;
            // The synthetic placeholder is re-emitted on every scan and never
            // represents a real user runtime, so it is exempt from missing.
            if key == provider::PLACEHOLDER_KEY {
                continue;
            }
            if availability_state != "missing" && !present_keys.iter().any(|k| k == &key) {
                to_mark.push(key);
            }
        }
    }

    let now = now_ms();
    for key in to_mark {
        conn.execute(
            "UPDATE runtime_profiles
             SET availability_state = 'missing', missing_since_ms = ?1
             WHERE provider_id = ?2 AND provider_runtime_key = ?3",
            params![now, provider_id.to_owned(), key],
        )
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn record_ownership_event(
    conn: &Connection,
    profile_id: &str,
    provider_id: &str,
    provider_runtime_key: &str,
    event_kind: &str,
    from_ownership_state: Option<&str>,
    to_ownership_state: Option<&str>,
    owner_token: Option<&str>,
    detail: Option<&str>,
) -> Result<(), turso::Error> {
    conn.execute(
        "INSERT INTO runtime_ownership_events (
            id, profile_id, provider_id, provider_runtime_key, event_kind,
            from_ownership_state, to_ownership_state, owner_token, detail, created_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            format!("rte_{}", uuid::Uuid::new_v4().simple()),
            profile_id.to_owned(),
            provider_id.to_owned(),
            provider_runtime_key.to_owned(),
            event_kind.to_owned(),
            from_ownership_state.map(str::to_owned),
            to_ownership_state.map(str::to_owned),
            owner_token.map(str::to_owned),
            detail.map(str::to_owned),
            now_ms(),
        ],
    )
    .await?;
    Ok(())
}

async fn list_profiles_for_provider(
    db: &Database,
    provider_id: &str,
) -> Result<Vec<RuntimeProfile>, turso::Error> {
    let conn = db.connect()?;
    let sql = format!(
        "SELECT {PROFILE_COLUMNS} FROM runtime_profiles
         WHERE provider_id = ?1 ORDER BY is_selected DESC, display_name ASC"
    );
    let mut rows = conn.query(&sql, params![provider_id.to_owned()]).await?;
    let mut profiles = Vec::new();
    while let Some(row) = rows.next().await? {
        profiles.push(profile_from_row(&row)?);
    }
    Ok(profiles)
}

fn profile_from_row(row: &turso::Row) -> Result<RuntimeProfile, turso::Error> {
    let runtime_class: String = row.get(6)?;
    let ownership_state: String = row.get(7)?;
    let source: String = row.get(8)?;
    let availability_state: String = row.get(16)?;
    let last_error_code: Option<String> = row.get(19)?;
    let last_error_detail: Option<String> = row.get(20)?;
    let last_error_at_ms: Option<i64> = row.get(21)?;
    let is_selected: i64 = row.get(22)?;
    let management =
        ManagementCapabilities::derive(&runtime_class, &ownership_state, &availability_state);
    let last_error = last_error_code.map(|code| RuntimeError {
        code,
        detail: last_error_detail,
        at_ms: last_error_at_ms.unwrap_or_default(),
    });

    Ok(RuntimeProfile {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        provider_runtime_key: row.get(2)?,
        display_name: row.get(3)?,
        product: row.get(4)?,
        platform: row.get(5)?,
        runtime_class,
        ownership_state,
        source,
        installation: RuntimeDimension {
            state: row.get(9)?,
            detail: row.get(10)?,
        },
        process: RuntimeDimension {
            state: row.get(11)?,
            detail: row.get(12)?,
        },
        connection: RuntimeDimension {
            state: row.get(13)?,
            detail: row.get(14)?,
        },
        endpoint_summary: row.get(15)?,
        availability_state,
        last_seen_at_ms: row.get(17)?,
        missing_since_ms: row.get(18)?,
        last_error,
        is_selected: is_selected != 0,
        observation_revision: row.get(23)?,
        observed_at_ms: row.get(24)?,
        management,
        freshness: "fresh".to_owned(),
    })
}

pub async fn list_all_profiles(db: &Database) -> Result<Vec<RuntimeProfile>, turso::Error> {
    let conn = db.connect()?;
    let sql = format!(
        "SELECT {PROFILE_COLUMNS} FROM runtime_profiles
         ORDER BY is_selected DESC, display_name ASC"
    );
    let mut rows = conn.query(&sql, ()).await?;
    let mut profiles = Vec::new();
    while let Some(row) = rows.next().await? {
        profiles.push(profile_from_row(&row)?);
    }
    Ok(profiles)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

pub(crate) fn stable_suffix(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests;
