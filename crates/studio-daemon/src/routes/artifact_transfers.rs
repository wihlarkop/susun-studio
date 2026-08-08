use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use susun::ContainerEngine;
use tokio::task::JoinHandle;
use turso::{Database, params};

use crate::{
    action_audit::{self, AffectedCount, AuditEntry, STATUS_COMPLETED},
    action_plans::{ActionKind, ActionPlanPayload, ImagePushPlan, PlanState},
    artifact_inventory::{self, DetailLookup},
    auth::authorize,
    error::ApiError,
    jobs::{error_taxonomy::classify_transfer_error, transfer_progress::persist_transfer_progress},
    registry::{
        credential_store::{CredentialStoreError, RegistryCredentialId, SecretValue},
        identity::RegistryIdentity,
        repository::{self, RegistryCredentialMetadata, RegistryRepositoryError},
    },
    routes::{
        artifacts::{ArtifactRuntimeContext, image_fingerprint},
        engines::{
            ResolvedEngine, connect_resolved_engine, resolve_and_validate_engine,
            revalidate_engine_still_selected,
        },
        jobs::JobResponse,
    },
    runtime,
    state::AppState,
    susun_integration,
};

const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_IMAGE_REFERENCE_BYTES: usize = 1_024;

fn runtime_binding_source_value(source: runtime::RuntimeBindingSource) -> &'static str {
    match source {
        runtime::RuntimeBindingSource::ProjectPin => "project_pin",
        runtime::RuntimeBindingSource::GlobalPreference => "global_preference",
        runtime::RuntimeBindingSource::PlatformDefault => "platform_default",
    }
}

#[derive(Deserialize)]
pub struct ImagePullRequest {
    pub image: String,
    pub credential_id: Option<String>,
}

enum PullWorkerError {
    RuntimeChanged,
    EngineUnavailable,
    Credential(CredentialStoreError),
    Provider(susun::EngineError),
}

enum PullJobOutcome {
    Finished(Result<String, PullWorkerError>),
    Cancelled,
    TimedOut,
    JoinFailed,
}

pub async fn start_image_pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
    Json(request): Json<ImagePullRequest>,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;
    let resolved = resolve_and_validate_engine(&state, &engine_id).await?;
    let image = validate_image_reference(request.image)?;
    let registry =
        RegistryIdentity::from_image_ref(&image).map_err(|_| ApiError::InvalidImageReference)?;
    let credential = resolve_credential(&state, request.credential_id, &registry).await?;

    let engine = connect_resolved_engine(&resolved).await?.engine;
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ApiError::EngineUnavailable(error.redacted_message()))?;
    if !capabilities.supports_registry_pull.is_supported() {
        return Err(ApiError::ActionUnavailable(
            "The selected engine does not support image pulls.".to_owned(),
        ));
    }
    if credential.is_some() && !capabilities.supports_registry_auth.is_supported() {
        return Err(ApiError::ActionUnavailable(
            "The selected engine does not support authenticated registry operations.".to_owned(),
        ));
    }
    drop(engine);

    let job_id = format!("job_{}", uuid::Uuid::new_v4().simple());
    let now = runtime::now_ms();
    insert_pull_job(
        &state,
        &job_id,
        &image,
        &registry,
        credential.as_ref(),
        &resolved,
        now,
    )
    .await?;
    let cancel_notify = state.transfer_jobs.register(job_id.clone());
    let response = queued_pull_response(job_id.clone(), now, &resolved);

    let worker_state = state.clone();
    tokio::spawn(async move {
        run_image_pull(
            worker_state,
            job_id,
            image,
            registry,
            credential,
            resolved,
            cancel_notify,
        )
        .await;
    });

    Ok(Json(response))
}

async fn resolve_credential(
    state: &AppState,
    credential_id: Option<String>,
    registry: &RegistryIdentity,
) -> Result<Option<RegistryCredentialMetadata>, ApiError> {
    let Some(credential_id) = credential_id else {
        return Ok(None);
    };
    let id = RegistryCredentialId::parse(credential_id)
        .map_err(|_| ApiError::RegistryCredentialNotFound)?;
    let metadata = repository::find_by_id(&state.db, &id)
        .await
        .map_err(map_repository_error)?
        .ok_or(ApiError::RegistryCredentialNotFound)?;
    if metadata.registry != *registry {
        return Err(ApiError::RegistryCredentialMismatch);
    }
    Ok(Some(metadata))
}

fn validate_image_reference(image: String) -> Result<String, ApiError> {
    if image.is_empty()
        || image.trim() != image
        || image.len() > MAX_IMAGE_REFERENCE_BYTES
        || image.chars().any(char::is_control)
    {
        return Err(ApiError::InvalidImageReference);
    }
    RegistryIdentity::from_image_ref(&image).map_err(|_| ApiError::InvalidImageReference)?;
    Ok(image)
}

async fn insert_pull_job(
    state: &AppState,
    job_id: &str,
    image: &str,
    registry: &RegistryIdentity,
    credential: Option<&RegistryCredentialMetadata>,
    resolved: &ResolvedEngine,
    now: i64,
) -> Result<(), ApiError> {
    let request_json = serde_json::to_string(&serde_json::json!({
        "kind": "image_pull",
        "image": image,
        "registry": registry.as_str(),
        "credential_id": credential.map(|value| value.id.as_str()),
        "runtime_profile_id": resolved.runtime_profile_id,
    }))?;
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO jobs (
            id, kind, status, project_id, engine_id, request_json,
            runtime_profile_id, runtime_class, runtime_binding_source, created_at_ms, updated_at_ms
         ) VALUES (?1, 'image_pull', 'queued', '', ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            job_id.to_owned(),
            resolved.engine_id.clone(),
            request_json,
            resolved.runtime_profile_id.clone(),
            resolved.runtime_class.clone(),
            runtime_binding_source_value(resolved.runtime_binding_source),
            now,
        ],
    )
    .await?;
    Ok(())
}

