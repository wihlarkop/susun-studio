use serde::Serialize;

use super::{RuntimeProfile, stable_suffix};

pub trait RuntimeProvider {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn product(&self) -> &'static str;
    fn platform(&self) -> &'static str;
    fn supported(&self) -> bool;
    fn detect(&self) -> RuntimeObservation;
}

pub struct RuntimeObservation {
    pub installation: super::RuntimeDimension,
    pub process: super::RuntimeDimension,
    pub connection: super::RuntimeDimension,
    pub summary: String,
    pub remediation: Vec<String>,
    pub profiles: Vec<RuntimeProfile>,
}

pub struct RuntimeCommand {
    pub program: &'static str,
    pub args: Vec<String>,
    pub timeout_secs: u64,
    pub success_message: String,
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
