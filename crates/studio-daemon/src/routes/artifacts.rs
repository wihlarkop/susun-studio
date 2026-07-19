//! Capability-gated endpoints for engine-wide artifacts: images, containers,
//! build cache, and registry capability. Listing and detail routes are
//! read-only. Image tag and remove are opaque-plan mutations mirroring
//! `routes::engines`'s prune preview/commit envelope exactly: a preview
//! mints a commit plan only when the capability is supported and nothing is
//! actively running, and commit revalidates engine identity, image
//! inventory, and active work immediately before calling the provider.
//! Image and build-cache pruning reuse `routes::engines`'s existing prune
//! preview/commit endpoints directly (scopes `"images"` / `"build_cache"`)
//! rather than duplicating that envelope here. Build execution, pull, push,
//! and container lifecycle mutations remain out of scope.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use super::engines::{self, resolve_and_validate_engine};
use crate::{
    action_audit::{self, AffectedCount, AuditEntry},
    action_plans::{ActionKind, ActionPlanPayload, ImageRemovePlan, ImageTagPlan, PlanState},
    artifact_inventory::{self, DetailLookup, RuntimeContextRow},
    auth::authorize,
    error::ApiError,
    logging, runtime,
    state::AppState,
    susun_integration,
};

/// Selected-runtime attribution attached to every engine-wide artifact
/// response, so built-in and external runtimes stay distinguishable.
#[derive(Debug, Serialize)]
pub struct ArtifactRuntimeContext {
    pub runtime_profile_id: Option<String>,
    pub runtime_class: Option<String>,
    pub display_name: Option<String>,
    pub is_selected: Option<bool>,
}

impl From<RuntimeContextRow> for ArtifactRuntimeContext {
    fn from(row: RuntimeContextRow) -> Self {
        Self {
            runtime_profile_id: row.runtime_profile_id,
            runtime_class: row.runtime_class,
            display_name: row.display_name,
            is_selected: row.is_selected,
        }
    }
}

/// Resolves and validates `requested_engine_id` exactly once, then connects
/// using that same resolution's `runtime_profile_id` — never a fresh
/// `attribution_for` call. Resolving twice (once to validate the path, once
/// to connect) would open a race: a concurrent profile switch between the
/// two queries could validate one engine's identity while actually
/// connecting to a different one.
async fn connect_selected_engine(
    state: &AppState,
    requested_engine_id: &str,
) -> Result<
    (
        susun::DockerCompatibleEngine,
        String,
        ArtifactRuntimeContext,
    ),
    ApiError,
> {
    let resolved = resolve_and_validate_engine(state, requested_engine_id).await?;
    let engine = susun_integration::connect_engine_for_profile(
        &state.db,
        resolved.runtime_profile_id.as_deref(),
    )
    .await
    .map_err(ApiError::EngineUnavailable)?;
    let runtime_ctx =
        artifact_inventory::runtime_context(&state.db, resolved.runtime_profile_id.as_deref())
            .await?
            .into();
    Ok((engine, resolved.engine_id, runtime_ctx))
}

#[derive(Debug, Serialize)]
pub struct ContainerArtifactSummary {
    pub id: String,
    pub name: String,
    pub state: String,
    pub health: Option<String>,
    pub image_reference: Option<String>,
    pub label_keys: Vec<String>,
    pub known_project_id: Option<String>,
    pub created_at_epoch_seconds: Option<u64>,
    pub writable_size_bytes: Option<u64>,
    pub root_filesystem_size_bytes: Option<u64>,
}

impl From<artifact_inventory::ContainerSummaryRow> for ContainerArtifactSummary {
    fn from(row: artifact_inventory::ContainerSummaryRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            state: row.state,
            health: row.health,
            image_reference: row.image_reference,
            label_keys: row.label_keys,
            known_project_id: row.known_project_id,
            created_at_epoch_seconds: row.created_at_epoch_seconds,
            writable_size_bytes: row.writable_size_bytes,
            root_filesystem_size_bytes: row.root_filesystem_size_bytes,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EngineContainerInventoryResponse {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// SDK support level for engine-wide container inventory on this
    /// provider ("supported", "unsupported", "unknown", ...).
    pub capability: String,
    pub observed_at_epoch_seconds: Option<u64>,
    pub containers: Vec<ContainerArtifactSummary>,
}

pub async fn list_engine_containers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
) -> Result<Json<EngineContainerInventoryResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, engine_id, runtime_ctx) = connect_selected_engine(&state, &engine_id).await?;

    let result = artifact_inventory::container_inventory(&state.db, &engine).await?;

    logging::info(
        "engine_containers_listed",
        &[
            ("engine_id", engine_id.clone()),
            ("capability", result.capability.clone()),
            (
                "container_count",
                result
                    .data
                    .as_ref()
                    .map(|inventory| inventory.containers.len())
                    .unwrap_or_default()
                    .to_string(),
            ),
        ],
    );

    Ok(Json(EngineContainerInventoryResponse {
        engine_id,
        runtime: runtime_ctx,
        capability: result.capability,
        observed_at_epoch_seconds: result
            .data
            .as_ref()
            .map(|inventory| inventory.observed_at_epoch_seconds),
        containers: result
            .data
            .map(|inventory| inventory.containers.into_iter().map(Into::into).collect())
            .unwrap_or_default(),
    }))
}

#[derive(Debug, Serialize)]
pub struct ContainerArtifactDetailResponse {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// SDK support level for engine-wide container inventory on this
    /// provider. `container` is only ever absent when this reads
    /// "unsupported" — a missing id on a supported provider is a 404, not a
    /// null field.
    pub capability: String,
    pub container: Option<ContainerArtifactSummary>,
}

