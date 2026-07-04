use axum::http::HeaderMap;
use subtle::ConstantTimeEq;

use crate::{error::ApiError, state::AppState};

pub fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(value) = headers.get("authorization") else {
        return Err(ApiError::Unauthorized);
    };

    let Ok(value) = value.to_str() else {
        return Err(ApiError::Unauthorized);
    };

    let expected = format!("Bearer {}", state.auth_token);
    verify_secret(value, &expected)
}

/// Compares two secrets in constant time to avoid leaking length-independent
/// timing information about the expected value.
pub fn verify_secret(candidate: &str, expected: &str) -> Result<(), ApiError> {
    if candidate
        .as_bytes()
        .ct_eq(expected.as_bytes())
        .into()
    {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}
