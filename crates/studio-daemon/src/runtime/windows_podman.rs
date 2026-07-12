use serde::Deserialize;
use susun::EngineEndpoint;

use super::{
    RuntimeProfile,
    command::{CommandKind, ExecutableCommand, ProcessElevation, TrustedProgram},
    command_output, dimension, now_ms,
    provider::{
        EndpointSummary, ObservedProfile, PLACEHOLDER_KEY, RESERVED_BUILT_IN_MACHINE,
        RuntimeAction, RuntimeClass, RuntimeObservation, RuntimeProvider, profile_id,
    },
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
        let not_created = observation.process.state == "not_created";
        let selected = profiles
            .iter()
            .find(|profile| profile.is_selected && profile.provider_id == self.id());
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
                id: "init".to_owned(),
                label: "Initialize".to_owned(),
                destructive: false,
                enabled: supported && installed && not_created,
                reason: if not_created {
                    "Create a Podman machine."
                } else if !installed {
                    "Install Podman first."
                } else {
                    "A Podman machine already exists."
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
                    "--accept-package-agreements".into(),
                    "--accept-source-agreements".into(),
                    "--disable-interactivity".into(),
                ],
                env_allowlist: Vec::new(),
                working_dir: None,
                timeout: Duration::from_secs(30 * 60),
                kind: CommandKind::PackageManager,
                elevation: ProcessElevation::OneShotOsMediated,
                success_message: "Podman install command finished.".to_owned(),
            }),
            "init" => Some(ExecutableCommand {
                program: TrustedProgram::Podman,
                args: vec!["machine".into(), "init".into()],
                env_allowlist: Vec::new(),
                working_dir: None,
                timeout: Duration::from_secs(10 * 60),
                kind: CommandKind::RuntimeCli,
                elevation: ProcessElevation::CurrentUser,
                success_message: "Podman machine created. Use Start to bring it online.".to_owned(),
            }),
            "start" | "stop" | "restart" => {
                let profile = profiles
                    .iter()
                    .find(|profile| profile.is_selected && profile.provider_id == self.id())?;
                let machine = profile.provider_runtime_key.strip_prefix("machine/")?;
                Some(ExecutableCommand {
                    program: TrustedProgram::Podman,
                    args: vec!["machine".into(), action.into(), machine.into()],
                    env_allowlist: Vec::new(),
                    working_dir: None,
                    timeout: Duration::from_secs(5 * 60),
                    kind: CommandKind::RuntimeCli,
                    elevation: ProcessElevation::CurrentUser,
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
        let Ok(machines) = serde_json::from_str::<Vec<PodmanMachine>>(&output) else {
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
            display_name: format!("Podman machine {name}"),
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