/// Maps a capability-gated detail lookup to its response shape. Pulled out
/// as a pure function so the Found/Unsupported/NotFound mapping can be
/// tested directly, without needing a live engine connection.
fn container_detail_response(
    engine_id: String,
    runtime_ctx: ArtifactRuntimeContext,
    lookup: DetailLookup<artifact_inventory::ContainerSummaryRow>,
) -> Result<ContainerArtifactDetailResponse, ApiError> {
    match lookup {
        DetailLookup::Found { capability, value } => Ok(ContainerArtifactDetailResponse {
            engine_id,
            runtime: runtime_ctx,
            capability,
            container: Some(value.into()),
        }),
        DetailLookup::Unsupported { capability } => Ok(ContainerArtifactDetailResponse {
            engine_id,
            runtime: runtime_ctx,
            capability,
            container: None,
        }),
        DetailLookup::NotFound => Err(ApiError::ArtifactNotFound),
    }
}

pub async fn read_engine_container(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((engine_id, container_id)): Path<(String, String)>,
) -> Result<Json<ContainerArtifactDetailResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, engine_id, runtime_ctx) = connect_selected_engine(&state, &engine_id).await?;

    let lookup = artifact_inventory::container_details(&state.db, &engine, &container_id).await?;

    Ok(Json(container_detail_response(
        engine_id,
        runtime_ctx,
        lookup,
    )?))
}

#[derive(Debug, Serialize)]
pub struct ImageArtifactSummary {
    pub id: String,
    pub references: Vec<String>,
    pub digests: Vec<String>,
    pub label_keys: Vec<String>,
    pub created_at_epoch_seconds: Option<u64>,
    pub size_bytes: Option<u64>,
    pub shared_size_bytes: Option<u64>,
    pub container_count: Option<u64>,
}

impl From<artifact_inventory::ImageSummaryRow> for ImageArtifactSummary {
    fn from(row: artifact_inventory::ImageSummaryRow) -> Self {
        Self {
            id: row.id,
            references: row.references,
            digests: row.digests,
            label_keys: row.label_keys,
            created_at_epoch_seconds: row.created_at_epoch_seconds,
            size_bytes: row.size_bytes,
            shared_size_bytes: row.shared_size_bytes,
            container_count: row.container_count,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EngineImageInventoryResponse {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// SDK support level for engine-wide image inventory on this provider.
    pub capability: String,
    pub observed_at_epoch_seconds: Option<u64>,
    pub images: Vec<ImageArtifactSummary>,
}

pub async fn list_engine_images(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
) -> Result<Json<EngineImageInventoryResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, engine_id, runtime_ctx) = connect_selected_engine(&state, &engine_id).await?;

    let result = artifact_inventory::image_inventory(&engine).await?;

    logging::info(
        "engine_images_listed",
        &[
            ("engine_id", engine_id.clone()),
            ("capability", result.capability.clone()),
            (
                "image_count",
                result
                    .data
                    .as_ref()
                    .map(|inventory| inventory.images.len())
                    .unwrap_or_default()
                    .to_string(),
            ),
        ],
    );

    Ok(Json(EngineImageInventoryResponse {
        engine_id,
        runtime: runtime_ctx,
        capability: result.capability,
        observed_at_epoch_seconds: result
            .data
            .as_ref()
            .map(|inventory| inventory.observed_at_epoch_seconds),
        images: result
            .data
            .map(|inventory| inventory.images.into_iter().map(Into::into).collect())
            .unwrap_or_default(),
    }))
}

#[derive(Debug, Serialize)]
pub struct ImageArtifactDetailResponse {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// SDK support level for engine-wide image inventory on this provider.
    /// `image` is only ever absent when this reads "unsupported" — a
    /// missing id on a supported provider is a 404, not a null field.
    pub capability: String,
    pub image: Option<ImageArtifactSummary>,
}

/// Maps a capability-gated detail lookup to its response shape. Pulled out
/// as a pure function so the Found/Unsupported/NotFound mapping can be
/// tested directly, without needing a live engine connection.
fn image_detail_response(
    engine_id: String,
    runtime_ctx: ArtifactRuntimeContext,
    lookup: DetailLookup<artifact_inventory::ImageSummaryRow>,
) -> Result<ImageArtifactDetailResponse, ApiError> {
    match lookup {
        DetailLookup::Found { capability, value } => Ok(ImageArtifactDetailResponse {
            engine_id,
            runtime: runtime_ctx,
            capability,
            image: Some(value.into()),
        }),
        DetailLookup::Unsupported { capability } => Ok(ImageArtifactDetailResponse {
            engine_id,
            runtime: runtime_ctx,
            capability,
            image: None,
        }),
        DetailLookup::NotFound => Err(ApiError::ArtifactNotFound),
    }
}

pub async fn read_engine_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((engine_id, image_id)): Path<(String, String)>,
) -> Result<Json<ImageArtifactDetailResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, engine_id, runtime_ctx) = connect_selected_engine(&state, &engine_id).await?;

    let lookup = artifact_inventory::image_details(&engine, &image_id).await?;

    Ok(Json(image_detail_response(engine_id, runtime_ctx, lookup)?))
}

/// Fingerprints an image's engine-reported identity (references, digests,
/// size) at preview time so a commit can detect any change since preview.
fn image_fingerprint(image: &artifact_inventory::ImageSummaryRow) -> String {
    let mut canonical = format!(
        "id={};size={};shared_size={};",
        image.id,
        image.size_bytes.unwrap_or_default(),
        image.shared_size_bytes.unwrap_or_default()
    );
    for reference in &image.references {
        canonical.push_str(reference);
        canonical.push('|');
    }
    for digest in &image.digests {
        canonical.push_str(digest);
        canonical.push('|');
    }
    runtime::stable_suffix(&canonical)
}

#[derive(Debug, Deserialize)]
pub struct TagImageRequest {
    pub target_reference: String,
}