fn queued_pull_response(job_id: String, now: i64, resolved: &ResolvedEngine) -> JobResponse {
    JobResponse {
        id: job_id,
        kind: "image_pull".to_owned(),
        status: "queued".to_owned(),
        project_id: String::new(),
        runtime_profile_id: resolved.runtime_profile_id.clone(),
        runtime_class: resolved.runtime_class.clone(),
        runtime_binding_source: Some(resolved.runtime_binding_source),
        service_name: None,
        actions: Vec::new(),
        result: None,
        error: None,
        error_code: None,
        progress: Vec::new(),
        transfer_progress: Vec::new(),
        created_at_ms: now,
        updated_at_ms: now,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_image_pull(
    state: AppState,
    job_id: String,
    image: String,
    registry: RegistryIdentity,
    credential: Option<RegistryCredentialMetadata>,
    resolved: ResolvedEngine,
    cancel_notify: Arc<tokio::sync::Notify>,
) {
    update_job_status(&state.db, &job_id, "running").await;
    let credential_id = credential.as_ref().map(|value| value.id.clone());
    let mut worker = tokio::spawn(execute_pull(
        state.clone(),
        job_id.clone(),
        image.clone(),
        registry.clone(),
        credential,
        resolved.clone(),
    ));
    let outcome = tokio::select! {
        result = &mut worker => match result {
            Ok(result) => PullJobOutcome::Finished(result),
            Err(_) => PullJobOutcome::JoinFailed,
        },
        () = cancel_notify.notified() => PullJobOutcome::Cancelled,
        () = tokio::time::sleep(TRANSFER_TIMEOUT) => PullJobOutcome::TimedOut,
    };
    state.transfer_jobs.finish(&job_id);
    match outcome {
        PullJobOutcome::Finished(result) => {
            finish_pull_job(
                &state.db,
                &job_id,
                &image,
                &registry,
                &resolved,
                credential_id.as_ref(),
                result,
            )
            .await;
        }
        PullJobOutcome::JoinFailed => {
            finish_pull_job(
                &state.db,
                &job_id,
                &image,
                &registry,
                &resolved,
                credential_id.as_ref(),
                Err(PullWorkerError::EngineUnavailable),
            )
            .await;
        }
        PullJobOutcome::Cancelled => {
            mark_pull_uncertain(
                &state.db,
                &job_id,
                "cancelled",
                "transfer_cancelled_result_uncertain",
                "Studio stopped waiting for the pull. The provider outcome is still being checked.",
            )
            .await;
            spawn_late_pull_correction(
                state.db.clone(),
                job_id,
                image,
                registry,
                resolved,
                credential_id,
                worker,
            );
        }
        PullJobOutcome::TimedOut => {
            mark_pull_uncertain(
                &state.db,
                &job_id,
                "failed",
                "transfer_timeout_result_uncertain",
                "The pull timed out in Studio. The provider outcome is still being checked.",
            )
            .await;
            spawn_late_pull_correction(
                state.db.clone(),
                job_id,
                image,
                registry,
                resolved,
                credential_id,
                worker,
            );
        }
    }
}

async fn execute_pull(
    state: AppState,
    job_id: String,
    image: String,
    registry: RegistryIdentity,
    credential: Option<RegistryCredentialMetadata>,
    resolved: ResolvedEngine,
) -> Result<String, PullWorkerError> {
    if !revalidate_engine_still_selected(&state.db, &resolved.engine_id)
        .await
        .map_err(|_| PullWorkerError::EngineUnavailable)?
    {
        return Err(PullWorkerError::RuntimeChanged);
    }
    // The worker revalidates the selected identity above, then connects from
    // this exact resolution. Re-reading a profile id here would leave a
    // second policy lookup between the durable job's attribution and its
    // provider call.
    let engine = connect_resolved_engine(&resolved)
        .await
        .map_err(|_| PullWorkerError::EngineUnavailable)?
        .engine;
    let progress = make_transfer_progress_sink(state.db.clone(), job_id);
    let mut request =
        susun::PullImageRequest::new(susun::ImageRef::new(image), susun::PullPolicy::Always);

    match credential {
        Some(metadata) => {
            let credential_ref = susun::RegistryCredentialRef::new(metadata.id.as_str().to_owned())
                .map_err(|_| PullWorkerError::Credential(CredentialStoreError::InvalidId))?;
            request = request.with_credential_ref(credential_ref);
            let id = metadata.id;
            let store = state.registry_credentials.clone();
            let secret = tokio::task::spawn_blocking(move || store.get(&id))
                .await
                .map_err(|_| PullWorkerError::Credential(CredentialStoreError::Platform))?
                .map_err(PullWorkerError::Credential)?;
            let auth = auth_material(metadata.username_label.as_deref(), secret, &registry);
            engine
                .pull_image_authenticated(request, auth, progress)
                .await
                .map(|pulled| pulled.reference)
                .map_err(PullWorkerError::Provider)
        }
        None => engine
            .pull_image(request, progress)
            .await
            .map(|pulled| pulled.reference)
            .map_err(PullWorkerError::Provider),
    }
}

fn auth_material(
    username: Option<&str>,
    secret: SecretValue,
    registry: &RegistryIdentity,
) -> susun::RegistryAuthMaterial {
    let auth = match username {
        Some(username) => susun::RegistryAuthMaterial::username_password(username, secret.expose()),
        None => susun::RegistryAuthMaterial::registry_token(secret.expose()),
    };
    auth.with_server_address(registry.as_str())
}

fn make_transfer_progress_sink(db: Arc<Database>, job_id: String) -> susun::ProgressSink {
    let sequence = Arc::new(AtomicI64::new(0));
    susun::ProgressSink::new(move |progress| {
        let db = db.clone();
        let job_id = job_id.clone();
        let sequence = sequence.clone();
        Box::pin(async move {
            let sequence = sequence.fetch_add(1, Ordering::SeqCst);
            let _ = persist_transfer_progress(&db, &job_id, sequence, progress).await;
        })
    })
}

async fn update_job_status(db: &Database, job_id: &str, status: &str) {
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "UPDATE jobs SET status = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![status.to_owned(), runtime::now_ms(), job_id.to_owned()],
        )
        .await;
}

async fn finish_pull_job(
    db: &Database,
    job_id: &str,
    image: &str,
    registry: &RegistryIdentity,
    resolved: &ResolvedEngine,
    credential_id: Option<&RegistryCredentialId>,
    result: Result<String, PullWorkerError>,
) {
    let Ok(conn) = db.connect() else {
        return;
    };
    let now = runtime::now_ms();
    match result {
        Ok(pulled_reference) => {
            let result_json = serde_json::to_string(&serde_json::json!({
                "image_reference": pulled_reference,
                "requested_image": image,
                "registry": registry.as_str(),
                "engine_id": resolved.engine_id,
                "runtime_profile_id": resolved.runtime_profile_id,
                "authenticated": credential_id.is_some(),
            }))
            .unwrap_or_default();
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = 'succeeded', result_json = ?1,
                     error = NULL, error_code = NULL, updated_at_ms = ?2 WHERE id = ?3",
                    params![result_json, now, job_id.to_owned()],
                )
                .await;
            if let Some(credential_id) = credential_id {
                let _ = repository::touch_success(db, credential_id, now).await;
            }
        }
        Err(error) => {
            let (code, message) = classify_pull_worker_error(&error);
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = 'failed', error = ?1, error_code = ?2,
                     updated_at_ms = ?3 WHERE id = ?4",
                    params![message, code, now, job_id.to_owned()],
                )
                .await;
        }
    }
}

