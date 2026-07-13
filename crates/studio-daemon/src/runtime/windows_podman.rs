use serde::Deserialize;
use susun::EngineEndpoint;

use super::{
    RuntimeProfile,
    command::{
        CommandKind, ExecutableCommand, ProcessElevation, SoftwareProvenance, TrustedProgram,
    },
    command_output, dimension, now_ms,
    provider::{
        EndpointSummary, ObservedProfile, PLACEHOLDER_KEY, RESERVED_BUILT_IN_MACHINE,
        RuntimeAction, RuntimeClass, RuntimeObservation, RuntimeProvider, RuntimeRecoveryAction,
        RuntimeRecoveryPlan, RuntimeResourceMetric, RuntimeResourceSnapshot, RuntimeResourceText,
        RuntimeResourceUpdate, RuntimeResourceUpdateCapabilities, RuntimeResourceUpdateCapability,
        profile_id,
    },
    trusted_exec, trusted_read_output,
};
use std::time::Duration;

pub struct WindowsPodmanProvider;

impl RuntimeProvider for WindowsPodmanProvider {
    fn id(&self) -> &'static str {
        "windows-podman"
    }

    fn display_name(&self) -> &'static str {
        "Podman on Windows"
    }

    fn product(&self) -> &'static str {
        "podman"
    }

    fn platform(&self) -> &'static str {
        "windows"
    }

    fn supported(&self) -> bool {
        cfg!(target_os = "windows")
    }

    fn detect(&self) -> RuntimeObservation {
        if !self.supported() {
            return RuntimeObservation {
                installation: dimension(
                    "not_applicable",
                    Some("Windows is the first Phase 13 target."),
                ),
                process: dimension("not_applicable", None),
                connection: dimension("not_applicable", None),
                summary: "Managed runtime setup is only enabled on Windows in this phase."
                    .to_owned(),
                remediation: vec![
                    "Continue using an existing Docker-compatible engine on this platform."
                        .to_owned(),
                    "Windows Podman guided setup is the first managed runtime implementation target."
                        .to_owned(),
                ],
                profiles: vec![self.placeholder_profile(
                    "not_applicable",
                    Some("Windows is the first Phase 13 target."),
                    "not_applicable",
                    None,
                    "not_applicable",
                    None,
                )],
                // Non-Windows: the provider does not run, so it cannot say
                // whether any previously-imported machine is still present.
                scanned_keys: None,
            };
        }

        match command_output("podman", &["--version"]) {
            Ok(version) => self.detect_installed(version.trim()),
            Err(error) => self.detect_missing(error.trim()),
        }
    }

    fn planned_actions(
        &self,
        observation: &RuntimeObservation,
        profiles: &[RuntimeProfile],
    ) -> Vec<RuntimeAction> {
        let supported = self.supported();
        let installed = observation.installation.state == "installed";
        let missing = observation.installation.state == "not_installed";
        let built_in = profiles.iter().find(|profile| {
            profile.provider_id == self.id()
                && profile.provider_runtime_key == format!("machine/{RESERVED_BUILT_IN_MACHINE}")
        });
        let selected = profiles
            .iter()
            .find(|profile| profile.is_selected && profile.provider_id == self.id());
        let selected_managed = selected.is_some_and(|profile| {
            profile.runtime_class == "built_in" && profile.ownership_state == "studio_managed"
        });
        let selected_running =
            selected_managed && selected.is_some_and(|profile| profile.process.state == "running");
        let selected_stopped =
            selected_managed && selected.is_some_and(|profile| profile.process.state == "stopped");
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
                id: "setup".to_owned(),
                label: "Set up Susun Runtime".to_owned(),
                destructive: false,
                enabled: supported && installed && built_in.is_none(),
                reason: if built_in.is_some() {
                    "The reserved Susun Runtime name already exists. Studio will not adopt it."
                } else if !installed {
                    "Install Podman first."
                } else {
                    "Create the dedicated Susun Runtime, powered by Podman."
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
                    "Select a stopped Studio-managed Susun Runtime first."
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
                    "Select a running Studio-managed Susun Runtime first."
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
                    "Select a running Studio-managed Susun Runtime first."
                }
                .to_owned(),
            },
        ]
    }

    fn command_for_action(
        &self,
        action: &str,
        profiles: &[RuntimeProfile],
    ) -> Option<ExecutableCommand> {
        if !self.supported() {
            return None;
        }
        self.build_command(action, profiles)
    }

    fn endpoint_for_runtime_key(&self, provider_runtime_key: &str) -> Option<EngineEndpoint> {
        let machine_name = provider_runtime_key.strip_prefix("machine/")?;
        let pipe = machine_pipe_from_inspect(machine_name)
            .unwrap_or_else(|| format!(r"\\.\pipe\podman-machine-{machine_name}"));
        Some(EngineEndpoint::WindowsNamedPipe(pipe.into()))
    }

    fn resource_snapshot(&self, profile: &RuntimeProfile) -> RuntimeResourceSnapshot {
        let machine_name = profile
            .provider_runtime_key
            .strip_prefix("machine/")
            .unwrap_or_default();
        let machine = trusted_exec::verify_trusted_program(TrustedProgram::Podman)
            .ok()
            .and_then(|target| {
                trusted_read_output(&target.path, &["machine", "list", "--format", "json"]).ok()
            })
            .and_then(|output| parse_machines(&output).ok())
            .and_then(|machines| {
                machines
                    .into_iter()
                    .find(|machine| machine.name.as_deref() == Some(machine_name))
            });
        podman_resource_snapshot(profile, machine.as_ref())
    }

    fn command_for_resource_update(
        &self,
        profile: &RuntimeProfile,
        update: RuntimeResourceUpdate,
    ) -> Option<ExecutableCommand> {
        if profile.runtime_class != "built_in"
            || profile.ownership_state != "studio_managed"
            || profile.availability_state != "available"
        {
            return None;
        }
        let machine_name = profile.provider_runtime_key.strip_prefix("machine/")?;
        if machine_name != RESERVED_BUILT_IN_MACHINE {
            return None;
        }
        let machine = trusted_machine(machine_name)?;
        podman_resource_update_command(profile, update, &machine)
    }

    fn recovery_plan(
        &self,
        profile: &RuntimeProfile,
        action: RuntimeRecoveryAction,
    ) -> Option<RuntimeRecoveryPlan> {
        if profile.runtime_class != "built_in"
            || profile.ownership_state != "studio_managed"
            || profile.availability_state != "available"
            || profile.provider_runtime_key != format!("machine/{RESERVED_BUILT_IN_MACHINE}")
        {
            return None;
        }
        let machine = trusted_machine(RESERVED_BUILT_IN_MACHINE)?;
        podman_recovery_plan(profile, action, &machine)
    }
}

