use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

use crate::{auth::authorize, error::ApiError, logging, runtime, state::AppState};

pub async fn runtime_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<runtime::RuntimeStatus>, ApiError> {
    authorize(&state, &headers)?;
    logging::info("runtime_status_requested", &[]);
    Ok(Json(runtime::status(&state.db).await?))
}

pub async fn runtime_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::info("runtime_logs_requested", &[]);
    Ok(Json(serde_json::json!({ "lines": runtime::logs() })))
}

pub async fn select_runtime_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::info(
        "runtime_profile_select_requested",
        &[("profile_id", profile_id.clone())],
    );
    if !runtime::select_profile(&state.db, &profile_id).await? {
        return Err(ApiError::RuntimeProfileNotFound);
    }
    Ok(Json(serde_json::json!({ "selected": true })))
}

pub async fn runtime_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(action): Path<String>,
) -> Result<Json<runtime::RuntimeActionResult>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn("runtime_action_requested", &[("action", action.clone())]);
    Ok(Json(runtime::action(&state.db, &action).await?))
}
