use susun::EngineEndpoint;

use super::{
    RuntimeProfile, command_output, dimension, now_ms,
    provider::{
        EndpointSummary, RuntimeAction, RuntimeCommand, RuntimeObservation, RuntimeProvider,
        profile_id,
    },
};

const DOCKER_DESKTOP_EXE: &str = r"C:\Program Files\Docker\Docker\frontend\Docker Desktop.exe";
const DOCKER_ENGINE_PIPE: &str = r"\\.\pipe\docker_engine";
/// Killing only the `Docker Desktop` (frontend) process leaves
/// `com.docker.backend.exe`/`com.docker.build.exe` running — the backend then
/// relaunches the frontend on its own, which looks like "Stop did nothing."
/// Match every process under the install directory instead.
const DOCKER_STOP_SCRIPT: &str = r"Get-Process | Where-Object { $_.Path -like 'C:\Program Files\Docker\*' } | Stop-Process -Force -ErrorAction SilentlyContinue";

pub struct WindowsDockerDesktopProvider;

impl RuntimeProvider for WindowsDockerDesktopProvider {
    fn id(&self) -> &'static str {
        "windows-docker-desktop"
    }

    fn display_name(&self) -> &'static str {
        "Docker Desktop"
    }

    fn product(&self) -> &'static str {
        "docker-desktop"
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
                summary: "Docker Desktop guided setup is only enabled on Windows in this phase."
                    .to_owned(),
                remediation: vec![
                    "Continue using an existing Docker-compatible engine on this platform."
                        .to_owned(),
                ],
                profiles: vec![self.placeholder(
                    "not_applicable",
                    Some("Windows is the first Phase 13 target."),
                    "not_applicable",
                    None,
                    "not_applicable",
                    None,
                )],
            };
        }

        // The `docker` CLI ships with Docker Desktop on Windows, so its
        // presence on PATH is used as the installation signal (mirrors the
        // `podman --version` heuristic in windows_podman.rs). This can't
        // distinguish Docker Desktop from a standalone Docker Engine CLI
        // install, but Docker Desktop is by far the common Windows case.
        match command_output("docker", &["--version"]) {
            Ok(version) => self.detect_installed(version.trim()),
            Err(error) => self.detect_missing(error.trim()),
        }
    }

    fn planned_actions(
        &self,
        observation: &RuntimeObservation,
        _profiles: &[RuntimeProfile],
    ) -> Vec<RuntimeAction> {
        let supported = self.supported();
        let installed = observation.installation.state == "installed";
        let missing = observation.installation.state == "not_installed";
        let running = observation.process.state == "running";
        let stopped = observation.process.state == "stopped";
        let winget_check = supported
            .then(|| command_output("winget", &["--version"]))
            .transpose();
        let winget_available = matches!(winget_check, Ok(Some(_)));
        let winget_reason = match &winget_check {
            Ok(Some(_)) => {
                "Install Docker Desktop with winget. Administrator approval will be requested."
            }
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
                    "Docker Desktop is already installed or the install state is unknown."
                } else {
                    winget_reason
                }
                .to_owned(),
            },
            RuntimeAction {
                id: "start".to_owned(),
                label: "Start".to_owned(),
                destructive: false,
                enabled: supported && installed && stopped,
                reason: if stopped {
                    "Launch Docker Desktop."
                } else if !installed {
                    "Install Docker Desktop first."
                } else {
                    "Docker Desktop is already running."
                }
                .to_owned(),
            },
            RuntimeAction {
                id: "stop".to_owned(),
                label: "Stop".to_owned(),
                destructive: false,
                enabled: supported && installed && running,
                reason: if running {
                    "Quit Docker Desktop."
                } else {
                    "Docker Desktop must be running first."
                }
                .to_owned(),
            },
            RuntimeAction {
                id: "restart".to_owned(),
                label: "Restart".to_owned(),
                destructive: false,
                enabled: supported && installed && running,
                reason: if running {
                    "Restart Docker Desktop."
                } else {
                    "Docker Desktop must be running first."
                }
                .to_owned(),
            },
        ]
    }

    fn command_for_action(
        &self,
        action: &str,
        _profiles: &[RuntimeProfile],
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
                    "Docker.DockerDesktop".to_owned(),
                    "--accept-package-agreements".to_owned(),
                    "--accept-source-agreements".to_owned(),
                    "--disable-interactivity".to_owned(),
                ],
                timeout_secs: 30 * 60,
                success_message: "Docker Desktop install command finished.".to_owned(),
                elevate_if_needed: true,
            }),
            "start" => Some(RuntimeCommand {
                program: "powershell",
                args: vec![
                    "-NoProfile".to_owned(),
                    "-WindowStyle".to_owned(),
                    "Hidden".to_owned(),
                    "-Command".to_owned(),
                    format!("Start-Process -FilePath '{DOCKER_DESKTOP_EXE}'"),
                ],
                timeout_secs: 30,
                success_message:
                    "Docker Desktop launch requested. It may take a minute to become ready."
                        .to_owned(),
                elevate_if_needed: false,
            }),
            "stop" => Some(RuntimeCommand {
                program: "powershell",
                args: vec![
                    "-NoProfile".to_owned(),
                    "-WindowStyle".to_owned(),
                    "Hidden".to_owned(),
                    "-Command".to_owned(),
                    DOCKER_STOP_SCRIPT.to_owned(),
                ],
                timeout_secs: 30,
                success_message: "Docker Desktop stop requested (engine and UI processes)."
                    .to_owned(),
                elevate_if_needed: false,
            }),
            "restart" => Some(RuntimeCommand {
                program: "powershell",
                args: vec![
                    "-NoProfile".to_owned(),
                    "-WindowStyle".to_owned(),
                    "Hidden".to_owned(),
                    "-Command".to_owned(),
                    format!(
                        "{DOCKER_STOP_SCRIPT}; Start-Sleep -Seconds 2; Start-Process -FilePath '{DOCKER_DESKTOP_EXE}'"
                    ),
                ],
                timeout_secs: 30,
                success_message: "Docker Desktop restart requested.".to_owned(),
                elevate_if_needed: false,
            }),
            _ => None,
        }
    }

    fn endpoint_for_runtime_key(&self, _provider_runtime_key: &str) -> Option<EngineEndpoint> {
        Some(EngineEndpoint::WindowsNamedPipe(DOCKER_ENGINE_PIPE.into()))
    }
}

