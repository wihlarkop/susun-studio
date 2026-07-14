//! Read-only, capability-gated endpoints for engine-wide artifacts: images,
//! containers, build cache, and registry capability. Nothing here mutates,
//! pulls, pushes, builds, tags, or prunes — that is Phase 15b+ scope.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Serialize;

use crate::{
    artifact_inventory::{self, RuntimeContextRow},
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

async fn connect_selected_engine(
    state: &AppState,
) -> Result<
    (
        susun::DockerCompatibleEngine,
        Option<String>,
        ArtifactRuntimeContext,
    ),
    ApiError,
> {
    let (runtime_profile_id, _) = runtime::attribution_for(&state.db, None).await?;
    let engine =
        susun_integration::connect_engine_for_profile(&state.db, runtime_profile_id.as_deref())
            .await
            .map_err(ApiError::EngineUnavailable)?;
    let runtime_ctx = artifact_inventory::runtime_context(&state.db, runtime_profile_id.as_deref())
        .await
        .into();
    Ok((engine, runtime_profile_id, runtime_ctx))
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
    let (engine, _, runtime_ctx) = connect_selected_engine(&state).await?;

    let result = artifact_inventory::container_inventory(&state.db, &engine)
        .await
        .map_err(ApiError::EngineUnavailable)?;

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
    pub container: ContainerArtifactSummary,
}

pub async fn read_engine_container(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((engine_id, container_id)): Path<(String, String)>,
) -> Result<Json<ContainerArtifactDetailResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, _, runtime_ctx) = connect_selected_engine(&state).await?;

    let container = artifact_inventory::container_details(&state.db, &engine, &container_id)
        .await
        .map_err(ApiError::ActionUnavailable)?;

    Ok(Json(ContainerArtifactDetailResponse {
        engine_id,
        runtime: runtime_ctx,
        container: container.into(),
    }))
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
    let (engine, _, runtime_ctx) = connect_selected_engine(&state).await?;

    let result = artifact_inventory::image_inventory(&engine)
        .await
        .map_err(ApiError::EngineUnavailable)?;

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
    pub image: ImageArtifactSummary,
}

pub async fn read_engine_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((engine_id, image_id)): Path<(String, String)>,
) -> Result<Json<ImageArtifactDetailResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (engine, _, runtime_ctx) = connect_selected_engine(&state).await?;

    let image = artifact_inventory::image_details(&engine, &image_id)
        .await
        .map_err(ApiError::ActionUnavailable)?;

    Ok(Json(ImageArtifactDetailResponse {
        engine_id,
        runtime: runtime_ctx,
        image: image.into(),
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
    let (engine, _, runtime_ctx) = connect_selected_engine(&state).await?;

    let status = artifact_inventory::build_cache_status(&engine)
        .await
        .map_err(ApiError::EngineUnavailable)?;

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
    let (engine, _, runtime_ctx) = connect_selected_engine(&state).await?;

    let capability = artifact_inventory::registry_capability(&engine)
        .await
        .map_err(ApiError::EngineUnavailable)?;

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
}
