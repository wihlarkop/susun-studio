use super::{
    CompatibilityInput, RuntimeCompatibilityReport, RuntimeWorkflowCompatibility, derive_report,
};
use crate::{runtime, susun_integration::EngineCapabilitiesRow};

fn profile(
    runtime_class: &str,
    ownership_state: &str,
    availability_state: &str,
) -> runtime::RuntimeProfile {
    runtime::RuntimeProfile {
        id: "profile".to_owned(),
        provider_id: "windows-podman".to_owned(),
        provider_runtime_key: "machine/profile".to_owned(),
        display_name: "Podman with //./pipe/private".to_owned(),
        product: "not-an-authority".to_owned(),
        platform: "windows".to_owned(),
        runtime_class: runtime_class.to_owned(),
        ownership_state: ownership_state.to_owned(),
        source: "provider_discovery".to_owned(),
        installation: runtime::RuntimeDimension {
            state: "installed".to_owned(),
            detail: None,
        },
        process: runtime::RuntimeDimension {
            state: "running".to_owned(),
            detail: None,
        },
        connection: runtime::RuntimeDimension {
            state: "summarized".to_owned(),
            detail: None,
        },
        endpoint_summary: None,
        availability_state: availability_state.to_owned(),
        last_seen_at_ms: Some(1),
        missing_since_ms: None,
        last_error: None,
        is_preferred: false,
        observation_revision: 1,
        observed_at_ms: 1,
        management: runtime::ManagementCapabilities::derive(
            runtime_class,
            ownership_state,
            availability_state,
        ),
        freshness: "fresh".to_owned(),
    }
}

fn capabilities() -> EngineCapabilitiesRow {
    EngineCapabilitiesRow {
        api_version: Some("1.47".to_owned()),
        supports_health: "supported".to_owned(),
        supports_named_volumes: "supported".to_owned(),
        supports_network_aliases: "supported".to_owned(),
        supports_log_follow: "supported_subset".to_owned(),
        supports_build: "unknown".to_owned(),
        supports_container_inventory: "supported".to_owned(),
        supports_image_inventory: "supported".to_owned(),
        supports_engine_information: "supported".to_owned(),
        supports_image_management: "supported_subset".to_owned(),
        supports_registry_pull: "supported".to_owned(),
        supports_registry_push: "unsupported".to_owned(),
        supports_registry_auth: "unknown".to_owned(),
        supports_build_cache: "unknown".to_owned(),
        supports_cleanup_preview: "supported".to_owned(),
        supports_mount_types: vec!["volume".to_owned()],
        max_container_name_length: Some(255),
    }
}

fn workflow<'a>(
    report: &'a RuntimeCompatibilityReport,
    id: &str,
) -> Option<&'a RuntimeWorkflowCompatibility> {
    report.workflows.iter().find(|workflow| workflow.id == id)
}

#[test]
fn compatibility_preserves_typed_support_and_external_ownership() -> Result<(), serde_json::Error> {
    let profile = profile("external_local", "external", "available");
    let capabilities = capabilities();
    let report: RuntimeCompatibilityReport = derive_report(CompatibilityInput {
        profile: &profile,
        provider_experience: Some(runtime::RuntimeProviderExperience {
            can_create_builtin: true,
            can_discover_external: true,
            can_manage_builtin_lifecycle: true,
            can_manage_external_lifecycle: false,
            can_manage_resources: true,
            requires_external_desktop_app: false,
        }),
        capabilities: Some(&capabilities),
    });
    assert_eq!(report.version_policy, "probe_based");
    assert_eq!(
        workflow(&report, "logs_events_exec").map(|workflow| workflow.level.as_str()),
        Some("limited")
    );
    assert_eq!(
        workflow(&report, "image_inventory_tag_remove_prune")
            .map(|workflow| workflow.level.as_str()),
        Some("limited")
    );
    assert_eq!(
        workflow(&report, "registry_push").map(|workflow| workflow.level.as_str()),
        Some("unsupported")
    );
    assert_eq!(
        workflow(&report, "build_cache").map(|workflow| workflow.level.as_str()),
        Some("unknown")
    );
    assert_eq!(
        workflow(&report, "runtime_lifecycle").map(|workflow| workflow.level.as_str()),
        Some("unsupported")
    );
    assert_eq!(
        workflow(&report, "runtime_resources").map(|workflow| workflow.level.as_str()),
        Some("unsupported")
    );
    assert_eq!(
        workflow(&report, "image_build").map(|workflow| workflow.reason_code.as_str()),
        Some("build_endpoint_not_supported")
    );

    let serialized = serde_json::to_string(&report)?;
    assert!(!serialized.contains("Podman with"));
    assert!(!serialized.contains("//./pipe/private"));
    assert!(!serialized.contains("not-an-authority"));
    Ok(())
}

#[test]
fn unavailable_profile_returns_bounded_unavailable_workflows() {
    let profile = profile("external_local", "external", "missing");
    let report = derive_report(CompatibilityInput {
        profile: &profile,
        provider_experience: None,
        capabilities: None,
    });

    assert_eq!(report.availability_state, "missing");
    assert!(
        report
            .workflows
            .iter()
            .all(|workflow| workflow.level == "unavailable")
    );
    assert!(
        report
            .workflows
            .iter()
            .all(|workflow| workflow.reason_code == "profile_missing")
    );
}
