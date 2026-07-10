use serde::Serialize;
use susun::EngineEndpoint;

use super::stable_suffix;

pub trait RuntimeProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn product(&self) -> &'static str;
    fn platform(&self) -> &'static str;
    fn supported(&self) -> bool;
    fn detect(&self) -> RuntimeObservation;
    fn planned_actions(
        &self,
        observation: &RuntimeObservation,
        profiles: &[RuntimeProfile],
    ) -> Vec<RuntimeAction>;
    fn command_for_action(
        &self,
        action: &str,
        profiles: &[RuntimeProfile],
    ) -> Option<RuntimeCommand>;
    fn endpoint_for_runtime_key(&self, provider_runtime_key: &str) -> Option<EngineEndpoint>;
}

pub struct RuntimeObservation {
    pub installation: RuntimeDimension,
    pub process: RuntimeDimension,
    pub connection: RuntimeDimension,
    pub summary: String,
    pub remediation: Vec<String>,
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

/// A single command executed on behalf of a runtime action. `elevate_if_needed`
/// signals that, if the unelevated attempt fails, Studio should retry once via
/// the OS's own UAC consent prompt (`Start-Process -Verb RunAs`) rather than
/// running a persistent privileged helper — matching the Phase 9 design's
/// "one-shot OS-mediated elevation only" decision.
pub struct RuntimeCommand {
    pub program: &'static str,
    pub args: Vec<String>,
    pub timeout_secs: u64,
    pub success_message: String,
    pub elevate_if_needed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EndpointSummary {
    pub kind: String,
    pub redacted: String,
}

impl EndpointSummary {
    pub fn windows_named_pipe() -> Self {
        Self {
            kind: "windows_named_pipe".to_owned(),
            redacted: "npipe://<local-pipe>".to_owned(),
        }
    }

    pub fn to_json_string(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
}

pub fn profile_id(provider_id: &str, provider_runtime_key: &str) -> String {
    format!(
        "runtime-{provider_id}-{}",
        stable_suffix(provider_runtime_key)
    )
}
