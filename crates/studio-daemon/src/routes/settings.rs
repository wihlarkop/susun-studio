use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use turso::params;

use crate::{auth::authorize, error::ApiError, state::AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct StudioSettings {
    pub default_project_root: String,
}

pub async fn get_settings(
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

pub async fn update_settings(
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
