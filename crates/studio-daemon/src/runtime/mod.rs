mod provider;
mod windows_podman;

use std::time::{SystemTime, UNIX_EPOCH};

use provider::{RuntimeCommand, RuntimeObservation, RuntimeProvider};
use serde::Serialize;
use susun::EngineEndpoint;
use tokio::time::{Duration, timeout};
use turso::{Database, params};
use windows_podman::WindowsPodmanProvider;

#[derive(Debug, Serialize)]
pub struct RuntimeStatus {
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

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDimension {
    pub state: String,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeAction {
    pub id: String,
    pub label: String,
    pub destructive: bool,
    pub enabled: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeProfile {
    pub id: String,
    pub provider_id: String,
    pub provider_runtime_key: String,
    pub display_name: String,
    pub product: String,
    pub platform: String,
    pub installation: RuntimeDimension,
    pub process: RuntimeDimension,
    pub connection: RuntimeDimension,
    pub endpoint_summary: Option<String>,
    pub is_selected: bool,
    pub observed_at_ms: i64,
    pub freshness: String,
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

pub async fn status(db: &Database) -> Result<RuntimeStatus, turso::Error> {
    let provider = WindowsPodmanProvider;
    let observation = provider.detect();
    persist_profiles(db, &observation.profiles).await?;
    let profiles = list_profiles(db).await?;
    let actions = planned_actions(&observation, &profiles);

    Ok(RuntimeStatus {
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
    })
}

pub async fn select_profile(db: &Database, profile_id: &str) -> Result<bool, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id FROM runtime_profiles WHERE id = ?1 LIMIT 1",
            params![profile_id.to_owned()],
        )
        .await?;
    if rows.next().await?.is_none() {
        return Ok(false);
    }

    conn.execute("UPDATE runtime_profiles SET is_selected = 0", ())
        .await?;
    conn.execute(
        "UPDATE runtime_profiles SET is_selected = 1, updated_at_ms = ?1 WHERE id = ?2",
        params![now_ms(), profile_id.to_owned()],
    )
    .await?;
    Ok(true)
}

pub async fn selected_engine_endpoint(
    db: &Database,
) -> Result<Option<EngineEndpoint>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT provider_id, provider_runtime_key, connection_state
             FROM runtime_profiles WHERE is_selected = 1 LIMIT 1",
            (),
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };

    let provider_id: String = row.get(0)?;
    let provider_runtime_key: String = row.get(1)?;
    let connection_state: String = row.get(2)?;
    if connection_state != "summarized" {
        return Ok(None);
    }

    Ok(match provider_id.as_str() {
        "windows-podman" => WindowsPodmanProvider::endpoint_for_runtime_key(&provider_runtime_key),
        _ => None,
    })
}

pub async fn action(db: &Database, action: &str) -> Result<RuntimeActionResult, turso::Error> {
    let provider = WindowsPodmanProvider;
    let observation = provider.detect();
    persist_profiles(db, &observation.profiles).await?;
    let profiles = list_profiles(db).await?;
    let actions = planned_actions(&observation, &profiles);
    let Some(action_state) = actions.iter().find(|candidate| candidate.id == action) else {
        return Ok(RuntimeActionResult {
            action: action.to_owned(),
            status: "not_executed".to_owned(),
            message: "Unknown runtime action.".to_owned(),
            next_steps: vec!["Use one of: install, start, stop, restart.".to_owned()],
        });
    };

    let mut next_steps = observation.remediation.clone();
    if next_steps.is_empty() {
        next_steps.push("Use the Runtime panel to review the current provider state.".to_owned());
    }
    if !action_state.enabled {
        return Ok(RuntimeActionResult {
            action: action.to_owned(),
            status: "not_executed".to_owned(),
            message: action_state.reason.clone(),
            next_steps,
        });
    }

    let command = if action == "install" {
        provider.command_for_profile_action("default", action)
    } else {
        profiles
            .iter()
            .find(|profile| profile.is_selected && profile.provider_id == provider.id())
            .and_then(|profile| {
                provider.command_for_profile_action(&profile.provider_runtime_key, action)
            })
    };

    let Some(command) = command else {
        return Ok(RuntimeActionResult {
            action: action.to_owned(),
            status: "not_executed".to_owned(),
            message: "No supported runtime profile is selected for this action.".to_owned(),
            next_steps,
        });
    };

    match run_command(&command).await {
        Ok(output) => {
            next_steps.push("Recheck runtime status to refresh observed state.".to_owned());
            Ok(RuntimeActionResult {
                action: action.to_owned(),
                status: "executed".to_owned(),
                message: if output.trim().is_empty() {
                    command.success_message
                } else {
                    format!("{} {}", command.success_message, output.trim())
                },
                next_steps,
            })
        }
        Err(error) => Ok(RuntimeActionResult {
            action: action.to_owned(),
            status: "failed".to_owned(),
            message: error,
            next_steps,
        }),
    }
}

pub fn logs() -> Vec<RuntimeLogLine> {
    let provider = WindowsPodmanProvider;
    let observation = provider.detect();
    let mut lines = vec![
        RuntimeLogLine {
            level: "info".to_owned(),
            message: format!(
                "{} status: {}",
                provider.display_name(),
                observation.installation.state
            ),
        },
        RuntimeLogLine {
            level: if provider.supported() { "info" } else { "warn" }.to_owned(),
            message: observation.summary,
        },
    ];

    for step in observation.remediation {
        lines.push(RuntimeLogLine {
            level: "info".to_owned(),
            message: step,
        });
    }

    lines
}

