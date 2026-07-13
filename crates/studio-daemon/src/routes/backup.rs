use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    response::IntoResponse,
};
use serde::Serialize;

use crate::{
    action_audit::{self, AffectedCount, AuditEntry},
    action_plans::{ActionKind, ActionPlanPayload, MetadataRestorePlan},
    auth::authorize,
    backup, db,
    error::ApiError,
    logging, restore, runtime,
    state::AppState,
};

/// A restore preview plus the opaque plan handle that binds the following
/// `prepare` call to *this* archive's validated identity.
#[derive(Debug, Serialize)]
pub struct RestorePreviewResponse {
    #[serde(flatten)]
    pub preview: backup::RestorePreview,
    pub plan_id: String,
    pub expires_in_seconds: u64,
}

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
) -> Result<Json<RestorePreviewResponse>, ApiError> {
    authorize(&state, &headers)?;
    logging::info(
        "restore_preview_requested",
        &[("bytes", body.len().to_string())],
    );

    // Validate once and bind the plan to the archive's database identity so the
    // following `prepare` cannot substitute a different archive.
    let (preview, db_bytes) = backup::validated_database(&body, db::latest_migration_version())
        .map_err(|error| ApiError::RestoreArchiveInvalid(error.to_string()))?;
    let archive_sha256 = backup::sha256_hex(&db_bytes);

    let owner = runtime::stable_suffix(&state.auth_token);
    let ticket = state.action_plans.prepare(
        &owner,
        ActionKind::MetadataRestore,
        ActionPlanPayload::MetadataRestore(MetadataRestorePlan {
            archive_sha256,
            manifest: preview.manifest.clone(),
        }),
    );

    logging::info(
        "restore_preview_finished",
        &[("compatible", preview.compatible.to_string())],
    );
    Ok(Json(RestorePreviewResponse {
        preview,
        plan_id: ticket.plan_id,
        expires_in_seconds: ticket.expires_in_seconds,
    }))
}

/// Prepares a restore: claims the opaque plan minted at preview, verifies the
/// uploaded archive still hashes to the previewed identity (rejecting any
/// substituted archive), stages a migrated copy, writes a pre-restore backup, and
/// returns the on-disk handoff for the Tauri supervisor. Active data is not
/// mutated here, and no replacement path is accepted — the daemon derives paths.
pub async fn prepare_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<restore::PreparedRestore>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "restore_prepare_requested",
        &[("bytes", body.len().to_string())],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    let started = crate::runtime::now_ms();

    let claimed = state
        .action_plans
        .claim(&plan_id, &owner, Some(ActionKind::MetadataRestore))
        .map_err(|error| ApiError::ActionUnavailable(error.to_string()))?;
    let ActionPlanPayload::MetadataRestore(plan) = claimed.payload else {
        return Err(ApiError::ActionUnavailable(
            "Plan did not match a metadata restore.".to_owned(),
        ));
    };

    // Reject a substituted archive: the bytes must hash to the previewed identity.
    let (_, db_bytes) =
        backup::validated_database(&body, db::latest_migration_version()).map_err(|error| {
            state
                .action_plans
                .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            ApiError::RestoreArchiveInvalid(error.to_string())
        })?;
    if backup::sha256_hex(&db_bytes) != plan.archive_sha256 {
        state
            .action_plans
            .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        let _ = action_audit::record_rejection(
            &state.db,
            ActionKind::MetadataRestore,
            None,
            "rejected_stale",
            "archive_substituted",
        )
        .await;
        return Err(ApiError::RestoreArchiveInvalid(
            "the uploaded archive does not match the previewed backup".to_owned(),
        ));
    }

    let prepared =
        match restore::prepare_restore(&state.restore, &state.db, &state.db_path, &body).await {
            Ok(prepared) => prepared,
            Err(error) => {
                state
                    .action_plans
                    .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
                let _ = action_audit::record_rejection(
                    &state.db,
                    ActionKind::MetadataRestore,
                    None,
                    "failed",
                    "restore_prepare_failed",
                )
                .await;
                return Err(map_restore_error(error));
            }
        };

    state
        .action_plans
        .finish(&claimed.plan_id, crate::action_plans::PlanState::Succeeded);
    let _ = action_audit::record(
        &state.db,
        AuditEntry {
            kind: ActionKind::MetadataRestore,
            profile_id: None,
            runtime_class: None,
            ownership_result: "authorized".to_owned(),
            command_kind: Some("metadata_restore_staged".to_owned()),
            elevation_mode: Some("none".to_owned()),
            terminal_status: action_audit::STATUS_COMPLETED.to_owned(),
            affected: vec![AffectedCount {
                category: "restored_projects".to_owned(),
                count: plan.manifest.project_count,
            }],
            failure_code: None,
            started_at_ms: started,
            completed_at_ms: Some(crate::runtime::now_ms()),
        },
    )
    .await;

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
