use serde::Serialize;
use susun::EngineEndpoint;

use super::command::ExecutableCommand;
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
    ) -> Option<ExecutableCommand>;
    fn endpoint_for_runtime_key(&self, provider_runtime_key: &str) -> Option<EngineEndpoint>;
    fn resource_snapshot(&self, profile: &RuntimeProfile) -> RuntimeResourceSnapshot {
        RuntimeResourceSnapshot::unsupported(profile, self.id())
    }
    fn command_for_resource_update(
        &self,
        _profile: &RuntimeProfile,
        _update: RuntimeResourceUpdate,
    ) -> Option<ExecutableCommand> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeResourceUpdate {
    NetworkUserMode(bool),
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
pub struct RuntimeResourceMetric {
    pub support: String,
    pub value: Option<u64>,
    pub unit: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeResourceText {
    pub support: String,
    pub value: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeResourceSnapshot {
    pub profile_id: String,
    pub provider_id: String,
    pub observed_at_ms: i64,
    pub managed: bool,
    pub cpu: RuntimeResourceMetric,
    pub memory: RuntimeResourceMetric,
    pub disk_allocation: RuntimeResourceMetric,
    pub disk_usage: RuntimeResourceMetric,
    pub data_location: RuntimeResourceText,
    pub network: RuntimeResourceText,
    pub volumes: RuntimeResourceMetric,
    pub updates: RuntimeResourceUpdateCapabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeResourceUpdateCapability {
    pub supported: bool,
    pub restart_required: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeResourceUpdateCapabilities {
    pub network_mode: RuntimeResourceUpdateCapability,
}

impl RuntimeResourceSnapshot {
    fn unsupported(profile: &RuntimeProfile, provider_id: &str) -> Self {
        let metric = |unit: &str, detail: &str| RuntimeResourceMetric {
            support: "unsupported".to_owned(),
            value: None,
            unit: unit.to_owned(),
            detail: Some(detail.to_owned()),
        };
        Self {
            profile_id: profile.id.clone(),
            provider_id: provider_id.to_owned(),
            observed_at_ms: profile.observed_at_ms,
            managed: profile.runtime_class == "built_in"
                && profile.ownership_state == "studio_managed",
            cpu: metric("cores", "This provider does not expose CPU allocation."),
            memory: metric("bytes", "This provider does not expose memory allocation."),
            disk_allocation: metric("bytes", "This provider does not expose disk allocation."),
            disk_usage: metric("bytes", "This provider does not expose disk usage."),
            data_location: RuntimeResourceText {
                support: "unsupported".to_owned(),
                value: None,
                detail: Some(
                    "This provider does not expose a safe data-location summary.".to_owned(),
                ),
            },
            network: RuntimeResourceText {
                support: "unsupported".to_owned(),
                value: None,
                detail: Some("This provider does not expose its network mode.".to_owned()),
            },
            volumes: metric("count", "Engine-wide volume inventory is unavailable."),
            updates: RuntimeResourceUpdateCapabilities {
                network_mode: RuntimeResourceUpdateCapability {
                    supported: false,
                    restart_required: false,
                    reason: "This provider does not support network-mode updates.".to_owned(),
                },
            },
        }
    }
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
            can_adopt: false,
            requires_recovery: unproven_built_in,
            blocks_destructive_actions: unproven_built_in,
        }
    }
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
