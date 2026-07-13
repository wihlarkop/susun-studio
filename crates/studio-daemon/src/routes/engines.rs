use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use turso::params;

use crate::{
    action_audit::{self, AffectedCount, AuditEntry},
    action_plans::{ActionKind, ActionPlanPayload, EnginePrunePlan},
    auth::authorize,
    error::ApiError,
    logging, runtime,
    state::AppState,
    susun_integration,
};

#[derive(Debug, Serialize)]
pub struct EngineResponse {
    pub id: String,
    pub provider_kind: String,
    pub display_name: String,
    pub enabled: bool,
    pub is_default: bool,
    pub last_health: Option<serde_json::Value>,
    pub last_health_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct EngineListResponse {
    pub engines: Vec<EngineResponse>,
}

#[derive(Debug, Serialize)]
pub struct EngineHealthResponse {
    pub reachable: bool,
    pub api_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EngineCapabilitiesResponse {
    pub api_version: Option<String>,
    pub supports_health: String,
    pub supports_named_volumes: String,
    pub supports_network_aliases: String,
    pub supports_log_follow: String,
    pub supports_build: String,
    pub supports_mount_types: Vec<String>,
    pub max_container_name_length: Option<usize>,
}

pub async fn list_engines(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<EngineListResponse>, ApiError> {
    authorize(&state, &headers)?;

    let conn = state.db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, provider_kind, display_name, enabled, is_default,
                    last_health_json, last_health_at_ms
             FROM engines ORDER BY is_default DESC, created_at_ms ASC",
            (),
        )
        .await?;

    let mut engines = Vec::new();
    while let Some(row) = rows.next().await? {
        let enabled: i64 = row.get(3)?;
        let is_default: i64 = row.get(4)?;
        let last_health_json: Option<String> = row.get(5)?;

        engines.push(EngineResponse {
            id: row.get(0)?,
            provider_kind: row.get(1)?,
            display_name: row.get(2)?,
            enabled: enabled != 0,
            is_default: is_default != 0,
            last_health: last_health_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok()),
            last_health_at_ms: row.get(6)?,
        });
    }

