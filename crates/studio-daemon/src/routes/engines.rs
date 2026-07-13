use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use turso::params;

use crate::{
    action_audit::{self, AffectedCount, AuditEntry},
    action_plans::{ActionKind, ActionPlanPayload, EnginePrunePlan},
    auth::authorize,
    error::ApiError,
    logging, runtime,
    state::AppState,
    susun_integration,
};

#[derive(Debug, Serialize)]
pub struct EngineResponse {
    pub id: String,
    pub provider_kind: String,
    pub display_name: String,
    pub enabled: bool,
    pub is_default: bool,
    pub last_health: Option<serde_json::Value>,
    pub last_health_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct EngineListResponse {
    pub engines: Vec<EngineResponse>,
}

#[derive(Debug, Serialize)]
pub struct EngineHealthResponse {
    pub reachable: bool,
    pub api_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EngineCapabilitiesResponse {
    pub api_version: Option<String>,
    pub supports_health: String,
    pub supports_named_volumes: String,
    pub supports_network_aliases: String,
    pub supports_log_follow: String,
    pub supports_build: String,
    pub supports_mount_types: Vec<String>,
    pub max_container_name_length: Option<usize>,
}

pub async fn list_engines(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<EngineListResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, provider_kind, display_name, enabled, is_default,
                    last_health_json, last_health_at_ms
             FROM engines ORDER BY is_default DESC, created_at_ms ASC",
            (),
        )
        .await?;

    let mut engines = Vec::new();
    while let Some(row) = rows.next().await? {
        let enabled: i64 = row.get(3)?;
        let is_default: i64 = row.get(4)?;
        let last_health_json: Option<String> = row.get(5)?;

        engines.push(EngineResponse {
            id: row.get(0)?,
            provider_kind: row.get(1)?,
            display_name: row.get(2)?,
            enabled: enabled != 0,
            is_default: is_default != 0,
            last_health: last_health_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            last_health_at_ms: row.get(6)?,
        });
    }

