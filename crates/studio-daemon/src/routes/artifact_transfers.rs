use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;
use susun::ContainerEngine;
use tokio::task::JoinHandle;
use turso::{Database, params};

use crate::{
    auth::authorize,
    error::ApiError,
    jobs::{error_taxonomy::classify_transfer_error, transfer_progress::persist_transfer_progress},
    registry::{
        credential_store::{CredentialStoreError, RegistryCredentialId, SecretValue},
        identity::RegistryIdentity,
        repository::{self, RegistryCredentialMetadata, RegistryRepositoryError},
    },
    routes::{
        engines::{ResolvedEngine, resolve_and_validate_engine, revalidate_engine_still_selected},
        jobs::JobResponse,
    },
    runtime,
    state::AppState,
    susun_integration,
};

const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_IMAGE_REFERENCE_BYTES: usize = 1_024;

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

    let engine = susun_integration::connect_engine_for_profile(
        &state.db,
        resolved.runtime_profile_id.as_deref(),
    )
    .await
    .map_err(ApiError::EngineUnavailable)?;
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
    let response = queued_pull_response(job_id.clone(), now);

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
            runtime_profile_id, runtime_class, created_at_ms, updated_at_ms
         ) VALUES (?1, 'image_pull', 'queued', '', ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            job_id.to_owned(),
            resolved.engine_id.clone(),
            request_json,
            resolved.runtime_profile_id.clone(),
            resolved.runtime_class.clone(),
            now,
        ],
    )
    .await?;
    Ok(())
}

fn queued_pull_response(job_id: String, now: i64) -> JobResponse {
    JobResponse {
        id: job_id,
        kind: "image_pull".to_owned(),
        status: "queued".to_owned(),
        project_id: String::new(),
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
    let engine = susun_integration::connect_engine_for_profile(
        &state.db,
        resolved.runtime_profile_id.as_deref(),
    )
    .await
    .map_err(|_| PullWorkerError::EngineUnavailable)?;
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
        let resolved = ResolvedEngine {
            engine_id: "profile-1".to_owned(),
            runtime_profile_id: Some("profile-1".to_owned()),
            runtime_class: Some("external_local".to_owned()),
        };
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
        let resolved = ResolvedEngine {
            engine_id: "engine-docker-local".to_owned(),
            runtime_profile_id: None,
            runtime_class: None,
        };
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
