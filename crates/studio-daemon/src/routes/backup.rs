use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::IntoResponse,
};

use crate::{auth::authorize, backup, db, error::ApiError, logging, state::AppState};

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
