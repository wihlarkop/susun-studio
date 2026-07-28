use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::authorize,
    error::ApiError,
    registry::{
        credential_store::{
            CredentialStoreError, RegistryCredentialId, RegistryCredentialStore, SecretValue,
        },
        identity::RegistryIdentity,
        repository::{self, RegistryCredentialMetadata, RegistryRepositoryError},
    },
    runtime::now_ms,
    state::AppState,
};

const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_USERNAME_BYTES: usize = 255;
pub(crate) const MAX_REQUEST_BYTES: usize = 70 * 1024;

#[derive(Deserialize)]
pub struct RegistryCredentialWriteRequest {
    pub registry: String,
    pub username: Option<String>,
    pub secret: String,
}

#[derive(Deserialize)]
pub struct RegistryCredentialRotateRequest {
    pub username: Option<String>,
    pub secret: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegistryCredentialStatus {
    Ready,
    ReauthenticationRequired,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RegistryCredentialResponse {
    pub id: String,
    pub registry: String,
    pub username_label: Option<String>,
    pub status: RegistryCredentialStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_success_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RegistryCredentialListResponse {
    pub credentials: Vec<RegistryCredentialResponse>,
}

pub async fn list_registry_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RegistryCredentialListResponse>, ApiError> {
    authorize(&state, &headers)?;
    let metadata = repository::list(&state.db)
        .await
        .map_err(map_repository_error)?;
    let mut credentials = Vec::with_capacity(metadata.len());
    for item in metadata {
        let status =
            credential_status(Arc::clone(&state.registry_credentials), item.id.clone()).await;
        credentials.push(response(item, status));
    }
    Ok(Json(RegistryCredentialListResponse { credentials }))
}

pub async fn create_registry_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RegistryCredentialWriteRequest>,
) -> Result<Json<RegistryCredentialResponse>, ApiError> {
    authorize(&state, &headers)?;
    let registry = RegistryIdentity::parse(&request.registry)
        .map_err(|_| ApiError::InvalidRegistryIdentity)?;
    validate_secret(&request.secret)?;
    let username_label = normalize_username(request.username)?;
    if repository::find_by_registry(&state.db, &registry)
        .await
        .map_err(map_repository_error)?
        .is_some()
    {
        return Err(ApiError::RegistryCredentialConflict);
    }

    let id = RegistryCredentialId::new();
    store_put(
        Arc::clone(&state.registry_credentials),
        id.clone(),
        SecretValue::new(request.secret),
    )
    .await?;
    let now = now_ms();
    let metadata = RegistryCredentialMetadata {
        id: id.clone(),
        registry,
        username_label,
        created_at_ms: now,
        updated_at_ms: now,
        last_success_at_ms: None,
    };
    if let Err(error) = repository::insert(&state.db, &metadata).await {
        let _ = store_delete(Arc::clone(&state.registry_credentials), id).await;
        return Err(map_repository_error(error));
    }
    Ok(Json(response(metadata, RegistryCredentialStatus::Ready)))
}

pub async fn rotate_registry_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<RegistryCredentialRotateRequest>,
) -> Result<Json<RegistryCredentialResponse>, ApiError> {
    authorize(&state, &headers)?;
    let id = parse_id(id)?;
    validate_secret(&request.secret)?;
    let username_label = normalize_username(request.username)?;
    let mut metadata = repository::find_by_id(&state.db, &id)
        .await
        .map_err(map_repository_error)?
        .ok_or(ApiError::RegistryCredentialNotFound)?;

    let previous = match store_get(Arc::clone(&state.registry_credentials), id.clone()).await {
        Ok(secret) => Some(secret),
        Err(ApiError::RegistryCredentialNotFound) => None,
        Err(error) => return Err(error),
    };
    store_put(
        Arc::clone(&state.registry_credentials),
        id.clone(),
        SecretValue::new(request.secret),
    )
    .await?;
    let now = now_ms();
    if let Err(error) =
        repository::update_username(&state.db, &id, username_label.clone(), now).await
    {
        match previous {
            Some(secret) => {
                let _ =
                    store_put(Arc::clone(&state.registry_credentials), id.clone(), secret).await;
            }
            None => {
                let _ = store_delete(Arc::clone(&state.registry_credentials), id.clone()).await;
            }
        }
        return Err(map_repository_error(error));
    }
    metadata.username_label = username_label;
    metadata.updated_at_ms = now;
    Ok(Json(response(metadata, RegistryCredentialStatus::Ready)))
}

pub async fn delete_registry_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    authorize(&state, &headers)?;
    let id = parse_id(id)?;
    if repository::find_by_id(&state.db, &id)
        .await
        .map_err(map_repository_error)?
        .is_none()
    {
        return Err(ApiError::RegistryCredentialNotFound);
    }
    match store_delete(Arc::clone(&state.registry_credentials), id.clone()).await {
        Ok(()) | Err(ApiError::RegistryCredentialNotFound) => {}
        Err(error) => return Err(error),
    }
    repository::delete(&state.db, &id)
        .await
        .map_err(map_repository_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn response(
    metadata: RegistryCredentialMetadata,
    status: RegistryCredentialStatus,
) -> RegistryCredentialResponse {
    RegistryCredentialResponse {
        id: metadata.id.as_str().to_owned(),
        registry: metadata.registry.as_str().to_owned(),
        username_label: metadata.username_label,
        status,
        created_at_ms: metadata.created_at_ms,
        updated_at_ms: metadata.updated_at_ms,
        last_success_at_ms: metadata.last_success_at_ms,
    }
}

fn parse_id(value: String) -> Result<RegistryCredentialId, ApiError> {
    RegistryCredentialId::parse(value).map_err(|_| ApiError::RegistryCredentialNotFound)
}

fn normalize_username(value: Option<String>) -> Result<Option<String>, ApiError> {
    value
        .map(|value| {
            if value.is_empty()
                || value.trim() != value
                || value.len() > MAX_USERNAME_BYTES
                || value.chars().any(char::is_control)
            {
                Err(ApiError::InvalidRegistryCredential)
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn validate_secret(value: &str) -> Result<(), ApiError> {
    if value.is_empty() || value.len() > MAX_SECRET_BYTES {
        Err(ApiError::InvalidRegistryCredential)
    } else {
        Ok(())
    }
}

async fn credential_status(
    store: Arc<dyn RegistryCredentialStore>,
    id: RegistryCredentialId,
) -> RegistryCredentialStatus {
    match tokio::task::spawn_blocking(move || store.contains(&id)).await {
        Ok(Ok(true)) => RegistryCredentialStatus::Ready,
        Ok(Ok(false)) | Ok(Err(CredentialStoreError::Missing)) => {
            RegistryCredentialStatus::ReauthenticationRequired
        }
        _ => RegistryCredentialStatus::Unavailable,
    }
}

async fn store_put(
    store: Arc<dyn RegistryCredentialStore>,
    id: RegistryCredentialId,
    secret: SecretValue,
) -> Result<(), ApiError> {
    tokio::task::spawn_blocking(move || store.put(&id, secret))
        .await
        .map_err(|_| ApiError::CredentialOperationFailed)?
        .map_err(map_store_error)
}

async fn store_get(
    store: Arc<dyn RegistryCredentialStore>,
    id: RegistryCredentialId,
) -> Result<SecretValue, ApiError> {
    tokio::task::spawn_blocking(move || store.get(&id))
        .await
        .map_err(|_| ApiError::CredentialOperationFailed)?
        .map_err(map_store_error)
}

async fn store_delete(
    store: Arc<dyn RegistryCredentialStore>,
    id: RegistryCredentialId,
) -> Result<(), ApiError> {
    tokio::task::spawn_blocking(move || store.delete(&id))
        .await
        .map_err(|_| ApiError::CredentialOperationFailed)?
        .map_err(map_store_error)
}

fn map_store_error(error: CredentialStoreError) -> ApiError {
    match error {
        CredentialStoreError::Missing => ApiError::RegistryCredentialNotFound,
        CredentialStoreError::Unavailable => ApiError::CredentialStoreUnavailable,
        CredentialStoreError::Denied => ApiError::CredentialStoreDenied,
        CredentialStoreError::TooLarge => ApiError::CredentialTooLarge,
        CredentialStoreError::InvalidId => ApiError::RegistryCredentialNotFound,
        CredentialStoreError::Platform => ApiError::CredentialOperationFailed,
    }
}

fn map_repository_error(error: RegistryRepositoryError) -> ApiError {
    match error {
        RegistryRepositoryError::Database(source) => ApiError::Database(source),
        RegistryRepositoryError::InvalidMetadata => ApiError::CredentialOperationFailed,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

    use super::*;
    use crate::{
        registry::credential_store::MemoryRegistryCredentialStore,
        test_support::{authorized_headers, fresh_db, test_state},
    };

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn sign_in_request(registry: &str, secret: &str) -> RegistryCredentialWriteRequest {
        RegistryCredentialWriteRequest {
            registry: registry.to_owned(),
            username: Some("studio-user".to_owned()),
            secret: secret.to_owned(),
        }
    }

    struct UnavailableStore;

    impl RegistryCredentialStore for UnavailableStore {
        fn put(
            &self,
            _id: &RegistryCredentialId,
            _secret: SecretValue,
        ) -> Result<(), CredentialStoreError> {
            Err(CredentialStoreError::Unavailable)
        }

        fn get(&self, _id: &RegistryCredentialId) -> Result<SecretValue, CredentialStoreError> {
            Err(CredentialStoreError::Unavailable)
        }

        fn delete(&self, _id: &RegistryCredentialId) -> Result<(), CredentialStoreError> {
            Err(CredentialStoreError::Unavailable)
        }

        fn contains(&self, _id: &RegistryCredentialId) -> Result<bool, CredentialStoreError> {
            Err(CredentialStoreError::Unavailable)
        }
    }

    #[tokio::test]
    async fn lifecycle_returns_metadata_and_never_returns_secret() -> TestResult {
        let db = fresh_db("registry-credential-lifecycle").await?;
        let mut state = test_state(db);
        let store = Arc::new(MemoryRegistryCredentialStore::default());
        state.registry_credentials = store;

        let created = create_registry_credential(
            State(state.clone()),
            authorized_headers(),
            Json(sign_in_request(
                "REGISTRY.EXAMPLE",
                "sentinel-registry-secret",
            )),
        )
        .await?
        .0;
        assert_eq!(created.registry, "registry.example");
        assert_eq!(created.status, RegistryCredentialStatus::Ready);
        assert!(!serde_json::to_string(&created)?.contains("sentinel-registry-secret"));

        let listed = list_registry_credentials(State(state.clone()), authorized_headers())
            .await?
            .0;
        assert_eq!(listed.credentials, vec![created.clone()]);

        let rotated = rotate_registry_credential(
            State(state.clone()),
            authorized_headers(),
            axum::extract::Path(created.id.clone()),
            Json(RegistryCredentialRotateRequest {
                username: Some("rotated-user".to_owned()),
                secret: "replacement-secret".to_owned(),
            }),
        )
        .await?
        .0;
        assert_eq!(rotated.username_label.as_deref(), Some("rotated-user"));
        assert!(!serde_json::to_string(&rotated)?.contains("replacement-secret"));

        let status = delete_registry_credential(
            State(state.clone()),
            authorized_headers(),
            axum::extract::Path(created.id),
        )
        .await?;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(
            list_registry_credentials(State(state), authorized_headers())
                .await?
                .0
                .credentials
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_normalized_registry_is_rejected() -> TestResult {
        let db = fresh_db("registry-credential-duplicate").await?;
        let state = test_state(db);
        let _ = create_registry_credential(
            State(state.clone()),
            authorized_headers(),
            Json(sign_in_request("index.docker.io", "first-secret")),
        )
        .await?;

        let result = create_registry_credential(
            State(state),
            authorized_headers(),
            Json(sign_in_request("docker.io", "second-secret")),
        )
        .await;
        assert!(matches!(result, Err(ApiError::RegistryCredentialConflict)));
        Ok(())
    }

    #[tokio::test]
    async fn missing_os_entry_requires_reauthentication() -> TestResult {
        let db = fresh_db("registry-credential-missing-entry").await?;
        let mut state = test_state(db);
        let store = Arc::new(MemoryRegistryCredentialStore::default());
        state.registry_credentials = store.clone();
        let created = create_registry_credential(
            State(state.clone()),
            authorized_headers(),
            Json(sign_in_request("registry.example", "temporary-secret")),
        )
        .await?
        .0;
        let id = RegistryCredentialId::parse(created.id)?;
        store.delete(&id)?;

        let listed = list_registry_credentials(State(state), authorized_headers())
            .await?
            .0;
        assert_eq!(
            listed.credentials[0].status,
            RegistryCredentialStatus::ReauthenticationRequired
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalid_registry_is_rejected_before_storage() -> TestResult {
        let db = fresh_db("registry-credential-invalid-registry").await?;
        let mut state = test_state(db);
        let store = Arc::new(MemoryRegistryCredentialStore::default());
        state.registry_credentials = store.clone();

        let result = create_registry_credential(
            State(state),
            authorized_headers(),
            Json(sign_in_request(
                "https://registry.example/path",
                "sentinel-registry-secret",
            )),
        )
        .await;
        assert!(matches!(result, Err(ApiError::InvalidRegistryIdentity)));
        assert!(store.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn unavailable_os_store_returns_fixed_error() -> TestResult {
        let db = fresh_db("registry-credential-store-unavailable").await?;
        let mut state = test_state(db);
        state.registry_credentials = Arc::new(UnavailableStore);

        let result = create_registry_credential(
            State(state),
            authorized_headers(),
            Json(sign_in_request(
                "registry.example",
                "sentinel-registry-secret",
            )),
        )
        .await;
        assert!(matches!(&result, Err(ApiError::CredentialStoreUnavailable)));
        let response = result.err().ok_or("missing error")?.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }

    #[tokio::test]
    async fn metadata_failure_removes_new_os_entry() -> TestResult {
        let db = fresh_db("registry-credential-compensation").await?;
        let conn = db.connect()?;
        conn.execute("DROP TABLE registry_credentials", ()).await?;
        let mut state = test_state(db);
        let store = Arc::new(MemoryRegistryCredentialStore::default());
        state.registry_credentials = store.clone();

        let result = create_registry_credential(
            State(state),
            authorized_headers(),
            Json(sign_in_request(
                "registry.example",
                "sentinel-registry-secret",
            )),
        )
        .await;

        assert!(matches!(result, Err(ApiError::Database(_))));
        assert!(store.is_empty());
        Ok(())
    }
}