fn podman_machine_command(args: Vec<std::ffi::OsString>, success: &str) -> ExecutableCommand {
    ExecutableCommand {
        program: TrustedProgram::Podman,
        args,
        env_allowlist: Vec::new(),
        working_dir: None,
        timeout: Duration::from_secs(10 * 60),
        kind: CommandKind::RuntimeCli,
        elevation: ProcessElevation::CurrentUser,
        software_provenance: None,
        success_message: success.to_owned(),
    }
}

fn podman_recovery_plan(
    profile: &RuntimeProfile,
    action: RuntimeRecoveryAction,
    machine: &PodmanMachine,
) -> Option<RuntimeRecoveryPlan> {
    if profile.runtime_class != "built_in"
        || profile.ownership_state != "studio_managed"
        || machine.name.as_deref() != Some(RESERVED_BUILT_IN_MACHINE)
    {
        return None;
    }
    let name = RESERVED_BUILT_IN_MACHINE;
    let plan = match action {
        RuntimeRecoveryAction::Repair => {
            let mut commands = Vec::new();
            if machine.running.unwrap_or(false) {
                commands.push(podman_machine_command(
                    vec!["machine".into(), "stop".into(), name.into()],
                    "Susun Runtime stopped for repair.",
                ));
            }
            commands.push(podman_machine_command(
                vec!["machine".into(), "start".into(), name.into()],
                "Susun Runtime started.",
            ));
            RuntimeRecoveryPlan {
                commands,
                command_kind: "provider_repair",
                success_message: "Susun Runtime lifecycle repair completed.",
                next_steps: vec!["Recheck project connectivity before resuming work.".to_owned()],
            }
        }
        RuntimeRecoveryAction::Reset => RuntimeRecoveryPlan {
            commands: vec![
                podman_machine_command(
                    vec!["machine".into(), "rm".into(), "--force".into(), name.into()],
                    "Previous Susun Runtime removed.",
                ),
                podman_machine_command(
                    vec!["machine".into(), "init".into(), name.into()],
                    "Fresh Susun Runtime created.",
                ),
            ],
            command_kind: "provider_reset",
            success_message: "Susun Runtime engine data was reset.",
            next_steps: vec!["Start Susun Runtime, then recreate project resources.".to_owned()],
        },
        RuntimeRecoveryAction::Remove => RuntimeRecoveryPlan {
            commands: vec![podman_machine_command(
                vec!["machine".into(), "rm".into(), "--force".into(), name.into()],
                "Susun Runtime removed.",
            )],
            command_kind: "provider_remove",
            success_message: "Susun Runtime was removed.",
            next_steps: vec![
                "Bound projects remain unavailable until they are reassigned or Susun Runtime is set up again."
                    .to_owned(),
            ],
        },
    };
    Some(plan)
}

