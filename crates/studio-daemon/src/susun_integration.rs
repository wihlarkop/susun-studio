use std::{path::PathBuf, sync::Arc, time::SystemTime};

use susun::{
    CancellationToken, ContainerEngine, DownPlanOptions, EngineCapabilities, EngineSnapshot,
    EventSink, ExecutionPlan, ExecutionReport, PlanOutcome, ProjectSummary, Runtime, SdkProject,
    SusunWorkspace, UpPlanOptions, render_diagnostics_json, render_plan_json,
};
use susun_engine_bollard::BollardEngine;

use crate::error::ApiError;

pub struct AnalyzedImport {
    pub source_id: Option<String>,
    pub project_name: Option<String>,
    pub project_directory: PathBuf,
    pub summary: ProjectSummary,
    pub diagnostics: serde_json::Value,
    pub has_errors: bool,
}

fn build_workspace(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
) -> SusunWorkspace {
    let mut workspace = SusunWorkspace::new().with_files(files.to_vec());
    if let Some(env_file) = env_file {
        workspace = workspace.with_env_file(env_file.clone());
    }
    if let Some(name) = project_name {
        workspace = workspace.with_project_name(name);
    }
    if !profiles.is_empty() {
        workspace = workspace.with_profiles(profiles.to_vec());
    }
    workspace
}

pub fn analyze_project(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
) -> Result<AnalyzedImport, susun::Error> {
    let workspace = build_workspace(files, env_file, project_name, profiles);
    let project_directory = workspace.project_directory();
    let sdk_project = workspace.analyze()?;

    let diagnostics_json = render_diagnostics_json(
        &sdk_project.analysis().report,
        &sdk_project.analysis().source_map,
    );
    let diagnostics = serde_json::from_str(&diagnostics_json)
        .unwrap_or_else(|_| serde_json::json!({ "diagnostics": [] }));

    let has_errors = sdk_project.analysis().report.has_errors();
    let summary = sdk_project.summary();
    let source_id = sdk_project
        .identity()
        .map(|identity| format!("{}@{}", identity.name, identity.working_set));
    let project_name = summary.project_name.clone();

    Ok(AnalyzedImport {
        source_id,
        project_name,
        project_directory,
        summary,
        diagnostics,
        has_errors,
    })
}

/// A single planned action, flattened for storage and UI display.
pub struct PlanActionRow {
    pub id: String,
    pub kind: &'static str,
    pub resource: String,
    pub safety: String,
    pub reason: String,
    pub dependencies: Vec<String>,
}

/// The result of a dry-run plan, ready to persist and return.
pub struct PlanRow {
    pub plan_json: String,
    pub schema_version: Option<String>,
    pub actions: Vec<PlanActionRow>,
    pub total_actions: usize,
    pub safe_actions: usize,
    pub caution_actions: usize,
    pub destructive_actions: usize,
    pub blocked_diagnostics: Option<serde_json::Value>,
}

pub fn plan_up(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
    options: UpPlanOptions,
) -> Result<PlanRow, ApiError> {
    let sdk_project = build_workspace(files, env_file, project_name, profiles)
        .analyze()
        .map_err(|error| ApiError::PlanningFailed(error.to_string()))?;

    let outcome = sdk_project
        .plan_up(
            EngineCapabilities::permissive_local(),
            EngineSnapshot::empty(SystemTime::UNIX_EPOCH),
            options,
        )
        .map_err(|error| ApiError::PlanningFailed(error.to_string()))?;

    plan_row(&sdk_project, outcome)
}

pub fn plan_down(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
    options: DownPlanOptions,
) -> Result<PlanRow, ApiError> {
    let sdk_project = build_workspace(files, env_file, project_name, profiles)
        .analyze()
        .map_err(|error| ApiError::PlanningFailed(error.to_string()))?;

    let outcome = sdk_project
        .plan_down(
            EngineCapabilities::permissive_local(),
            EngineSnapshot::empty(SystemTime::UNIX_EPOCH),
            options,
        )
        .map_err(|error| ApiError::PlanningFailed(error.to_string()))?;

    plan_row(&sdk_project, outcome)
}