fn classify_pull_worker_error(error: &PullWorkerError) -> (&'static str, &'static str) {
    match error {
        PullWorkerError::RuntimeChanged => (
            "runtime_changed",
            "The selected runtime changed before the pull started.",
        ),
        PullWorkerError::EngineUnavailable => (
            "engine_unavailable",
            "The selected engine could not be reached.",
        ),
        PullWorkerError::Credential(CredentialStoreError::Missing) => (
            "credential_missing",
            "The saved registry credential is missing. Sign in again and retry.",
        ),
        PullWorkerError::Credential(_) => (
            "credential_unavailable",
            "The native credential store could not provide this credential.",
        ),
        PullWorkerError::Provider(error) => classify_transfer_error(error),
    }
}

async fn mark_pull_uncertain(db: &Database, job_id: &str, status: &str, code: &str, message: &str) {
    let Ok(conn) = db.connect() else {
        return;
    };
    let _ = conn
        .execute(
            "UPDATE jobs SET status = ?1, error = ?2, error_code = ?3,
             updated_at_ms = ?4 WHERE id = ?5",
            params![status, message, code, runtime::now_ms(), job_id.to_owned()],
        )
        .await;
}

fn spawn_late_pull_correction(
    db: Arc<Database>,
    job_id: String,
    image: String,
    registry: RegistryIdentity,
    resolved: ResolvedEngine,
    credential_id: Option<RegistryCredentialId>,
    worker: JoinHandle<Result<String, PullWorkerError>>,
) {
    tokio::spawn(async move {
        let result = worker
            .await
            .unwrap_or(Err(PullWorkerError::EngineUnavailable));
        finish_pull_job(
            &db,
            &job_id,
            &image,
            &registry,
            &resolved,
            credential_id.as_ref(),
            result,
        )
        .await;
    });
}

fn map_repository_error(error: RegistryRepositoryError) -> ApiError {
    match error {
        RegistryRepositoryError::Database(source) => ApiError::Database(source),
        RegistryRepositoryError::InvalidMetadata => ApiError::CredentialOperationFailed,
    }
}

#[cfg(test)]
mod pull_tests {
    use axum::{
        Json,
        extract::{Path, State},
    };
    use turso::params;