fn podman_resource_update_command(
    profile: &RuntimeProfile,
    update: RuntimeResourceUpdate,
    machine: &PodmanMachine,
) -> Option<ExecutableCommand> {
    if profile.runtime_class != "built_in"
        || profile.ownership_state != "studio_managed"
        || profile.availability_state != "available"
        || machine.name.as_deref() != Some(RESERVED_BUILT_IN_MACHINE)
        || machine.vm_type.as_deref() != Some("wsl")
    {
        return None;
    }
    let RuntimeResourceUpdate::NetworkUserMode(enabled) = update;
    Some(ExecutableCommand {
        program: TrustedProgram::Podman,
        args: vec![
            "machine".into(),
            "set".into(),
            format!("--user-mode-networking={enabled}").into(),
            RESERVED_BUILT_IN_MACHINE.into(),
        ],
        env_allowlist: Vec::new(),
        working_dir: None,
        timeout: Duration::from_secs(5 * 60),
        kind: CommandKind::RuntimeCli,
        elevation: ProcessElevation::CurrentUser,
        software_provenance: None,
        success_message: "Susun Runtime network mode updated.".to_owned(),
    })
}

impl WindowsPodmanProvider {
    /// Builds the trusted command for an action, independent of the platform
    /// gate in [`command_for_action`]. Kept separate so it is unit-testable on
    /// any host, and so command content lives entirely in trusted provider code.
    pub(crate) fn build_command(
        &self,
        action: &str,
        profiles: &[RuntimeProfile],
    ) -> Option<ExecutableCommand> {
        match action {
            "install" => Some(ExecutableCommand {
                program: TrustedProgram::Winget,
                args: vec![
                    "install".into(),
                    "--id".into(),
                    "RedHat.Podman".into(),
                    "--exact".into(),
                    "--source".into(),
                    "winget".into(),
                    "--accept-package-agreements".into(),
                    "--accept-source-agreements".into(),
                    "--disable-interactivity".into(),
                ],
                env_allowlist: Vec::new(),
                working_dir: None,
                timeout: Duration::from_secs(30 * 60),
                kind: CommandKind::PackageManager,
                elevation: ProcessElevation::OneShotOsMediated,
                software_provenance: Some(SoftwareProvenance {
                    package_id: "RedHat.Podman",
                    source: "winget",
                    source_url: "https://cdn.winget.microsoft.com/cache",
                    source_identifier: "Microsoft.Winget.Source_8wekyb3d8bbwe",
                    expected_publisher: "Red Hat, Inc.",
                    version_intent: "Latest version published by the pinned source",
                    restart_impact: "No automatic restart; runtime setup is required after install",
                }),
                success_message: "Podman install command finished.".to_owned(),
            }),
            "setup" => Some(ExecutableCommand {
                program: TrustedProgram::Podman,
                args: vec![
                    "machine".into(),
                    "init".into(),
                    RESERVED_BUILT_IN_MACHINE.into(),
                ],
                env_allowlist: Vec::new(),
                working_dir: None,
                timeout: Duration::from_secs(10 * 60),
                kind: CommandKind::RuntimeCli,
                elevation: ProcessElevation::CurrentUser,
                software_provenance: None,
                success_message: "Susun Runtime created. Use Start to bring it online.".to_owned(),
            }),
            "start" | "stop" | "restart" => {
                let profile = profiles.iter().find(|profile| {
                    profile.is_selected
                        && profile.provider_id == self.id()
                        && profile.runtime_class == "built_in"
                        && profile.ownership_state == "studio_managed"
                })?;
                let machine = profile.provider_runtime_key.strip_prefix("machine/")?;
                Some(ExecutableCommand {
                    program: TrustedProgram::Podman,
                    args: vec!["machine".into(), action.into(), machine.into()],
                    env_allowlist: Vec::new(),
                    working_dir: None,
                    timeout: Duration::from_secs(5 * 60),
                    kind: CommandKind::RuntimeCli,
                    elevation: ProcessElevation::CurrentUser,
                    software_provenance: None,
                    success_message: format!("Podman machine {action} command finished."),
                })
            }
            _ => None,
        }
    }

