use std::path::PathBuf;

use turso::params;

use crate::{error::ApiError, state::AppState};

pub struct ProjectSource {
    pub files: Vec<PathBuf>,
    pub env_file: Option<PathBuf>,
    pub project_name: Option<String>,
    pub profiles: Vec<String>,
}

pub async fn load_project_source(
    state: &AppState,
    project_id: &str,
) -> Result<ProjectSource, ApiError> {
    let conn = state.db.connect()?;

    // Read in a scope so the cursor closes before any later write.
    let (compose_files_json, env_file, project_name, profiles_json): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = {
        let mut rows = conn
            .query(
                "SELECT compose_files, env_file, project_name_override, profiles
                 FROM projects WHERE id = ?1 LIMIT 1",
                params![project_id.to_owned()],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Err(ApiError::ProjectNotFound);
        };
        (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)
    };

    let Some(compose_files_json) = compose_files_json else {
        return Err(ApiError::PlanningFailed(
            "project has no source metadata; import it first".to_owned(),
        ));
    };

    let stored_files: Vec<String> = serde_json::from_str(&compose_files_json).unwrap_or_default();
    let files = stored_files
        .iter()
        .map(|path| resolve_path(path))
        .collect::<Result<Vec<PathBuf>, ApiError>>()?;
    let env_file = match env_file.as_deref() {
        Some(path) => Some(resolve_path(path)?),
        None => None,
    };
    let profiles: Vec<String> = profiles_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();

    Ok(ProjectSource {
        files,
        env_file,
        project_name,
        profiles,
    })
}

pub fn resolve_path(path: &str) -> Result<PathBuf, ApiError> {
    std::fs::canonicalize(path)
        .map_err(|source| ApiError::PlanningFailed(format!("`{path}`: {source}")))
}
