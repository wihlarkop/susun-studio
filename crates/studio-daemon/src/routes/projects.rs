use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use turso::params;

use crate::{auth::authorize, error::ApiError, state::AppState, susun_integration};

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
    pub last_analyzed_at_ms: Option<i64>,
    pub has_errors: Option<bool>,
    pub summary: Option<serde_json::Value>,
    pub diagnostics: Option<serde_json::Value>,
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
            "SELECT id, name, path, created_at_ms, last_analyzed_at_ms, has_errors,
                    summary_json, diagnostics_json
             FROM projects ORDER BY created_at_ms DESC",
            (),
        )
        .await?;
    let mut projects = Vec::new();

    while let Some(row) = rows.next().await? {
        let has_errors: Option<i64> = row.get(5)?;
        let summary_json: Option<String> = row.get(6)?;
        let diagnostics_json: Option<String> = row.get(7)?;

        projects.push(ProjectResponse {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            created_at_ms: row.get(3)?,
            last_analyzed_at_ms: row.get(4)?,
            has_errors: has_errors.map(|value| value != 0),
            summary: summary_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            diagnostics: diagnostics_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
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
        last_analyzed_at_ms: None,
        has_errors: None,
        summary: None,
        diagnostics: None,
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

#[derive(Debug, Deserialize)]
pub struct ImportProjectRequest {
    pub files: Vec<String>,
    pub env_file: Option<String>,
    pub project_name: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportProjectResponse {
    pub project: Option<ProjectResponse>,
    pub summary: Option<serde_json::Value>,
    pub diagnostics: serde_json::Value,
    pub has_errors: bool,
}

pub async fn import_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ImportProjectRequest>,
) -> Result<(StatusCode, Json<ImportProjectResponse>), ApiError> {
    authorize(&state, &headers)?;

    if request.files.is_empty() {
        return Err(ApiError::MissingComposeFiles);
    }

    let files = canonicalize_paths(&request.files)?;
    let env_file = match &request.env_file {
        Some(path) => Some(canonicalize_path(path)?),
        None => None,
    };

    let analyzed = susun_integration::analyze_project(
        &files,
        env_file.as_ref(),
        request.project_name.as_deref(),
        &request.profiles,
    )?;

    let Some(source_id) = analyzed.source_id.clone() else {
        return Ok((
            StatusCode::OK,
            Json(ImportProjectResponse {
                project: None,
                summary: None,
                diagnostics: analyzed.diagnostics,
                has_errors: true,
            }),
        ));
    };

    let now = now_ms()?;
    let display_name = analyzed
        .project_name
        .clone()
        .unwrap_or_else(|| source_id.clone());
    let project_directory = analyzed.project_directory.to_string_lossy().into_owned();
    let compose_files_json = serde_json::to_string(&request.files).unwrap_or_default();
    let profiles_json = serde_json::to_string(&request.profiles).unwrap_or_default();
    let summary_value = serde_json::to_value(&analyzed.summary).unwrap_or(serde_json::Value::Null);
    let summary_json = serde_json::to_string(&analyzed.summary).unwrap_or_default();
    let diagnostics_json = analyzed.diagnostics.to_string();

    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO projects (
            id, name, path, created_at_ms,
            compose_files, env_file, project_name_override, profiles,
            last_analyzed_at_ms, summary_json, diagnostics_json, has_errors
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            path = excluded.path,
            compose_files = excluded.compose_files,
            env_file = excluded.env_file,
            project_name_override = excluded.project_name_override,
            profiles = excluded.profiles,
            last_analyzed_at_ms = excluded.last_analyzed_at_ms,
            summary_json = excluded.summary_json,
            diagnostics_json = excluded.diagnostics_json,
            has_errors = excluded.has_errors",
        params![
            source_id.clone(),
            display_name.clone(),
            project_directory.clone(),
            now,
            compose_files_json,
            request.env_file.clone(),
            request.project_name.clone(),
            profiles_json,
            now,
            summary_json,
            diagnostics_json,
            i64::from(analyzed.has_errors),
        ],
    )
    .await?;

    let mut created_rows = conn
        .query(
            "SELECT created_at_ms FROM projects WHERE id = ?1 LIMIT 1",
            params![source_id.clone()],
        )
        .await?;
    let created_at_ms = match created_rows.next().await? {
        Some(row) => row.get(0)?,
        None => now,
    };

    Ok((
        StatusCode::CREATED,
        Json(ImportProjectResponse {
            project: Some(ProjectResponse {
                id: source_id,
                name: display_name,
                path: project_directory,
                created_at_ms,
                last_analyzed_at_ms: Some(now),
                has_errors: Some(analyzed.has_errors),
                summary: Some(summary_value.clone()),
                diagnostics: Some(analyzed.diagnostics.clone()),
            }),
            summary: Some(summary_value),
            diagnostics: analyzed.diagnostics,
            has_errors: analyzed.has_errors,
        }),
    ))
}

fn canonicalize_paths(paths: &[String]) -> Result<Vec<PathBuf>, ApiError> {
    paths.iter().map(|path| canonicalize_path(path)).collect()
}

fn canonicalize_path(path: &str) -> Result<PathBuf, ApiError> {
    std::fs::canonicalize(path)
        .map_err(|source| ApiError::InvalidImport(format!("`{path}`: {source}")))
}