    use super::*;
    use crate::registry::credential_store::RegistryCredentialId;
    use crate::test_support::{authorized_headers, fresh_db, test_state};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn fake_engine_id_is_rejected_before_job_creation() -> TestResult {
        let state = test_state(fresh_db("pull-fake-engine").await?);
        let result = start_image_pull(
            State(state.clone()),
            authorized_headers(),
            Path("fake-engine".to_owned()),
            Json(ImagePullRequest {
                image: "alpine:latest".to_owned(),
                credential_id: None,
            }),
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::error::ApiError::EngineNotFound)
        ));
        let conn = state.db.connect()?;
        let mut rows = conn.query("SELECT COUNT(*) FROM jobs", ()).await?;
        let count: i64 = rows.next().await?.ok_or("missing count")?.get(0)?;
        assert_eq!(count, 0);
        Ok(())
    }

    async fn insert_credential(
        state: &AppState,
        registry: &str,
    ) -> Result<RegistryCredentialMetadata, Box<dyn std::error::Error>> {
        let metadata = RegistryCredentialMetadata {
            id: RegistryCredentialId::new(),
            registry: RegistryIdentity::parse(registry)?,
            username_label: Some("studio-user".to_owned()),
            created_at_ms: 1,
            updated_at_ms: 1,
            last_success_at_ms: None,
        };
        repository::insert(&state.db, &metadata).await?;
        Ok(metadata)
    }

    #[tokio::test]
    async fn credential_registry_mismatch_is_rejected_before_engine_connection() -> TestResult {
        let state = test_state(fresh_db("pull-credential-mismatch").await?);
        let credential = insert_credential(&state, "registry.example").await?;
        let result = start_image_pull(
            State(state),
            authorized_headers(),
            Path(crate::routes::engines::PLATFORM_DEFAULT_ENGINE_ID.to_owned()),
            Json(ImagePullRequest {
                image: "other.example/team/app:latest".to_owned(),
                credential_id: Some(credential.id.as_str().to_owned()),
            }),
        )
        .await;
        assert!(matches!(result, Err(ApiError::RegistryCredentialMismatch)));
        Ok(())
    }

    #[tokio::test]
    async fn missing_credential_is_rejected_before_engine_connection() -> TestResult {
        let state = test_state(fresh_db("pull-credential-missing").await?);
        let result = start_image_pull(
            State(state),
            authorized_headers(),
            Path(crate::routes::engines::PLATFORM_DEFAULT_ENGINE_ID.to_owned()),
            Json(ImagePullRequest {
                image: "registry.example/team/app:latest".to_owned(),
                credential_id: Some(RegistryCredentialId::new().as_str().to_owned()),
            }),
        )
        .await;
        assert!(matches!(result, Err(ApiError::RegistryCredentialNotFound)));
        Ok(())
    }

    #[test]
    fn malformed_image_references_are_rejected() {
        for image in [
            "",
            " alpine:latest",
            "https://registry.example/app:latest",
            "user@registry.example/app:latest",
            "registry.example/app:latest?token=secret",
        ] {
            assert!(
                validate_image_reference(image.to_owned()).is_err(),
                "{image}"
            );
        }
    }

    #[tokio::test]
    async fn job_payload_contains_only_opaque_credential_metadata() -> TestResult {
        let state = test_state(fresh_db("pull-secret-free-job").await?);
        let credential = insert_credential(&state, "registry.example").await?;
        let resolved = crate::routes::engines::resolved_engine_for_test(
            "profile-1",
            Some("profile-1"),
            Some("external_local"),
            runtime::RuntimeBindingSource::GlobalPreference,
        );
        insert_pull_job(
            &state,
            "job-1",
            "registry.example/team/app:latest",
            &RegistryIdentity::parse("registry.example")?,
            Some(&credential),
            &resolved,
            1,
        )
        .await?;
        let conn = state.db.connect()?;
        let mut rows = conn
            .query(
                "SELECT request_json FROM jobs WHERE id = ?1",
                params!["job-1"],
            )
            .await?;
        let request_json: String = rows.next().await?.ok_or("missing job")?.get(0)?;
        assert!(request_json.contains(credential.id.as_str()));
        assert!(!request_json.contains("password"));
        assert!(!request_json.contains("secret"));
        assert!(!request_json.contains("token"));
        Ok(())
    }

    #[tokio::test]
    async fn completed_provider_result_corrects_an_uncertain_terminal_state() -> TestResult {
        let db = fresh_db("pull-late-correction").await?;
        let conn = db.connect()?;
        conn.execute(
            "INSERT INTO jobs (
                id, kind, status, project_id, engine_id, request_json,
                created_at_ms, updated_at_ms
             ) VALUES ('job-1', 'image_pull', 'cancelled', '', 'engine-docker-local', '{}', 1, 1)",
            (),
        )
        .await?;
        let resolved = crate::routes::engines::resolved_engine_for_test(
            "engine-docker-local",
            None,
            None,
            runtime::RuntimeBindingSource::PlatformDefault,
        );
        finish_pull_job(
            &db,
            "job-1",
            "alpine:latest",
            &RegistryIdentity::parse("docker.io")?,
            &resolved,
            None,
            Ok("docker.io/library/alpine:latest".to_owned()),
        )
        .await;
        let mut rows = conn
            .query(
                "SELECT status, error_code, result_json FROM jobs WHERE id = 'job-1'",
                (),
            )
            .await?;
        let row = rows.next().await?.ok_or("missing corrected job")?;
        assert_eq!(row.get::<String>(0)?, "succeeded");
        assert_eq!(row.get::<Option<String>>(1)?, None);
        assert!(
            row.get::<String>(2)?
                .contains("docker.io/library/alpine:latest")
        );
        Ok(())
    }
}

enum PushWorkerError {
    RuntimeChanged,
    EngineUnavailable,
    Credential(CredentialStoreError),
    Provider(susun::EngineError),
}

struct PushProviderResult {
    image: String,
    digest: Option<String>,
}

enum PushJobOutcome {
    Finished(Result<PushProviderResult, PushWorkerError>),
    Cancelled,
    TimedOut,
    JoinFailed,
}

#[derive(Deserialize)]
pub struct ImagePushPreviewRequest {
    pub destination: String,
    pub credential_id: Option<String>,
}

