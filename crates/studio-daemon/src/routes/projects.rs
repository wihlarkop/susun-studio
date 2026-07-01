use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use turso::params;

use crate::{auth::authorize, error::ApiError, state::AppState};

#[derive(Debug, Serialize)]
pub struct ProjectListResponse {
    pub projects: Vec<ProjectResponse>,
}

#[derive(Debug, Serialize)]
pub struct ProjectResponse {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub path: String,
}

pub async fn list_projects(
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

pub async fn create_project(
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

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}
