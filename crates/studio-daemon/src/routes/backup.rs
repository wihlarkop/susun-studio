use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::IntoResponse,
};

use crate::{auth::authorize, backup, db, error::ApiError, logging, restore, state::AppState};

/// Streams a freshly-built Studio metadata backup archive. The caller (the
/// Tauri app) writes the bytes to the user-chosen path atomically.
pub async fn create_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    authorize(&state, &headers)?;
    logging::info("backup_requested", &[]);

    let archive = backup::create_backup_archive(&state.db, &state.db_path)
        .await
        .map_err(|error| ApiError::BackupFailed(error.to_string()))?;

    logging::info("backup_finished", &[("bytes", archive.len().to_string())]);
    let response_headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-tar"),
        ),
        (
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"susun-studio-backup.tar\""),
        ),
    ];
    Ok((response_headers, archive))
}

/// Validates an uploaded backup archive and returns a safe, non-mutating
/// preview (compatibility, replacement scope, what must be re-entered). No
/// active data is touched here — the actual restore is a separate flow.
pub async fn preview_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<backup::RestorePreview>, ApiError> {
    authorize(&state, &headers)?;
    logging::info(
        "restore_preview_requested",
        &[("bytes", body.len().to_string())],
    );

    let preview = backup::validate_restore_archive(&body, db::latest_migration_version())
        .map_err(|error| ApiError::RestoreArchiveInvalid(error.to_string()))?;

    logging::info(
        "restore_preview_finished",
        &[("compatible", preview.compatible.to_string())],
    );
    Ok(Json(preview))
}

/// Prepares a restore: validates, stages a migrated copy, writes a pre-restore
/// backup, and returns the on-disk handoff for the Tauri supervisor. Active data
/// is not mutated here.
pub async fn prepare_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<restore::PreparedRestore>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "restore_prepare_requested",
        &[("bytes", body.len().to_string())],
    );

    let prepared = restore::prepare_restore(&state.restore, &state.db, &state.db_path, &body)
        .await
        .map_err(map_restore_error)?;

    logging::warn(
        "restore_prepare_finished",
        &[("restore_id", prepared.restore_id.clone())],
    );
    Ok(Json(prepared))
}

/// Enters the shutdown-pending state and triggers graceful shutdown so the
/// supervisor can swap the database file with no live handle held here.
pub async fn begin_restore_shutdown(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn("restore_shutdown_requested", &[]);
    state.restore.begin_restore_shutdown();
    Ok(Json(serde_json::json!({ "status": "shutting_down" })))
}

/// Reports daemon availability so the supervisor/UI can block while a restore
/// is preparing or the daemon is about to exit for a swap.
pub async fn restore_availability(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(
        serde_json::json!({ "availability": state.restore.availability() }),
    ))
}

fn map_restore_error(error: restore::RestoreError) -> ApiError {
    match error {
        restore::RestoreError::Archive(_) | restore::RestoreError::Incompatible(_) => {
            ApiError::RestoreArchiveInvalid(error.to_string())
        }
        other => ApiError::RestoreFailed(other.to_string()),
    }
}
