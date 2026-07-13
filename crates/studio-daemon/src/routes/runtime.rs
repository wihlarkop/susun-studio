use axum::{
    Json,
    body::Bytes,
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
    Ok(Json(
        serde_json::json!({ "lines": runtime::logs(&state.db).await? }),
    ))
}

pub async fn list_runtime_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let profiles = runtime::list_all_profiles(&state.db).await?;
    Ok(Json(serde_json::json!({ "profiles": profiles })))
}

pub async fn runtime_profile_resources(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<runtime::RuntimeResourceSnapshot>, ApiError> {
    authorize(&state, &headers)?;
    logging::info(
        "runtime_profile_resources_requested",
        &[("profile_id", profile_id.clone())],
    );
    match runtime::resource_snapshot(&state.db, &profile_id).await? {
        runtime::ResourceSnapshotOutcome::Found(snapshot) => Ok(Json(*snapshot)),
        runtime::ResourceSnapshotOutcome::NotFound => Err(ApiError::RuntimeProfileNotFound),
        runtime::ResourceSnapshotOutcome::ProviderUnavailable => Err(ApiError::ActionUnavailable(
            "The runtime provider is unavailable.".to_owned(),
        )),
    }
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
        runtime::AdoptOutcome::NotFound => Err(ApiError::RuntimeProfileNotFound),
        runtime::AdoptOutcome::NotBuiltIn => Err(ApiError::ActionUnavailable(
            "Only a built-in runtime can be adopted by Studio.".to_owned(),
        )),
        runtime::AdoptOutcome::AlreadyManaged => Err(ApiError::ActionUnavailable(
            "This built-in runtime is already Studio-managed.".to_owned(),
        )),
        runtime::AdoptOutcome::OwnershipUnproven => Err(ApiError::ActionUnavailable(
            "Studio cannot adopt a runtime it did not create. Remove the naming conflict and use Set up Susun Runtime."
                .to_owned(),
        )),
    }
}

pub async fn prepare_runtime_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((provider_id, action)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    reject_trusted_plan_content(&body)?;
    logging::warn(
        "runtime_trusted_plan_prepare_requested",
        &[
            ("provider_id", provider_id.clone()),
            ("action", action.clone()),
        ],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    match runtime::prepare_trusted_action(
        &state.db,
        &state.trusted_plans,
        &owner,
        &provider_id,
        &action,
    )
    .await?
    {
        Ok(preview) => Ok(Json(serde_json::json!({ "plan": preview }))),
        Err(result) => Ok(Json(serde_json::json!({ "result": result }))),
    }
}

pub async fn execute_runtime_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<runtime::RuntimeActionResult>, ApiError> {
    authorize(&state, &headers)?;
    reject_trusted_plan_content(&body)?;
    logging::warn(
        "runtime_trusted_plan_execute_requested",
        &[("plan_id", plan_id.clone())],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    let result =
        runtime::execute_trusted_action(&state.db, &state.trusted_plans, &owner, &plan_id).await;
    logging::info(
        "runtime_trusted_plan_execute_completed",
        &[
            ("plan_id", plan_id),
            ("action", result.action.clone()),
            ("status", result.status.clone()),
        ],
    );
    Ok(Json(result))
}

pub async fn cancel_runtime_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<runtime::RuntimeActionResult>, ApiError> {
    authorize(&state, &headers)?;
    reject_trusted_plan_content(&body)?;
    logging::info(
        "runtime_trusted_plan_cancel_requested",
        &[("plan_id", plan_id.clone())],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    let result = runtime::cancel_trusted_action(&state.trusted_plans, &owner, &plan_id);
    logging::info(
        "runtime_trusted_plan_cancel_completed",
        &[("plan_id", plan_id), ("status", result.status.clone())],
    );
    Ok(Json(result))
}

fn reject_trusted_plan_content(body: &Bytes) -> Result<(), ApiError> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(ApiError::TrustedPlanContentRejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_plan_endpoints_reject_frontend_executable_content() {
        let body = Bytes::from_static(
            br#"{"executable":"evil.exe","args":["& calc"],"env":{"TOKEN":"secret"},"elevation":"admin"}"#,
        );
        assert!(matches!(
            reject_trusted_plan_content(&body),
            Err(ApiError::TrustedPlanContentRejected)
        ));
        assert!(reject_trusted_plan_content(&Bytes::new()).is_ok());
    }
}