/// Non-mutating tag preview: confirms the provider advertises tagging
/// support and the source image still exists, then mints a commit plan
/// pinned to both. A commit plan is minted only when the capability is
/// supported, the source image is found, and nothing is actively running
/// against this engine.
#[derive(Debug, Serialize)]
pub struct ImageTagPreview {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// SDK support level for image tagging on this provider.
    pub capability: String,
    pub source_image_id: String,
    pub source_references: Vec<String>,
    pub target_reference: String,
    pub active_jobs: i64,
    pub active_watch_sessions: i64,
    pub commit_enabled: bool,
    pub plan_id: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ImageTagResponse {
    pub source: String,
    pub target: String,
}

pub async fn preview_tag_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((engine_id, image_id)): Path<(String, String)>,
    Json(request): Json<TagImageRequest>,
) -> Result<Json<ImageTagPreview>, ApiError> {
    authorize(&state, &headers)?;
    let resolved = resolve_and_validate_engine(&state, &engine_id).await?;
    let engine_id = resolved.engine_id;
    let runtime_profile_id = resolved.runtime_profile_id;
    let owner = runtime::stable_suffix(&state.auth_token);

    let target_reference = request.target_reference.trim().to_owned();
    if target_reference.is_empty() {
        return Err(ApiError::ActionUnavailable(
            "A target repository:tag reference is required.".to_owned(),
        ));
    }

    // Connect using the exact profile just resolved above — never
    // re-resolve selection here, for the same race-avoidance reason as the
    // read-only artifact routes.
    let engine =
        susun_integration::connect_engine_for_profile(&state.db, runtime_profile_id.as_deref())
            .await
            .map_err(ApiError::EngineUnavailable)?;
    let runtime_ctx = artifact_inventory::runtime_context(&state.db, runtime_profile_id.as_deref())
        .await?
        .into();

    let lookup = artifact_inventory::image_for_mutation(&engine, &image_id).await?;
    let (active_jobs, active_watch_sessions) =
        engines::engine_active_work(&state.db, runtime_profile_id.as_deref()).await?;
    let has_active_work = active_jobs > 0 || active_watch_sessions > 0;

    let (capability, source) = match lookup {
        DetailLookup::Unsupported { capability } => {
            logging::warn(
                "image_tag_previewed",
                &[
                    ("engine_id", engine_id.clone()),
                    ("capability", capability.clone()),
                ],
            );
            return Ok(Json(ImageTagPreview {
                engine_id,
                runtime: runtime_ctx,
                capability,
                source_image_id: image_id,
                source_references: Vec::new(),
                target_reference,
                active_jobs,
                active_watch_sessions,
                commit_enabled: false,
                plan_id: None,
                expires_in_seconds: None,
            }));
        }
        DetailLookup::NotFound => return Err(ApiError::ArtifactNotFound),
        DetailLookup::Found { capability, value } => (capability, value),
    };

    let (plan_id, expires_in_seconds, commit_enabled) = if has_active_work {
        (None, None, false)
    } else {
        let identity_fingerprint =
            engines::engine_identity_fingerprint(&state.db, &engine, runtime_profile_id.as_deref())
                .await;
        let ticket = state.action_plans.prepare(
            &owner,
            ActionKind::ImageTag,
            ActionPlanPayload::ImageTag(ImageTagPlan {
                engine_id: engine_id.clone(),
                runtime_profile_id: runtime_profile_id.clone(),
                source_image_id: source.id.clone(),
                target_reference: target_reference.clone(),
                identity_fingerprint,
                source_fingerprint: image_fingerprint(&source),
            }),
        );
        (Some(ticket.plan_id), Some(ticket.expires_in_seconds), true)
    };

    logging::warn(
        "image_tag_previewed",
        &[
            ("engine_id", engine_id.clone()),
            ("commit_enabled", commit_enabled.to_string()),
        ],
    );

    Ok(Json(ImageTagPreview {
        engine_id,
        runtime: runtime_ctx,
        capability,
        source_image_id: source.id,
        source_references: source.references,
        target_reference,
        active_jobs,
        active_watch_sessions,
        commit_enabled,
        plan_id,
        expires_in_seconds,
    }))
}

