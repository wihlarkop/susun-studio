use serde::Deserialize;
use susun::EngineEndpoint;

use super::{
    RuntimeProfile, command_output, dimension, now_ms,
    provider::{EndpointSummary, RuntimeCommand, RuntimeObservation, RuntimeProvider, profile_id},
};

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
            };
        }

        match command_output("podman", &["--version"]) {
            Ok(version) => self.detect_installed(version.trim()),
            Err(error) => self.detect_missing(error.trim()),
        }
    }
}

impl WindowsPodmanProvider {
    pub fn endpoint_for_runtime_key(provider_runtime_key: &str) -> Option<EngineEndpoint> {
        let machine_name = provider_runtime_key.strip_prefix("machine/")?;
        let pipe = machine_pipe_from_inspect(machine_name)
            .unwrap_or_else(|| format!(r"\\.\pipe\podman-machine-{machine_name}"));
        Some(EngineEndpoint::WindowsNamedPipe(pipe.into()))
    }

    pub fn command_for_profile_action(
        &self,
        provider_runtime_key: &str,
        action: &str,
    ) -> Option<RuntimeCommand> {
        if !self.supported() {
            return None;
        }

        match action {
            "install" => Some(RuntimeCommand {
                program: "winget",
                args: vec![
                    "install".to_owned(),
                    "--id".to_owned(),
                    "RedHat.Podman".to_owned(),
                    "--accept-package-agreements".to_owned(),
                    "--accept-source-agreements".to_owned(),
                    "--disable-interactivity".to_owned(),
                ],
                timeout_secs: 30 * 60,
                success_message: "Podman install command finished.".to_owned(),
            }),
            "start" | "stop" => {
                let machine = provider_runtime_key.strip_prefix("machine/")?;
                Some(RuntimeCommand {
                    program: "podman",
                    args: vec!["machine".to_owned(), action.to_owned(), machine.to_owned()],
                    timeout_secs: 5 * 60,
                    success_message: format!("Podman machine {action} command finished."),
                })
            }
            "restart" => {
                let machine = provider_runtime_key.strip_prefix("machine/")?;
                Some(RuntimeCommand {
                    program: "podman",
                    args: vec![
                        "machine".to_owned(),
                        "restart".to_owned(),
                        machine.to_owned(),
                    ],
                    timeout_secs: 5 * 60,
                    success_message: "Podman machine restart command finished.".to_owned(),
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
                process: dimension("stopped", Some("No Podman machine was detected.")),
                connection: dimension("not_applicable", None),
                summary: "Podman is installed, but no machine was detected.".to_owned(),
                remediation: vec![
                    "Next: prepare a trusted setup plan to create a Podman machine.".to_owned(),
                    "Manual setup for now: podman machine init && podman machine start".to_owned(),
                ],
                profiles: vec![self.placeholder_profile(
                    "installed",
                    Some(version),
                    "stopped",
                    Some("No Podman machine was detected."),
                    "not_applicable",
                    None,
                )],
            };
        }

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
    ) -> RuntimeProfile {
        let key = "default";
        RuntimeProfile {
            id: profile_id(self.id(), key),
            provider_id: self.id().to_owned(),
            provider_runtime_key: key.to_owned(),
            display_name: self.display_name().to_owned(),
            product: self.product().to_owned(),
            platform: self.platform().to_owned(),
            installation: dimension(installation_state, installation_detail),
            process: dimension(process_state, process_detail),
            connection: dimension(connection_state, connection_detail),
            endpoint_summary: None,
            is_selected: false,
            observed_at_ms: now_ms(),
            freshness: "fresh".to_owned(),
        }
    }

    fn machine_profiles(&self, version: &str) -> Vec<RuntimeProfile> {
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
    ) -> Option<RuntimeProfile> {
        let name = machine.name.as_deref()?;
        let key = format!("machine/{name}");
        let running = machine.running.unwrap_or(false);
        let process_detail = machine_detail(&machine);
        let endpoint_summary = running
            .then(EndpointSummary::windows_named_pipe)
            .and_then(|summary| summary.to_json_string());

        Some(RuntimeProfile {
            id: profile_id(self.id(), &key),
            provider_id: self.id().to_owned(),
            provider_runtime_key: key,
            display_name: format!("Podman machine {name}"),
            product: self.product().to_owned(),
            platform: self.platform().to_owned(),
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
            is_selected: machine.default.unwrap_or(false),
            observed_at_ms,
            freshness: "fresh".to_owned(),
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