#[derive(Serialize)]
pub struct ImagePushPreview {
    pub engine_id: String,
    pub runtime: ArtifactRuntimeContext,
    pub source_image_id: String,
    pub source_references: Vec<String>,
    pub destination: String,
    pub registry: String,
    pub push_capability: String,
    pub auth_capability: String,
    pub authenticated: bool,
    pub active_jobs: i64,
    pub active_watch_sessions: i64,
    pub commit_enabled: bool,
    pub warning: Option<String>,
    pub plan_id: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

pub async fn preview_image_push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((engine_id, image_id)): Path<(String, String)>,
    Json(request): Json<ImagePushPreviewRequest>,
) -> Result<Json<ImagePushPreview>, ApiError> {
    authorize(&state, &headers)?;
    let resolved = resolve_and_validate_engine(&state, &engine_id).await?;
    let destination = validate_push_destination(request.destination)?;
    let registry = RegistryIdentity::from_image_ref(&destination)
        .map_err(|_| ApiError::InvalidImageReference)?;
    let credential = resolve_credential(&state, request.credential_id, &registry).await?;
    let runtime_ctx = ArtifactRuntimeContext::from(&resolved);
    let engine = connect_resolved_engine(&resolved).await?.engine;
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ApiError::EngineUnavailable(error.redacted_message()))?;
    let registry_capabilities = artifact_inventory::registry_capability(&engine).await?;
    let lookup = artifact_inventory::image_details(&engine, &image_id).await?;
    let source = match lookup {
        DetailLookup::Found { value, .. } => value,
        DetailLookup::NotFound => return Err(ApiError::ArtifactNotFound),
        DetailLookup::Unsupported { .. } => {
            return Err(ApiError::ActionUnavailable(
                "Image details are unavailable for this runtime.".to_owned(),
            ));
        }
    };
    if !source.references.iter().any(|value| value == &destination) {
        return Err(ApiError::ActionUnavailable(
            "Tag this image with the destination reference before pushing it.".to_owned(),
        ));
    }
    let push_supported = capabilities.supports_registry_push.is_supported();
    let auth_supported = capabilities.supports_registry_auth.is_supported();
    let (active_jobs, active_watch_sessions) = crate::routes::engines::engine_active_work(
        &state.db,
        resolved.runtime_profile_id.as_deref(),
    )
    .await?;
    let has_active_work = active_jobs > 0 || active_watch_sessions > 0;
    let authenticated = credential.is_some();
    let capability_blocked = !push_supported || (authenticated && !auth_supported);
    let warning = if !push_supported {
        Some("Image push is not supported by this runtime.".to_owned())
    } else if authenticated && !auth_supported {
        Some("Authenticated registry operations are not supported by this runtime.".to_owned())
    } else if has_active_work {
        Some("Stop active jobs and watch sessions before pushing.".to_owned())
    } else {
        Some(
            "Registry tags are mutable. A successful push does not establish image trust."
                .to_owned(),
        )
    };
    let (plan_id, expires_in_seconds) = if capability_blocked || has_active_work {
        (None, None)
    } else {
        let identity_fingerprint = crate::routes::engines::engine_identity_fingerprint(
            &state.db,
            &engine,
            resolved.runtime_profile_id.as_deref(),
        )
        .await;
        let ticket = state.action_plans.prepare(
            &runtime::stable_suffix(&state.auth_token),
            ActionKind::ImagePush,
            ActionPlanPayload::ImagePush(ImagePushPlan {
                engine_id: resolved.engine_id.clone(),
                runtime_profile_id: resolved.runtime_profile_id.clone(),
                source_image_id: source.id.clone(),
                destination: destination.clone(),
                registry: registry.as_str().to_owned(),
                credential_id: credential
                    .as_ref()
                    .map(|value| value.id.as_str().to_owned()),
                credential_updated_at_ms: credential.as_ref().map(|value| value.updated_at_ms),
                identity_fingerprint,
                source_fingerprint: image_fingerprint(&source),
                active_work_fingerprint: active_work_fingerprint(
                    active_jobs,
                    active_watch_sessions,
                ),
            }),
        );
        (Some(ticket.plan_id), Some(ticket.expires_in_seconds))
    };

    Ok(Json(ImagePushPreview {
        engine_id: resolved.engine_id,
        runtime: runtime_ctx,
        source_image_id: source.id,
        source_references: source.references,
        destination,
        registry: registry.as_str().to_owned(),
        push_capability: registry_capabilities.supports_push,
        auth_capability: registry_capabilities.supports_auth,
        authenticated,
        active_jobs,
        active_watch_sessions,
        commit_enabled: plan_id.is_some(),
        warning,
        plan_id,
        expires_in_seconds,
    }))
}

pub async fn commit_image_push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<JobResponse>, ApiError> {
    authorize(&state, &headers)?;
    crate::routes::engines::reject_commit_body(&body)?;
    let owner = runtime::stable_suffix(&state.auth_token);
    let claimed = state
        .action_plans
        .claim(&plan_id, &owner, &[ActionKind::ImagePush])
        .map_err(|error| ApiError::ActionUnavailable(error.to_string()))?;
    let ActionPlanPayload::ImagePush(plan) = claimed.payload else {
        state
            .action_plans
            .finish(&claimed.plan_id, PlanState::Failed);
        return Err(ApiError::ActionUnavailable(
            "Plan did not match an image push.".to_owned(),
        ));
    };

    let result = revalidate_push_plan(&state, &plan).await;
    let (resolved, source, registry, credential) = match result {
        Ok(value) => value,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::ImagePush,
                plan.runtime_profile_id,
                "rejected_state",
                "push_revalidation_failed",
            )
            .await;
            return Err(error);
        }
    };
    let job_id = format!("job_{}", uuid::Uuid::new_v4().simple());
    let now = runtime::now_ms();
    if let Err(error) = insert_push_job(
        &state,
        &job_id,
        &source.id,
        &plan.destination,
        &registry,
        credential.as_ref(),
        &resolved,
        now,
    )
    .await
    {
        state
            .action_plans
            .finish(&claimed.plan_id, PlanState::Failed);
        return Err(error);
    }
    let cancel_notify = state.transfer_jobs.register(job_id.clone());
    state
        .action_plans
        .finish(&claimed.plan_id, PlanState::Succeeded);
    let _ = action_audit::record(
        &state.db,
        AuditEntry {
            kind: ActionKind::ImagePush,
            profile_id: resolved.runtime_profile_id.clone(),
            runtime_class: resolved.runtime_class.clone(),
            ownership_result: "authorized".to_owned(),
            command_kind: Some("durable_image_push_queued".to_owned()),
            elevation_mode: Some("none".to_owned()),
            terminal_status: STATUS_COMPLETED.to_owned(),
            affected: vec![AffectedCount {
                category: "images".to_owned(),
                count: 1,
            }],
            failure_code: None,
            correlation_token: None,
            started_at_ms: now,
            completed_at_ms: Some(now),
        },
    )
    .await;
    let response = queued_transfer_response(&job_id, "image_push", now, &resolved);
    let worker_state = state.clone();
    tokio::spawn(async move {
        run_image_push(
            worker_state,
            job_id,
            plan.destination,
            registry,
            credential,
            resolved,
            cancel_notify,
        )
        .await;
    });
    Ok(Json(response))
}

async fn revalidate_push_plan(
    state: &AppState,
    plan: &ImagePushPlan,
) -> Result<
    (
        ResolvedEngine,
        artifact_inventory::ImageSummaryRow,
        RegistryIdentity,
        Option<RegistryCredentialMetadata>,
    ),
    ApiError,