fn planned_actions(
    observation: &RuntimeObservation,
    profiles: &[RuntimeProfile],
) -> Vec<RuntimeAction> {
    let supported = cfg!(target_os = "windows");
    let installed = observation.installation.state == "installed";
    let missing = observation.installation.state == "not_installed";
    let selected = profiles
        .iter()
        .find(|profile| profile.is_selected && profile.provider_id == "windows-podman");
    let selected_running = selected.is_some_and(|profile| profile.process.state == "running");
    let selected_stopped = selected.is_some_and(|profile| profile.process.state == "stopped");
    let winget_check = supported
        .then(|| command_output("winget", &["--version"]))
        .transpose();
    let winget_available = matches!(winget_check, Ok(Some(_)));
    let winget_reason = match &winget_check {
        Ok(Some(_)) => "Install Podman with winget.",
        Ok(None) => "Runtime install is only available on Windows in this phase.",
        Err(_) => "winget is unavailable from the daemon session.",
    };

    vec![
        RuntimeAction {
            id: "install".to_owned(),
            label: "Install".to_owned(),
            destructive: false,
            enabled: supported && missing && winget_available,
            reason: if !supported {
                "Runtime install is only available on Windows in this phase."
            } else if !missing {
                "Podman is already installed or the install state is unknown."
            } else {
                winget_reason
            }
            .to_owned(),
        },
        RuntimeAction {
            id: "start".to_owned(),
            label: "Start".to_owned(),
            destructive: false,
            enabled: supported && installed && selected_stopped,
            reason: if selected_stopped {
                "Start the selected Podman machine."
            } else {
                "Select a stopped Podman machine first."
            }
            .to_owned(),
        },
        RuntimeAction {
            id: "stop".to_owned(),
            label: "Stop".to_owned(),
            destructive: false,
            enabled: supported && installed && selected_running,
            reason: if selected_running {
                "Stop the selected Podman machine."
            } else {
                "Select a running Podman machine first."
            }
            .to_owned(),
        },
        RuntimeAction {
            id: "restart".to_owned(),
            label: "Restart".to_owned(),
            destructive: false,
            enabled: supported && installed && selected_running,
            reason: if selected_running {
                "Restart the selected Podman machine."
            } else {
                "Select a running Podman machine first."
            }
            .to_owned(),
        },
    ]
}

pub(crate) fn dimension(state: &str, detail: Option<&str>) -> RuntimeDimension {
    RuntimeDimension {
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

async fn run_command(command: &RuntimeCommand) -> Result<String, String> {
    let output = timeout(
        Duration::from_secs(command.timeout_secs),
        tokio::process::Command::new(command.program)
            .args(&command.args)
            .output(),
    )
    .await
    .map_err(|_| format!("{} timed out", command.program))?
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("{} exited with {}", command.program, output.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn persist_profiles(db: &Database, profiles: &[RuntimeProfile]) -> Result<(), turso::Error> {
    let conn = db.connect()?;
    for profile in profiles {
        conn.execute(
            "INSERT INTO runtime_profiles (
                id, provider_id, provider_runtime_key, display_name, product, platform,
                installation_state, installation_detail, process_state, process_detail,
                connection_state, connection_detail, endpoint_summary, is_selected,
                observed_at_ms, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15, ?15)
            ON CONFLICT(provider_id, provider_runtime_key) DO UPDATE SET
                display_name = excluded.display_name,
                product = excluded.product,
                platform = excluded.platform,
                installation_state = excluded.installation_state,
                installation_detail = excluded.installation_detail,
                process_state = excluded.process_state,
                process_detail = excluded.process_detail,
                connection_state = excluded.connection_state,
                connection_detail = excluded.connection_detail,
                endpoint_summary = excluded.endpoint_summary,
                observed_at_ms = excluded.observed_at_ms,
                updated_at_ms = excluded.updated_at_ms",
            params![
                profile.id.clone(),
                profile.provider_id.clone(),
                profile.provider_runtime_key.clone(),
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
                i64::from(profile.is_selected),
                profile.observed_at_ms,
            ],
        )
        .await?;
    }
    Ok(())
}

async fn list_profiles(db: &Database) -> Result<Vec<RuntimeProfile>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, provider_id, provider_runtime_key, display_name, product, platform,
                    installation_state, installation_detail, process_state, process_detail,
                    connection_state, connection_detail, endpoint_summary, is_selected,
                    observed_at_ms
             FROM runtime_profiles ORDER BY is_selected DESC, display_name ASC",
            (),
        )
        .await?;
    let mut profiles = Vec::new();
    while let Some(row) = rows.next().await? {
        let is_selected: i64 = row.get(13)?;
        profiles.push(RuntimeProfile {
            id: row.get(0)?,
            provider_id: row.get(1)?,
            provider_runtime_key: row.get(2)?,
            display_name: row.get(3)?,
            product: row.get(4)?,
            platform: row.get(5)?,
            installation: RuntimeDimension {
                state: row.get(6)?,
                detail: row.get(7)?,
            },
            process: RuntimeDimension {
                state: row.get(8)?,
                detail: row.get(9)?,
            },
            connection: RuntimeDimension {
                state: row.get(10)?,
                detail: row.get(11)?,
            },
            endpoint_summary: row.get(12)?,
            is_selected: is_selected != 0,
            observed_at_ms: row.get(14)?,
            freshness: "fresh".to_owned(),
        });
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