impl WindowsDockerDesktopProvider {
    fn detect_installed(&self, version: &str) -> RuntimeObservation {
        let running =
            command_output("docker", &["info", "--format", "{{json .ServerVersion}}"]).is_ok();
        let key = "default";
        let observed_at_ms = now_ms();
        let endpoint_summary = running
            .then(EndpointSummary::windows_named_pipe)
            .and_then(|summary| summary.to_json_string());

        RuntimeObservation {
            installation: dimension("installed", Some(version)),
            process: dimension(
                if running { "running" } else { "stopped" },
                Some(if running {
                    "Observed with docker info."
                } else {
                    "Docker Desktop engine is not responding."
                }),
            ),
            connection: dimension(
                if running { "summarized" } else { "not_applicable" },
                Some(if running {
                    "Safe endpoint summary is available."
                } else {
                    "Engine is stopped."
                }),
            ),
            summary: if running {
                "Docker Desktop detected and running. Safe endpoint summary is ready.".to_owned()
            } else {
                "Docker Desktop is installed but not running.".to_owned()
            },
            remediation: vec![
                "Next: use the selected profile's endpoint summary to connect through the engine provider model."
                    .to_owned(),
            ],
            profiles: vec![RuntimeProfile {
                id: profile_id(self.id(), key),
                provider_id: self.id().to_owned(),
                provider_runtime_key: key.to_owned(),
                display_name: self.display_name().to_owned(),
                product: self.product().to_owned(),
                platform: self.platform().to_owned(),
                installation: dimension("installed", Some(version)),
                process: dimension(if running { "running" } else { "stopped" }, None),
                connection: dimension(if running { "summarized" } else { "not_applicable" }, None),
                endpoint_summary,
                is_selected: false,
                observed_at_ms,
                freshness: "fresh".to_owned(),
            }],
        }
    }

    fn detect_missing(&self, error: &str) -> RuntimeObservation {
        let winget = command_output("winget", &["--version"]);
        RuntimeObservation {
            installation: dimension("not_installed", Some(error)),
            process: dimension("not_applicable", None),
            connection: dimension("not_applicable", None),
            summary: "Docker Desktop was not detected on PATH.".to_owned(),
            remediation: match winget {
                Ok(version) => vec![
                    format!(
                        "winget is available ({}) for a guided install.",
                        version.trim()
                    ),
                    "Manual install command: winget install Docker.DockerDesktop".to_owned(),
                ],
                Err(error) => vec![
                    format!("winget is not available to the daemon session: {error}"),
                    "Install or repair winget/App Installer, or use the official Docker Desktop installer."
                        .to_owned(),
                    "Manual install command after winget is available: winget install Docker.DockerDesktop"
                        .to_owned(),
                ],
            },
            profiles: vec![self.placeholder(
                "not_installed",
                Some(error),
                "not_applicable",
                None,
                "not_applicable",
                None,
            )],
        }
    }

    fn placeholder(
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
}
