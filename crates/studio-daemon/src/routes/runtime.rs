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

pub async fn list_runtime_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let profiles = runtime::list_all_profiles(&state.db).await?;
    Ok(Json(serde_json::json!({ "profiles": profiles })))
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
    match runtime::select_profile(&state.db, &profile_id).await? {
        runtime::SelectOutcome::Selected => {}
        runtime::SelectOutcome::NotFound => return Err(ApiError::RuntimeProfileNotFound),
        runtime::SelectOutcome::Unavailable => {
            return Err(ApiError::ActionUnavailable(
                "This runtime is missing or its built-in ownership is not proven.".to_owned(),
            ));
        }
    }
    Ok(Json(serde_json::json!({ "selected": true })))
}

pub async fn forget_runtime_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "runtime_profile_forget_requested",
        &[("profile_id", profile_id.clone())],
    );
    match runtime::forget_profile(&state.db, &profile_id).await? {
        runtime::ForgetOutcome::Forgotten => Ok(Json(serde_json::json!({ "forgotten": true }))),
        runtime::ForgetOutcome::NotFound => Err(ApiError::RuntimeProfileNotFound),
        runtime::ForgetOutcome::NotExternal => Err(ApiError::ActionUnavailable(
            "Built-in runtime metadata can't be forgotten; use recovery or the dedicated teardown flow."
                .to_owned(),
        )),
        runtime::ForgetOutcome::StudioManaged => Err(ApiError::ActionUnavailable(
            "A Studio-managed built-in runtime can't be forgotten; use its teardown flow instead."
                .to_owned(),
        )),
    }
}

pub async fn adopt_runtime_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "runtime_profile_adopt_requested",
        &[("profile_id", profile_id.clone())],
    );
    match runtime::adopt_profile(&state.db, &profile_id).await? {
        runtime::AdoptOutcome::Adopted => Ok(Json(serde_json::json!({ "adopted": true }))),
        runtime::AdoptOutcome::NotFound => Err(ApiError::RuntimeProfileNotFound),
        runtime::AdoptOutcome::NotBuiltIn => Err(ApiError::ActionUnavailable(
            "Only a built-in runtime can be adopted by Studio.".to_owned(),
        )),
        runtime::AdoptOutcome::AlreadyManaged => Err(ApiError::ActionUnavailable(
            "This built-in runtime is already Studio-managed.".to_owned(),
        )),
    }
}

pub async fn runtime_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((provider_id, action)): Path<(String, String)>,
) -> Result<Json<runtime::RuntimeActionResult>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "runtime_action_requested",
        &[
            ("provider_id", provider_id.clone()),
            ("action", action.clone()),
        ],
    );
    Ok(Json(
        runtime::action(&state.db, &provider_id, &action).await?,
    ))
}
