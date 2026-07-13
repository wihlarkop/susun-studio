use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};

use crate::{
    action_audit,
    auth::authorize,
    error::ApiError,
    logging,
    runtime::{
        self,
        transitions::{
            self, CommitRejected, DestructiveCommitResult, DestructivePreview,
            DestructivePreviewRequest, MigrationPreview, MigrationRequest, MigrationResult,
            MigrationRollbackPreview, MigrationRollbackResult, UninstallPolicy,
        },
    },
    state::AppState,
};

/// Commit endpoints carry no request body — the executable target lives only in
/// the server-side plan. Reject any content so a caller cannot smuggle a target
/// list past the plan.
fn reject_commit_body(body: &Bytes) -> Result<(), ApiError> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(ApiError::TrustedPlanContentRejected)
    }
}

fn owner(state: &AppState) -> String {
    runtime::stable_suffix(&state.auth_token)
}

fn map_rejection(rejection: CommitRejected) -> ApiError {
    ApiError::ActionUnavailable(rejection.message)
}

pub async fn preview_migration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MigrationRequest>,
) -> Result<Json<MigrationPreview>, ApiError> {
    authorize(&state, &headers)?;
    let preview =
        transitions::preview_migration(&state.db, &state.action_plans, &owner(&state), &request)
            .await?
            .ok_or(ApiError::RuntimeProfileNotFound)?;
    logging::info(
        "runtime_migration_previewed",
        &[
            ("source_profile_id", request.source_profile_id),
            ("target_profile_id", request.target_profile_id),
            ("project_count", request.project_ids.len().to_string()),
            ("can_migrate", preview.can_migrate.to_string()),
        ],
    );
    Ok(Json(preview))
}

pub async fn commit_migration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<MigrationResult>, ApiError> {
    authorize(&state, &headers)?;
    reject_commit_body(&body)?;
    let result =
        transitions::commit_migration(&state.db, &state.action_plans, &owner(&state), &plan_id)
            .await
            .map_err(map_rejection)?;
    logging::warn(
        "runtime_migration_finished",
        &[
            ("migration_id", result.migration_id.clone()),
            ("status", result.status.to_owned()),
            ("project_count", result.project_count.to_string()),
        ],
    );
    Ok(Json(result))
}

pub async fn prepare_migration_rollback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(migration_id): Path<String>,
    body: Bytes,
) -> Result<Json<MigrationRollbackPreview>, ApiError> {
    authorize(&state, &headers)?;
    reject_commit_body(&body)?;
    let preview = transitions::preview_migration_rollback(
        &state.db,
        &state.action_plans,
        &owner(&state),
        &migration_id,
    )
    .await?
    .ok_or_else(|| ApiError::ActionUnavailable("Runtime migration not found.".to_owned()))?;
    logging::info(
        "runtime_migration_rollback_prepared",
        &[
            ("migration_id", migration_id),
            ("restorable", preview.restorable.to_string()),
        ],
    );
    Ok(Json(preview))
}

pub async fn commit_migration_rollback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<MigrationRollbackResult>, ApiError> {
    authorize(&state, &headers)?;
    reject_commit_body(&body)?;
    let result = transitions::commit_migration_rollback(
        &state.db,
        &state.action_plans,
        &owner(&state),
        &plan_id,
    )
    .await
    .map_err(map_rejection)?;
    logging::warn(
        "runtime_migration_rollback_finished",
        &[
            ("migration_id", result.migration_id.clone()),
            ("status", result.status.to_owned()),
            (
                "restored_project_count",
                result.restored_project_count.to_string(),
            ),
        ],
    );
    Ok(Json(result))
}

pub async fn preview_destructive_operation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    Json(request): Json<DestructivePreviewRequest>,
) -> Result<Json<DestructivePreview>, ApiError> {
    authorize(&state, &headers)?;
    let preview = transitions::preview_destructive_operation(
        &state.db,
        &state.action_plans,
        &owner(&state),
        &profile_id,
        &request,
    )
    .await?
    .ok_or(ApiError::RuntimeProfileNotFound)?;
    logging::warn(
        "runtime_destructive_operation_previewed",
        &[
            ("operation_id", preview.operation_id.clone()),
            ("profile_id", profile_id),
            ("action", preview.action.clone()),
            ("allowed", preview.allowed.to_string()),
        ],
    );
    Ok(Json(preview))
}

pub async fn commit_destructive_operation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<DestructiveCommitResult>, ApiError> {
    authorize(&state, &headers)?;
    reject_commit_body(&body)?;
    let result = transitions::commit_destructive_operation(
        &state.db,
        &state.action_plans,
        &owner(&state),
        &plan_id,
    )
    .await
    .map_err(map_rejection)?;
    logging::warn(
        "runtime_destructive_operation_committed",
        &[
            ("profile_id", result.profile_id.clone()),
            ("action", result.action.clone()),
            ("status", result.status.to_owned()),
        ],
    );
    Ok(Json(result))
}

pub async fn uninstall_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UninstallPolicy>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(transitions::uninstall_policy()))
}

pub async fn list_action_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let entries = action_audit::list(&state.db, 200).await?;
    Ok(Json(serde_json::json!({ "entries": entries })))
}

pub async fn clear_action_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    reject_commit_body(&body)?;
    let removed = action_audit::clear(&state.db).await?;
    logging::warn(
        "runtime_action_audit_cleared",
        &[("removed_count", removed.to_string())],
    );
    Ok(Json(
        serde_json::json!({ "cleared": true, "removed_count": removed }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_endpoints_reject_frontend_content() {
        let body = Bytes::from_static(br#"{"project_ids":["p1","p2"],"target":"evil"}"#);
        assert!(matches!(
            reject_commit_body(&body),
            Err(ApiError::TrustedPlanContentRejected)
        ));
        assert!(reject_commit_body(&Bytes::new()).is_ok());
    }
}