/// Flattens an [`ExecutionPlan`]'s action graph into displayable rows. Reused by
/// the create path and by re-reading a persisted plan back from `plan_json`.
pub fn plan_action_rows(plan: &ExecutionPlan) -> Vec<PlanActionRow> {
    plan.actions
        .iter()
        .map(|(id, node)| {
            let key = node.action.resource_key();
            let resource = key
                .split_once(':')
                .map(|(_, rest)| rest.to_owned())
                .unwrap_or(key);
            let safety = serde_json::to_value(node.safety)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "safe".to_owned());

            PlanActionRow {
                id: id.to_string(),
                kind: node.action.kind(),
                resource,
                safety,
                reason: node.reason.message.clone(),
                dependencies: node.dependencies.iter().map(ToString::to_string).collect(),
            }
        })
        .collect()
}

fn plan_row(sdk_project: &SdkProject, outcome: PlanOutcome) -> Result<PlanRow, ApiError> {
    let Some(plan) = outcome.plan else {
        let diagnostics_json =
            render_diagnostics_json(&outcome.diagnostics, &sdk_project.analysis().source_map);
        let blocked = serde_json::from_str(&diagnostics_json)
            .unwrap_or_else(|_| serde_json::json!({ "diagnostics": [] }));

        return Ok(PlanRow {
            plan_json: String::new(),
            schema_version: None,
            actions: Vec::new(),
            total_actions: 0,
            safe_actions: 0,
            caution_actions: 0,
            destructive_actions: 0,
            blocked_diagnostics: Some(blocked),
        });
    };

    let plan_json =
        render_plan_json(&plan).map_err(|error| ApiError::PlanningFailed(error.to_string()))?;
    let schema_version = format!(
        "{}.{}",
        plan.schema_version.major, plan.schema_version.minor
    );

    Ok(PlanRow {
        plan_json,
        schema_version: Some(schema_version),
        total_actions: plan.summary.total_actions,
        safe_actions: plan.summary.safe_actions,
        caution_actions: plan.summary.caution_actions,
        destructive_actions: plan.summary.destructive_actions,
        actions: plan_action_rows(&plan),
        blocked_diagnostics: None,
    })
}

/// Studio-owned view of an engine reachability check.
pub struct EngineHealthRow {
    pub reachable: bool,
    pub api_version: Option<String>,
    pub error: Option<String>,
}

/// Studio-owned view of engine capabilities. Support levels are rendered as the
/// SDK's own snake_case strings ("supported", "supported_subset", "unsupported",
/// "unknown") so the UI can distinguish "unknown" from "unsupported".
pub struct EngineCapabilitiesRow {
    pub api_version: Option<String>,
    pub supports_health: String,
    pub supports_named_volumes: String,
    pub supports_network_aliases: String,
    pub supports_log_follow: String,
    pub supports_build: String,
    pub supports_mount_types: Vec<String>,
    pub max_container_name_length: Option<usize>,
}

/// Constructs a local Docker client handle. This does not contact Docker; a
/// stopped daemon surfaces later as an error from the first real API call.
pub fn connect_docker_engine() -> Result<BollardEngine, String> {
    BollardEngine::connect_local().map_err(|error| error.to_string())
}

/// Checks engine reachability by requesting its capabilities.
pub async fn engine_health(engine: &BollardEngine) -> EngineHealthRow {
    match engine.capabilities().await {
        Ok(capabilities) => EngineHealthRow {
            reachable: true,
            api_version: capabilities
                .api_version
                .as_ref()
                .map(|version| version.as_str().to_owned()),
            error: None,
        },
        Err(error) => EngineHealthRow {
            reachable: false,
            api_version: None,
            error: Some(error.to_string()),
        },
    }
}