    fn detect_installed(&self, version: &str) -> RuntimeObservation {
        let machine_profiles = self.machine_profiles(version);
        if machine_profiles.is_empty() {
            return RuntimeObservation {
                installation: dimension("installed", Some(version)),
                process: dimension("not_created", Some("No Podman machine was detected.")),
                connection: dimension("not_applicable", None),
                summary: "Podman is installed, but no machine was detected.".to_owned(),
                remediation: vec![
                    "Next: click Initialize to create a Podman machine.".to_owned(),
                    "Manual setup for now: podman machine init && podman machine start".to_owned(),
                ],
                profiles: vec![self.placeholder_profile(
                    "installed",
                    Some(version),
                    "not_created",
                    Some("No Podman machine was detected."),
                    "not_applicable",
                    None,
                )],
                // Authoritative empty inventory: any previously-seen machine is
                // genuinely gone (removed outside Studio), not just unobservable.
                scanned_keys: Some(Vec::new()),
            };
        }
        let scanned_keys = machine_profiles
            .iter()
            .map(|profile| profile.provider_runtime_key.clone())
            .collect::<Vec<_>>();

        let running = machine_profiles
            .iter()
            .any(|profile| profile.process.state == "running");
        RuntimeObservation {
            installation: dimension("installed", Some(version)),
            process: dimension(
                if running { "running" } else { "stopped" },
                Some("Observed with podman machine list."),
            ),
            connection: dimension(
                if running { "summarized" } else { "not_applicable" },
                Some(if running {
                    "Safe endpoint summary is available."
                } else {
                    "Machine is stopped."
                }),
            ),
            summary: if running {
                "Podman machine detected and running. Safe endpoint summary is ready.".to_owned()
            } else {
                "Podman machine detected but not running.".to_owned()
            },
            remediation: vec![
                "Next: use the selected profile's endpoint summary to connect through the engine provider model."
                    .to_owned(),
                "Then: prepare trusted start/stop plans for machine lifecycle.".to_owned(),
            ],
            profiles: machine_profiles,
            scanned_keys: Some(scanned_keys),
        }
    }

