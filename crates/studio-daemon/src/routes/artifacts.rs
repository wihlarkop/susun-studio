//! Read-only, capability-gated endpoints for engine-wide artifacts: images,
//! containers, build cache, and registry capability. Nothing here mutates,
//! pulls, pushes, builds, tags, or prunes — that is Phase 15b+ scope.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Serialize;

use super::engines::resolve_and_validate_engine;
use crate::{
    artifact_inventory::{self, DetailLookup, RuntimeContextRow},
    auth::authorize,
    error::ApiError,
    logging,
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
}