> {
    let resolved = resolve_and_validate_engine(state, &plan.engine_id).await?;
    if resolved.runtime_profile_id != plan.runtime_profile_id {
        return Err(ApiError::ActionUnavailable(
            "The selected runtime changed. Preview the push again.".to_owned(),
        ));
    }
    let engine = susun_integration::connect_engine_for_profile(
        &state.db,
        plan.runtime_profile_id.as_deref(),
    )
    .await
    .map_err(ApiError::EngineUnavailable)?;
    let identity = crate::routes::engines::engine_identity_fingerprint(
        &state.db,
        &engine,
        plan.runtime_profile_id.as_deref(),
    )
    .await;
    if identity != plan.identity_fingerprint {
        return Err(ApiError::ActionUnavailable(
            "The engine identity changed. Preview the push again.".to_owned(),
        ));
    }
    let capabilities = engine
        .capabilities()
        .await
        .map_err(|error| ApiError::EngineUnavailable(error.redacted_message()))?;
    if !capabilities.supports_registry_push.is_supported() {
        return Err(ApiError::ActionUnavailable(
            "Image push is no longer supported by this runtime.".to_owned(),
        ));
    }
    let source = match artifact_inventory::image_details(&engine, &plan.source_image_id).await? {
        DetailLookup::Found { value, .. } => value,
        DetailLookup::NotFound => return Err(ApiError::ArtifactNotFound),
        DetailLookup::Unsupported { .. } => {
            return Err(ApiError::ActionUnavailable(
                "Image details are no longer available.".to_owned(),
            ));
        }
    };
    if image_fingerprint(&source) != plan.source_fingerprint
        || !source
            .references
            .iter()
            .any(|value| value == &plan.destination)
    {
        return Err(ApiError::ActionUnavailable(
            "The source image changed. Preview the push again.".to_owned(),
        ));
    }
    let registry =
        RegistryIdentity::parse(&plan.registry).map_err(|_| ApiError::InvalidRegistryIdentity)?;
    if RegistryIdentity::from_image_ref(&plan.destination)
        .ok()
        .as_ref()
        != Some(&registry)
    {
        return Err(ApiError::ActionUnavailable(
            "The destination registry changed. Preview the push again.".to_owned(),
        ));
    }
    let credential = resolve_credential(state, plan.credential_id.clone(), &registry).await?;
    if credential.as_ref().map(|value| value.updated_at_ms) != plan.credential_updated_at_ms {
        return Err(ApiError::ActionUnavailable(
            "The registry credential changed. Preview the push again.".to_owned(),
        ));
    }
    if credential.is_some() && !capabilities.supports_registry_auth.is_supported() {
        return Err(ApiError::ActionUnavailable(
            "Authenticated push is no longer supported by this runtime.".to_owned(),
        ));
    }
    if let Some(metadata) = &credential {
        let id = metadata.id.clone();
        let store = state.registry_credentials.clone();
        let present = tokio::task::spawn_blocking(move || store.contains(&id))
            .await
            .map_err(|_| ApiError::CredentialStoreUnavailable)?
            .map_err(|_| ApiError::CredentialStoreUnavailable)?;
        if !present {
            return Err(ApiError::RegistryCredentialNotFound);
        }
    }
    let (jobs, watch) =
        crate::routes::engines::engine_active_work(&state.db, plan.runtime_profile_id.as_deref())
            .await?;
    if active_work_fingerprint(jobs, watch) != plan.active_work_fingerprint {
        return Err(ApiError::ActionUnavailable(
            "Active work changed. Preview the push again.".to_owned(),
        ));
    }
    Ok((resolved, source, registry, credential))
}

fn validate_push_destination(destination: String) -> Result<String, ApiError> {
    validate_image_reference(destination)
}

fn active_work_fingerprint(jobs: i64, watch: i64) -> String {
    format!("{}:{}", jobs.max(0), watch.max(0))
}

#[allow(clippy::too_many_arguments)]
async fn insert_push_job(
    state: &AppState,
    job_id: &str,
    source_image_id: &str,
    destination: &str,
    registry: &RegistryIdentity,
    credential: Option<&RegistryCredentialMetadata>,
    resolved: &ResolvedEngine,
    now: i64,
) -> Result<(), ApiError> {
    let request_json = serde_json::to_string(&serde_json::json!({
        "kind": "image_push",
        "source_image_id": source_image_id,
        "destination": destination,
        "registry": registry.as_str(),
        "credential_id": credential.map(|value| value.id.as_str()),
        "runtime_profile_id": resolved.runtime_profile_id,
    }))?;
    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO jobs (
            id, kind, status, project_id, engine_id, request_json,
            runtime_profile_id, runtime_class, runtime_binding_source, created_at_ms, updated_at_ms
         ) VALUES (?1, 'image_push', 'queued', '', ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            job_id.to_owned(),
            resolved.engine_id.clone(),
            request_json,
            resolved.runtime_profile_id.clone(),
            resolved.runtime_class.clone(),
            runtime_binding_source_value(resolved.runtime_binding_source),
            now,
        ],
    )
    .await?;
    Ok(())
}