    fn detect_missing(&self, error: &str) -> RuntimeObservation {
        let winget = command_output("winget", &["--version"]);
        RuntimeObservation {
            installation: dimension("not_installed", Some(error)),
            process: dimension("not_applicable", None),
            connection: dimension("not_applicable", None),
            summary: "Podman was not detected on PATH.".to_owned(),
            remediation: match winget {
                Ok(version) => vec![
                    format!(
                        "winget is available ({}) for a future guided install plan.",
                        version.trim()
                    ),
                    "Manual install command: winget install RedHat.Podman".to_owned(),
                ],
                Err(error) => vec![
                    format!("winget is not available to the daemon session: {error}"),
                    "Install or repair winget/App Installer, or use the official Podman installer."
                        .to_owned(),
                    "Manual install command after winget is available: winget install RedHat.Podman"
                        .to_owned(),
                ],
            },
            profiles: vec![self.placeholder_profile(
                "not_installed",
                Some(error),
                "not_applicable",
                None,
                "not_applicable",
                None,
            )],
            // Provider unavailable: cannot enumerate machines, so do not treat
            // previously-imported ones as removed.
            scanned_keys: None,
        }
    }

    fn placeholder_profile(
        &self,
        installation_state: &str,
        installation_detail: Option<&str>,
        process_state: &str,
        process_detail: Option<&str>,
        connection_state: &str,
        connection_detail: Option<&str>,
    ) -> ObservedProfile {
        ObservedProfile {
            id: profile_id(self.id(), PLACEHOLDER_KEY),
            provider_id: self.id().to_owned(),
            provider_runtime_key: PLACEHOLDER_KEY.to_owned(),
            display_name: self.display_name().to_owned(),
            product: self.product().to_owned(),
            platform: self.platform().to_owned(),
            runtime_class: RuntimeClass::ExternalLocal,
            installation: dimension(installation_state, installation_detail),
            process: dimension(process_state, process_detail),
            connection: dimension(connection_state, connection_detail),
            endpoint_summary: None,
            provider_default: false,
            observed_at_ms: now_ms(),
        }
    }

    fn machine_profiles(&self, version: &str) -> Vec<ObservedProfile> {
        let Ok(output) = command_output("podman", &["machine", "list", "--format", "json"]) else {
            return Vec::new();
        };
        let Ok(machines) = parse_machines(&output) else {
            return Vec::new();
        };
        let observed_at_ms = now_ms();

        machines
            .into_iter()
            .filter_map(|machine| self.machine_profile(version, observed_at_ms, machine))
            .collect()
    }

    fn machine_profile(
        &self,
        version: &str,
        observed_at_ms: i64,
        machine: PodmanMachine,
    ) -> Option<ObservedProfile> {
        let name = machine.name.as_deref()?;
        let key = format!("machine/{name}");
        let running = machine.running.unwrap_or(false);
        let process_detail = machine_detail(&machine);
        let endpoint_summary = running
            .then(EndpointSummary::windows_named_pipe)
            .and_then(|summary| summary.to_json_string());
        // A machine carrying Studio's reserved name is classified built-in so
        // that, absent an ownership token, reconciliation records an ownership
        // conflict instead of silently adopting a machine Studio did not create.
        let runtime_class = if name == RESERVED_BUILT_IN_MACHINE {
            RuntimeClass::BuiltIn
        } else {
            RuntimeClass::ExternalLocal
        };

        Some(ObservedProfile {
            id: profile_id(self.id(), &key),
            provider_id: self.id().to_owned(),
            provider_runtime_key: key,
            display_name: if name == RESERVED_BUILT_IN_MACHINE {
                "Susun Runtime".to_owned()
            } else {
                format!("Podman machine {name}")
            },
            product: self.product().to_owned(),
            platform: self.platform().to_owned(),
            runtime_class,
            installation: dimension("installed", Some(version)),
            process: dimension(
                if running { "running" } else { "stopped" },
                Some(process_detail.as_str()),
            ),
            connection: dimension(
                if running {
                    "summarized"
                } else {
                    "not_applicable"
                },
                Some(if running {
                    "npipe://<local-pipe>"
                } else {
                    "Machine is stopped."
                }),
            ),
            endpoint_summary,
            provider_default: machine.default.unwrap_or(false),
            observed_at_ms,
        })
    }
}

