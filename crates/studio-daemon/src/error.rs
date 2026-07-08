use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::db;

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("invalid {name} value `{value}`: {source}")]
    InvalidBindAddr {
        name: &'static str,
        value: String,
        source: std::net::AddrParseError,
    },

    #[error(
        "{name} must be a loopback address (127.0.0.1 or ::1), got `{value}` — Susun Studio's daemon is loopback-only by design"
    )]
    NonLoopbackBindAddr { name: &'static str, value: String },

    #[error(
        "{env_var} is required in a release build (no dev-token fallback outside debug builds)"
    )]
    MissingAuthToken { env_var: &'static str },

    #[error("database startup failed: {0}")]
    Database(#[from] db::DbError),

    #[error("failed to bind daemon listener on {addr}: {source}")]
    Bind {
        addr: std::net::SocketAddr,
        source: std::io::Error,
    },

    #[error("daemon server failed: {0}")]
    Serve(std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,

    #[error("name is required")]
    MissingName,

    #[error("path is required")]
    MissingPath,

    #[error("at least one compose file is required")]
    MissingComposeFiles,

    #[error("invalid import: {0}")]
    InvalidImport(String),

    #[error("project not found")]
    ProjectNotFound,

    #[error("plan not found")]
    PlanNotFound,

    #[error("job not found")]
    JobNotFound,

    #[error("service not found in project")]
    ServiceNotFound,

    #[error("watch session not found")]
    WatchNotFound,

    #[error("planning failed: {0}")]
    PlanningFailed(String),

    #[error("engine unavailable: {0}")]
    EngineUnavailable(String),

    #[error("action unavailable: {0}")]
    ActionUnavailable(String),

    #[error("database error: {0}")]
    Database(#[from] turso::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("clock error")]
    Clock,
}

impl From<susun::Error> for ApiError {
    fn from(error: susun::Error) -> Self {
        Self::InvalidImport(error.to_string())
    }
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::MissingName | Self::MissingPath | Self::MissingComposeFiles => {
                StatusCode::BAD_REQUEST
            }
            Self::ProjectNotFound
            | Self::PlanNotFound
            | Self::JobNotFound
            | Self::ServiceNotFound
            | Self::WatchNotFound => StatusCode::NOT_FOUND,
            Self::EngineUnavailable(_) => StatusCode::BAD_GATEWAY,
            Self::InvalidImport(_) | Self::PlanningFailed(_) | Self::ActionUnavailable(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::Database(_) | Self::Json(_) | Self::Clock => StatusCode::INTERNAL_SERVER_ERROR,
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