fn queued_transfer_response(
    job_id: &str,
    kind: &str,
    now: i64,
    resolved: &ResolvedEngine,
) -> JobResponse {
    JobResponse {
        id: job_id.to_owned(),
        kind: kind.to_owned(),
        status: "queued".to_owned(),
        project_id: String::new(),
        runtime_profile_id: resolved.runtime_profile_id.clone(),
        runtime_class: resolved.runtime_class.clone(),
        runtime_binding_source: Some(resolved.runtime_binding_source),
        service_name: None,
        actions: Vec::new(),
        result: None,
        error: None,
        error_code: None,
        progress: Vec::new(),
        transfer_progress: Vec::new(),
        created_at_ms: now,
        updated_at_ms: now,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_image_push(
    state: AppState,
    job_id: String,
    destination: String,
    registry: RegistryIdentity,
    credential: Option<RegistryCredentialMetadata>,
    resolved: ResolvedEngine,
    cancel_notify: Arc<tokio::sync::Notify>,
) {
    update_job_status(&state.db, &job_id, "running").await;
    let credential_id = credential.as_ref().map(|value| value.id.clone());
    let mut worker = tokio::spawn(execute_push(
        state.clone(),
        job_id.clone(),
        destination.clone(),
        registry.clone(),
        credential,
        resolved.clone(),
    ));
    let outcome = tokio::select! {
        result = &mut worker => match result {
            Ok(result) => PushJobOutcome::Finished(result),
            Err(_) => PushJobOutcome::JoinFailed,
        },
        () = cancel_notify.notified() => PushJobOutcome::Cancelled,
        () = tokio::time::sleep(TRANSFER_TIMEOUT) => PushJobOutcome::TimedOut,
    };
    state.transfer_jobs.finish(&job_id);
    match outcome {
        PushJobOutcome::Finished(result) => {
            finish_push_job(
                &state.db,
                &job_id,
                &destination,
                &registry,
                &resolved,
                credential_id.as_ref(),
                result,
            )
            .await;
        }
        PushJobOutcome::JoinFailed => {
            finish_push_job(
                &state.db,
                &job_id,
                &destination,
                &registry,
                &resolved,
                credential_id.as_ref(),
                Err(PushWorkerError::EngineUnavailable),
            )
            .await;
        }
        PushJobOutcome::Cancelled => {
            mark_pull_uncertain(
                &state.db,
                &job_id,
                "cancelled",
                "transfer_cancelled_result_uncertain",
                "Studio stopped waiting for the push. The provider outcome is still being checked.",
            )
            .await;
            spawn_late_push_correction(
                state.db.clone(),
                job_id,
                destination,
                registry,
                resolved,
                credential_id,
                worker,
            );
        }
        PushJobOutcome::TimedOut => {
            mark_pull_uncertain(
                &state.db,
                &job_id,
                "failed",
                "transfer_timeout_result_uncertain",
                "The push timed out in Studio. The provider outcome is still being checked.",
            )
            .await;
            spawn_late_push_correction(
                state.db.clone(),
                job_id,
                destination,
                registry,
                resolved,
                credential_id,
                worker,
            );
        }
    }
}

async fn execute_push(
    state: AppState,
    job_id: String,
    destination: String,
    registry: RegistryIdentity,
    credential: Option<RegistryCredentialMetadata>,
    resolved: ResolvedEngine,
) -> Result<PushProviderResult, PushWorkerError> {
    if !revalidate_engine_still_selected(&state.db, &resolved.engine_id)
        .await
        .map_err(|_| PushWorkerError::EngineUnavailable)?
    {
        return Err(PushWorkerError::RuntimeChanged);
    }
    // See `execute_pull`: the revalidated resolution owns both the provider
    // connection and the durable job attribution.
    let engine = connect_resolved_engine(&resolved)
        .await
        .map_err(|_| PushWorkerError::EngineUnavailable)?
        .engine;
    let progress = make_transfer_progress_sink(state.db.clone(), job_id);
    let mut request = susun::ImagePushRequest::new(susun::ImageRef::new(destination));
    let result = match credential {
        Some(metadata) => {
            let credential_ref = susun::RegistryCredentialRef::new(metadata.id.as_str().to_owned())
                .map_err(|_| PushWorkerError::Credential(CredentialStoreError::InvalidId))?;
            request = request.with_credential_ref(credential_ref);
            let id = metadata.id;
            let store = state.registry_credentials.clone();
            let secret = tokio::task::spawn_blocking(move || store.get(&id))
                .await
                .map_err(|_| PushWorkerError::Credential(CredentialStoreError::Platform))?
                .map_err(PushWorkerError::Credential)?;
            let auth = auth_material(metadata.username_label.as_deref(), secret, &registry);
            engine
                .push_image_authenticated(request, auth, progress)
                .await
                .map_err(PushWorkerError::Provider)?
        }
        None => engine
            .push_image(request, progress)
            .await
            .map_err(PushWorkerError::Provider)?,
    };
    Ok(PushProviderResult {
        image: result.image.as_str().to_owned(),
        digest: result.digest,
    })
}

async fn finish_push_job(
    db: &Database,
    job_id: &str,
    destination: &str,
    registry: &RegistryIdentity,
    resolved: &ResolvedEngine,
    credential_id: Option<&RegistryCredentialId>,
    result: Result<PushProviderResult, PushWorkerError>,
) {
    let Ok(conn) = db.connect() else {
        return;
    };
    let now = runtime::now_ms();
    match result {
        Ok(pushed) => {
            let result_json = serde_json::to_string(&serde_json::json!({
                "image_reference": pushed.image,
                "destination": destination,
                "registry": registry.as_str(),
                "digest": pushed.digest,
                "engine_id": resolved.engine_id,
                "runtime_profile_id": resolved.runtime_profile_id,
                "authenticated": credential_id.is_some(),
            }))
            .unwrap_or_default();
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = 'succeeded', result_json = ?1,
                     error = NULL, error_code = NULL, updated_at_ms = ?2 WHERE id = ?3",
                    params![result_json, now, job_id.to_owned()],
                )
                .await;
            if let Some(credential_id) = credential_id {
                let _ = repository::touch_success(db, credential_id, now).await;
            }
        }
        Err(error) => {
            let (code, message) = classify_push_worker_error(&error);
            let _ = conn
                .execute(
                    "UPDATE jobs SET status = 'failed', error = ?1, error_code = ?2,
                     updated_at_ms = ?3 WHERE id = ?4",
                    params![message, code, now, job_id.to_owned()],
                )
                .await;
        }
    }
}

fn classify_push_worker_error(error: &PushWorkerError) -> (&'static str, &'static str) {
    match error {
        PushWorkerError::RuntimeChanged => (
            "runtime_changed",
            "The selected runtime changed before the push started.",
        ),
        PushWorkerError::EngineUnavailable => (
            "engine_unavailable",
            "The selected engine could not be reached.",
        ),
        PushWorkerError::Credential(CredentialStoreError::Missing) => (
            "credential_missing",
            "The saved registry credential is missing. Sign in again and retry.",
        ),
        PushWorkerError::Credential(_) => (
            "credential_unavailable",
            "The native credential store could not provide this credential.",
        ),
        PushWorkerError::Provider(error) => classify_transfer_error(error),
    }
}

fn spawn_late_push_correction(
    db: Arc<Database>,
    job_id: String,
    destination: String,
    registry: RegistryIdentity,
    resolved: ResolvedEngine,
    credential_id: Option<RegistryCredentialId>,
    worker: JoinHandle<Result<PushProviderResult, PushWorkerError>>,
) {
    tokio::spawn(async move {
        let result = worker
            .await
            .unwrap_or(Err(PushWorkerError::EngineUnavailable));
        finish_push_job(
            &db,
            &job_id,
            &destination,
            &registry,
            &resolved,
            credential_id.as_ref(),
            result,
        )
        .await;
    });
}

#[cfg(test)]
mod push_tests {
    use axum::{
        Json,
        body::Bytes,
        extract::{Path, State},
    };
    use turso::params;

    use super::*;
    use crate::test_support::{authorized_headers, fresh_db, test_state};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn push_destination_rejects_unsafe_and_ambiguous_references() -> TestResult {
        for destination in [
            "",
            " alpine:latest",
            "https://registry.example/team/app:latest",
            "registry.example/team/app:latest?token=secret",
            "user@registry.example/team/app:latest",
        ] {
            assert!(validate_push_destination(destination.to_owned()).is_err());
        }
        let valid = validate_push_destination("registry.example/team/app:v1".to_owned())?;
        assert_eq!(valid, "registry.example/team/app:v1");
        Ok(())
    }

    #[test]
    fn active_work_fingerprint_changes_with_jobs_or_watch_sessions() {
        assert_eq!(active_work_fingerprint(0, 0), "0:0");
        assert_ne!(active_work_fingerprint(1, 0), active_work_fingerprint(0, 0));
        assert_ne!(active_work_fingerprint(0, 1), active_work_fingerprint(0, 0));
    }

    #[tokio::test]
    async fn preview_rejects_a_fake_engine_before_minting_a_plan() -> TestResult {
        let state = test_state(fresh_db("push-fake-engine").await?);
        let result = preview_image_push(
            State(state),
            authorized_headers(),
            Path(("fake-engine".to_owned(), "sha256:abc".to_owned())),
            Json(ImagePushPreviewRequest {
                destination: "registry.example/team/app:v1".to_owned(),
                credential_id: None,
            }),
        )
        .await;
        assert!(matches!(result, Err(ApiError::EngineNotFound)));
        Ok(())
    }

    #[tokio::test]
    async fn commit_rejects_frontend_content_before_claiming_a_plan() -> TestResult {
        let state = test_state(fresh_db("push-commit-body").await?);
        let result = commit_image_push(
            State(state),
            authorized_headers(),
            Path("opaque-plan".to_owned()),
            Bytes::from_static(br#"{"destination":"substitution"}"#),
        )
        .await;
        assert!(matches!(result, Err(ApiError::TrustedPlanContentRejected)));
        Ok(())
    }

    #[tokio::test]
    async fn push_job_payload_contains_only_opaque_credential_metadata() -> TestResult {
        let state = test_state(fresh_db("push-secret-free-job").await?);
        let credential = RegistryCredentialMetadata {
            id: RegistryCredentialId::new(),
            registry: RegistryIdentity::parse("registry.example")?,
            username_label: Some("studio-user".to_owned()),
            created_at_ms: 1,
            updated_at_ms: 1,
            last_success_at_ms: None,
        };
        let resolved = crate::routes::engines::resolved_engine_for_test(
            "profile-1",
            Some("profile-1"),
            Some("external_local"),
            runtime::RuntimeBindingSource::GlobalPreference,
        );
        insert_push_job(
            &state,
            "job-1",
            "sha256:abc",
            "registry.example/team/app:v1",
            &credential.registry,
            Some(&credential),
            &resolved,
            1,
        )
        .await?;
        let conn = state.db.connect()?;
        let mut rows = conn
            .query(
                "SELECT request_json FROM jobs WHERE id = ?1",
                params!["job-1"],
            )
            .await?;
        let request_json: String = rows.next().await?.ok_or("missing job")?.get(0)?;
        assert!(request_json.contains(credential.id.as_str()));
        assert!(!request_json.contains("password"));
        assert!(!request_json.contains("secret"));
        assert!(!request_json.contains("token"));
        Ok(())
    }

    #[tokio::test]
    async fn completed_push_result_corrects_an_uncertain_terminal_state() -> TestResult {
        let db = fresh_db("push-late-correction").await?;
        let conn = db.connect()?;
        conn.execute(
            "INSERT INTO jobs (
                id, kind, status, project_id, engine_id, request_json,
                created_at_ms, updated_at_ms
             ) VALUES ('job-1', 'image_push', 'cancelled', '', 'engine-docker-local', '{}', 1, 1)",
            (),
        )
        .await?;
        let resolved = crate::routes::engines::resolved_engine_for_test(
            "engine-docker-local",
            None,
            None,
            runtime::RuntimeBindingSource::PlatformDefault,
        );
        finish_push_job(
            &db,
            "job-1",
            "registry.example/team/app:v1",
            &RegistryIdentity::parse("registry.example")?,
            &resolved,
            None,
            Ok(PushProviderResult {
                image: "registry.example/team/app:v1".to_owned(),
                digest: Some("sha256:digest".to_owned()),
            }),
        )
        .await;
        let mut rows = conn
            .query(
                "SELECT status, error_code, result_json FROM jobs WHERE id = 'job-1'",
                (),
            )
            .await?;
        let row = rows.next().await?.ok_or("missing corrected job")?;
        assert_eq!(row.get::<String>(0)?, "succeeded");
        assert_eq!(row.get::<Option<String>>(1)?, None);
        let result: String = row.get(2)?;
        assert!(result.contains("sha256:digest"));
        assert!(result.contains("registry.example/team/app:v1"));
        Ok(())
    }
}