#[derive(Debug, Deserialize)]
struct PodmanMachine {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Running")]
    running: Option<bool>,
    #[serde(rename = "VMType")]
    vm_type: Option<String>,
    #[serde(rename = "Default")]
    default: Option<bool>,
    #[serde(rename = "CPUs")]
    cpus: Option<JsonU64>,
    #[serde(rename = "Memory")]
    memory: Option<JsonU64>,
    #[serde(rename = "DiskSize")]
    disk_size: Option<JsonU64>,
    #[serde(rename = "UserModeNetworking")]
    user_mode_networking: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonU64 {
    Number(u64),
    String(String),
}

impl JsonU64 {
    fn value(&self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::String(value) => value.parse().ok(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PodmanMachineInspect {
    #[serde(rename = "ConnectionInfo")]
    connection_info: Option<PodmanConnectionInfo>,
}

#[derive(Debug, Deserialize)]
struct PodmanConnectionInfo {
    #[serde(rename = "PodmanPipe")]
    podman_pipe: Option<PodmanPipe>,
    #[serde(rename = "DockerPipe")]
    docker_pipe: Option<PodmanPipe>,
}

#[derive(Debug, Deserialize)]
struct PodmanPipe {
    #[serde(rename = "Path")]
    path: Option<String>,
}

fn machine_detail(machine: &PodmanMachine) -> String {
    let mut details = Vec::new();
    if let Some(vm_type) = &machine.vm_type {
        details.push(vm_type.clone());
    }
    if machine.default.unwrap_or(false) {
        details.push("default".to_owned());
    }
    if details.is_empty() {
        "podman machine".to_owned()
    } else {
        details.join(", ")
    }
}

fn parse_machines(output: &str) -> Result<Vec<PodmanMachine>, serde_json::Error> {
    serde_json::from_str(output)
}

fn trusted_machine(machine_name: &str) -> Option<PodmanMachine> {
    let target = trusted_exec::verify_trusted_program(TrustedProgram::Podman).ok()?;
    let output =
        trusted_read_output(&target.path, &["machine", "list", "--format", "json"]).ok()?;
    parse_machines(&output)
        .ok()?
        .into_iter()
        .find(|machine| machine.name.as_deref() == Some(machine_name))
}

fn podman_resource_snapshot(
    profile: &RuntimeProfile,
    machine: Option<&PodmanMachine>,
) -> RuntimeResourceSnapshot {
    let unavailable_metric = |unit: &str| RuntimeResourceMetric {
        support: "unavailable".to_owned(),
        value: None,
        unit: unit.to_owned(),
        detail: Some("The machine is not present in the latest provider inventory.".to_owned()),
    };
    let unsupported_metric = |unit: &str, detail: &str| RuntimeResourceMetric {
        support: "unsupported".to_owned(),
        value: None,
        unit: unit.to_owned(),
        detail: Some(detail.to_owned()),
    };
    let metric = |value: Option<u64>, unit: &str, detail: &str| RuntimeResourceMetric {
        support: if value.is_some() {
            "supported"
        } else {
            "unknown"
        }
        .to_owned(),
        value,
        unit: unit.to_owned(),
        detail: value.is_none().then(|| detail.to_owned()),
    };
    let managed =
        profile.runtime_class == "built_in" && profile.ownership_state == "studio_managed";
    let Some(machine) = machine else {
        return RuntimeResourceSnapshot {
            profile_id: profile.id.clone(),
            provider_id: profile.provider_id.clone(),
            observed_at_ms: now_ms(),
            managed,
            cpu: unavailable_metric("cores"),
            memory: unavailable_metric("bytes"),
            disk_allocation: unavailable_metric("bytes"),
            disk_usage: unavailable_metric("bytes"),
            data_location: RuntimeResourceText {
                support: "unavailable".to_owned(),
                value: None,
                detail: Some(
                    "The machine is not present in the latest provider inventory.".to_owned(),
                ),
            },
            network: RuntimeResourceText {
                support: "unavailable".to_owned(),
                value: None,
                detail: Some(
                    "The machine is not present in the latest provider inventory.".to_owned(),
                ),
            },
            volumes: unavailable_metric("count"),
            updates: RuntimeResourceUpdateCapabilities {
                network_mode: RuntimeResourceUpdateCapability {
                    supported: false,
                    restart_required: false,
                    reason: "The machine is unavailable.".to_owned(),
                },
            },
        };
    };
    let network_mode = machine
        .user_mode_networking
        .map(|enabled| if enabled { "user_mode" } else { "wsl" }.to_owned());
    RuntimeResourceSnapshot {
        profile_id: profile.id.clone(),
        provider_id: profile.provider_id.clone(),
        observed_at_ms: now_ms(),
        managed,
        cpu: metric(
            machine.cpus.as_ref().and_then(JsonU64::value),
            "cores",
            "Podman did not report CPU allocation.",
        ),
        memory: metric(
            machine.memory.as_ref().and_then(JsonU64::value),
            "bytes",
            "Podman did not report memory allocation.",
        ),
        disk_allocation: metric(
            machine.disk_size.as_ref().and_then(JsonU64::value),
            "bytes",
            "Podman did not report disk allocation.",
        ),
        disk_usage: unsupported_metric(
            "bytes",
            "Podman machine inventory does not report used disk space.",
        ),
        data_location: RuntimeResourceText {
            support: "supported".to_owned(),
            value: Some("provider_managed_user_scope".to_owned()),
            detail: Some(
                "Stored in Podman's current-user machine data; raw paths are not exposed."
                    .to_owned(),
            ),
        },
        network: RuntimeResourceText {
            support: if network_mode.is_some() {
                "supported"
            } else {
                "unknown"
            }
            .to_owned(),
            value: network_mode,
            detail: machine
                .user_mode_networking
                .is_none()
                .then(|| "Podman did not report the machine network mode.".to_owned()),
        },
        volumes: unsupported_metric(
            "count",
            "Engine-wide volume inventory is not available through the current SDK contract.",
        ),
        updates: RuntimeResourceUpdateCapabilities {
            network_mode: RuntimeResourceUpdateCapability {
                supported: managed && machine.vm_type.as_deref() == Some("wsl"),
                restart_required: machine.running.unwrap_or(false),
                reason: if !managed {
                    "Only a Studio-owned Susun Runtime can be reconfigured."
                } else if machine.vm_type.as_deref() != Some("wsl") {
                    "Network-mode updates are supported only by Podman's Windows WSL provider."
                } else if machine.running.unwrap_or(false) {
                    "Restart Susun Runtime after applying this change."
                } else {
                    "The new mode is used the next time Susun Runtime starts."
                }
                .to_owned(),
            },
        },
    }
}

fn machine_pipe_from_inspect(machine_name: &str) -> Option<String> {
    let output = command_output("podman", &["machine", "inspect", machine_name]).ok()?;
    let inspect = serde_json::from_str::<Vec<PodmanMachineInspect>>(&output)
        .ok()?
        .into_iter()
        .next()?;
    inspect
        .connection_info
        .and_then(|info| info.podman_pipe.or(info.docker_pipe))
        .and_then(|pipe| pipe.path)
        .filter(|path| !path.trim().is_empty())
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    use crate::runtime::provider::ManagementCapabilities;

    fn managed_profile() -> RuntimeProfile {
        RuntimeProfile {
            id: "runtime-windows-podman-managed".to_owned(),
            provider_id: "windows-podman".to_owned(),
            provider_runtime_key: format!("machine/{RESERVED_BUILT_IN_MACHINE}"),
            display_name: "Susun Runtime".to_owned(),
            product: "podman".to_owned(),
            platform: "windows".to_owned(),
            runtime_class: "built_in".to_owned(),
            ownership_state: "studio_managed".to_owned(),
            source: "studio_setup".to_owned(),
            installation: dimension("installed", None),
            process: dimension("running", None),
            connection: dimension("summarized", None),
            endpoint_summary: None,
            availability_state: "available".to_owned(),
            last_seen_at_ms: Some(1),
            missing_since_ms: None,
            last_error: None,
            is_selected: true,
            observation_revision: 1,
            observed_at_ms: 1,
            management: ManagementCapabilities::derive("built_in", "studio_managed", "available"),
            freshness: "fresh".to_owned(),
        }
    }

    #[test]
    fn machine_inventory_accepts_numeric_and_string_resource_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let machines = parse_machines(
            r#"[{
                "Name":"susun-runtime-default",
                "Running":true,
                "VMType":"wsl",
                "Default":false,
                "CPUs":4,
                "Memory":"4294967296",
                "DiskSize":"107374182400",
                "UserModeNetworking":false
            }]"#,
        )?;
        let snapshot = podman_resource_snapshot(&managed_profile(), machines.first());

        assert!(snapshot.managed);
        assert_eq!(snapshot.cpu.value, Some(4));
        assert_eq!(snapshot.memory.value, Some(4_294_967_296));
        assert_eq!(snapshot.disk_allocation.value, Some(107_374_182_400));
        assert_eq!(snapshot.network.value.as_deref(), Some("wsl"));
        assert_eq!(snapshot.disk_usage.support, "unsupported");
        assert_eq!(snapshot.volumes.support, "unsupported");
        assert!(snapshot.updates.network_mode.supported);
        assert!(snapshot.updates.network_mode.restart_required);
        assert_eq!(
            snapshot.data_location.value.as_deref(),
            Some("provider_managed_user_scope")
        );
        Ok(())
    }

