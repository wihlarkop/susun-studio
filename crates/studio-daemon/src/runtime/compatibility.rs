use serde::Serialize;

use crate::{runtime, susun_integration};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeCompatibilityReport {
    pub profile_id: String,
    pub provider_id: String,
    pub runtime_class: String,
    pub ownership_state: String,
    pub availability_state: String,
    pub observed_api_version: Option<String>,
    pub version_policy: String,
    pub workflows: Vec<RuntimeWorkflowCompatibility>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeWorkflowCompatibility {
    pub id: String,
    pub level: String,
    pub reason_code: String,
    pub detail: String,
}

pub(crate) struct CompatibilityInput<'a> {
    pub profile: &'a runtime::RuntimeProfile,
    pub provider_experience: Option<runtime::RuntimeProviderExperience>,
    pub capabilities: Option<&'a susun_integration::EngineCapabilitiesRow>,
}

/// Probe one persisted profile without consulting global policy or project
/// bindings. A failed connection is represented as a bounded unavailable
/// report rather than a provider error string.
pub(crate) async fn report_for_profile(
    db: &turso::Database,
    profile_id: &str,
) -> Result<Option<RuntimeCompatibilityReport>, turso::Error> {
    let profiles = runtime::list_all_profiles(db).await?;
    let Some(profile) = profiles
        .into_iter()
        .find(|profile| profile.id == profile_id)
    else {
        return Ok(None);
    };
    let capabilities = if profile.availability_state == "available" {
        match susun_integration::connect_engine_for_profile(db, Some(&profile.id)).await {
            Ok(engine) => susun_integration::engine_capabilities(&engine).await.ok(),
            Err(_) => None,
        }
    } else {
        None
    };
    let provider_experience = runtime::provider_experience(&profile.provider_id);
    Ok(Some(derive_report(CompatibilityInput {
        profile: &profile,
        provider_experience,
        capabilities: capabilities.as_ref(),
    })))
}

/// Pure compatibility derivation. It deliberately consumes typed persisted
/// profile/provider state and SDK capability labels only; display names,
/// products, versions, endpoints, and profile-id shapes are never inputs.
pub(crate) fn derive_report(input: CompatibilityInput<'_>) -> RuntimeCompatibilityReport {
    let profile = input.profile;
    let (availability_state, workflows) = match input.capabilities {
        Some(capabilities) => (
            profile.availability_state.clone(),
            available_workflows(profile, input.provider_experience, capabilities),
        ),
        None => {
            let availability_state = if profile.availability_state == "missing" {
                "missing".to_owned()
            } else {
                "unavailable".to_owned()
            };
            let workflows = unavailable_workflows(&availability_state);
            (availability_state, workflows)
        }
    };

    RuntimeCompatibilityReport {
        profile_id: profile.id.clone(),
        provider_id: profile.provider_id.clone(),
        runtime_class: profile.runtime_class.clone(),
        ownership_state: profile.ownership_state.clone(),
        availability_state,
        observed_api_version: input
            .capabilities
            .and_then(|capabilities| capabilities.api_version.clone()),
        version_policy: "probe_based".to_owned(),
        workflows,
        observed_at_ms: profile.observed_at_ms,
    }
}

const WORKFLOW_IDS: &[&str] = &[
    "project_plan_execute",
    "logs_events_exec",
    "watch",
    "container_inventory_actions",
    "image_inventory_tag_remove_prune",
    "registry_pull",
    "registry_push",
    "registry_credentials",
    "image_build",
    "build_cache",
    "volumes_networks",
    "runtime_lifecycle",
    "runtime_resources",
    "diagnostics",
    "metadata_migration",
];

fn unavailable_workflows(availability_state: &str) -> Vec<RuntimeWorkflowCompatibility> {
    let reason_code = if availability_state == "missing" {
        "profile_missing"
    } else {
        "profile_unavailable"
    };
    WORKFLOW_IDS
        .iter()
        .map(|id| RuntimeWorkflowCompatibility {
            id: (*id).to_owned(),
            level: "unavailable".to_owned(),
            reason_code: reason_code.to_owned(),
            detail: "This runtime cannot be reached for a compatibility probe.".to_owned(),
        })
        .collect()
}