pub async fn commit_tag_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<ImageTagResponse>, ApiError> {
    authorize(&state, &headers)?;
    engines::reject_commit_body(&body)?;
    let owner = runtime::stable_suffix(&state.auth_token);
    let started = engines::now_ms()?;

    let claimed = state
        .action_plans
        .claim(&plan_id, &owner, &[ActionKind::ImageTag])
        .map_err(|error| ApiError::ActionUnavailable(error.to_string()))?;
    let ActionPlanPayload::ImageTag(plan) = claimed.payload else {
        state
            .action_plans
            .finish(&claimed.plan_id, PlanState::Failed);
        return Err(ApiError::ActionUnavailable(
            "Plan did not match an image tag.".to_owned(),
        ));
    };

    logging::warn(
        "image_tag_started",
        &[("engine_id", plan.engine_id.clone())],
    );

    async fn reject(
        state: &AppState,
        plan_id: &str,
        profile_id: Option<String>,
        code: &'static str,
        message: String,
    ) -> ApiError {
        state.action_plans.finish(plan_id, PlanState::Failed);
        let _ = action_audit::record_rejection(
            &state.db,
            ActionKind::ImageTag,
            profile_id,
            "rejected_state",
            code,
        )
        .await;
        ApiError::ActionUnavailable(message)
    }

    let engine = match susun_integration::connect_engine_for_profile(
        &state.db,
        plan.runtime_profile_id.as_deref(),
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::ImageTag,
                plan.runtime_profile_id.clone(),
                "rejected_state",
                "provider_unreachable",
            )
            .await;
            return Err(ApiError::EngineUnavailable(error));
        }
    };

    if engines::engine_identity_fingerprint(&state.db, &engine, plan.runtime_profile_id.as_deref())
        .await
        != plan.identity_fingerprint
    {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "engine_identity_changed",
            "The selected engine or its endpoint changed since preview. Preview the tag again."
                .to_owned(),
        )
        .await);
    }

    let lookup = match artifact_inventory::image_for_mutation(&engine, &plan.source_image_id).await
    {
        Ok(lookup) => lookup,
        Err(_) => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "inventory_unavailable",
                "Image inventory is no longer available. Preview the tag again.".to_owned(),
            )
            .await);
        }
    };
    let source = match lookup {
        DetailLookup::Found { value, .. } => value,
        DetailLookup::NotFound => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "source_image_missing",
                "The source image no longer exists. Preview the tag again.".to_owned(),
            )
            .await);
        }
        DetailLookup::Unsupported { .. } => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "capability_withdrawn",
                "Image tagging is no longer supported on this engine.".to_owned(),
            )
            .await);
        }
    };
    if image_fingerprint(&source) != plan.source_fingerprint {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "stale_inventory",
            "The source image changed since preview. Preview the tag again.".to_owned(),
        )
        .await);
    }

    let (jobs, watch) =
        match engines::engine_active_work(&state.db, plan.runtime_profile_id.as_deref()).await {
            Ok(active) => active,
            Err(_) => {
                return Err(reject(
                    &state,
                    &claimed.plan_id,
                    plan.runtime_profile_id.clone(),
                    "active_work_unavailable",
                    "Active work could not be verified. Preview the tag again.".to_owned(),
                )
                .await);
            }
        };
    if jobs > 0 || watch > 0 {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "active_work",
            "A job or watch session is running. Stop it and preview the tag again.".to_owned(),
        )
        .await);
    }

    match engines::revalidate_engine_still_selected(&state.db, &plan.engine_id).await {
        Ok(true) => {}
        Ok(false) => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "engine_removed_or_changed",
                "The selected engine changed or is no longer available since preview. Preview the tag again."
                    .to_owned(),
            )
            .await);
        }
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, PlanState::Failed);
            return Err(error);
        }
    }

    let result =
        match susun_integration::tag_image(&engine, &plan.source_image_id, &plan.target_reference)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                state
                    .action_plans
                    .finish(&claimed.plan_id, PlanState::Failed);
                let _ = action_audit::record_rejection(
                    &state.db,
                    ActionKind::ImageTag,
                    plan.runtime_profile_id.clone(),
                    "failed",
                    "tag_failed",
                )
                .await;
                return Err(ApiError::EngineUnavailable(error));
            }
        };

    state
        .action_plans
        .finish(&claimed.plan_id, PlanState::Succeeded);
    let _ = action_audit::record(
        &state.db,
        AuditEntry {
            kind: ActionKind::ImageTag,
            profile_id: plan.runtime_profile_id,
            runtime_class: None,
            ownership_result: "authorized".to_owned(),
            command_kind: Some("provider_image_tag".to_owned()),
            elevation_mode: Some("none".to_owned()),
            terminal_status: action_audit::STATUS_COMPLETED.to_owned(),
            affected: vec![AffectedCount {
                category: "images_tagged".to_owned(),
                count: 1,
            }],
            failure_code: None,
            correlation_token: None,
            started_at_ms: started,
            completed_at_ms: Some(engines::now_ms()?),
        },
    )
    .await;

    logging::warn(
        "image_tag_finished",
        &[("engine_id", plan.engine_id.clone())],
    );

    Ok(Json(ImageTagResponse {
        source: result.source,
        target: result.target,
    }))
}

/// Non-mutating remove preview: confirms the provider advertises removal
/// support and the image still exists, then mints a commit plan pinned to
/// it. `estimated_reclaim_bytes` is the image's own reported size — a
/// best-effort estimate, since shared layers may not all be reclaimed.
#[derive(Debug, Serialize)]
pub struct ImageRemovePreview {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// SDK support level for image removal on this provider.
    pub capability: String,
    pub image_id: String,
    pub references: Vec<String>,
    pub digests: Vec<String>,
    pub estimated_reclaim_bytes: Option<u64>,
    pub active_jobs: i64,
    pub active_watch_sessions: i64,
    pub commit_enabled: bool,
    pub plan_id: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ImageRemoveResponse {
    pub deleted: Vec<String>,
    pub untagged: Vec<String>,
}

pub async fn preview_remove_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((engine_id, image_id)): Path<(String, String)>,
) -> Result<Json<ImageRemovePreview>, ApiError> {
    authorize(&state, &headers)?;
    let resolved = resolve_and_validate_engine(&state, &engine_id).await?;
    let engine_id = resolved.engine_id;
    let runtime_profile_id = resolved.runtime_profile_id;
    let owner = runtime::stable_suffix(&state.auth_token);

    let engine =
        susun_integration::connect_engine_for_profile(&state.db, runtime_profile_id.as_deref())
            .await
            .map_err(ApiError::EngineUnavailable)?;
    let runtime_ctx = artifact_inventory::runtime_context(&state.db, runtime_profile_id.as_deref())
        .await?
        .into();

    let lookup = artifact_inventory::image_for_mutation(&engine, &image_id).await?;
    let (active_jobs, active_watch_sessions) =
        engines::engine_active_work(&state.db, runtime_profile_id.as_deref()).await?;
    let has_active_work = active_jobs > 0 || active_watch_sessions > 0;

    let (capability, target) = match lookup {
        DetailLookup::Unsupported { capability } => {
            logging::warn(
                "image_remove_previewed",
                &[
                    ("engine_id", engine_id.clone()),
                    ("capability", capability.clone()),
                ],
            );
            return Ok(Json(ImageRemovePreview {
                engine_id,
                runtime: runtime_ctx,
                capability,
                image_id,
                references: Vec::new(),
                digests: Vec::new(),
                estimated_reclaim_bytes: None,
                active_jobs,
                active_watch_sessions,
                commit_enabled: false,
                plan_id: None,
                expires_in_seconds: None,
            }));
        }
        DetailLookup::NotFound => return Err(ApiError::ArtifactNotFound),
        DetailLookup::Found { capability, value } => (capability, value),
    };

    let (plan_id, expires_in_seconds, commit_enabled) = if has_active_work {
        (None, None, false)
    } else {
        let identity_fingerprint =
            engines::engine_identity_fingerprint(&state.db, &engine, runtime_profile_id.as_deref())
                .await;
        let ticket = state.action_plans.prepare(
            &owner,
            ActionKind::ImageRemove,
            ActionPlanPayload::ImageRemove(ImageRemovePlan {
                engine_id: engine_id.clone(),
                runtime_profile_id: runtime_profile_id.clone(),
                image_id: target.id.clone(),
                force: false,
                identity_fingerprint,
                source_fingerprint: image_fingerprint(&target),
            }),
        );
        (Some(ticket.plan_id), Some(ticket.expires_in_seconds), true)
    };

    logging::warn(
        "image_remove_previewed",
        &[
            ("engine_id", engine_id.clone()),
            ("commit_enabled", commit_enabled.to_string()),
        ],
    );

    Ok(Json(ImageRemovePreview {
        engine_id,
        runtime: runtime_ctx,
        capability,
        image_id: target.id,
        references: target.references,
        digests: target.digests,
        estimated_reclaim_bytes: target.size_bytes,
        active_jobs,
        active_watch_sessions,
        commit_enabled,
        plan_id,
        expires_in_seconds,
    }))
}

