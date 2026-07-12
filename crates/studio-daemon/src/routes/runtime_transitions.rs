use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};

use crate::{
    auth::authorize,
    error::ApiError,
    logging,
    runtime::transitions::{
        self, DestructivePreview, DestructivePreviewRequest, MigrationPreview, MigrationRequest,
        MigrationResult, MigrationRollbackResult, UninstallPolicy,
    },
    state::AppState,
};

pub async fn preview_migration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MigrationRequest>,
) -> Result<Json<MigrationPreview>, ApiError> {
    authorize(&state, &headers)?;
    let preview = transitions::preview_migration(&state.db, &request)
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

pub async fn rollback_migration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(migration_id): Path<String>,
) -> Result<Json<MigrationRollbackResult>, ApiError> {
    authorize(&state, &headers)?;
    let result = transitions::rollback_migration(&state.db, &migration_id)
        .await?
        .ok_or_else(|| ApiError::ActionUnavailable("Runtime migration not found.".to_owned()))?;
    logging::warn(
        "runtime_migration_rollback_finished",
        &[
            ("migration_id", migration_id),
            ("status", result.status.to_owned()),
            (
                "restored_project_count",
                result.restored_project_count.to_string(),
            ),
        ],
    );
    Ok(Json(result))
}

pub async fn execute_migration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MigrationRequest>,
) -> Result<Json<MigrationResult>, ApiError> {
    authorize(&state, &headers)?;
    let result = transitions::execute_migration(&state.db, &request)
        .await?
        .ok_or(ApiError::RuntimeProfileNotFound)?;
    logging::info(
        "runtime_migration_finished",
        &[
            ("migration_id", result.migration_id.clone()),
            ("status", result.status.to_owned()),
            ("project_count", result.project_count.to_string()),
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
    let preview = transitions::preview_destructive_operation(&state.db, &profile_id, &request)
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

pub async fn uninstall_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UninstallPolicy>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(transitions::uninstall_policy()))
}