    Ok(Json(EngineListResponse { engines }))
}

pub async fn engine_health(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
) -> Result<Json<EngineHealthResponse>, ApiError> {
    authorize(&state, &headers)?;
    logging::info("engine_health_started", &[("engine_id", engine_id.clone())]);

    // Connect and check Docker (no DB cursor open here).
    let health = match susun_integration::connect_engine(&state.db, None).await {
        Ok(engine) => susun_integration::engine_health(&engine).await,
        Err(error) => susun_integration::EngineHealthRow {
            reachable: false,
            api_version: None,
            error: Some(error),
        },
    };
    let response = EngineHealthResponse {
        reachable: health.reachable,
        api_version: health.api_version,
        error: health.error,
    };

    // Persist the latest result. No read cursor is open on this connection.
    let health_json = serde_json::to_string(&response)?;
    let now = now_ms()?;
    let conn = state.db.connect()?;
    conn.execute(
        "UPDATE engines SET last_health_json = ?1, last_health_at_ms = ?2 WHERE id = ?3",
        params![health_json, now, engine_id.clone()],
    )
    .await?;

    if response.reachable {
        logging::info(
            "engine_health_finished",
            &[
                ("engine_id", engine_id),
                ("reachable", response.reachable.to_string()),
                (
                    "api_version",
                    response.api_version.clone().unwrap_or_default(),
                ),
            ],
        );
    } else {
        logging::warn(
            "engine_health_finished",
            &[
                ("engine_id", engine_id),
                ("reachable", response.reachable.to_string()),
                ("error", response.error.clone().unwrap_or_default()),
            ],
        );
    }

    Ok(Json(response))
}

pub async fn engine_capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(_engine_id): Path<String>,
) -> Result<Json<EngineCapabilitiesResponse>, ApiError> {
    authorize(&state, &headers)?;

    let engine = susun_integration::connect_engine(&state.db, None)
        .await
        .map_err(ApiError::EngineUnavailable)?;
    let capabilities = susun_integration::engine_capabilities(&engine)
        .await
        .map_err(ApiError::EngineUnavailable)?;

    Ok(Json(EngineCapabilitiesResponse {
        api_version: capabilities.api_version,
        supports_health: capabilities.supports_health,
        supports_named_volumes: capabilities.supports_named_volumes,
        supports_network_aliases: capabilities.supports_network_aliases,
        supports_log_follow: capabilities.supports_log_follow,
        supports_build: capabilities.supports_build,
        supports_mount_types: capabilities.supports_mount_types,
        max_container_name_length: capabilities.max_container_name_length,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PruneRequest {
    /// Prune *policy*: which resource classes to reclaim. This is not a resource
    /// target list — the engine derives the exact resources to remove at commit.
    pub scopes: Vec<String>,
    #[serde(default)]
    pub all_images: bool,
}

/// One resource scope in a server-derived, non-destructive prune inventory.
#[derive(Debug, Clone, Serialize)]
pub struct PruneScopeInventory {
    pub scope: String,
    /// SDK support level for this scope's estimate ("supported", "unsupported"…).
    pub support: String,
    pub candidate_count: Option<u64>,
    pub reclaimable_bytes: Option<u64>,
    /// "exact" | "lower_bound" | "unavailable".
    pub estimate_kind: String,
}

/// Non-mutating prune preview. Inventory is derived by the engine, never the
/// frontend. A commit plan is minted only when inventory is available and no
/// active work targets the engine.
#[derive(Debug, Serialize)]
pub struct PrunePreview {
    pub engine_id: String,
    pub scopes: Vec<String>,
    pub all_images: bool,
    /// Prune operates engine-wide: resources created by other tools or projects
    /// sharing this engine may also be removed. Surfaced so the UI can warn.
    pub affects_shared_engine: bool,
    /// Server-derived per-scope inventory (counts/reclaim/support).
    pub inventory: Vec<PruneScopeInventory>,
    /// False when the engine cannot provide a reliable inventory; commit is then
    /// disabled rather than pruning blind.
    pub inventory_supported: bool,
    /// Sum of reclaimable bytes across scopes that reported an estimate.
    pub estimated_reclaim_bytes: Option<u64>,
    pub active_jobs: i64,
    pub active_watch_sessions: i64,
    /// Whether a commit plan was minted (inventory available and no active work).
    pub commit_enabled: bool,
    pub plan_id: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct PruneResponse {
    pub containers_removed: Vec<String>,
    pub networks_removed: Vec<String>,
    pub volumes_removed: Vec<String>,
    pub images_removed: Vec<String>,
    pub space_reclaimed_bytes: u64,
}

/// Commit endpoints carry no body; the executable policy lives in the plan.
fn reject_commit_body(body: &Bytes) -> Result<(), ApiError> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(ApiError::TrustedPlanContentRejected)
    }
}

/// Fingerprints the engine identity a prune runs against: the selected runtime
/// profile's identity (or `platform_default`) plus the engine API version. Any
/// selection swap, endpoint change, re-observation, or provider version change
/// changes this, so a plan cannot be committed against a different engine.
async fn engine_identity_fingerprint(
    db: &turso::Database,
    engine: &susun_engine_bollard::BollardEngine,
    profile_id: Option<&str>,
) -> String {
    let api_version = susun_integration::engine_health(engine)
        .await
        .api_version
        .unwrap_or_default();
    let profile = runtime::list_all_profiles(db)
        .await
        .ok()
        .and_then(|profiles| {
            profile_id.and_then(|id| profiles.into_iter().find(|profile| profile.id == id))
        });
    let identity = match (profile_id, profile) {
        (Some(profile_id), None) => format!("missing_profile={profile_id}"),
        (_, Some(profile)) => format!(
            "profile={};class={};conn={};avail={};endpoint={};rev={};selected={}",
            profile.id,
            profile.runtime_class,
            profile.connection.state,
            profile.availability_state,
            profile.endpoint_summary.unwrap_or_default(),
            profile.observation_revision,
            profile.is_selected
        ),
        (None, None) => "platform_default".to_owned(),
    };
    runtime::stable_suffix(&format!("{identity};api={api_version}"))
}

/// Fingerprints a server-derived cleanup inventory so a commit can detect any
/// change in candidate counts, reclaimable bytes, or support since preview.
fn inventory_fingerprint(preview: &susun_integration::CleanupPreviewRow) -> String {
    let mut canonical = String::new();
    for scope in &preview.scopes {
        canonical.push_str(&format!(
            "{}|{}|{}|{}|{};",
            scope.scope,
            scope.support,
            scope.candidate_count.unwrap_or_default(),
            scope.reclaimable_bytes.unwrap_or_default(),
            scope.estimate_kind
        ));
    }
    runtime::stable_suffix(&canonical)
}

fn to_inventory(preview: &susun_integration::CleanupPreviewRow) -> Vec<PruneScopeInventory> {
    preview
        .scopes
        .iter()
        .map(|scope| PruneScopeInventory {
            scope: scope.scope.clone(),
            support: scope.support.clone(),
            candidate_count: scope.candidate_count,
            reclaimable_bytes: scope.reclaimable_bytes,
            estimate_kind: scope.estimate_kind.clone(),
        })
        .collect()
}

fn estimated_reclaim(preview: &susun_integration::CleanupPreviewRow) -> Option<u64> {
    let known: Vec<u64> = preview
        .scopes
        .iter()
        .filter_map(|scope| scope.reclaimable_bytes)
        .collect();
    if known.is_empty() {
        None
    } else {
        Some(known.iter().sum())
    }
}

/// Active work attributed to the exact runtime profile. Prune affects the whole
/// engine behind that profile, so any matching job or watch session blocks it.
async fn engine_active_work(
    db: &turso::Database,
    profile_id: Option<&str>,
) -> Result<(i64, i64), turso::Error> {
    let conn = db.connect()?;
    let mut job_rows = match profile_id {
        Some(profile_id) => {
            conn.query(
                "SELECT COUNT(*) FROM jobs
                 WHERE status = 'running'
                   AND (runtime_profile_id = ?1 OR runtime_profile_id IS NULL)",
                params![profile_id.to_owned()],
            )
            .await?
        }
        None => {
            conn.query(
                "SELECT COUNT(*) FROM jobs
                 WHERE status = 'running' AND runtime_profile_id IS NULL",
                (),
            )
            .await?
        }
    };
    let jobs: i64 = match job_rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    };
    let mut watch_rows = match profile_id {
        Some(profile_id) => {
            conn.query(
                "SELECT COUNT(*) FROM watch_sessions w
                 JOIN projects p ON p.id = w.project_id
                 WHERE w.status = 'running'
                   AND (p.runtime_profile_id = ?1 OR p.runtime_profile_id IS NULL)",
                params![profile_id.to_owned()],
            )
            .await?
        }
        None => {
            conn.query(
                "SELECT COUNT(*) FROM watch_sessions w
                 JOIN projects p ON p.id = w.project_id
                 WHERE w.status = 'running' AND p.runtime_profile_id IS NULL",
                (),
            )
            .await?
        }
    };
    let watch: i64 = match watch_rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    };
    Ok((jobs, watch))
}

pub async fn preview_prune(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(engine_id): Path<String>,
    Json(request): Json<PruneRequest>,
) -> Result<Json<PrunePreview>, ApiError> {
    authorize(&state, &headers)?;
    let owner = runtime::stable_suffix(&state.auth_token);

    if request.scopes.is_empty()
        || request.scopes.iter().any(|scope| {
            !matches!(
                scope.as_str(),
                "containers" | "networks" | "volumes" | "images" | "build_cache"
            )
        })
    {
        return Err(ApiError::ActionUnavailable(
            "Choose at least one recognized prune scope.".to_owned(),
        ));
    }

    let (runtime_profile_id, _) = runtime::attribution_for(&state.db, None).await?;

    // Connect to derive a real inventory; an unreachable engine can't be previewed.
    let engine =
        susun_integration::connect_engine_for_profile(&state.db, runtime_profile_id.as_deref())
            .await
            .map_err(ApiError::EngineUnavailable)?;

    let (active_jobs, active_watch_sessions) =
        engine_active_work(&state.db, runtime_profile_id.as_deref()).await?;
    let has_active_work = active_jobs > 0 || active_watch_sessions > 0;

    // Server-derived, non-destructive inventory. If the SDK cannot provide it,
    // report unsupported and keep commit disabled rather than pruning blind.
    let cleanup = susun_integration::cleanup_preview(&engine, &request.scopes, request.all_images)
        .await
        .ok();

    let base = PrunePreview {
        engine_id: engine_id.clone(),
        scopes: request.scopes.clone(),
        all_images: request.all_images,
        affects_shared_engine: true,
        inventory: Vec::new(),
        inventory_supported: false,
        estimated_reclaim_bytes: None,
        active_jobs,
        active_watch_sessions,
        commit_enabled: false,
        plan_id: None,
        expires_in_seconds: None,
    };

    let Some(cleanup) = cleanup else {
        logging::warn(
            "engine_prune_previewed",
            &[
                ("engine_id", engine_id),
                ("inventory_supported", "false".to_owned()),
            ],
        );
        return Ok(Json(base));
    };

    let inventory = to_inventory(&cleanup);
    let estimated_reclaim_bytes = estimated_reclaim(&cleanup);
    let inventory_supported = cleanup.scopes.len() == request.scopes.len()
        && cleanup.scopes.iter().all(|scope| {
            scope.support == "supported"
                && scope.estimate_kind == "exact"
                && scope.candidate_count.is_some()
                && scope.reclaimable_bytes.is_some()
        });

    // Mint a commit plan only when inventory is available and nothing is running.
    let (plan_id, expires_in_seconds, commit_enabled) = if has_active_work || !inventory_supported {
        (None, None, false)
    } else {
        let identity_fingerprint =
            engine_identity_fingerprint(&state.db, &engine, runtime_profile_id.as_deref()).await;
        let ticket = state.action_plans.prepare(
            &owner,
            ActionKind::EnginePrune,
            ActionPlanPayload::EnginePrune(EnginePrunePlan {
                engine_id: engine_id.clone(),
                runtime_profile_id: runtime_profile_id.clone(),
                scopes: request.scopes.clone(),
                all_images: request.all_images,
                identity_fingerprint,
                inventory_fingerprint: inventory_fingerprint(&cleanup),
            }),
        );
        (Some(ticket.plan_id), Some(ticket.expires_in_seconds), true)
    };

    logging::warn(
        "engine_prune_previewed",
        &[
            ("engine_id", engine_id.clone()),
            ("scope_count", request.scopes.len().to_string()),
            ("all_images", request.all_images.to_string()),
            ("commit_enabled", commit_enabled.to_string()),
        ],
    );
    Ok(Json(PrunePreview {
        engine_id,
        scopes: request.scopes,
        all_images: request.all_images,
        affects_shared_engine: true,
        inventory,
        inventory_supported,
        estimated_reclaim_bytes,
        active_jobs,
        active_watch_sessions,
        commit_enabled,
        plan_id,
        expires_in_seconds,
    }))
}

pub async fn commit_prune(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<PruneResponse>, ApiError> {
    authorize(&state, &headers)?;
    reject_commit_body(&body)?;
    let owner = runtime::stable_suffix(&state.auth_token);
    let started = now_ms()?;

    let claimed = state
        .action_plans
        .claim(&plan_id, &owner, &[ActionKind::EnginePrune])
        .map_err(|error| ApiError::ActionUnavailable(error.to_string()))?;
    let ActionPlanPayload::EnginePrune(plan) = claimed.payload else {
        state
            .action_plans
            .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return Err(ApiError::ActionUnavailable(
            "Plan did not match an engine prune.".to_owned(),
        ));
    };

    logging::warn(
        "engine_prune_started",
        &[
            ("engine_id", plan.engine_id.clone()),
            ("scope_count", plan.scopes.len().to_string()),
            ("all_images", plan.all_images.to_string()),
        ],
    );

    // Helper to fail: mark the plan spent, record a redacted rejection, and map
    // to a user-facing error.
    async fn reject(
        state: &AppState,
        plan_id: &str,
        profile_id: Option<String>,
        code: &'static str,
        message: String,
    ) -> ApiError {
        state
            .action_plans
            .finish(plan_id, crate::action_plans::PlanState::Failed);
        let _ = action_audit::record_rejection(
            &state.db,
            ActionKind::EnginePrune,
            profile_id,
            "rejected_state",
            code,
        )
        .await;
        ApiError::ActionUnavailable(message)
    }

    // Revalidate provider state.
    let engine = match susun_integration::connect_engine_for_profile(
        &state.db,
        plan.runtime_profile_id.as_deref(),
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::EnginePrune,
                plan.runtime_profile_id.clone(),
                "rejected_state",
                "provider_unreachable",
            )
            .await;
            return Err(ApiError::EngineUnavailable(error));
        }
    };

    // Revalidate engine identity: never silently prune a different engine.
    if engine_identity_fingerprint(&state.db, &engine, plan.runtime_profile_id.as_deref()).await
        != plan.identity_fingerprint
    {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "engine_identity_changed",
            "The selected engine or its endpoint changed since preview. Preview prune again."
                .to_owned(),
        )
        .await);
    }

    // Revalidate inventory: reject a stale preview.
    let cleanup =
        match susun_integration::cleanup_preview(&engine, &plan.scopes, plan.all_images).await {
            Ok(cleanup) => cleanup,
            Err(_) => {
                return Err(reject(
                    &state,
                    &claimed.plan_id,
                    plan.runtime_profile_id.clone(),
                    "inventory_unavailable",
                    "Engine inventory is no longer available. Preview prune again.".to_owned(),
                )
                .await);
            }
        };
    if inventory_fingerprint(&cleanup) != plan.inventory_fingerprint {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "stale_inventory",
            "Engine inventory changed since preview. Preview prune again.".to_owned(),
        )
        .await);
    }

    // Revalidate active work: nothing may be running when prune executes.
    let (jobs, watch) =
        match engine_active_work(&state.db, plan.runtime_profile_id.as_deref()).await {
            Ok(active) => active,
            Err(_) => {
                return Err(reject(
                    &state,
                    &claimed.plan_id,
                    plan.runtime_profile_id.clone(),
                    "active_work_unavailable",
                    "Active work could not be verified. Preview prune again.".to_owned(),
                )
                .await);
            }
        };
    if jobs > 0 || watch > 0 {
        return Err(reject(
            &state,
            &claimed.plan_id,
            plan.runtime_profile_id.clone(),
            "active_work",
            "A job or watch session is running. Stop it and preview prune again.".to_owned(),
        )
        .await);
    }

    let report = match susun_integration::system_prune(&engine, &plan.scopes, plan.all_images).await
    {
        Ok(report) => report,
        Err(error) => {
            state
                .action_plans
                .finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            let _ = action_audit::record_rejection(
                &state.db,
                ActionKind::EnginePrune,
                None,
                "failed",
                "prune_failed",
            )
            .await;
            return Err(ApiError::EngineUnavailable(error));
        }
    };

    let response = PruneResponse {
        containers_removed: report.containers_removed,
        networks_removed: report.networks_removed,
        volumes_removed: report.volumes_removed,
        images_removed: report.images_removed,
        space_reclaimed_bytes: report.space_reclaimed_bytes,
    };
    state
        .action_plans
        .finish(&claimed.plan_id, crate::action_plans::PlanState::Succeeded);
    let _ = action_audit::record(
        &state.db,
        AuditEntry {
            kind: ActionKind::EnginePrune,
            profile_id: plan.runtime_profile_id,
            runtime_class: None,
            ownership_result: "authorized".to_owned(),
            command_kind: Some("provider_prune".to_owned()),
            elevation_mode: Some("none".to_owned()),
            terminal_status: action_audit::STATUS_COMPLETED.to_owned(),
            affected: vec![
                AffectedCount {
                    category: "containers".to_owned(),
                    count: response.containers_removed.len() as i64,
                },
                AffectedCount {
                    category: "networks".to_owned(),
                    count: response.networks_removed.len() as i64,
                },
                AffectedCount {
                    category: "volumes".to_owned(),
                    count: response.volumes_removed.len() as i64,
                },
                AffectedCount {
                    category: "images".to_owned(),
                    count: response.images_removed.len() as i64,
                },
            ],
            failure_code: None,
            correlation_token: None,
            started_at_ms: started,
            completed_at_ms: Some(now_ms()?),
        },
    )
    .await;
    logging::warn(
        "engine_prune_finished",
        &[
            (
                "containers_removed",
                response.containers_removed.len().to_string(),
            ),
            (
                "networks_removed",
                response.networks_removed.len().to_string(),
            ),
            (
                "volumes_removed",
                response.volumes_removed.len().to_string(),
            ),
            ("images_removed", response.images_removed.len().to_string()),
            (
                "space_reclaimed_bytes",
                response.space_reclaimed_bytes.to_string(),
            ),
        ],
    );

    Ok(Json(response))
}

fn now_ms() -> Result<i64, ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::Clock)?;
    i64::try_from(duration.as_millis()).map_err(|_| ApiError::Clock)
}
