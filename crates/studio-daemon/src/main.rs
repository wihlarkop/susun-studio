mod db;

use std::{
    net::{AddrParseError, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use turso::{Database, params};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:7377";
const DEFAULT_AUTH_TOKEN: &str = "susun-studio-dev-token";
const BIND_ADDR_ENV: &str = "SUSUN_STUDIO_DAEMON_ADDR";
const AUTH_TOKEN_ENV: &str = "SUSUN_STUDIO_DAEMON_TOKEN";
const DB_PATH_ENV: &str = "SUSUN_STUDIO_DB_PATH";

#[derive(Clone)]
struct AppState {
    db: Arc<Database>,
    auth_token: Arc<str>,
}

#[derive(Debug, thiserror::Error)]
enum DaemonError {
    #[error("invalid {name} value `{value}`: {source}")]
    InvalidBindAddr {
        name: &'static str,
        value: String,
        source: AddrParseError,
    },

    #[error("database startup failed: {0}")]
    Database(#[from] db::DbError),

    #[error("failed to bind daemon listener on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },

    #[error("daemon server failed: {0}")]
    Serve(std::io::Error),
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("unauthorized")]
    Unauthorized,

    #[error("name is required")]
    MissingName,

    #[error("path is required")]
    MissingPath,

    #[error("database error: {0}")]
    Database(#[from] turso::Error),

    #[error("clock error")]
    Clock,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    product: &'static str,
    version: &'static str,
    api_version: &'static str,
}

#[derive(Debug, Serialize)]
struct ProjectListResponse {
    projects: Vec<ProjectResponse>,
}

#[derive(Debug, Serialize)]
struct ProjectResponse {
    id: String,
    name: String,
    path: String,
    created_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    name: String,
    path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StudioSettings {
    default_project_root: String,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), DaemonError> {
    let bind_addr = bind_addr()?;
    let db_path = db_path();
    let db = db::open_database(db_path.clone()).await?;
    let state = AppState {
        db: Arc::new(db),
        auth_token: Arc::from(auth_token()),
    };

    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|source| DaemonError::Bind {
            addr: bind_addr,
            source,
        })?;
    let local_addr = listener.local_addr().map_err(|source| DaemonError::Bind {
        addr: bind_addr,
        source,
    })?;

    println!("susun-studio-daemon listening on http://{local_addr}");
    println!("susun-studio-daemon database at {}", db_path.display());

    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(DaemonError::Serve)
}

fn bind_addr() -> Result<SocketAddr, DaemonError> {
    let value = std::env::var(BIND_ADDR_ENV).unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    value
        .parse()
        .map_err(|source| DaemonError::InvalidBindAddr {
            name: BIND_ADDR_ENV,
            value,
            source,
        })
}

fn auth_token() -> String {
    std::env::var(AUTH_TOKEN_ENV).unwrap_or_else(|_| DEFAULT_AUTH_TOKEN.to_owned())
}

fn db_path() -> PathBuf {
    std::env::var(DB_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".susun-studio/studio.db"))
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/projects", get(list_projects).post(create_project))
        .route("/v1/settings", get(get_settings).put(update_settings))
        .with_state(state)
        .layer(CorsLayer::permissive())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        product: "susun-studio",
        version: env!("CARGO_PKG_VERSION"),
        api_version: "1",
    })
}

async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProjectListResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, name, path, created_at_ms FROM projects ORDER BY created_at_ms DESC",
            (),
        )
        .await?;
    let mut projects = Vec::new();

    while let Some(row) = rows.next().await? {
        projects.push(ProjectResponse {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            created_at_ms: row.get(3)?,
        });
    }

    Ok(Json(ProjectListResponse { projects }))
}

async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), ApiError> {
    authorize(&state, &headers)?;

    let name = request.name.trim();
    if name.is_empty() {
        return Err(ApiError::MissingName);
    }

    let path = request.path.trim();
    if path.is_empty() {
        return Err(ApiError::MissingPath);
    }

    let created_at_ms = now_ms()?;
    let project = ProjectResponse {
        id: format!("project-{created_at_ms}"),
        name: name.to_owned(),
        path: path.to_owned(),
        created_at_ms,
    };

    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
        params![
            project.id.clone(),
            project.name.clone(),
            project.path.clone(),
            project.created_at_ms
        ],
    )
    .await?;

    Ok((StatusCode::CREATED, Json(project)))
}

async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StudioSettings>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT value FROM settings WHERE key = 'default_project_root' LIMIT 1",
            (),
        )
        .await?;
    let default_project_root = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => String::new(),
    };

    Ok(Json(StudioSettings {
        default_project_root,
    }))
}

async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<StudioSettings>,
) -> Result<Json<StudioSettings>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('default_project_root', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![settings.default_project_root.clone()],
    )
    .await?;

    Ok(Json(settings))
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(value) = headers.get("authorization") else {
        return Err(ApiError::Unauthorized);
    };

    let Ok(value) = value.to_str() else {
        return Err(ApiError::Unauthorized);
    };

    let expected = format!("Bearer {}", state.auth_token);
    if value == expected {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::MissingName | Self::MissingPath => StatusCode::BAD_REQUEST,
            Self::Database(_) | Self::Clock => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (
            status,
            Json(ErrorResponse {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("failed to listen for shutdown signal: {error}");
    }
}