/// Reads and flattens the engine's capabilities.
pub async fn engine_capabilities(engine: &BollardEngine) -> Result<EngineCapabilitiesRow, String> {
    let capabilities: EngineCapabilities = engine
        .capabilities()
        .await
        .map_err(|error| error.to_string())?;

    Ok(EngineCapabilitiesRow {
        api_version: capabilities
            .api_version
            .as_ref()
            .map(|version| version.as_str().to_owned()),
        supports_health: enum_label(capabilities.supports_health),
        supports_named_volumes: enum_label(capabilities.supports_named_volumes),
        supports_network_aliases: enum_label(capabilities.supports_network_aliases),
        supports_log_follow: enum_label(capabilities.supports_log_follow),
        supports_build: enum_label(capabilities.supports_build),
        supports_mount_types: capabilities
            .supports_mount_types
            .iter()
            .map(enum_label)
            .collect(),
        max_container_name_length: capabilities.max_container_name_length,
    })
}

/// Serializes a serde enum (SupportLevel, MountType) to its snake_case string.
fn enum_label<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// One planned step, named for the UI (e.g. action "Pull", resource "nginx:latest").
pub struct JobActionManifest {
    pub id: String,
    pub action: String,
    pub resource: String,
}

/// Plans `up` against the real engine and returns the executable plan plus a
/// named manifest of its steps for the UI checklist.
pub async fn plan_up_for_execution(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
    options: UpPlanOptions,
    engine: &BollardEngine,
) -> Result<(ExecutionPlan, Vec<JobActionManifest>), String> {
    let sdk_project = build_workspace(files, env_file, project_name, profiles)
        .analyze()
        .map_err(|error| error.to_string())?;
    let identity = sdk_project
        .identity()
        .cloned()
        .ok_or_else(|| "project has no derivable identity".to_owned())?;

    let snapshot = engine
        .snapshot(&identity)
        .await
        .map_err(|e| e.to_string())?;
    let capabilities = engine.capabilities().await.map_err(|e| e.to_string())?;
    let outcome = sdk_project
        .plan_up(capabilities, snapshot, options)
        .map_err(|error| error.to_string())?;
    let plan = outcome
        .plan
        .ok_or_else(|| "planning was blocked; check project diagnostics".to_owned())?;

    let manifest = job_action_manifest(&plan);
    Ok((plan, manifest))
}

/// Plans `down` against the real engine and returns the plan plus its manifest.
pub async fn plan_down_for_execution(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
    options: DownPlanOptions,
    engine: &BollardEngine,
) -> Result<(ExecutionPlan, Vec<JobActionManifest>), String> {
    let sdk_project = build_workspace(files, env_file, project_name, profiles)
        .analyze()
        .map_err(|error| error.to_string())?;
    let identity = sdk_project
        .identity()
        .cloned()
        .ok_or_else(|| "project has no derivable identity".to_owned())?;

    let snapshot = engine
        .snapshot(&identity)
        .await
        .map_err(|e| e.to_string())?;
    let capabilities = engine.capabilities().await.map_err(|e| e.to_string())?;
    let outcome = sdk_project
        .plan_down(capabilities, snapshot, options)
        .map_err(|error| error.to_string())?;
    let plan = outcome
        .plan
        .ok_or_else(|| "planning was blocked; check project diagnostics".to_owned())?;

    let manifest = job_action_manifest(&plan);
    Ok((plan, manifest))
}

/// Executes an already-planned operation, streaming events and honoring cancel.
pub async fn execute_plan(
    engine: Arc<BollardEngine>,
    plan: ExecutionPlan,
    events: EventSink,
    cancellation: CancellationToken,
) -> Result<ExecutionReport, String> {
    Runtime::new(engine)
        .with_events(events)
        .apply_cancellable(&plan, cancellation)
        .await
        .map_err(|error| error.to_string())
}

/// Builds a named step manifest from a plan's action graph.
pub fn job_action_manifest(plan: &ExecutionPlan) -> Vec<JobActionManifest> {
    plan_action_rows(plan)
        .into_iter()
        .map(|row| JobActionManifest {
            id: row.id,
            action: friendly_action(row.kind).to_owned(),
            resource: row.resource,
        })
        .collect()
}

/// Analyzed project handles needed by runtime operations.
pub struct RuntimeContext {
    pub identity: susun::ProjectIdentity,
    pub project: susun::Project,
}

