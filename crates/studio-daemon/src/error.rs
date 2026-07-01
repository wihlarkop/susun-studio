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

    #[error("database error: {0}")]
    Database(#[from] turso::Error),

    #[error("clock error")]
    Clock,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
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