pub async fn commit_remove_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<ImageRemoveResponse>, ApiError> {
    authorize(&state, &headers)?;
    engines::reject_commit_body(&body)?;
    let owner = runtime::stable_suffix(&state.auth_token);
    let started = engines::now_ms()?;

    let claimed = state
        .action_plans
        .claim(&plan_id, &owner, &[ActionKind::ImageRemove])
        .map_err(|error| ApiError::ActionUnavailable(error.to_string()))?;
    let ActionPlanPayload::ImageRemove(plan) = claimed.payload else {
        state
            .action_plans
            .finish(&claimed.plan_id, PlanState::Failed);
        return Err(ApiError::ActionUnavailable(
            "Plan did not match an image removal.".to_owned(),
        ));
    };

    logging::warn(
        "image_remove_started",
        &[("engine_id", plan.engine_id.clone())],
    );

    async fn reject(
        state: &AppState,
        plan_id: &str,
        profile_id: Option<String>,
        code: &'static str,
        message: String,
    ) -> ApiError {
        state.action_plans.finish(plan_id, PlanState::Failed);
        let _ = action_audit::record_rejection(
            &state.db,
            ActionKind::ImageRemove,
            profile_id,
            "rejected_state",
            code,
        )
        .await;
        ApiError::ActionUnavailable(message)
    }

    let engine = match susun_integration::connect_engine_for_profile(
        &state.db,
        plan.runtime_profile_id.as_deref(),
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::ImageRemove,
                plan.runtime_profile_id.clone(),
                "rejected_state",
                "provider_unreachable",
            )
            .await;
            return Err(ApiError::EngineUnavailable(error));
        }
    };

    if engines::engine_identity_fingerprint(&state.db, &engine, plan.runtime_profile_id.as_deref())
        .await
        != plan.identity_fingerprint
    {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "engine_identity_changed",
            "The selected engine or its endpoint changed since preview. Preview the removal again."
                .to_owned(),
        )
        .await);
    }

    let lookup = match artifact_inventory::image_for_mutation(&engine, &plan.image_id).await {
        Ok(lookup) => lookup,
        Err(_) => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "inventory_unavailable",
                "Image inventory is no longer available. Preview the removal again.".to_owned(),
            )
            .await);
        }
    };
    let target = match lookup {
        DetailLookup::Found { value, .. } => value,
        DetailLookup::NotFound => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "image_already_gone",
                "The image no longer exists. Preview the removal again.".to_owned(),
            )
            .await);
        }
        DetailLookup::Unsupported { .. } => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "capability_withdrawn",
                "Image removal is no longer supported on this engine.".to_owned(),
            )
            .await);
        }
    };
    if image_fingerprint(&target) != plan.source_fingerprint {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "stale_inventory",
            "The image changed since preview. Preview the removal again.".to_owned(),
        )
        .await);
    }

    let (jobs, watch) =
        match engines::engine_active_work(&state.db, plan.runtime_profile_id.as_deref()).await {
            Ok(active) => active,
            Err(_) => {
                return Err(reject(
                    &state,
                    &claimed.plan_id,
                    plan.runtime_profile_id.clone(),
                    "active_work_unavailable",
                    "Active work could not be verified. Preview the removal again.".to_owned(),
                )
                .await);
            }
        };
    if jobs > 0 || watch > 0 {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "active_work",
            "A job or watch session is running. Stop it and preview the removal again.".to_owned(),
        )
        .await);
    }

    match engines::revalidate_engine_still_selected(&state.db, &plan.engine_id).await {
        Ok(true) => {}
        Ok(false) => {
            return Err(reject(
                &state,
                &claimed.plan_id,
                plan.runtime_profile_id.clone(),
                "engine_removed_or_changed",
                "The selected engine changed or is no longer available since preview. Preview the removal again."
                    .to_owned(),
            )
            .await);
        }
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, PlanState::Failed);
            return Err(error);
        }
    }

    let result = match susun_integration::remove_image(&engine, &plan.image_id, plan.force).await {
        Ok(result) => result,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::ImageRemove,
                plan.runtime_profile_id.clone(),
                "failed",
                "remove_failed",
            )
            .await;
            return Err(ApiError::EngineUnavailable(error));
        }
    };

    state
        .action_plans
        .finish(&claimed.plan_id, PlanState::Succeeded);
    let _ = action_audit::record(
        &state.db,
        AuditEntry {
            kind: ActionKind::ImageRemove,
            profile_id: plan.runtime_profile_id,
            runtime_class: None,
            ownership_result: "authorized".to_owned(),
            command_kind: Some("provider_image_remove".to_owned()),
            elevation_mode: Some("none".to_owned()),
            terminal_status: action_audit::STATUS_COMPLETED.to_owned(),
            affected: vec![AffectedCount {
                category: "images_removed".to_owned(),
                count: result.deleted.len() as i64,
            }],
            failure_code: None,
            correlation_token: None,
            started_at_ms: started,
            completed_at_ms: Some(engines::now_ms()?),
        },
    )
    .await;

    logging::warn(
        "image_remove_finished",
        &[
            ("engine_id", plan.engine_id.clone()),
            ("deleted_count", result.deleted.len().to_string()),
        ],
    );

    Ok(Json(ImageRemoveResponse {
        deleted: result.deleted,
        untagged: result.untagged,
    }))
}

