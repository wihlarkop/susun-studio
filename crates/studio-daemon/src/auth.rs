use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request},
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;

use crate::{error::ApiError, logging, state::AppState};

pub fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(value) = headers.get("authorization") else {
        logging::warn(
            "auth_failed",
            &[("reason", "missing_authorization".to_owned())],
        );
        return Err(ApiError::Unauthorized);
    };

    let Ok(value) = value.to_str() else {
        logging::warn("auth_failed", &[("reason", "invalid_header".to_owned())]);
        return Err(ApiError::Unauthorized);
    };

    let expected = format!("Bearer {}", state.auth_token);
    verify_secret(value, &expected)
}

/// Compares two secrets in constant time to avoid leaking length-independent
/// timing information about the expected value.
pub fn verify_secret(candidate: &str, expected: &str) -> Result<(), ApiError> {
    if candidate.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

pub async fn require_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    authorize(&state, &headers)?;
    Ok(next.run(request).await)
}
