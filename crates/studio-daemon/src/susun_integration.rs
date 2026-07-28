use std::{path::PathBuf, sync::Arc, time::SystemTime};

use susun::{
    BuildCancellationToken, BuildDefinition, BuildEngine, BuildEventSink, BuildInputManifest,
    BuildRequest, BuildResult, BuildxProcessBuildEngine, CancellationToken, ContainerEngine,
    DownPlanOptions, EngineCapabilities, EngineEndpoint, EngineSnapshot, EventSink, ExecutionPlan,
    ExecutionReport, ImageRef, ImageRemoveRequest, ImageSelector, ImageTagRequest, PlanOutcome,
    ProjectSummary, Runtime, SdkProject, SusunWorkspace, UpPlanOptions, render_diagnostics_json,
    render_plan_json, resolve_build_inputs, validate_dockerfile_source,
};
use susun_engine_bollard::BollardEngine;
use turso::Database;

use crate::{error::ApiError, runtime};

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

/// Analyzes a project's already-persisted Compose files and returns the full
/// SDK project (service definitions included), for callers that need more
/// than the display-oriented [`analyze_project`] summary — currently, build
/// target discovery.
pub fn analyze_sdk_project(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
) -> Result<SdkProject, susun::Error> {
    build_workspace(files, env_file, project_name, profiles).analyze()
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

/// Constructs a Docker-compatible client handle. A configured project/global
/// profile never silently falls back when unavailable; platform local defaults
/// are used only when the user has not selected or bound a runtime.
pub async fn connect_engine(
    db: &Database,
    project_id: Option<&str>,
) -> Result<BollardEngine, String> {
    let endpoint = match runtime::engine_endpoint_for(db, project_id)
        .await
        .map_err(|error| error.to_string())?
    {
        runtime::EngineEndpointResolution::Explicit(endpoint) => endpoint,
        runtime::EngineEndpointResolution::PlatformDefault => EngineEndpoint::Local,
        runtime::EngineEndpointResolution::Unavailable { profile_id } => {
            return Err(format!(
                "runtime profile `{profile_id}` is unavailable; Studio will not switch engines automatically"
            ));
        }
    };
    // Fail closed on any endpoint Studio is not allowed to reach (e.g. a
    // remote/TCP endpoint): built-in engine access stays OS-scoped and local.
    runtime::validate_engine_endpoint(&endpoint).map_err(|error| error.to_string())?;
    BollardEngine::connect_to(endpoint).map_err(|error| error.to_string())
}

/// Connect to one exact runtime profile. Unlike `connect_engine`, this never
/// follows a later global selection change. `None` explicitly means the local
/// platform default.
pub async fn connect_engine_for_profile(
    db: &Database,
    profile_id: Option<&str>,
) -> Result<BollardEngine, String> {
    let endpoint = match profile_id {
        Some(profile_id) => runtime::endpoint_for_profile(db, profile_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("runtime profile `{profile_id}` is unavailable"))?,
        None => EngineEndpoint::Local,
    };
    runtime::validate_engine_endpoint(&endpoint).map_err(|error| error.to_string())?;
    BollardEngine::connect_to(endpoint).map_err(|error| error.to_string())
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
pub(crate) fn enum_label<T: serde::Serialize>(value: T) -> String {
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

/// A single resource scope in a non-destructive cleanup preview, flattened for
/// Studio. Counts/bytes are engine-reported; `support`/`estimate_kind` say how
/// much to trust them.
pub struct CleanupScopeRow {
    pub scope: String,
    pub support: String,
    pub candidate_count: Option<u64>,
    pub reclaimable_bytes: Option<u64>,
    pub estimate_kind: String,
}

/// Server-derived, non-destructive prune inventory. Produced by the engine, never
/// by the frontend.
pub struct CleanupPreviewRow {
    pub scopes: Vec<CleanupScopeRow>,
}

/// Maps Studio scope strings to SDK scopes. Unknown strings are ignored.
fn prune_scopes(scopes: &[String]) -> Vec<susun::PruneScope> {
    scopes
        .iter()
        .filter_map(|scope| match scope.as_str() {
            "containers" => Some(susun::PruneScope::Containers),
            "networks" => Some(susun::PruneScope::Networks),
            "volumes" => Some(susun::PruneScope::Volumes),
            "images" => Some(susun::PruneScope::Images),
            "build_cache" => Some(susun::PruneScope::BuildCache),
            _ => None,
        })
        .collect()
}

/// Collects a non-destructive cleanup preview (counts + reclaim estimate) for the
/// requested scopes. Never prunes. Errors if the engine cannot provide it.
pub async fn cleanup_preview(
    engine: &BollardEngine,
    scopes: &[String],
    all_images: bool,
) -> Result<CleanupPreviewRow, String> {
    let request = susun::PruneRequest {
        scopes: prune_scopes(scopes),
        all_images,
    };
    let preview = engine
        .cleanup_preview(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(CleanupPreviewRow {
        scopes: preview
            .scopes
            .iter()
            .map(|scope| CleanupScopeRow {
                scope: enum_label(scope.scope),
                support: enum_label(scope.support),
                candidate_count: scope.candidate_count,
                reclaimable_bytes: scope.reclaimable_bytes,
                estimate_kind: enum_label(scope.estimate_kind),
            })
            .collect(),
    })
}

/// Runs a system-wide prune. Unknown scope strings are silently ignored —
/// the route validates the request shape; this only interprets it.
pub async fn system_prune(
    engine: &BollardEngine,
    scopes: &[String],
    all_images: bool,
) -> Result<PruneReportRow, String> {
    let report = engine
        .prune(susun::PruneRequest {
            scopes: prune_scopes(scopes),
            all_images,
        })
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

/// Display-safe result of adding a new reference to an existing image.
pub struct ImageTagRow {
    pub source: String,
    pub target: String,
}

/// Adds `target` (a `repository:tag` reference) to the image identified by
/// `source` (an opaque engine id or existing reference). Never removes or
/// replaces any existing reference.
pub async fn tag_image(
    engine: &BollardEngine,
    source: &str,
    target: &str,
) -> Result<ImageTagRow, String> {
    let source = ImageSelector::new(source.to_owned()).map_err(|error| error.to_string())?;
    let target = ImageRef::new(target.to_owned());
    let request = ImageTagRequest::new(source, target);
    let result = engine
        .tag_image(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ImageTagRow {
        source: result.source.as_str().to_owned(),
        target: result.target.as_str().to_owned(),
    })
}

/// Display-safe result of removing one image.
pub struct ImageRemoveRow {
    pub deleted: Vec<String>,
    pub untagged: Vec<String>,
}

/// Removes one image, identified by an opaque engine id or existing
/// reference. `force` allows removal of an image still referenced by a
/// stopped container; untagged parent layers are left alone.
pub async fn remove_image(
    engine: &BollardEngine,
    image: &str,
    force: bool,
) -> Result<ImageRemoveRow, String> {
    let image = ImageSelector::new(image.to_owned()).map_err(|error| error.to_string())?;
    let request = ImageRemoveRequest::new(image).with_force(force);
    let result = engine
        .remove_image(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ImageRemoveRow {
        deleted: result.deleted.iter().map(ToString::to_string).collect(),
        untagged: result.untagged.iter().map(ToString::to_string).collect(),
    })
}

/// Reads a project's `.dockerignore` if present, otherwise no ignore rules.
pub fn resolve_dockerignore(root: &std::path::Path) -> susun::Dockerignore {
    match std::fs::read_to_string(root.join(".dockerignore")) {
        Ok(contents) => susun::Dockerignore::parse(&contents),
        Err(_) => susun::Dockerignore::default(),
    }
}

/// One build-declared service in a project, server-resolved from its already
/// analyzed Compose files — never accepted as free-form input from the
/// frontend.
pub struct BuildableServiceRow {
    pub service_name: String,
    /// Whether the service also declares `image:` — Compose builds and tags
    /// as that reference when both are present; Studio synthesizes a name
    /// otherwise (see [`default_build_image_tag`]).
    pub has_image: bool,
    /// Whether this build declares secrets or SSH forwarding — both require
    /// resolving local file paths or agent details Studio does not handle
    /// in this phase, so builds requesting them are rejected up front
    /// rather than silently started without them.
    pub requires_unsupported_build_inputs: bool,
}

/// Lists every service with a `build:` declaration in an already-analyzed
/// project. Server-side only: the caller must have obtained `project` via
/// the project's own persisted Compose files, never from client input.
pub fn buildable_services(project: &susun::Project) -> Vec<BuildableServiceRow> {
    project
        .services
        .iter()
        .filter_map(|(name, service)| {
            service.build.as_ref().map(|build| BuildableServiceRow {
                service_name: name.as_str().to_owned(),
                has_image: service.image.is_some(),
                requires_unsupported_build_inputs: !build.secrets.is_empty()
                    || !build.ssh.is_empty(),
            })
        })
        .collect()
}

/// The image reference a build for `service_name` will be tagged as: the
/// service's own `image:` when set (matching Compose's own build+image
/// semantics — the built image replaces what that reference points to),
/// otherwise a Studio-owned `{project}-{service}:latest` convention. This is
/// Studio's own naming choice, not derived from any Compose default-naming
/// spec — services without an explicit `image:` have no other stable
/// identity to build under.
pub fn default_build_image_tag(
    project_name: &str,
    service_name: &str,
    image: Option<&ImageRef>,
) -> String {
    match image {
        Some(image) => image.as_str().to_owned(),
        None => format!("{project_name}-{service_name}:latest"),
    }
}

/// Server-resolved, validated build inputs ready to hand to a `BuildEngine`.
/// Every path here has already been canonicalized and confirmed to stay
/// within the project directory — never derived from unchecked client input.
pub struct PreparedBuild {
    pub context_dir: PathBuf,
    pub dockerfile: PathBuf,
    pub manifest: BuildInputManifest,
}

/// Bounded, redacted reason a build could not be prepared — never wraps the
/// underlying SDK error types directly, since their `Display` output can
/// legitimately contain raw host paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareBuildError {
    /// `resolve_build_inputs` rejected the context or Dockerfile location —
    /// an IO failure, or (the security-relevant case) the resolved path
    /// escaped the project directory, including via a symlink or reparse
    /// point that `canonicalize` resolved to somewhere outside it.
    PathResolution,
    /// `validate_dockerfile_source` rejected the resolved Dockerfile (not a
    /// regular file, or an invalid target stage name).
    DockerfileInvalid,
    /// `BuildInputManifest::from_context` could not enumerate or hash the
    /// build context.
    ManifestFailed,
}

impl PrepareBuildError {
    pub fn code(self) -> &'static str {
        match self {
            Self::PathResolution => "build_path_resolution_failed",
            Self::DockerfileInvalid => "build_dockerfile_invalid",
            Self::ManifestFailed => "build_context_unreadable",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::PathResolution => {
                "The build context or Dockerfile location could not be resolved within the project."
            }
            Self::DockerfileInvalid => "The resolved Dockerfile is invalid.",
            Self::ManifestFailed => "The build context could not be read.",
        }
    }
}

/// Resolves and validates a service's build context and Dockerfile against
/// `project_dir` (the project's own canonical root, never a client-supplied
/// path) using the Susun facade's own path-containment and Dockerfile
/// validation helpers, then hashes the context into a deterministic
/// manifest. This is deliberately synchronous and potentially slow (it walks
/// and hashes the whole context directory) — callers run it from within a
/// spawned job task, after the job already exists in a `queued` state, never
/// inline in the HTTP handler that creates it.
pub fn prepare_build(
    project_dir: &std::path::Path,
    definition: &BuildDefinition,
) -> Result<PreparedBuild, PrepareBuildError> {
    let paths = resolve_build_inputs(project_dir, definition)
        .map_err(|_| PrepareBuildError::PathResolution)?;
    validate_dockerfile_source(&paths.dockerfile, definition.target.as_deref())
        .map_err(|_| PrepareBuildError::DockerfileInvalid)?;
    let dockerignore = resolve_dockerignore(&paths.context_dir);
    let manifest = BuildInputManifest::from_context(&paths.context_dir, &dockerignore)
        .map_err(|_| PrepareBuildError::ManifestFailed)?;
    Ok(PreparedBuild {
        context_dir: paths.context_dir,
        dockerfile: paths.dockerfile,
        manifest,
    })
}

/// Display-safe, redacted outcome of a completed image build.
pub struct BuildResultRow {
    pub image_reference: String,
    pub image_digest: Option<String>,
}

impl From<BuildResult> for BuildResultRow {
    fn from(result: BuildResult) -> Self {
        Self {
            image_reference: result.image.reference,
            image_digest: result.image.digest,
        }
    }
}

/// Runs an image build through the buildx process adapter — the only
/// concrete `BuildEngine` the public facade exposes. Bound to whatever
/// `docker` CLI context is ambient in the daemon's own process environment;
/// it does not accept or route through a specific `EngineEndpoint`, so a
/// build may not exactly target a non-default selected runtime profile (see
/// the Phase 15d PR notes). Never reports success until the provider itself
/// returns a result.
pub async fn run_build(
    prepared: &PreparedBuild,
    definition: &BuildDefinition,
    image_tag: &str,
    events: BuildEventSink,
    cancellation: BuildCancellationToken,
) -> Result<BuildResultRow, susun::BuildError> {
    let engine = BuildxProcessBuildEngine::default();
    let request = BuildRequest {
        definition: definition.clone(),
        context_dir: prepared.context_dir.clone(),
        dockerfile: prepared.dockerfile.clone(),
        manifest: prepared.manifest.clone(),
        image_tag: Some(image_tag.to_owned()),
        // Secrets/SSH are rejected up front by `buildable_services`'
        // `requires_unsupported_build_inputs` gate — never reached here.
        secrets: Vec::new(),
        ssh: Vec::new(),
        cache_from: Vec::new(),
        cache_to: Vec::new(),
        insecure_entitlements: susun::InsecureEntitlements::default(),
        labels: indexmap::IndexMap::new(),
    };
    let result = engine.build(request, events, cancellation).await?;
    Ok(BuildResultRow::from(result))
}

#[cfg(test)]
mod build_tests {
    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn unique_temp_dir_early(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("studio-{label}-{}", uuid::Uuid::new_v4().simple()))
    }

    /// Builds a real `Project` by writing a small Compose fixture to disk
    /// and running it through the facade's own analyzer — `susun::Service`
    /// is not re-exported (only `Project`/`BuildDefinition`/etc. are, since
    /// production code never needs to construct one, only read fields off
    /// values the analyzer already produced), so this is the only way test
    /// code can produce project fixtures with build declarations without
    /// reaching past the facade.
    fn analyzed_sample_project(dir: &std::path::Path) -> TestResult<susun::SdkProject> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join("docker-compose.yml"),
            r#"
services:
  web:
    build:
      context: .
    image: myapp-web:latest
  worker:
    build:
      context: .
      secrets:
        - api_key
  db:
    image: postgres:16
secrets:
  api_key:
    environment: API_KEY
"#,
        )?;
        Ok(analyze_sdk_project(
            &[dir.join("docker-compose.yml")],
            None,
            Some("myapp"),
            &[],
        )?)
    }

    /// `db` has no `build:` at all and must be excluded entirely; `worker`
    /// declares a build secret Studio does not resolve in this phase and
    /// must be flagged unsupported rather than silently started without it;
    /// `web` is the plain, fully-supported case.
    #[test]
    fn buildable_services_excludes_non_build_services_and_flags_unsupported_inputs() -> TestResult {
        let dir = unique_temp_dir_early("buildable-services");
        let sdk_project = analyzed_sample_project(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let sdk_project = sdk_project?;
        let project = sdk_project
            .project()
            .ok_or("analysis did not produce a project")?;

        let mut rows = buildable_services(project);
        rows.sort_by(|a, b| a.service_name.cmp(&b.service_name));

        assert_eq!(
            rows.iter()
                .map(|row| row.service_name.as_str())
                .collect::<Vec<_>>(),
            vec!["web", "worker"]
        );
        assert!(rows[0].has_image);
        assert!(!rows[0].requires_unsupported_build_inputs);
        assert!(!rows[1].has_image);
        assert!(rows[1].requires_unsupported_build_inputs);
        Ok(())
    }

    #[test]
    fn default_build_image_tag_prefers_the_declared_image() {
        let image = ImageRef::new("myapp-web:latest");
        assert_eq!(
            default_build_image_tag("myapp", "web", Some(&image)),
            "myapp-web:latest"
        );
    }

    #[test]
    fn default_build_image_tag_synthesizes_a_name_when_no_image_is_declared() {
        assert_eq!(
            default_build_image_tag("myapp", "worker", None),
            "myapp-worker:latest"
        );
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("studio-{label}-{}", uuid::Uuid::new_v4().simple()))
    }

    fn write_file(path: &std::path::Path, contents: &str) -> TestResult {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// The happy path, end to end against a real filesystem: a normal
    /// context resolves, the Dockerfile validates, and the manifest hashes
    /// every included file.
    #[test]
    fn prepare_build_accepts_a_context_within_the_project_directory() -> TestResult {
        let dir = unique_temp_dir("build-ok");
        write_file(&dir.join("Dockerfile"), "FROM scratch\n")?;
        write_file(&dir.join("app.txt"), "hello\n")?;

        let result = prepare_build(&dir, &BuildDefinition::default());
        std::fs::remove_dir_all(&dir).ok();

        let prepared = result.map_err(|error| format!("{error:?}"))?;
        assert!(prepared.dockerfile.ends_with("Dockerfile"));
        assert!(
            prepared
                .manifest
                .entries
                .iter()
                .any(|entry| entry.path == "app.txt")
        );
        Ok(())
    }

    /// The security-critical case: a `build.context` that resolves (after
    /// canonicalization, so this also covers a symlink pointing the same
    /// way) outside the project directory must be rejected, never silently
    /// followed.
    #[test]
    fn prepare_build_rejects_a_context_that_escapes_the_project_directory() -> TestResult {
        let root = unique_temp_dir("build-escape");
        let project_dir = root.join("project");
        let outside_dir = root.join("outside");
        std::fs::create_dir_all(&project_dir)?;
        write_file(&outside_dir.join("Dockerfile"), "FROM scratch\n")?;

        let definition = BuildDefinition {
            context: Some("../outside".to_owned()),
            ..Default::default()
        };
        let result = prepare_build(&project_dir, &definition);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(result.err(), Some(PrepareBuildError::PathResolution));
        Ok(())
    }

    /// Same rejection path, reached via a missing Dockerfile instead of an
    /// escaping context — `resolve_build_inputs` itself fails to canonicalize
    /// a path that doesn't exist, before `validate_dockerfile_source` is
    /// ever reached.
    #[test]
    fn prepare_build_rejects_a_missing_dockerfile() -> TestResult {
        let dir = unique_temp_dir("build-missing-dockerfile");
        std::fs::create_dir_all(&dir)?;

        let result = prepare_build(&dir, &BuildDefinition::default());
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.err(), Some(PrepareBuildError::PathResolution));
        Ok(())
    }

    /// `validate_dockerfile_source` itself is reached and rejects a
    /// `Dockerfile` path that resolves but is not a regular file (here, a
    /// directory of that name).
    #[test]
    fn prepare_build_rejects_a_dockerfile_that_is_not_a_regular_file() -> TestResult {
        let dir = unique_temp_dir("build-dockerfile-not-a-file");
        std::fs::create_dir_all(dir.join("Dockerfile"))?;

        let result = prepare_build(&dir, &BuildDefinition::default());
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.err(), Some(PrepareBuildError::DockerfileInvalid));
        Ok(())
    }
}