pub fn runtime_context(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
) -> Result<RuntimeContext, String> {
    let sdk_project = build_workspace(files, env_file, project_name, profiles)
        .analyze()
        .map_err(|error| error.to_string())?;
    let identity = sdk_project
        .identity()
        .cloned()
        .ok_or_else(|| "project has no derivable identity".to_owned())?;
    let project = sdk_project
        .project()
        .cloned()
        .ok_or_else(|| "project failed to analyze".to_owned())?;
    Ok(RuntimeContext { identity, project })
}

pub struct SnapshotContainerRow {
    pub id: String,
    pub name: String,
    pub service: Option<String>,
    pub replica: Option<u32>,
    pub state: String,
    pub health: Option<String>,
    pub image: Option<String>,
}

pub struct SnapshotResourceRow {
    pub id: String,
    pub name: String,
}

pub struct SnapshotRow {
    pub observed_at_ms: i64,
    pub containers: Vec<SnapshotContainerRow>,
    pub networks: Vec<SnapshotResourceRow>,
    pub volumes: Vec<SnapshotResourceRow>,
}

/// Project-scoped live engine state, flattened for the UI.
pub async fn project_snapshot(
    engine: &BollardEngine,
    identity: &susun::ProjectIdentity,
) -> Result<SnapshotRow, String> {
    let snapshot = engine.snapshot(identity).await.map_err(|e| e.to_string())?;
    let observed_at_ms = snapshot
        .observed_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or_default())
        .unwrap_or_default();

    let containers = snapshot
        .containers
        .values()
        .map(|container| SnapshotContainerRow {
            id: container.id.as_str().to_owned(),
            name: container.name.as_str().to_owned(),
            service: container
                .service_identity
                .as_ref()
                .map(|s| s.service.to_string()),
            replica: container
                .service_identity
                .as_ref()
                .map(|s| s.replica.as_u32()),
            state: enum_label(container.state),
            health: container.health.map(enum_label),
            image: match &container.image {
                susun::ObservedImageRef::Id(id) => Some(id.as_str().to_owned()),
                susun::ObservedImageRef::Reference(image) => Some(image.to_string()),
                susun::ObservedImageRef::Unknown => None,
            },
        })
        .collect();

    let networks = snapshot
        .networks
        .values()
        .map(|network| SnapshotResourceRow {
            id: network.id.as_str().to_owned(),
            name: network.name.as_str().to_owned(),
        })
        .collect();
    let volumes = snapshot
        .volumes
        .values()
        .map(|volume| SnapshotResourceRow {
            id: volume.id.as_str().to_owned(),
            name: volume.name.as_str().to_owned(),
        })
        .collect();

    Ok(SnapshotRow {
        observed_at_ms,
        containers,
        networks,
        volumes,
    })
}

/// Containers currently belonging to one service, with their state labels.
pub async fn service_containers(
    engine: &BollardEngine,
    identity: &susun::ProjectIdentity,
    service: &str,
) -> Result<Vec<(susun::ContainerRef, String)>, String> {
    let snapshot = engine.snapshot(identity).await.map_err(|e| e.to_string())?;
    Ok(snapshot
        .containers
        .values()
        .filter(|container| {
            container
                .service_identity
                .as_ref()
                .is_some_and(|s| s.service.to_string() == service)
        })
        .map(|container| {
            (
                susun::ContainerRef {
                    id: container.id.clone(),
                },
                enum_label(container.state),
            )
        })
        .collect())
}

