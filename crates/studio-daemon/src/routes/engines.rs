use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Serialize;
use turso::params;

use crate::{auth::authorize, error::ApiError, state::AppState, susun_integration};

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

    // Connect and check Docker (no DB cursor open here).
    let health = match susun_integration::connect_docker_engine() {
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
        params![health_json, now, engine_id],
    )
    .await?;

    Ok(Json(response))
}

pub async fn engine_capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(_engine_id): Path<String>,
) -> Result<Json<EngineCapabilitiesResponse>, ApiError> {
    authorize(&state, &headers)?;

    let engine =
        susun_integration::connect_docker_engine().map_err(ApiError::EngineUnavailable)?;
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

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}
