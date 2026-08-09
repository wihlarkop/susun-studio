use std::{
    collections::HashMap,
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

use crate::{
    auth::authorize, error::ApiError, logging, runtime, state::AppState, susun_integration,
};

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
    pub last_opened_at_ms: Option<i64>,
    pub last_analyzed_at_ms: Option<i64>,
    pub has_errors: Option<bool>,
    pub summary: Option<ProjectSummary>,
    pub diagnostics: Option<serde_json::Value>,
    pub runtime_profile_id: Option<String>,
    pub runtime_binding: runtime::RuntimeBindingSummary,
}

struct ProjectRecord {
    id: String,
    name: String,
    path: String,
    created_at_ms: i64,
    last_opened_at_ms: Option<i64>,
    last_analyzed_at_ms: Option<i64>,
    has_errors: Option<bool>,
    summary: Option<ProjectSummary>,
    diagnostics: Option<serde_json::Value>,
    runtime_profile_id: Option<String>,
}

impl ProjectRecord {
    fn into_response(self, runtime_binding: runtime::RuntimeBindingSummary) -> ProjectResponse {
        ProjectResponse {
            id: self.id,
            name: self.name,
            path: self.path,
            created_at_ms: self.created_at_ms,
            last_opened_at_ms: self.last_opened_at_ms,
            last_analyzed_at_ms: self.last_analyzed_at_ms,
            has_errors: self.has_errors,
            summary: self.summary,
            diagnostics: self.diagnostics,
            runtime_profile_id: self.runtime_profile_id,
            runtime_binding,
        }
    }
}

/// Policy hydration is a total operation for the requested project ids. If a
/// future query change breaks that invariant, fail as a daemon fault instead
/// of panicking or substituting another runtime binding.
fn require_runtime_binding(
    bindings: &HashMap<String, runtime::RuntimeBindingSummary>,
    project_id: &str,
) -> Result<runtime::RuntimeBindingSummary, ApiError> {
    bindings
        .get(project_id)
        .cloned()
        .ok_or(ApiError::RuntimePolicyIncomplete)
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
            "SELECT id, name, path, created_at_ms, last_opened_at_ms, last_analyzed_at_ms, has_errors,
                    summary_json, diagnostics_json, runtime_profile_id
             FROM projects
             ORDER BY COALESCE(last_opened_at_ms, created_at_ms) DESC, name ASC",
            (),
        )
        .await?;
    let mut project_records = Vec::new();

    while let Some(row) = rows.next().await? {
        project_records.push(project_record_from_row(&row)?);
    }

    drop(rows);
    let project_ids = project_records
        .iter()
        .map(|project| project.id.clone())
        .collect::<Vec<_>>();
    let bindings = runtime::policy::summarize_projects(&state.db, &project_ids).await?;
    let projects = project_records
        .into_iter()
        .map(|project| {
            let runtime_binding = require_runtime_binding(&bindings, &project.id)?;
            Ok(project.into_response(runtime_binding))
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

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
    let project_id = format!("project-{created_at_ms}");
    let project_name = name.to_owned();
    let project_path = path.to_owned();

    let conn = state.db.connect()?;
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at_ms, last_opened_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![
            project_id.clone(),
            project_name.clone(),
            project_path.clone(),
            created_at_ms
        ],
    )
    .await?;

    logging::info(
        "project_created",
        &[("project_id", project_id.clone()), ("name", project_name)],
    );

    let project = read_project_response(&state.db, &project_id)
        .await?
        .ok_or(ApiError::ProjectNotFound)?;
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
            id, name, path, created_at_ms, last_opened_at_ms,
            compose_files, env_file, project_name_override, profiles,
            last_analyzed_at_ms, summary_json, diagnostics_json, has_errors,
            runtime_profile_id
        ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            path = excluded.path,
            last_opened_at_ms = excluded.last_opened_at_ms,
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

    let project = read_project_response(&state.db, &source_id)
        .await?
        .ok_or(ApiError::ProjectNotFound)?;

    Ok((
        StatusCode::CREATED,
        Json(ImportProjectResponse {
            project: Some(project),
            summary: Some(analyzed.summary),
            diagnostics: analyzed.diagnostics,
            has_errors: analyzed.has_errors,
        }),
    ))
}