/// Mirrors `PruneScopeInventory`'s support/estimate vocabulary so the
/// frontend reads one consistent capability language across prune and
/// build-cache status.
#[derive(Debug, Serialize)]
pub struct BuildCacheScopeStatus {
    pub support: String,
    pub candidate_count: Option<u64>,
    pub reclaimable_bytes: Option<u64>,
    pub estimate_kind: String,
}

#[derive(Debug, Serialize)]
pub struct BuildCacheStatusResponse {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// SDK support level for build-cache inventory and cleanup on this
    /// provider.
    pub support: String,
    pub usage: Option<BuildCacheScopeStatus>,
}

pub async fn engine_build_cache_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
) -> Result<Json<BuildCacheStatusResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, engine_id, runtime_ctx) = connect_selected_engine(&state, &engine_id).await?;

    let status = artifact_inventory::build_cache_status(&engine).await?;

    Ok(Json(BuildCacheStatusResponse {
        engine_id,
        runtime: runtime_ctx,
        support: status.support,
        usage: status.scope.map(|scope| BuildCacheScopeStatus {
            support: scope.support,
            candidate_count: scope.candidate_count,
            reclaimable_bytes: scope.reclaimable_bytes,
            estimate_kind: scope.estimate_kind,
        }),
    }))
}

#[derive(Debug, Serialize)]
pub struct RegistryCapabilityResponse {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    /// Capability flags only — no live login state, no configured-registry
    /// list, and no credential material. Credential storage is not part of
    /// this slice.
    pub supports_pull: String,
    pub supports_push: String,
    pub supports_auth: String,
}