fn available_workflows(
    profile: &runtime::RuntimeProfile,
    provider_experience: Option<runtime::RuntimeProviderExperience>,
    capabilities: &susun_integration::EngineCapabilitiesRow,
) -> Vec<RuntimeWorkflowCompatibility> {
    let lifecycle_managed = profile.runtime_class == "built_in"
        && profile.ownership_state == "studio_managed"
        && provider_experience.is_some_and(|experience| experience.can_manage_builtin_lifecycle);
    let resource_managed = profile.runtime_class == "built_in"
        && profile.ownership_state == "studio_managed"
        && provider_experience.is_some_and(|experience| experience.can_manage_resources);

    vec![
        supported_workflow("project_plan_execute"),
        from_support("logs_events_exec", &capabilities.supports_log_follow),
        from_support("watch", &capabilities.supports_log_follow),
        combine_support(
            "container_inventory_actions",
            &[
                &capabilities.supports_container_inventory,
                &capabilities.supports_engine_information,
            ],
        ),
        combine_support(
            "image_inventory_tag_remove_prune",
            &[
                &capabilities.supports_image_inventory,
                &capabilities.supports_image_management,
                &capabilities.supports_cleanup_preview,
            ],
        ),
        from_support("registry_pull", &capabilities.supports_registry_pull),
        from_support("registry_push", &capabilities.supports_registry_push),
        from_support("registry_credentials", &capabilities.supports_registry_auth),
        RuntimeWorkflowCompatibility {
            id: "image_build".to_owned(),
            level: "unsupported".to_owned(),
            reason_code: "build_endpoint_not_supported".to_owned(),
            detail: "Image builds cannot be pinned to this runtime endpoint.".to_owned(),
        },
        from_support("build_cache", &capabilities.supports_build_cache),
        combine_support(
            "volumes_networks",
            &[
                &capabilities.supports_named_volumes,
                &capabilities.supports_network_aliases,
            ],
        ),
        management_workflow("runtime_lifecycle", lifecycle_managed),
        management_workflow("runtime_resources", resource_managed),
        supported_workflow("diagnostics"),
        supported_workflow("metadata_migration"),
    ]
}

fn supported_workflow(id: &str) -> RuntimeWorkflowCompatibility {
    RuntimeWorkflowCompatibility {
        id: id.to_owned(),
        level: "supported".to_owned(),
        reason_code: "studio_supported".to_owned(),
        detail: "Studio supports this workflow for the observed runtime.".to_owned(),
    }
}

fn management_workflow(id: &str, managed: bool) -> RuntimeWorkflowCompatibility {
    if managed {
        supported_workflow(id)
    } else {
        RuntimeWorkflowCompatibility {
            id: id.to_owned(),
            level: "unsupported".to_owned(),
            reason_code: "external_runtime_not_studio_managed".to_owned(),
            detail:
                "Studio lifecycle and resource controls apply only to its managed built-in runtime."
                    .to_owned(),
        }
    }
}

fn from_support(id: &str, support: &str) -> RuntimeWorkflowCompatibility {
    match support {
        "supported" => supported_workflow(id),
        "supported_subset" | "experimental" => RuntimeWorkflowCompatibility {
            id: id.to_owned(),
            level: "limited".to_owned(),
            reason_code: format!("capability_{support}"),
            detail: "The runtime reports a limited capability for this workflow.".to_owned(),
        },
        "unsupported" => RuntimeWorkflowCompatibility {
            id: id.to_owned(),
            level: "unsupported".to_owned(),
            reason_code: "capability_unsupported".to_owned(),
            detail: "The runtime reports that this workflow is unsupported.".to_owned(),
        },
        _ => RuntimeWorkflowCompatibility {
            id: id.to_owned(),
            level: "unknown".to_owned(),
            reason_code: "capability_unknown".to_owned(),
            detail: "The runtime did not provide enough capability information.".to_owned(),
        },
    }
}

fn combine_support(id: &str, supports: &[&str]) -> RuntimeWorkflowCompatibility {
    if supports.contains(&"unsupported") {
        return from_support(id, "unsupported");
    }
    if supports
        .iter()
        .any(|support| matches!(*support, "supported_subset" | "experimental"))
    {
        return from_support(id, "supported_subset");
    }
    if supports.iter().all(|support| *support == "supported") {
        return supported_workflow(id);
    }
    from_support(id, "unknown")
}

#[cfg(test)]
mod tests;
