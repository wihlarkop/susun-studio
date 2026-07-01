use axum::http::HeaderMap;

use crate::{error::ApiError, state::AppState};

pub fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
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