/// Builds a one-off "compose run"-style container request for a service:
/// service env/entrypoint/volumes/networks, optional command override, NO
/// published ports (compose-run default), NO config/secret mounts (v1 gap,
/// surfaced in the UI).
pub fn build_run_request(
    context: &RuntimeContext,
    service: &str,
    command: Option<Vec<String>>,
) -> Result<susun::CreateContainerRequest, String> {
    let (service_name, definition) = context
        .project
        .services
        .iter()
        .find(|(name, _)| name.to_string() == service)
        .ok_or_else(|| format!("service `{service}` not found"))?;
    let image = definition.image.clone().ok_or_else(|| {
        "service has no image (build-only services cannot run one-offs)".to_owned()
    })?;

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = susun::ResourceName::new(format!(
        "{}-{}-run-{}",
        context.identity.name,
        service,
        &suffix[..8]
    ))
    .map_err(|e| e.to_string())?;

    Ok(susun::CreateContainerRequest {
        project: context.identity.clone(),
        service: susun::ServiceInstanceId::new(
            context.identity.working_set.clone(),
            service_name.clone(),
            susun::ReplicaIndex::FIRST,
        ),
        name,
        image: Some(image),
        command: command
            .map(susun::Command::Exec)
            .or_else(|| definition.command.clone()),
        entrypoint: definition.entrypoint.clone(),
        environment: definition.environment.clone(),
        container_labels: definition.labels.clone(),
        ports: Vec::new(),
        volumes: definition.volumes.clone(),
        configs: Vec::new(),
        secrets: Vec::new(),
        networks: definition
            .networks
            .iter()
            .map(|(name, attachment)| {
                let name = susun::ResourceName::new(name.to_string()).map_err(|e| e.to_string())?;
                Ok::<_, String>((name, attachment.clone()))
            })
            .collect::<Result<indexmap::IndexMap<_, _>, String>>()?,
        healthcheck: None,
        restart: None,
        labels: indexmap::IndexMap::new(),
    })
}

/// Maps a stable action kind to a short human verb phrase for the UI.
fn friendly_action(kind: &str) -> &'static str {
    match kind {
        "pull_image" => "Pull",
        "build_image" => "Build",
        "verify_build_inputs" => "Verify build",
        "create_network" => "Create network",
        "create_volume" => "Create volume",
        "create_container" => "Create",
        "start_container" => "Start",
        "wait_for_dependency" => "Wait for",
        "stop_container" => "Stop",
        "remove_container" => "Remove",
        "remove_network" => "Remove network",
        "remove_volume" => "Remove volume",
        "rename_container" => "Rename",
        "recreate_container" => "Recreate",
        "preserve_volume" => "Preserve volume",
        "verify_replacement" => "Verify",
        "remove_orphan" => "Remove orphan",
        "scale_up_replica" => "Scale up",
        "scale_down_replica" => "Scale down",
        "no_op" => "No change",
        _ => "Action",
    }
}

/// Studio-owned view of a system-wide prune result.
pub struct PruneReportRow {
    pub containers_removed: Vec<String>,
    pub networks_removed: Vec<String>,
    pub volumes_removed: Vec<String>,
    pub images_removed: Vec<String>,
    pub space_reclaimed_bytes: u64,
}

/// Runs a system-wide prune. Unknown scope strings are silently ignored —
/// the route validates the request shape; this only interprets it.
pub async fn system_prune(
    engine: &BollardEngine,
    scopes: &[String],
    all_images: bool,
) -> Result<PruneReportRow, String> {
    let scopes = scopes
        .iter()
        .filter_map(|scope| match scope.as_str() {
            "containers" => Some(susun::PruneScope::Containers),
            "networks" => Some(susun::PruneScope::Networks),
            "volumes" => Some(susun::PruneScope::Volumes),
            "images" => Some(susun::PruneScope::Images),
            _ => None,
        })
        .collect();

    let report = engine
        .prune(susun::PruneRequest { scopes, all_images })
        .await
        .map_err(|error| error.to_string())?;

    Ok(PruneReportRow {
        containers_removed: report
            .containers_removed
            .iter()
            .map(ToString::to_string)
            .collect(),
        networks_removed: report
            .networks_removed
            .iter()
            .map(ToString::to_string)
            .collect(),
        volumes_removed: report
            .volumes_removed
            .iter()
            .map(ToString::to_string)
            .collect(),
        images_removed: report
            .images_removed
            .iter()
            .map(ToString::to_string)
            .collect(),
        space_reclaimed_bytes: report.space_reclaimed_bytes,
    })
}

/// Reads a project's `.dockerignore` if present, otherwise no ignore rules.
pub fn resolve_dockerignore(root: &std::path::Path) -> susun::Dockerignore {
    match std::fs::read_to_string(root.join(".dockerignore")) {
        Ok(contents) => susun::Dockerignore::parse(&contents),
        Err(_) => susun::Dockerignore::default(),
    }
}