    Ok(Json(EngineListResponse { engines }))
}

pub async fn engine_health(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
) -> Result<Json<EngineHealthResponse>, ApiError> {
    authorize(&state, &headers)?;
    logging::info("engine_health_started", &[("engine_id", engine_id.clone())]);

    // Connect and check Docker (no DB cursor open here).
    let health = match susun_integration::connect_engine(&state.db, None).await {
        Ok(engine) => susun_integration::engine_health(&engine).await,
        Err(error) => susun_integration::EngineHealthRow {
            reachable: false,
            api_version: None,
            error: Some(error),
        },
    };
    let response = EngineHealthResponse {
        reachable: health.reachable,
        api_version: health.api_version,
        error: health.error,
    };

    // Persist the latest result. No read cursor is open on this connection.
    let health_json = serde_json::to_string(&response)?;
    let now = now_ms()?;
    let conn = state.db.connect()?;
    conn.execute(
        "UPDATE engines SET last_health_json = ?1, last_health_at_ms = ?2 WHERE id = ?3",
        params![health_json, now, engine_id.clone()],
    )
    .await?;

    if response.reachable {
        logging::info(
            "engine_health_finished",
            &[
                ("engine_id", engine_id),
                ("reachable", response.reachable.to_string()),
                (
                    "api_version",
                    response.api_version.clone().unwrap_or_default(),
                ),
            ],
        );
    } else {
        logging::warn(
            "engine_health_finished",
            &[
                ("engine_id", engine_id),
                ("reachable", response.reachable.to_string()),
                ("error", response.error.clone().unwrap_or_default()),
            ],
        );
    }

    Ok(Json(response))
}

pub async fn engine_capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(_engine_id): Path<String>,
) -> Result<Json<EngineCapabilitiesResponse>, ApiError> {
    authorize(&state, &headers)?;

    let engine = susun_integration::connect_engine(&state.db, None)
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let capabilities = susun_integration::engine_capabilities(&engine)
        .await
        .map_err(ApiError::EngineUnavailable)?;

    Ok(Json(EngineCapabilitiesResponse {
        api_version: capabilities.api_version,
        supports_health: capabilities.supports_health,
        supports_named_volumes: capabilities.supports_named_volumes,
        supports_network_aliases: capabilities.supports_network_aliases,
        supports_log_follow: capabilities.supports_log_follow,
        supports_build: capabilities.supports_build,
        supports_mount_types: capabilities.supports_mount_types,
        max_container_name_length: capabilities.max_container_name_length,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PruneRequest {
    /// Prune *policy*: which resource classes to reclaim. This is not a resource
    /// target list — the engine derives the exact resources to remove at commit.
    pub scopes: Vec<String>,
    #[serde(default)]
    pub all_images: bool,
}

/// Non-mutating prune preview that mints an opaque, single-use commit plan.
#[derive(Debug, Serialize)]
pub struct PrunePreview {
    pub engine_id: String,
    pub scopes: Vec<String>,
    pub all_images: bool,
    /// Prune operates engine-wide: resources created by other tools or projects
    /// sharing this engine may also be removed. Surfaced so the UI can warn.
    pub affects_shared_engine: bool,
    pub plan_id: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct PruneResponse {
    pub containers_removed: Vec<String>,
    pub networks_removed: Vec<String>,
    pub volumes_removed: Vec<String>,
    pub images_removed: Vec<String>,
    pub space_reclaimed_bytes: u64,
}

/// Commit endpoints carry no body; the executable policy lives in the plan.
fn reject_commit_body(body: &Bytes) -> Result<(), ApiError> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(ApiError::TrustedPlanContentRejected)
    }
}

pub async fn preview_prune(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
    Json(request): Json<PruneRequest>,
) -> Result<Json<PrunePreview>, ApiError> {
    authorize(&state, &headers)?;
    let owner = runtime::stable_suffix(&state.auth_token);
    let ticket = state.action_plans.prepare(
        &owner,
        ActionKind::EnginePrune,
        ActionPlanPayload::EnginePrune(EnginePrunePlan {
            engine_id: engine_id.clone(),
            scopes: request.scopes.clone(),
            all_images: request.all_images,
        }),
    );
    logging::warn(
        "engine_prune_previewed",
        &[
            ("engine_id", engine_id.clone()),
            ("scope_count", request.scopes.len().to_string()),
            ("all_images", request.all_images.to_string()),
        ],
    );
    Ok(Json(PrunePreview {
        engine_id,
        scopes: request.scopes,
        all_images: request.all_images,
        affects_shared_engine: true,
        plan_id: ticket.plan_id,
        expires_in_seconds: ticket.expires_in_seconds,
    }))
}

pub async fn commit_prune(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<PruneResponse>, ApiError> {
    authorize(&state, &headers)?;
    reject_commit_body(&body)?;
    let owner = runtime::stable_suffix(&state.auth_token);
    let started = now_ms()?;

    let claimed = state
        .action_plans
        .claim(&plan_id, &owner, Some(ActionKind::EnginePrune))
        .map_err(|error| ApiError::ActionUnavailable(error.to_string()))?;
    let ActionPlanPayload::EnginePrune(plan) = claimed.payload else {
        return Err(ApiError::ActionUnavailable(
            "Plan did not match an engine prune.".to_owned(),
        ));
    };

    logging::warn(
        "engine_prune_started",
        &[
            ("engine_id", plan.engine_id.clone()),
            ("scope_count", plan.scopes.len().to_string()),
            ("all_images", plan.all_images.to_string()),
        ],
    );

    // Revalidate provider state at commit; the server derives the exact targets.
    let engine = match susun_integration::connect_engine(&state.db, None).await {
        Ok(engine) => engine,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::EnginePrune,
                None,
                "rejected_state",
                "provider_unreachable",
            )
            .await;
            return Err(ApiError::EngineUnavailable(error));
        }
    };
    let report = match susun_integration::system_prune(&engine, &plan.scopes, plan.all_images).await
    {
        Ok(report) => report,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::EnginePrune,
                None,
                "failed",
                "prune_failed",
            )
            .await;
            return Err(ApiError::EngineUnavailable(error));
        }
    };

    let response = PruneResponse {
        containers_removed: report.containers_removed,
        networks_removed: report.networks_removed,
        volumes_removed: report.volumes_removed,
        images_removed: report.images_removed,
        space_reclaimed_bytes: report.space_reclaimed_bytes,
    };
    state
        .action_plans
        .finish(&claimed.plan_id, crate::action_plans::PlanState::Succeeded);
    let _ = action_audit::record(
        &state.db,
        AuditEntry {
            kind: ActionKind::EnginePrune,
            profile_id: None,
            runtime_class: None,
            ownership_result: "authorized".to_owned(),
            command_kind: Some("provider_prune".to_owned()),
            elevation_mode: Some("none".to_owned()),
            terminal_status: action_audit::STATUS_COMPLETED.to_owned(),
            affected: vec![
                AffectedCount {
                    category: "containers".to_owned(),
                    count: response.containers_removed.len() as i64,
                },
                AffectedCount {
                    category: "networks".to_owned(),
                    count: response.networks_removed.len() as i64,
                },
                AffectedCount {
                    category: "volumes".to_owned(),
                    count: response.volumes_removed.len() as i64,
                },
                AffectedCount {
                    category: "images".to_owned(),
                    count: response.images_removed.len() as i64,
                },
            ],
            failure_code: None,
            started_at_ms: started,
            completed_at_ms: Some(now_ms()?),
        },
    )
    .await;
    logging::warn(
        "engine_prune_finished",
        &[
            (
                "containers_removed",
                response.containers_removed.len().to_string(),
            ),
            (
                "networks_removed",
                response.networks_removed.len().to_string(),
            ),
            (
                "volumes_removed",
                response.volumes_removed.len().to_string(),
            ),
            ("images_removed", response.images_removed.len().to_string()),
            (
                "space_reclaimed_bytes",
                response.space_reclaimed_bytes.to_string(),
            ),
        ],
    );

    Ok(Json(response))
}

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}
