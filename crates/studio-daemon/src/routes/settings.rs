use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use turso::params;

use crate::{auth::authorize, error::ApiError, state::AppState};

#[derive(Debug, Serialize, Deserialize)]
pub struct StudioSettings {
    pub default_project_root: String,
    #[serde(default)]
    pub last_project_id: String,
}

pub async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StudioSettings>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT key, value FROM settings WHERE key IN ('default_project_root', 'last_project_id')",
            (),
        )
        .await?;

    let mut default_project_root = String::new();
    let mut last_project_id = String::new();
    while let Some(row) = rows.next().await? {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        match key.as_str() {
            "default_project_root" => default_project_root = value,
            "last_project_id" => last_project_id = value,
            _ => {}
        }
    }

    Ok(Json(StudioSettings {
        default_project_root,
        last_project_id,
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
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('last_project_id', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![settings.last_project_id.clone()],
    )
    .await?;

    Ok(Json(settings))
}