fn canonicalize_paths(paths: &[String]) -> Result<Vec<PathBuf>, ApiError> {
    paths.iter().map(|path| canonicalize_path(path)).collect()
}

fn project_record_from_row(row: &turso::Row) -> Result<ProjectRecord, turso::Error> {
    let has_errors: Option<i64> = row.get(6)?;
    let summary_json: Option<String> = row.get(7)?;
    let diagnostics_json: Option<String> = row.get(8)?;

    Ok(ProjectRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        created_at_ms: row.get(3)?,
        last_opened_at_ms: row.get(4)?,
        last_analyzed_at_ms: row.get(5)?,
        has_errors: has_errors.map(|value| value != 0),
        summary: summary_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        diagnostics: diagnostics_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        runtime_profile_id: row.get(9)?,
    })
}

async fn read_project_response(
    db: &turso::Database,
    project_id: &str,
) -> Result<Option<ProjectResponse>, ApiError> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, name, path, created_at_ms, last_opened_at_ms, last_analyzed_at_ms, has_errors,
                    summary_json, diagnostics_json, runtime_profile_id
             FROM projects WHERE id = ?1 LIMIT 1",
            params![project_id.to_owned()],
        )
        .await?;
    let project = match rows.next().await? {
        Some(row) => project_record_from_row(&row)?,
        None => return Ok(None),
    };
    drop(rows);

    let bindings =
        runtime::policy::summarize_projects(db, std::slice::from_ref(&project.id)).await?;
    let runtime_binding = require_runtime_binding(&bindings, &project.id)?;
    Ok(Some(project.into_response(runtime_binding)))
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
) -> Result<Json<ProjectResponse>, ApiError> {
    authorize(&state, &headers)?;
    let runtime_profile_id = request.runtime_profile_id;

    if let Some(profile_id) = runtime_profile_id.as_deref() {
        match runtime::policy::validate_profile_for_binding(&state.db, profile_id).await? {
            runtime::policy::SetPreferredOutcome::Updated => {}
            runtime::policy::SetPreferredOutcome::NotFound => {
                return Err(ApiError::RuntimeProfileNotFound);
            }
            runtime::policy::SetPreferredOutcome::Unavailable => {
                return Err(ApiError::ActionUnavailable(
                    "the runtime profile is not available for project binding".to_owned(),
                ));
            }
        }
    }

    let conn = state.db.connect()?;
    // Policy validation above completes before the pin is persisted.
    let affected = conn
        .execute(
            "UPDATE projects SET runtime_profile_id = ?1 WHERE id = ?2",
            params![runtime_profile_id.clone(), project_id.clone()],
        )
        .await?;
    if affected == 0 {
        return Err(ApiError::ProjectNotFound);
    }

    logging::info(
        "project_engine_bound",
        &[
            ("project_id", project_id.clone()),
            (
                "runtime_profile_id",
                runtime_profile_id.unwrap_or_else(|| "<active>".to_owned()),
            ),
        ],
    );
    let project = read_project_response(&state.db, &project_id)
        .await?
        .ok_or(ApiError::ProjectNotFound)?;
    Ok(Json(project))
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

#[cfg(test)]
mod tests {
    use axum::{Json, extract::Path};
    use turso::params;

    use super::*;
    use crate::{
        runtime,
        test_support::{authorized_headers, fresh_db, test_state},
    };

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    async fn insert_project(state: &AppState, id: &str, profile_id: Option<&str>) -> TestResult {
        let conn = state.db.connect()?;
        conn.execute(
            "INSERT INTO projects (id, name, path, created_at_ms, runtime_profile_id)
             VALUES (?1, ?1, ?2, 1, ?3)",
            params![
                id.to_owned(),
                format!("C:/projects/{id}"),
                profile_id.map(str::to_owned)
            ],
        )
        .await?;
        Ok(())
    }

    async fn insert_profile(
        state: &AppState,
        id: &str,
        availability: &str,
        ownership: &str,
    ) -> TestResult {
        let conn = state.db.connect()?;
        conn.execute(
            "INSERT INTO runtime_profiles (
                id, provider_id, provider_runtime_key, display_name, product, platform,
                runtime_class, ownership_state, source,
                installation_state, process_state, connection_state,
                availability_state, missing_since_ms, observation_revision, observed_at_ms, created_at_ms, updated_at_ms
            ) VALUES (?1, 'windows-docker-desktop', ?2, ?3, 'docker-desktop', 'windows',
                'external_local', ?4, 'provider_discovery',
                'installed', 'running', 'summarized', ?5, ?6, 0, 1, 1, 1)",
            params![
                id.to_owned(),
                format!("engine-{id}"),
                format!("Runtime {id}"),
                ownership.to_owned(),
                availability.to_owned(),
                (availability == "missing").then_some(1_i64),
            ],
        )
        .await?;
        Ok(())
    }

    fn project<'a>(projects: &'a [ProjectResponse], id: &str) -> TestResult<&'a ProjectResponse> {
        projects
            .iter()
            .find(|project| project.id == id)
            .ok_or_else(|| std::io::Error::other(format!("missing project {id}")).into())
    }

    #[test]
    fn incomplete_policy_hydration_is_a_typed_daemon_fault() {
        let bindings = std::collections::HashMap::new();

        let result = require_runtime_binding(&bindings, "project-1");

        assert!(matches!(result, Err(ApiError::RuntimePolicyIncomplete)));
    }

    #[tokio::test]
    async fn project_responses_report_policy_backed_runtime_bindings() -> TestResult {
        let state = test_state(fresh_db("project-runtime-bindings").await?);
        insert_profile(&state, "global", "available", "external").await?;
        insert_profile(&state, "pinned", "available", "external").await?;
        insert_profile(&state, "unavailable", "missing", "external").await?;
        assert!(matches!(
            runtime::policy::set_preferred(&state.db, Some("global")).await?,
            runtime::policy::SetPreferredOutcome::Updated
        ));
        insert_project(&state, "unpinned", None).await?;
        insert_project(&state, "pinned-ready", Some("pinned")).await?;
        insert_project(&state, "pinned-missing", Some("gone")).await?;
        insert_project(&state, "pinned-unavailable", Some("unavailable")).await?;

        let response = list_projects(State(state.clone()), authorized_headers())
            .await?
            .0;
        let unpinned = project(&response.projects, "unpinned")?;
        assert_eq!(
            unpinned.runtime_binding.source,
            runtime::RuntimeBindingSource::GlobalPreference
        );
        assert_eq!(
            unpinned.runtime_binding.state,
            runtime::RuntimeBindingState::Ready
        );
        let ready = project(&response.projects, "pinned-ready")?;
        assert_eq!(
            ready.runtime_binding.source,
            runtime::RuntimeBindingSource::ProjectPin
        );
        assert_eq!(
            ready.runtime_binding.state,
            runtime::RuntimeBindingState::Ready
        );
        let missing = project(&response.projects, "pinned-missing")?;
        assert_eq!(
            missing.runtime_binding.state,
            runtime::RuntimeBindingState::Missing
        );
        assert_eq!(missing.runtime_binding.profile_id.as_deref(), Some("gone"));
        let unavailable = project(&response.projects, "pinned-unavailable")?;
        assert_eq!(
            unavailable.runtime_binding.state,
            runtime::RuntimeBindingState::Unavailable
        );

        let cleared = set_project_engine(
            State(state.clone()),
            authorized_headers(),
            Path("pinned-ready".to_owned()),
            Json(SetProjectEngineRequest {
                runtime_profile_id: None,
            }),
        )
        .await?
        .0;
        assert_eq!(cleared.runtime_profile_id, None);
        assert_eq!(
            cleared.runtime_binding.source,
            runtime::RuntimeBindingSource::GlobalPreference
        );

        let no_preference = test_state(fresh_db("project-platform-default").await?);
        let created = create_project(
            State(no_preference),
            authorized_headers(),
            Json(CreateProjectRequest {
                name: "New project".to_owned(),
                path: "C:/projects/new".to_owned(),
            }),
        )
        .await?
        .1
        .0;
        assert_eq!(
            created.runtime_binding.source,
            runtime::RuntimeBindingSource::PlatformDefault
        );
        assert_eq!(
            created.runtime_binding.state,
            runtime::RuntimeBindingState::Unconfigured
        );
        Ok(())
    }

    #[tokio::test]
    async fn project_pin_rejects_unavailable_and_ownership_conflicted_profiles() -> TestResult {
        let state = test_state(fresh_db("project-runtime-pin-rejection").await?);
        insert_project(&state, "project", None).await?;
        insert_profile(&state, "unavailable", "missing", "external").await?;
        insert_profile(&state, "conflicted", "available", "ownership_conflict").await?;

        for profile_id in ["unavailable", "conflicted"] {
            let result = set_project_engine(
                State(state.clone()),
                authorized_headers(),
                Path("project".to_owned()),
                Json(SetProjectEngineRequest {
                    runtime_profile_id: Some(profile_id.to_owned()),
                }),
            )
            .await;
            assert!(matches!(result, Err(ApiError::ActionUnavailable(_))));
        }
        Ok(())
    }

    #[tokio::test]
    async fn imported_project_includes_the_runtime_binding_summary() -> TestResult {
        let state = test_state(fresh_db("import-project-runtime-binding").await?);
        let fixture_directory = std::env::temp_dir().join(format!(
            "susun-studio-import-runtime-binding-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&fixture_directory)?;
        let compose_file = fixture_directory.join("compose.yaml");
        std::fs::write(
            &compose_file,
            "services:\n  app:\n    image: nginx:alpine\n",
        )?;

        let result = import_project(
            State(state),
            authorized_headers(),
            Json(ImportProjectRequest {
                files: vec![compose_file.to_string_lossy().into_owned()],
                env_file: None,
                project_name: None,
                profiles: Vec::new(),
                runtime_profile_id: None,
            }),
        )
        .await;
        std::fs::remove_dir_all(&fixture_directory)?;

        let response = result?.1.0;
        let project = response
            .project
            .ok_or_else(|| std::io::Error::other("expected imported project"))?;
        assert_eq!(
            project.runtime_binding.source,
            runtime::RuntimeBindingSource::PlatformDefault
        );
        assert_eq!(
            project.runtime_binding.state,
            runtime::RuntimeBindingState::Unconfigured
        );
        Ok(())
    }

    #[tokio::test]
    async fn opening_a_project_updates_only_recency_and_returns_the_updated_project() -> TestResult
    {
        let state = test_state(fresh_db("project-recency-route").await?);
        insert_project(&state, "older", None).await?;
        insert_project(&state, "newer", None).await?;

        let conn = state.db.connect()?;
        conn.execute(
            "UPDATE projects SET created_at_ms = ?1 WHERE id = ?2",
            params![10_i64, "older"],
        )
        .await?;
        conn.execute(
            "UPDATE projects SET created_at_ms = ?1 WHERE id = ?2",
            params![20_i64, "newer"],
        )
        .await?;

        let opened = mark_project_opened(
            State(state.clone()),
            authorized_headers(),
            Path("older".to_owned()),
        )
        .await?
        .0;
        assert!(opened.last_opened_at_ms.is_some());
        assert_eq!(opened.last_analyzed_at_ms, None);

        let listed = list_projects(State(state), authorized_headers()).await?.0;
        assert_eq!(listed.projects[0].id, "older");
        assert_eq!(
            listed.projects[0].last_opened_at_ms,
            opened.last_opened_at_ms
        );
        Ok(())
    }

    #[tokio::test]
    async fn opening_an_unknown_project_is_not_found() -> TestResult {
        let state = test_state(fresh_db("project-recency-not-found").await?);
        let result = mark_project_opened(
            State(state),
            authorized_headers(),
            Path("missing".to_owned()),
        )
        .await;

        assert!(matches!(result, Err(ApiError::ProjectNotFound)));
        Ok(())
    }
}

pub async fn mark_project_opened(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectResponse>, ApiError> {
    authorize(&state, &headers)?;

    let opened_at_ms = now_ms()?;
    let conn = state.db.connect()?;
    let affected = conn
        .execute(
            "UPDATE projects SET last_opened_at_ms = ?1 WHERE id = ?2",
            params![opened_at_ms, project_id.clone()],
        )
        .await?;
    if affected == 0 {
        return Err(ApiError::ProjectNotFound);
    }

    let project = read_project_response(&state.db, &project_id)
        .await?
        .ok_or(ApiError::ProjectNotFound)?;
    Ok(Json(project))
}
