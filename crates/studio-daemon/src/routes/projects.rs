use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use susun::ProjectSummary;
use turso::params;

use crate::{auth::authorize, error::ApiError, logging, state::AppState, susun_integration};

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
    pub summary: Option<ProjectSummary>,
    pub diagnostics: Option<serde_json::Value>,
    pub runtime_profile_id: Option<String>,
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
                    summary_json, diagnostics_json, runtime_profile_id
             FROM projects ORDER BY created_at_ms DESC",
            (),
        )
        .await?;
    let mut projects = Vec::new();

    while let Some(row) = rows.next().await? {
        let has_errors: Option<i64> = row.get(5)?;
        let summary_json: Option<String> = row.get(6)?;
        let diagnostics_json: Option<String> = row.get(7)?;
        let runtime_profile_id: Option<String> = row.get(8)?;

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
            runtime_profile_id,
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
        runtime_profile_id: None,
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

    logging::info(
        "project_created",
        &[
            ("project_id", project.id.clone()),
            ("name", project.name.clone()),
        ],
    );

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
    #[serde(default)]
    pub runtime_profile_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportProjectResponse {
    pub project: Option<ProjectResponse>,
    pub summary: Option<ProjectSummary>,
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
        logging::warn(
            "project_import_rejected",
            &[("reason", "missing_compose_files".to_owned())],
        );
        return Err(ApiError::MissingComposeFiles);
    }

    logging::info(
        "project_import_started",
        &[
            ("file_count", request.files.len().to_string()),
            ("has_env_file", request.env_file.is_some().to_string()),
            ("profile_count", request.profiles.len().to_string()),
        ],
    );

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
        logging::warn(
            "project_import_blocked",
            &[("has_errors", analyzed.has_errors.to_string())],
        );
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
    let summary_json = serde_json::to_string(&analyzed.summary)?;
    let diagnostics_json = analyzed.diagnostics.to_string();

    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO projects (
            id, name, path, created_at_ms,
            compose_files, env_file, project_name_override, profiles,
            last_analyzed_at_ms, summary_json, diagnostics_json, has_errors,
            runtime_profile_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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
            has_errors = excluded.has_errors,
            runtime_profile_id = COALESCE(excluded.runtime_profile_id, projects.runtime_profile_id)",
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
            request.runtime_profile_id.clone(),
        ],
    )
    .await?;

    logging::info(
        "project_import_finished",
        &[
            ("project_id", source_id.clone()),
            ("name", display_name.clone()),
            ("has_errors", analyzed.has_errors.to_string()),
            (
                "diagnostic_count",
                analyzed
                    .diagnostics
                    .get("diagnostics")
                    .and_then(|value| value.as_array())
                    .map(|items| items.len())
                    .unwrap_or_default()
                    .to_string(),
            ),
        ],
    );

    let mut created_rows = conn
        .query(
            "SELECT created_at_ms, runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
            params![source_id.clone()],
        )
        .await?;
    let (created_at_ms, stored_runtime_profile_id): (i64, Option<String>) =
        match created_rows.next().await? {
            Some(row) => (row.get(0)?, row.get(1)?),
            None => (now, request.runtime_profile_id.clone()),
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
                summary: Some(analyzed.summary.clone()),
                diagnostics: Some(analyzed.diagnostics.clone()),
                runtime_profile_id: stored_runtime_profile_id,
            }),
            summary: Some(analyzed.summary),
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

#[derive(Debug, Deserialize)]
pub struct SetProjectEngineRequest {
    pub runtime_profile_id: Option<String>,
}

pub async fn set_project_engine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<SetProjectEngineRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let conn = state.db.connect()?;

    if let Some(profile_id) = &request.runtime_profile_id {
        // Materialize the existence check before writing on the same
        // connection — turso silently drops a write issued while an earlier
        // read cursor is still open.
        let exists = {
            let mut rows = conn
                .query(
                    "SELECT id FROM runtime_profiles WHERE id = ?1 LIMIT 1",
                    params![profile_id.clone()],
                )
                .await?;
            rows.next().await?.is_some()
        };
        if !exists {
            return Err(ApiError::RuntimeProfileNotFound);
        }
    }

    let affected = conn
        .execute(
            "UPDATE projects SET runtime_profile_id = ?1 WHERE id = ?2",
            params![request.runtime_profile_id.clone(), project_id.clone()],
        )
        .await?;
    if affected == 0 {
        return Err(ApiError::ProjectNotFound);
    }

    logging::info(
        "project_engine_bound",
        &[
            ("project_id", project_id),
            (
                "runtime_profile_id",
                request
                    .runtime_profile_id
                    .unwrap_or_else(|| "<active>".to_owned()),
            ),
        ],
    );
    Ok(Json(serde_json::json!({ "updated": true })))
}

pub async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let affected = conn
        .execute(
            "DELETE FROM projects WHERE id = ?1",
            params![project_id.clone()],
        )
        .await?;
    if affected == 0 {
        return Err(ApiError::ProjectNotFound);
    }

    logging::info("project_deleted", &[("project_id", project_id)]);

    Ok(Json(serde_json::json!({ "deleted": true })))
}
