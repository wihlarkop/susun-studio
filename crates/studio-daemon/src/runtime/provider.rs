use serde::Serialize;
use susun::EngineEndpoint;

use super::stable_suffix;

/// The machine name Studio reserves for the built-in managed runtime. A machine
/// with this name that Studio cannot prove it created is treated as an
/// ownership conflict rather than silently adopted.
pub const RESERVED_BUILT_IN_MACHINE: &str = "susun-runtime-default";

/// The synthetic profile key providers emit when a provider is present but has
/// no real runtime yet (e.g. Podman installed with no machine). It is never a
/// persisted runtime the user manages, so missing-reconciliation ignores it.
pub const PLACEHOLDER_KEY: &str = "default";

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
    pub profiles: Vec<ObservedProfile>,
    /// Authoritative list of provider runtime keys currently present, or `None`
    /// when the provider is unavailable / the scan could not enumerate runtimes.
    /// Missing-reconciliation only runs when this is `Some`, which keeps a
    /// temporarily-failing provider from marking real profiles as removed.
    pub scanned_keys: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeClass {
    BuiltIn,
    ExternalLocal,
    // Part of the persisted class vocabulary (schema + DTOs). No provider emits
    // a remote runtime yet, so it is not constructed in Rust today.
    #[allow(dead_code)]
    ExternalRemote,
}

impl RuntimeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeClass::BuiltIn => "built_in",
            RuntimeClass::ExternalLocal => "external_local",
            RuntimeClass::ExternalRemote => "external_remote",
        }
    }
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

/// What a provider observes about one runtime during detection. It carries only
/// identity and observed health; ownership, source, and selection live in the
/// database and are never derived from a fresh scan (except the one-time
/// initial-import selection, gated on `provider_default`).
#[derive(Debug, Clone)]
pub struct ObservedProfile {
    pub id: String,
    pub provider_id: String,
    pub provider_runtime_key: String,
    pub display_name: String,
    pub product: String,
    pub platform: String,
    pub runtime_class: RuntimeClass,
    pub installation: RuntimeDimension,
    pub process: RuntimeDimension,
    pub connection: RuntimeDimension,
    pub endpoint_summary: Option<String>,
    /// The provider's own "default" marker. Honoured only for the very first
    /// import selection when nothing is selected yet — never to override a
    /// later user choice on a recheck.
    pub provider_default: bool,
    pub observed_at_ms: i64,
}

/// The persisted, API-facing runtime profile: stable identity + ownership +
/// observed health + availability + derived management capabilities.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeProfile {
    pub id: String,
    pub provider_id: String,
    pub provider_runtime_key: String,
    pub display_name: String,
    pub product: String,
    pub platform: String,
    pub runtime_class: String,
    pub ownership_state: String,
    pub source: String,
    pub installation: RuntimeDimension,
    pub process: RuntimeDimension,
    pub connection: RuntimeDimension,
    pub endpoint_summary: Option<String>,
    pub availability_state: String,
    pub last_seen_at_ms: Option<i64>,
    pub missing_since_ms: Option<i64>,
    pub last_error: Option<RuntimeError>,
    pub is_selected: bool,
    pub observation_revision: i64,
    pub observed_at_ms: i64,
    pub management: ManagementCapabilities,
    pub freshness: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeError {
    pub code: String,
    pub detail: Option<String>,
    pub at_ms: i64,
}

/// Which management operations Studio will allow for a profile, derived from its
/// class, ownership, and availability. Surfaced to the UI so it can present
/// truthful, guarded controls instead of guessing.
#[derive(Debug, Clone, Serialize)]
pub struct ManagementCapabilities {
    pub can_select: bool,
    pub can_forget: bool,
    pub can_adopt: bool,
    pub requires_recovery: bool,
    pub blocks_destructive_actions: bool,
}

impl ManagementCapabilities {
    pub fn derive(runtime_class: &str, ownership_state: &str, availability_state: &str) -> Self {
        let built_in = runtime_class == "built_in";
        let studio_managed = ownership_state == "studio_managed";
        let unproven_built_in = built_in && !studio_managed;
        Self {
            can_select: availability_state == "available" && !unproven_built_in,
            can_forget: !built_in && !studio_managed,
            can_adopt: built_in
                && matches!(ownership_state, "ownership_conflict" | "ownership_unknown"),
            requires_recovery: unproven_built_in,
            blocks_destructive_actions: unproven_built_in,
        }
    }
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