pub async fn engine_registry_capability(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
) -> Result<Json<RegistryCapabilityResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, engine_id, runtime_ctx) = connect_selected_engine(&state, &engine_id).await?;

    let capability = artifact_inventory::registry_capability(&engine).await?;

    Ok(Json(RegistryCapabilityResponse {
        engine_id,
        runtime: runtime_ctx,
        supports_pull: capability.supports_pull,
        supports_push: capability.supports_push,
        supports_auth: capability.supports_auth,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn sample_runtime() -> ArtifactRuntimeContext {
        ArtifactRuntimeContext {
            runtime_profile_id: Some("profile-1".to_owned()),
            runtime_class: Some("built_in".to_owned()),
            display_name: Some("Susun Runtime".to_owned()),
            is_selected: Some(true),
        }
    }

    /// Registry status must never carry credential material, tokens, or a raw
    /// endpoint — only capability flags. Fixing the exact key set here means a
    /// future field addition has to be a deliberate, reviewable change.
    #[test]
    fn registry_capability_response_exposes_only_capability_flags() -> TestResult {
        let response = RegistryCapabilityResponse {
            engine_id: "engine-1".to_owned(),
            runtime: sample_runtime(),
            supports_pull: "supported".to_owned(),
            supports_push: "unsupported".to_owned(),
            supports_auth: "unknown".to_owned(),
        };

        let value = serde_json::to_value(&response)?;
        let object = value.as_object().ok_or("expected a JSON object")?;
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();

        assert_eq!(
            keys,
            vec![
                "engine_id",
                "runtime",
                "supports_auth",
                "supports_pull",
                "supports_push"
            ]
        );
        for forbidden in ["token", "credential", "password", "secret", "endpoint"] {
            assert!(
                !value.to_string().to_lowercase().contains(forbidden),
                "registry response leaked a `{forbidden}`-shaped field"
            );
        }
        Ok(())
    }

    /// Every engine-wide artifact response must carry the selected-runtime
    /// attribution so built-in and external runtimes stay distinguishable.
    #[test]
    fn container_inventory_response_carries_runtime_attribution() -> TestResult {
        let response = EngineContainerInventoryResponse {
            engine_id: "engine-1".to_owned(),
            runtime: sample_runtime(),
            capability: "supported".to_owned(),
            observed_at_epoch_seconds: Some(1_700_000_000),
            containers: vec![ContainerArtifactSummary {
                id: "c1".to_owned(),
                name: "web-1".to_owned(),
                state: "running".to_owned(),
                health: None,
                image_reference: Some("nginx:latest".to_owned()),
                label_keys: vec!["com.docker.compose.project".to_owned()],
                known_project_id: Some("proj-1".to_owned()),
                created_at_epoch_seconds: Some(1_700_000_000),
                writable_size_bytes: Some(1024),
                root_filesystem_size_bytes: Some(2048),
            }],
        };

        let value = serde_json::to_value(&response)?;
        assert_eq!(value["runtime"]["runtime_class"], "built_in");
        assert_eq!(value["runtime"]["is_selected"], true);
        assert_eq!(value["containers"][0]["known_project_id"], "proj-1");
        // A container's own filesystem path never appears in the wire shape.
        assert!(!value.to_string().contains("C:\\") && !value.to_string().contains("C:/"));
        Ok(())
    }

    /// An unsupported provider must surface as an explicit capability state,
    /// never as an empty-looking success.
    #[test]
    fn build_cache_status_response_represents_unsupported_explicitly() -> TestResult {
        let response = BuildCacheStatusResponse {
            engine_id: "engine-1".to_owned(),
            runtime: sample_runtime(),
            support: "unsupported".to_owned(),
            usage: None,
        };

        let value = serde_json::to_value(&response)?;
        assert_eq!(value["support"], "unsupported");
        assert!(value["usage"].is_null());
        Ok(())
    }

    fn sample_container_row() -> artifact_inventory::ContainerSummaryRow {
        artifact_inventory::ContainerSummaryRow {
            id: "c1".to_owned(),
            name: "web-1".to_owned(),
            state: "running".to_owned(),
            health: None,
            image_reference: None,
            label_keys: Vec::new(),
            known_project_id: None,
            created_at_epoch_seconds: None,
            writable_size_bytes: None,
            root_filesystem_size_bytes: None,
        }
    }

    /// A found artifact reads as a normal 200, carrying whatever real
    /// support level the provider reported.
    #[test]
    fn container_detail_response_maps_found_to_populated_body() -> TestResult {
        let response = container_detail_response(
            "engine-1".to_owned(),
            sample_runtime(),
            DetailLookup::Found {
                capability: "supported".to_owned(),
                value: sample_container_row(),
            },
        )?;

        assert_eq!(response.capability, "supported");
        assert!(response.container.is_some());
        Ok(())
    }

    /// A provider that only supports a documented subset must not be
    /// reported as fully "supported" — the route used to hardcode that
    /// string regardless of what the SDK actually reported.
    #[test]
    fn container_detail_response_preserves_supported_subset_capability() -> TestResult {
        let response = container_detail_response(
            "engine-1".to_owned(),
            sample_runtime(),
            DetailLookup::Found {
                capability: "supported_subset".to_owned(),
                value: sample_container_row(),
            },
        )?;

        assert_eq!(response.capability, "supported_subset");
        assert!(response.container.is_some());
        Ok(())
    }

    /// An unsupported provider is still a 200: `capability` carries the real
    /// support level and `container` is explicitly absent, never a fake
    /// empty-looking success or a generic error.
    #[test]
    fn container_detail_response_maps_unsupported_to_explicit_capability_state() -> TestResult {
        let response = container_detail_response(
            "engine-1".to_owned(),
            sample_runtime(),
            DetailLookup::Unsupported {
                capability: "unsupported".to_owned(),
            },
        )?;

        assert_eq!(response.capability, "unsupported");
        assert!(response.container.is_none());
        Ok(())
    }

    /// A supported provider with no such id is a real 404, not a 422 or a
    /// null-shaped 200.
    #[test]
    fn container_detail_response_maps_not_found_to_artifact_not_found_error() {
        let result = container_detail_response(
            "engine-1".to_owned(),
            sample_runtime(),
            DetailLookup::NotFound,
        );

        assert!(matches!(result, Err(ApiError::ArtifactNotFound)));
    }

    #[test]
    fn image_detail_response_maps_not_found_to_artifact_not_found_error() {
        let result = image_detail_response(
            "engine-1".to_owned(),
            sample_runtime(),
            DetailLookup::NotFound,
        );

        assert!(matches!(result, Err(ApiError::ArtifactNotFound)));
    }

    /// Mirrors the container-side regression: a documented-subset provider
    /// must not be reported as fully "supported".
    #[test]
    fn image_detail_response_preserves_supported_subset_capability() -> TestResult {
        let row = artifact_inventory::ImageSummaryRow {
            id: "img1".to_owned(),
            references: Vec::new(),
            digests: Vec::new(),
            label_keys: Vec::new(),
            created_at_epoch_seconds: None,
            size_bytes: None,
            shared_size_bytes: None,
            container_count: None,
        };

        let response = image_detail_response(
            "engine-1".to_owned(),
            sample_runtime(),
            DetailLookup::Found {
                capability: "supported_subset".to_owned(),
                value: row,
            },
        )?;

        assert_eq!(response.capability, "supported_subset");
        assert!(response.image.is_some());
        Ok(())
    }

    fn sample_image_row() -> artifact_inventory::ImageSummaryRow {
        artifact_inventory::ImageSummaryRow {
            id: "sha256:abc".to_owned(),
            references: vec!["myapp:latest".to_owned()],
            digests: vec!["sha256:def".to_owned()],
            label_keys: Vec::new(),
            created_at_epoch_seconds: Some(1_700_000_000),
            size_bytes: Some(1024),
            shared_size_bytes: Some(512),
            container_count: Some(0),
        }
    }

    /// A commit must be able to detect that the previewed image changed —
    /// gained/lost a reference, a different digest, a different size — so an
    /// identical fingerprint over identical input, and a *different* one
    /// after any of those fields change, is the whole safety property.
    #[test]
    fn image_fingerprint_is_stable_for_identical_input_and_changes_with_the_image() {
        let base = sample_image_row();
        assert_eq!(
            image_fingerprint(&base),
            image_fingerprint(&sample_image_row())
        );

        let mut different_reference = sample_image_row();
        different_reference.references = vec!["myapp:v2".to_owned()];
        assert_ne!(
            image_fingerprint(&base),
            image_fingerprint(&different_reference)
        );

        let mut different_digest = sample_image_row();
        different_digest.digests = vec!["sha256:changed".to_owned()];
        assert_ne!(
            image_fingerprint(&base),
            image_fingerprint(&different_digest)
        );

        let mut different_size = sample_image_row();
        different_size.size_bytes = Some(2048);
        assert_ne!(image_fingerprint(&base), image_fingerprint(&different_size));
    }
}

#[cfg(test)]
mod engine_id_validation_tests {
    use super::*;
    use crate::test_support::{authorized_headers, fresh_db, test_state};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A fabricated `engine_id` must be rejected before the handler connects
    /// to the selected engine at all, so a caller can never label real
    /// engine-wide data with a made-up id. Representative of all six
    /// artifact routes, which share the exact same validation call.
    #[tokio::test]
    async fn list_engine_containers_rejects_fake_engine_id_without_touching_the_engine()
    -> TestResult {
        let state = test_state(fresh_db("artifacts-containers-fake").await?);

        let result = list_engine_containers(
            State(state),
            authorized_headers(),
            Path("fake-engine".to_owned()),
        )
        .await;

        assert!(matches!(result, Err(ApiError::EngineNotFound)));
        Ok(())
    }

    /// A fabricated `engine_id` must be rejected before a tag commit plan is
    /// ever minted — no capability probe, no image lookup, nothing an
    /// attacker could use to enumerate real inventory under a made-up id.
    #[tokio::test]
    async fn preview_tag_image_rejects_fake_engine_id_before_minting_a_plan() -> TestResult {
        let state = test_state(fresh_db("artifacts-tag-fake-engine").await?);

        let result = preview_tag_image(
            State(state),
            authorized_headers(),
            Path(("fake-engine".to_owned(), "sha256:abc".to_owned())),
            Json(TagImageRequest {
                target_reference: "myapp:latest".to_owned(),
            }),
        )
        .await;

        assert!(matches!(result, Err(ApiError::EngineNotFound)));
        Ok(())
    }

    /// Mirrors the tag-side regression: a fabricated `engine_id` must be
    /// rejected before a remove commit plan is minted.
    #[tokio::test]
    async fn preview_remove_image_rejects_fake_engine_id_before_minting_a_plan() -> TestResult {
        let state = test_state(fresh_db("artifacts-remove-fake-engine").await?);

        let result = preview_remove_image(
            State(state),
            authorized_headers(),
            Path(("fake-engine".to_owned(), "sha256:abc".to_owned())),
        )
        .await;

        assert!(matches!(result, Err(ApiError::EngineNotFound)));
        Ok(())
    }
}

#[cfg(test)]
mod commit_plan_validation_tests {
    use super::*;
    use crate::test_support::{authorized_headers, fresh_db, test_state};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// Commit endpoints carry no body; the executable policy lives entirely
    /// in the server-held plan. A non-empty body must be rejected before the
    /// plan store is ever touched — this runs the check ahead of `claim`, so
    /// even a well-formed-looking JSON body cannot smuggle a substitute
    /// target in at commit time.
    #[tokio::test]
    async fn commit_tag_image_rejects_a_non_empty_body() -> TestResult {
        let state = test_state(fresh_db("artifacts-tag-commit-body").await?);

        let result = commit_tag_image(
            State(state),
            authorized_headers(),
            Path("rap_whatever".to_owned()),
            axum::body::Bytes::from_static(b"{\"target_reference\":\"evil:latest\"}"),
        )
        .await;

        assert!(matches!(result, Err(ApiError::TrustedPlanContentRejected)));
        Ok(())
    }

    /// Mirrors the tag-side regression for image removal.
    #[tokio::test]
    async fn commit_remove_image_rejects_a_non_empty_body() -> TestResult {
        let state = test_state(fresh_db("artifacts-remove-commit-body").await?);

        let result = commit_remove_image(
            State(state),
            authorized_headers(),
            Path("rap_whatever".to_owned()),
            axum::body::Bytes::from_static(b"{}"),
        )
        .await;

        assert!(matches!(result, Err(ApiError::TrustedPlanContentRejected)));
        Ok(())
    }

    /// An unknown, expired-looking, or never-issued plan id must be rejected
    /// as `ActionUnavailable`, not panic or fall through to a live engine
    /// call — `claim` runs, and fails, before any connection is attempted.
    #[tokio::test]
    async fn commit_tag_image_rejects_an_unknown_plan_id() -> TestResult {
        let state = test_state(fresh_db("artifacts-tag-commit-unknown-plan").await?);

        let result = commit_tag_image(
            State(state),
            authorized_headers(),
            Path("rap_does_not_exist".to_owned()),
            axum::body::Bytes::new(),
        )
        .await;

        assert!(matches!(result, Err(ApiError::ActionUnavailable(_))));
        Ok(())
    }

    /// Mirrors the tag-side regression for image removal.
    #[tokio::test]
    async fn commit_remove_image_rejects_an_unknown_plan_id() -> TestResult {
        let state = test_state(fresh_db("artifacts-remove-commit-unknown-plan").await?);

        let result = commit_remove_image(
            State(state),
            authorized_headers(),
            Path("rap_does_not_exist".to_owned()),
            axum::body::Bytes::new(),
        )
        .await;

        assert!(matches!(result, Err(ApiError::ActionUnavailable(_))));
        Ok(())
    }

    /// A plan minted by prune (or any other domain) must never be spendable
    /// through the tag/remove commit endpoints — `claim`'s `allowed` list is
    /// the enforcement point, and this exercises it through the real route
    /// handlers rather than the plan store directly.
    #[tokio::test]
    async fn commit_tag_image_rejects_a_plan_from_a_different_domain() -> TestResult {
        let state = test_state(fresh_db("artifacts-tag-commit-wrong-domain").await?);
        let owner = crate::runtime::stable_suffix(&state.auth_token);
        let ticket = state.action_plans.prepare(
            &owner,
            crate::action_plans::ActionKind::EnginePrune,
            crate::action_plans::ActionPlanPayload::EnginePrune(
                crate::action_plans::EnginePrunePlan {
                    engine_id: "engine-docker-local".to_owned(),
                    runtime_profile_id: None,
                    scopes: vec!["images".to_owned()],
                    all_images: false,
                    identity_fingerprint: "fp".to_owned(),
                    inventory_fingerprint: "fp".to_owned(),
                },
            ),
        );

        let result = commit_tag_image(
            State(state),
            authorized_headers(),
            Path(ticket.plan_id),
            axum::body::Bytes::new(),
        )
        .await;

        assert!(matches!(result, Err(ApiError::ActionUnavailable(_))));
        Ok(())
    }
}
