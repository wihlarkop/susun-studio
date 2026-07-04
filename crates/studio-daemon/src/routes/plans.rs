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
use turso::params;

use crate::{auth::authorize, error::ApiError, state::AppState, susun_integration};

#[derive(Debug, Serialize)]
pub struct PlanActionResponse {
    pub id: String,
    pub kind: String,
    pub resource: String,
    pub safety: String,
    pub reason: String,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanSummaryResponse {
    pub total_actions: usize,
    pub safe_actions: usize,
    pub caution_actions: usize,
    pub destructive_actions: usize,
}

#[derive(Debug, Serialize)]
pub struct PlanResponse {
    pub id: String,
    pub project_id: String,
    pub operation: String,
    pub summary: PlanSummaryResponse,
    pub actions: Vec<PlanActionResponse>,
    pub blocked_diagnostics: Option<serde_json::Value>,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct PlanListResponse {
    pub plans: Vec<PlanResponse>,
}

pub async fn create_up_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<(StatusCode, Json<PlanResponse>), ApiError> {
    authorize(&state, &headers)?;
    create_plan(&state, &project_id, PlanOperation::Up).await
}

pub async fn create_down_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<(StatusCode, Json<PlanResponse>), ApiError> {
    authorize(&state, &headers)?;
    create_plan(&state, &project_id, PlanOperation::Down).await
}

#[derive(Clone, Copy)]
enum PlanOperation {
    Up,
    Down,
}

impl PlanOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

async fn create_plan(
    state: &AppState,
    project_id: &str,
    operation: PlanOperation,
) -> Result<(StatusCode, Json<PlanResponse>), ApiError> {
    let conn = state.db.connect()?;

    // Read the project source in a scope so the query cursor is fully closed
    // before the INSERT below. turso silently discards a write issued while a
    // read cursor is still open on the same connection.
    let (compose_files_json, env_file, project_name_override, profiles_json) = {
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
        let compose_files_json: Option<String> = row.get(0)?;
        let env_file: Option<String> = row.get(1)?;
        let project_name_override: Option<String> = row.get(2)?;
        let profiles_json: Option<String> = row.get(3)?;
        (compose_files_json, env_file, project_name_override, profiles_json)
    };

    let Some(compose_files_json) = compose_files_json else {
        return Err(ApiError::PlanningFailed(
            "project has no source metadata; import it first".to_owned(),
        ));
    };

    let stored_files: Vec<String> =
        serde_json::from_str(&compose_files_json).unwrap_or_default();
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

    let plan_row = match operation {
        PlanOperation::Up => susun_integration::plan_up(
            &files,
            env_file.as_ref(),
            project_name_override.as_deref(),
            &profiles,
            susun::UpPlanOptions::default(),
        )?,
        PlanOperation::Down => susun_integration::plan_down(
            &files,
            env_file.as_ref(),
            project_name_override.as_deref(),
            &profiles,
            susun::DownPlanOptions::default(),
        )?,
    };

    let now = now_ms()?;
    let plan_id = format!("plan-{now}-{}", operation.as_str());

    let summary = PlanSummaryResponse {
        total_actions: plan_row.total_actions,
        safe_actions: plan_row.safe_actions,
        caution_actions: plan_row.caution_actions,
        destructive_actions: plan_row.destructive_actions,
    };
    let summary_json = serde_json::to_string(&summary)?;
    let blocked_diagnostics_json = plan_row
        .blocked_diagnostics
        .as_ref()
        .map(std::string::ToString::to_string);

    conn.execute(
        "INSERT INTO plans (
            id, project_id, operation, plan_json, summary_json,
            blocked_diagnostics_json, susun_schema_version, created_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            plan_id.clone(),
            project_id.to_owned(),
            operation.as_str().to_owned(),
            plan_row.plan_json,
            summary_json,
            blocked_diagnostics_json,
            plan_row.schema_version,
            now,
        ],
    )
    .await?;

    let actions = plan_row
        .actions
        .into_iter()
        .map(|action| PlanActionResponse {
            id: action.id,
            kind: action.kind.to_owned(),
            resource: action.resource,
            safety: action.safety,
            reason: action.reason,
            dependencies: action.dependencies,
        })
        .collect();

    Ok((
        StatusCode::CREATED,
        Json(PlanResponse {
            id: plan_id,
            project_id: project_id.to_owned(),
            operation: operation.as_str().to_owned(),
            summary,
            actions,
            blocked_diagnostics: plan_row.blocked_diagnostics,
            created_at_ms: now,
        }),
    ))
}

pub async fn list_project_plans(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<PlanListResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, project_id, operation, summary_json, blocked_diagnostics_json, created_at_ms
             FROM plans WHERE project_id = ?1 ORDER BY created_at_ms DESC",
            params![project_id],
        )
        .await?;

    let mut plans = Vec::new();
    while let Some(row) = rows.next().await? {
        let summary_json: String = row.get(3)?;
        let blocked_diagnostics_json: Option<String> = row.get(4)?;

        plans.push(PlanResponse {
            id: row.get(0)?,
            project_id: row.get(1)?,
            operation: row.get(2)?,
            summary: serde_json::from_str(&summary_json).unwrap_or(PlanSummaryResponse {
                total_actions: 0,
                safe_actions: 0,
                caution_actions: 0,
                destructive_actions: 0,
            }),
            actions: Vec::new(),
            blocked_diagnostics: blocked_diagnostics_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            created_at_ms: row.get(5)?,
        });
    }

    Ok(Json(PlanListResponse { plans }))
}

pub async fn read_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
) -> Result<Json<PlanResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, project_id, operation, plan_json, blocked_diagnostics_json, created_at_ms
             FROM plans WHERE id = ?1 LIMIT 1",
            params![plan_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Err(ApiError::PlanNotFound);
    };

    let id: String = row.get(0)?;
    let project_id: String = row.get(1)?;
    let operation: String = row.get(2)?;
    let plan_json: String = row.get(3)?;
    let blocked_diagnostics_json: Option<String> = row.get(4)?;
    let created_at_ms: i64 = row.get(5)?;

    // A blocked plan stored an empty plan_json; report it as an empty action set.
    let (summary, actions) = if plan_json.is_empty() {
        (
            PlanSummaryResponse {
                total_actions: 0,
                safe_actions: 0,
                caution_actions: 0,
                destructive_actions: 0,
            },
            Vec::new(),
        )
    } else {
        let plan: susun::ExecutionPlan = serde_json::from_str(&plan_json)
            .map_err(|error| ApiError::PlanningFailed(error.to_string()))?;
        let actions = susun_integration::plan_action_rows(&plan)
            .into_iter()
            .map(|action| PlanActionResponse {
                id: action.id,
                kind: action.kind.to_owned(),
                resource: action.resource,
                safety: action.safety,
                reason: action.reason,
                dependencies: action.dependencies,
            })
            .collect();
        (
            PlanSummaryResponse {
                total_actions: plan.summary.total_actions,
                safe_actions: plan.summary.safe_actions,
                caution_actions: plan.summary.caution_actions,
                destructive_actions: plan.summary.destructive_actions,
            },
            actions,
        )
    };

    Ok(Json(PlanResponse {
        id,
        project_id,
        operation,
        summary,
        actions,
        blocked_diagnostics: blocked_diagnostics_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
        created_at_ms,
    }))
}

fn resolve_path(path: &str) -> Result<PathBuf, ApiError> {
    std::fs::canonicalize(path)
        .map_err(|source| ApiError::PlanningFailed(format!("`{path}`: {source}")))
}

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}