    #[test]
    fn wsl_network_update_builds_an_exact_provider_owned_command()
    -> Result<(), Box<dyn std::error::Error>> {
        let machines =
            parse_machines(r#"[{"Name":"susun-runtime-default","Running":false,"VMType":"wsl"}]"#)?;
        let machine = machines.first().ok_or("missing machine")?;
        let command = podman_resource_update_command(
            &managed_profile(),
            RuntimeResourceUpdate::NetworkUserMode(true),
            machine,
        )
        .ok_or("missing command")?;
        assert_eq!(
            command.args,
            vec![
                "machine",
                "set",
                "--user-mode-networking=true",
                "susun-runtime-default"
            ]
        );
        assert_eq!(command.elevation, ProcessElevation::CurrentUser);
        Ok(())
    }

    #[test]
    fn missing_machine_inventory_is_explicitly_unavailable() {
        let snapshot = podman_resource_snapshot(&managed_profile(), None);
        assert_eq!(snapshot.cpu.support, "unavailable");
        assert_eq!(snapshot.network.support, "unavailable");
        assert!(snapshot.cpu.value.is_none());
    }

    #[test]
    fn reset_plan_targets_only_the_reserved_machine() -> Result<(), Box<dyn std::error::Error>> {
        let machines =
            parse_machines(r#"[{"Name":"susun-runtime-default","Running":true,"VMType":"wsl"}]"#)?;
        let machine = machines.first().ok_or("missing machine")?;
        let plan = podman_recovery_plan(&managed_profile(), RuntimeRecoveryAction::Reset, machine)
            .ok_or("missing plan")?;

        assert_eq!(plan.commands.len(), 2);
        assert_eq!(
            plan.commands[0].args,
            vec!["machine", "rm", "--force", "susun-runtime-default"]
        );
        assert_eq!(
            plan.commands[1].args,
            vec!["machine", "init", "susun-runtime-default"]
        );
        assert!(
            plan.commands
                .iter()
                .all(|command| command.elevation == ProcessElevation::CurrentUser)
        );
        Ok(())
    }

    #[test]
    fn repair_running_machine_stops_then_starts() -> Result<(), Box<dyn std::error::Error>> {
        let machines =
            parse_machines(r#"[{"Name":"susun-runtime-default","Running":true,"VMType":"wsl"}]"#)?;
        let machine = machines.first().ok_or("missing machine")?;
        let plan = podman_recovery_plan(&managed_profile(), RuntimeRecoveryAction::Repair, machine)
            .ok_or("missing plan")?;

        assert_eq!(plan.commands.len(), 2);
        assert_eq!(
            plan.commands[0].args,
            vec!["machine", "stop", "susun-runtime-default"]
        );
        assert_eq!(
            plan.commands[1].args,
            vec!["machine", "start", "susun-runtime-default"]
        );
        Ok(())
    }
}
