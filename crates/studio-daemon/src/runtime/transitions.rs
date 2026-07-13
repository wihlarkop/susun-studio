//! Runtime Data 3 destructive flows, gated by the Runtime Security 4 envelope.
//!
//! Every mutating action here is a two-step, server-owned flow:
//!
//! 1. **preview/prepare** validates the request against live state and, when the
//!    action is allowed, mints an opaque plan in [`crate::action_plans`]. The
//!    resolved executable target (which project bindings to move, which runtime to
//!    reset) is stored *server-side* inside that plan; the frontend receives only
//!    an opaque `plan_id`.
//! 2. **commit** takes just the `plan_id`, claims the plan exactly once (owner /
//!    expiry / single-use enforced by the store), **revalidates** ownership,
//!    runtime identity, provider/target state, project bindings, an inventory
//!    fingerprint, and active jobs/watch sessions, then executes. A changed,
//!    stale, replayed, expired, or wrong-owner plan is rejected and requires a
//!    fresh preview.
//!
//! Migration commit/rollback are fully executed here (pure metadata moves).
//! Runtime reset/remove/repair go through the same gate but their engine work is
//! a Phase 14b provider command; commit revalidates and returns an explicit
//! `deferred_to_phase_14b` terminal result without mutating any engine.

use serde::{Deserialize, Serialize};
use turso::{Connection, Database, params};

use super::{RuntimeProfile, list_all_profiles, now_ms, stable_suffix};
use crate::action_audit::{self, AffectedCount, AuditEntry};
use crate::action_plans::{
    ActionKind, ActionPlanPayload, ActionPlanStore, DestructivePlan, MigrationCommitPlan,
    MigrationRollbackPlan,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
pub struct MigrationRequest {
    pub source_profile_id: String,
    pub target_profile_id: String,
    pub project_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MigrationPreview {
    pub source: RuntimeProfile,
    pub target: RuntimeProfile,
    pub projects: Vec<MigrationProject>,
    pub can_migrate: bool,
    pub blockers: Vec<String>,
    pub unavailable_capabilities: Vec<String>,
    pub artifact_policy: Vec<ArtifactPolicy>,
    pub rollback_available: bool,
    /// Opaque, single-use commit handle. Present only when `can_migrate`.
    pub plan_id: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct MigrationProject {
    pub id: String,
    pub name: String,
    pub currently_bound_to_source: bool,
}

#[derive(Debug, Serialize)]
pub struct ArtifactPolicy {
    pub category: &'static str,
    pub disposition: &'static str,
    pub exactness: &'static str,
}

#[derive(Debug, Serialize)]
pub struct MigrationResult {
    pub migration_id: String,
    pub status: &'static str,
    pub source_profile_id: String,
    pub target_profile_id: String,
    pub project_count: usize,
    pub skipped_items: Vec<String>,
    pub failures: Vec<String>,
    pub rollback_available: bool,
}

#[derive(Debug, Serialize)]
pub struct MigrationRollbackPreview {
    pub migration_id: String,
    pub restorable: bool,
    pub blocker: Option<String>,
    pub plan_id: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct MigrationRollbackResult {
    pub migration_id: String,
    pub status: &'static str,
    pub restored_project_count: usize,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DestructiveAction {
    Repair,
    ResetEngineData,
    RemoveBuiltInRuntime,
}

impl DestructiveAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Repair => "repair",
            Self::ResetEngineData => "reset_engine_data",
            Self::RemoveBuiltInRuntime => "remove_built_in_runtime",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "repair" => Some(Self::Repair),
            "reset_engine_data" => Some(Self::ResetEngineData),
            "remove_built_in_runtime" => Some(Self::RemoveBuiltInRuntime),
            _ => None,
        }
    }

    fn action_kind(self) -> ActionKind {
        match self {
            Self::Repair => ActionKind::DestructiveRepair,
            Self::ResetEngineData => ActionKind::DestructiveResetEngineData,
            Self::RemoveBuiltInRuntime => ActionKind::DestructiveRemoveBuiltInRuntime,
        }
    }

    /// The safe command kind recorded in audit — describes the *class* of the
    /// deferred Phase 14b provider operation, never a command line.
    fn deferred_command_kind(self) -> &'static str {
        match self {
            Self::Repair => "deferred_provider_repair",
            Self::ResetEngineData => "deferred_provider_reset",
            Self::RemoveBuiltInRuntime => "deferred_provider_remove",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DestructivePreviewRequest {
    pub action: DestructiveAction,
}

#[derive(Debug, Serialize)]
pub struct DestructivePreview {
    pub operation_id: String,
    pub profile_id: String,
    pub action: String,
    pub allowed: bool,
    pub blocker: Option<String>,
    pub affected: Vec<AffectedCategory>,
    pub preserved: Vec<&'static str>,
    pub requires_fresh_preview: bool,
    /// Opaque, single-use commit handle. Present only when `allowed`.
    pub plan_id: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct AffectedCategory {
    pub category: &'static str,
    pub count: Option<i64>,
    pub exactness: &'static str,
    pub effect: &'static str,
}

/// Outcome of a destructive commit. Because the engine-mutating provider command
/// is Phase 14b, a successful gate returns `deferred_to_phase_14b` — it never
/// reports `executed`, `completed`, or any mutation success.
#[derive(Debug, Serialize)]
pub struct DestructiveCommitResult {
    pub profile_id: String,
    pub action: String,
    pub status: &'static str,
    pub revalidated: bool,
    pub message: String,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UninstallPolicy {
    pub default_choice: &'static str,
    pub choices: Vec<UninstallChoice>,
    pub unattended_behavior: &'static str,
    pub reinstall_rule: &'static str,
}

#[derive(Debug, Serialize)]
pub struct UninstallChoice {
    pub id: &'static str,
    pub label: &'static str,
    pub mutates_external_runtimes: bool,
    pub selected_by_default: bool,
}

/// Why a gated commit was refused. Carries a user-facing message and a short,
/// secret-free audit code.
#[derive(Debug)]
pub struct CommitRejected {
    pub message: String,
    pub audit_code: &'static str,
    pub ownership_result: &'static str,
}

impl CommitRejected {
    fn new(
        message: impl Into<String>,
        ownership_result: &'static str,
        audit_code: &'static str,
    ) -> Self {
        Self {
            message: message.into(),
            audit_code,
            ownership_result,
        }
    }
}

// --------------------------------------------------------------------------
// Migration
// --------------------------------------------------------------------------

pub async fn preview_migration(
    db: &Database,
    store: &ActionPlanStore,
    owner: &str,
    request: &MigrationRequest,
) -> Result<Option<MigrationPreview>, turso::Error> {
    let profiles = list_all_profiles(db).await?;
    let Some(source) = profiles
        .iter()
        .find(|profile| profile.id == request.source_profile_id)
        .cloned()
    else {
        return Ok(None);
    };
    let Some(target) = profiles
        .iter()
        .find(|profile| profile.id == request.target_profile_id)
        .cloned()
    else {
        return Ok(None);
    };

    let projects = migration_projects(db, &request.project_ids, &source.id).await?;
    let mut blockers = Vec::new();
    if source.id == target.id {
        blockers.push("Source and target runtime must differ.".to_owned());
    }
    if request.project_ids.is_empty() {
        blockers.push("Select at least one project.".to_owned());
    }
    if projects.len() != request.project_ids.len() {
        blockers.push("One or more selected projects do not exist.".to_owned());
    }
    if projects
        .iter()
        .any(|project| !project.currently_bound_to_source)
    {
        blockers
            .push("Every selected project must still be bound to the source runtime.".to_owned());
    }
    if target.availability_state != "available"
        || target.connection.state != "summarized"
        || !target.management.can_select
    {
        blockers.push("Target runtime must be available, reachable, and selectable.".to_owned());
    }
    let (running_jobs, running_watch) = active_work(db, &source.id).await?;
    if running_jobs > 0 || running_watch > 0 {
        blockers.push(
            "Stop running jobs and watch sessions on the source runtime before migrating."
                .to_owned(),
        );
    }

    let can_migrate = blockers.is_empty();
    let (plan_id, expires_in_seconds) = if can_migrate {
        let fingerprint =
            migration_fingerprint(db, &source.id, &target, &request.project_ids).await?;
        let ticket = store.prepare(
            owner,
            ActionKind::MigrationCommit,
            ActionPlanPayload::MigrationCommit(MigrationCommitPlan {
                source_profile_id: source.id.clone(),
                target_profile_id: target.id.clone(),
                project_ids: request.project_ids.clone(),
                fingerprint,
            }),
        );
        (Some(ticket.plan_id), Some(ticket.expires_in_seconds))
    } else {
        (None, None)
    };

    Ok(Some(MigrationPreview {
        source,
        target,
        projects,
        can_migrate,
        blockers,
        unavailable_capabilities: vec![
            "Volume data migration is not supported.".to_owned(),
            "Runtime ownership is not transferred.".to_owned(),
        ],
        artifact_policy: vec![
            ArtifactPolicy {
                category: "images",
                disposition: "pull_or_rebuild_as_durable_jobs",
                exactness: "capability_limited",
            },
            ArtifactPolicy {
                category: "volumes",
                disposition: "not_migrated",
                exactness: "explicitly_excluded",
            },
            ArtifactPolicy {
                category: "registry_credentials",
                disposition: "reauthenticate_on_target",
                exactness: "explicitly_excluded",
            },
        ],
        rollback_available: true,
        plan_id,
        expires_in_seconds,
    }))
}

/// Commit a previously previewed migration by its opaque plan id. Accepts no
/// target list from the caller — the moved bindings come only from the plan.
pub async fn commit_migration(
    db: &Database,
    store: &ActionPlanStore,
    owner: &str,
    plan_id: &str,
) -> Result<MigrationResult, CommitRejected> {
    let claimed = match store.claim(plan_id, owner, &[ActionKind::MigrationCommit]) {
        Ok(claimed) => claimed,
        Err(error) => {
            let code = error.audit_code();
            reject_and_audit(db, ActionKind::MigrationCommit, None, code).await;
            return Err(CommitRejected::new(
                error.to_string(),
                "rejected_plan",
                code,
            ));
        }
    };
    let ActionPlanPayload::MigrationCommit(plan) = claimed.payload else {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return Err(CommitRejected::new(
            "Plan payload did not match a migration commit.",
            "rejected_plan",
            "plan_kind_mismatch",
        ));
    };

    let started = now_ms();
    // Revalidate the inventory fingerprint captured at preview.
    let target = match load_profile(db, &plan.target_profile_id).await {
        Ok(Some(target)) => target,
        _ => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            return fail_commit(
                db,
                ActionKind::MigrationCommit,
                Some(plan.target_profile_id.clone()),
                "target_missing",
                "Target runtime no longer exists. Preview the migration again.",
                started,
            )
            .await;
        }
    };
    let fresh = migration_fingerprint(db, &plan.source_profile_id, &target, &plan.project_ids)
        .await
        .unwrap_or_default();
    if fresh != plan.fingerprint {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return fail_commit(
            db,
            ActionKind::MigrationCommit,
            Some(plan.target_profile_id.clone()),
            "stale_preview",
            "Bindings or target state changed since preview. Preview the migration again.",
            started,
        )
        .await;
    }
    let (running_jobs, running_watch) = active_work(db, &plan.source_profile_id)
        .await
        .unwrap_or((0, 0));
    if running_jobs > 0 || running_watch > 0 {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return fail_commit(
            db,
            ActionKind::MigrationCommit,
            Some(plan.source_profile_id.clone()),
            "active_work",
            "A job or watch session started on the source runtime. Stop it and preview again.",
            started,
        )
        .await;
    }

    let migration_id = format!("rtm_{}", uuid::Uuid::new_v4().simple());
    let now = now_ms();
    let project_ids_json = serde_json::to_string(&plan.project_ids).unwrap_or_else(|_| "[]".into());
    let skipped = vec![
        "volume data".to_owned(),
        "registry credentials".to_owned(),
        "runtime ownership".to_owned(),
    ];
    let skipped_json = serde_json::to_string(&skipped).unwrap_or_else(|_| "[]".into());

    let mut conn = match db.connect() {
        Ok(conn) => conn,
        Err(_) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            return Err(CommitRejected::new(
                "Could not open a database connection.",
                "rejected_internal",
                "db_error",
            ));
        }
    };
    let executed = execute_binding_moves(
        &mut conn,
        &plan,
        &migration_id,
        &project_ids_json,
        &skipped_json,
        now,
    )
    .await;
    match executed {
        Ok(true) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Succeeded);
            let _ = action_audit::record(
                db,
                AuditEntry {
                    kind: ActionKind::MigrationCommit,
                    profile_id: Some(plan.target_profile_id.clone()),
                    runtime_class: Some(target.runtime_class.clone()),
                    ownership_result: "authorized".to_owned(),
                    command_kind: Some("metadata_only".to_owned()),
                    elevation_mode: Some("none".to_owned()),
                    terminal_status: action_audit::STATUS_COMPLETED.to_owned(),
                    affected: vec![AffectedCount {
                        category: "project_bindings".to_owned(),
                        count: plan.project_ids.len() as i64,
                    }],
                    failure_code: None,
                    correlation_token: None,
                    started_at_ms: started,
                    completed_at_ms: Some(now_ms()),
                },
            )
            .await;
            Ok(MigrationResult {
                migration_id,
                status: "completed",
                source_profile_id: plan.source_profile_id,
                target_profile_id: plan.target_profile_id,
                project_count: plan.project_ids.len(),
                skipped_items: skipped,
                failures: Vec::new(),
                rollback_available: true,
            })
        }
        Ok(false) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            let result = failed_result(
                &plan,
                vec![
                    "Project bindings changed during commit; no migration was applied.".to_owned(),
                ],
            );
            persist_failed_migration(db, &plan, &result).await.ok();
            let _ = action_audit::record(
                db,
                AuditEntry {
                    kind: ActionKind::MigrationCommit,
                    profile_id: Some(plan.target_profile_id.clone()),
                    runtime_class: Some(target.runtime_class.clone()),
                    ownership_result: "authorized".to_owned(),
                    command_kind: Some("metadata_only".to_owned()),
                    elevation_mode: Some("none".to_owned()),
                    terminal_status: action_audit::STATUS_FAILED.to_owned(),
                    affected: Vec::new(),
                    failure_code: Some("binding_race".to_owned()),
                    correlation_token: None,
                    started_at_ms: started,
                    completed_at_ms: Some(now_ms()),
                },
            )
            .await;
            Ok(result)
        }
        Err(_) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            Err(CommitRejected::new(
                "The migration could not be applied.",
                "rejected_internal",
                "db_error",
            ))
        }
    }
}

async fn execute_binding_moves(
    conn: &mut Connection,
    plan: &MigrationCommitPlan,
    migration_id: &str,
    project_ids_json: &str,
    skipped_json: &str,
    now: i64,
) -> Result<bool, turso::Error> {
    let tx = conn.transaction().await?;
    for project_id in &plan.project_ids {
        let changed = tx
            .execute(
                "UPDATE projects SET runtime_profile_id = ?1
                 WHERE id = ?2 AND runtime_profile_id = ?3",
                params![
                    plan.target_profile_id.clone(),
                    project_id.clone(),
                    plan.source_profile_id.clone()
                ],
            )
            .await?;
        if changed != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
    }
    tx.execute(
        "INSERT INTO runtime_migrations (
            id, source_profile_id, target_profile_id, status, project_count,
            project_ids_json, skipped_items_json, failures_json,
            rollback_available, created_at_ms, completed_at_ms
         ) VALUES (?1, ?2, ?3, 'completed', ?4, ?5, ?6, '[]', 1, ?7, ?7)",
        params![
            migration_id.to_owned(),
            plan.source_profile_id.clone(),
            plan.target_profile_id.clone(),
            plan.project_ids.len() as i64,
            project_ids_json.to_owned(),
            skipped_json.to_owned(),
            now
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn preview_migration_rollback(
    db: &Database,
    store: &ActionPlanStore,
    owner: &str,
    migration_id: &str,
) -> Result<Option<MigrationRollbackPreview>, turso::Error> {
    let Some((source, target, project_ids_json, status, rollback_available)) =
        load_migration(db, migration_id).await?
    else {
        return Ok(None);
    };
    let restorable = status == "completed" && rollback_available == 1;
    if !restorable {
        return Ok(Some(MigrationRollbackPreview {
            migration_id: migration_id.to_owned(),
            restorable: false,
            blocker: Some("This migration can no longer be rolled back.".to_owned()),
            plan_id: None,
            expires_in_seconds: None,
        }));
    }
    let project_ids: Vec<String> = serde_json::from_str(&project_ids_json).unwrap_or_default();
    let fingerprint =
        rollback_fingerprint(db, migration_id, &source, &target, &project_ids).await?;
    let ticket = store.prepare(
        owner,
        ActionKind::MigrationRollback,
        ActionPlanPayload::MigrationRollback(MigrationRollbackPlan {
            migration_id: migration_id.to_owned(),
            fingerprint,
        }),
    );
    Ok(Some(MigrationRollbackPreview {
        migration_id: migration_id.to_owned(),
        restorable: true,
        blocker: None,
        plan_id: Some(ticket.plan_id),
        expires_in_seconds: Some(ticket.expires_in_seconds),
    }))
}

pub async fn commit_migration_rollback(
    db: &Database,
    store: &ActionPlanStore,
    owner: &str,
    plan_id: &str,
) -> Result<MigrationRollbackResult, CommitRejected> {
    let claimed = match store.claim(plan_id, owner, &[ActionKind::MigrationRollback]) {
        Ok(claimed) => claimed,
        Err(error) => {
            let code = error.audit_code();
            reject_and_audit(db, ActionKind::MigrationRollback, None, code).await;
            return Err(CommitRejected::new(
                error.to_string(),
                "rejected_plan",
                code,
            ));
        }
    };
    let ActionPlanPayload::MigrationRollback(plan) = claimed.payload else {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return Err(CommitRejected::new(
            "Plan payload did not match a rollback.",
            "rejected_plan",
            "plan_kind_mismatch",
        ));
    };
    let started = now_ms();

    let Some((source, target, project_ids_json, status, rollback_available)) =
        load_migration(db, &plan.migration_id).await.unwrap_or(None)
    else {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return Err(CommitRejected::new(
            "Migration not found.",
            "rejected_not_found",
            "migration_missing",
        ));
    };
    if status != "completed" || rollback_available != 1 {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        reject_and_audit(
            db,
            ActionKind::MigrationRollback,
            None,
            "already_rolled_back",
        )
        .await;
        return Ok(MigrationRollbackResult {
            migration_id: plan.migration_id,
            status: "unavailable",
            restored_project_count: 0,
        });
    }
    let project_ids: Vec<String> = serde_json::from_str(&project_ids_json).unwrap_or_default();
    let fresh = rollback_fingerprint(db, &plan.migration_id, &source, &target, &project_ids)
        .await
        .unwrap_or_default();
    if fresh != plan.fingerprint {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        reject_and_audit(db, ActionKind::MigrationRollback, None, "stale_preview").await;
        return Ok(MigrationRollbackResult {
            migration_id: plan.migration_id,
            status: "failed",
            restored_project_count: 0,
        });
    }

    let mut conn = match db.connect() {
        Ok(conn) => conn,
        Err(_) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            return Err(CommitRejected::new(
                "Could not open a database connection.",
                "rejected_internal",
                "db_error",
            ));
        }
    };
    let restored = restore_bindings(
        &mut conn,
        &plan.migration_id,
        &source,
        &target,
        &project_ids,
    )
    .await;
    match restored {
        Ok(true) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Succeeded);
            let _ = action_audit::record(
                db,
                AuditEntry {
                    kind: ActionKind::MigrationRollback,
                    profile_id: Some(source.clone()),
                    runtime_class: None,
                    ownership_result: "authorized".to_owned(),
                    command_kind: Some("metadata_only".to_owned()),
                    elevation_mode: Some("none".to_owned()),
                    terminal_status: action_audit::STATUS_COMPLETED.to_owned(),
                    affected: vec![AffectedCount {
                        category: "project_bindings".to_owned(),
                        count: project_ids.len() as i64,
                    }],
                    failure_code: None,
                    correlation_token: None,
                    started_at_ms: started,
                    completed_at_ms: Some(now_ms()),
                },
            )
            .await;
            Ok(MigrationRollbackResult {
                migration_id: plan.migration_id,
                status: "rolled_back",
                restored_project_count: project_ids.len(),
            })
        }
        Ok(false) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            reject_and_audit(db, ActionKind::MigrationRollback, None, "binding_race").await;
            Ok(MigrationRollbackResult {
                migration_id: plan.migration_id,
                status: "failed",
                restored_project_count: 0,
            })
        }
        Err(_) => {
            store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
            Err(CommitRejected::new(
                "The rollback could not be applied.",
                "rejected_internal",
                "db_error",
            ))
        }
    }
}

async fn restore_bindings(
    conn: &mut Connection,
    migration_id: &str,
    source: &str,
    target: &str,
    project_ids: &[String],
) -> Result<bool, turso::Error> {
    let tx = conn.transaction().await?;
    for project_id in project_ids {
        let changed = tx
            .execute(
                "UPDATE projects SET runtime_profile_id = ?1
                 WHERE id = ?2 AND runtime_profile_id = ?3",
                params![source.to_owned(), project_id.clone(), target.to_owned()],
            )
            .await?;
        if changed != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
    }
    tx.execute(
        "UPDATE runtime_migrations
         SET status = 'rolled_back', rollback_available = 0, rolled_back_at_ms = ?1
         WHERE id = ?2 AND status = 'completed'",
        params![now_ms(), migration_id.to_owned()],
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

// --------------------------------------------------------------------------
// Destructive reset / remove / repair
// --------------------------------------------------------------------------

pub async fn preview_destructive_operation(
    db: &Database,
    store: &ActionPlanStore,
    owner: &str,
    profile_id: &str,
    request: &DestructivePreviewRequest,
) -> Result<Option<DestructivePreview>, turso::Error> {
    let Some(profile) = load_profile(db, profile_id).await? else {
        return Ok(None);
    };
    let ownership_ok = profile.runtime_class == "built_in"
        && profile.ownership_state == "studio_managed"
        && profile.availability_state == "available";
    let (running_jobs, running_watch) = active_work(db, profile_id).await?;
    let has_active_work = running_jobs > 0 || running_watch > 0;
    let allowed = ownership_ok && !has_active_work;

    let blocker = if !ownership_ok {
        Some(
            "Reset and removal require an available built-in runtime with verified Studio ownership."
                .to_owned(),
        )
    } else if has_active_work {
        Some("Stop running jobs and watch sessions on this runtime before continuing.".to_owned())
    } else {
        None
    };

    let project_count = bound_project_count(db, profile_id).await?;
    let operation_id = format!("rto_{}", uuid::Uuid::new_v4().simple());
    let (engine_effect, volume_effect, binding_effect) = match request.action {
        DestructiveAction::Repair => (
            "inspected_and_preserved",
            "inspected_and_preserved",
            "preserved",
        ),
        DestructiveAction::ResetEngineData => {
            ("removed_during_reset", "data_loss_expected", "preserved")
        }
        DestructiveAction::RemoveBuiltInRuntime => (
            "removed_with_runtime",
            "data_loss_expected",
            "preserved_as_unavailable_after_removal",
        ),
    };
    let affected = vec![
        engine_category("containers", engine_effect),
        engine_category("images", engine_effect),
        engine_category("volumes", volume_effect),
        engine_category("networks", engine_effect),
        engine_category("build_cache", engine_effect),
        AffectedCategory {
            category: "project_bindings",
            count: Some(project_count),
            exactness: "exact",
            effect: binding_effect,
        },
    ];
    let scope_json = serde_json::to_string(&affected).unwrap_or_else(|_| "[]".into());
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO runtime_destructive_operations (
            id, profile_id, action, status, scope_json, created_at_ms
         ) VALUES (?1, ?2, ?3, 'prepared', ?4, ?5)",
        params![
            operation_id.clone(),
            profile_id.to_owned(),
            request.action.as_str().to_owned(),
            scope_json,
            now_ms()
        ],
    )
    .await?;

    let (plan_id, expires_in_seconds) = if allowed {
        let fingerprint = destructive_fingerprint(&profile, project_count);
        let ticket = store.prepare(
            owner,
            request.action.action_kind(),
            ActionPlanPayload::Destructive(DestructivePlan {
                profile_id: profile_id.to_owned(),
                action: request.action.as_str().to_owned(),
                fingerprint,
            }),
        );
        (Some(ticket.plan_id), Some(ticket.expires_in_seconds))
    } else {
        (None, None)
    };

    Ok(Some(DestructivePreview {
        operation_id,
        profile_id: profile_id.to_owned(),
        action: request.action.as_str().to_owned(),
        allowed,
        blocker,
        affected,
        preserved: vec![
            "Studio projects",
            "plans",
            "jobs",
            "preferences",
            "external runtimes",
        ],
        requires_fresh_preview: true,
        plan_id,
        expires_in_seconds,
    }))
}

/// Commit a destructive runtime action by its opaque plan id. Fully revalidates
/// ownership, runtime identity, provider/binding state, and active work, then —
/// because the engine-mutating provider command is Phase 14b — records the
/// attempt and returns `deferred_to_phase_14b`. It never mutates any engine and
/// never reports success.
pub async fn commit_destructive_operation(
    db: &Database,
    store: &ActionPlanStore,
    owner: &str,
    plan_id: &str,
) -> Result<DestructiveCommitResult, CommitRejected> {
    // Destructive plans have three action kinds; claim without a single expected
    // kind, then confirm the domain.
    let claimed = match store.claim(
        plan_id,
        owner,
        &[
            ActionKind::DestructiveRepair,
            ActionKind::DestructiveResetEngineData,
            ActionKind::DestructiveRemoveBuiltInRuntime,
        ],
    ) {
        Ok(claimed) => claimed,
        Err(error) => {
            let code = error.audit_code();
            reject_and_audit(db, ActionKind::DestructiveResetEngineData, None, code).await;
            return Err(CommitRejected::new(
                error.to_string(),
                "rejected_plan",
                code,
            ));
        }
    };
    let ActionPlanPayload::Destructive(plan) = claimed.payload else {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return Err(CommitRejected::new(
            "Plan payload did not match a destructive action.",
            "rejected_plan",
            "plan_kind_mismatch",
        ));
    };
    let Some(action) = DestructiveAction::parse(&plan.action) else {
        store.finish(&claimed.plan_id, crate::action_plans::PlanState::Failed);
        return Err(CommitRejected::new(
            "Unknown destructive action.",
            "rejected_plan",
            "unknown_action",
        ));
    };
    let kind = action.action_kind();
    let started = now_ms();

    // Revalidate runtime identity + ownership.
    let Some(profile) = load_profile(db, &plan.profile_id).await.unwrap_or(None) else {
        return reject_destructive(
            db,
            kind,
            &plan.profile_id,
            store,
            &claimed.plan_id,
            CommitRejected::new(
                "Runtime no longer exists. Preview the action again.",
                "rejected_not_found",
                "runtime_missing",
            ),
        )
        .await;
    };
    // External runtimes must never expose Studio-owned reset/remove.
    if profile.runtime_class != "built_in" || profile.ownership_state != "studio_managed" {
        return reject_destructive(
            db,
            kind,
            &plan.profile_id,
            store,
            &claimed.plan_id,
            CommitRejected::new(
                "This runtime is not a Studio-managed built-in runtime.",
                "rejected_external",
                "external_runtime",
            ),
        )
        .await;
    }
    if profile.availability_state != "available" {
        return reject_destructive(
            db,
            kind,
            &plan.profile_id,
            store,
            &claimed.plan_id,
            CommitRejected::new(
                "The runtime is not currently available.",
                "rejected_state",
                "runtime_unavailable",
            ),
        )
        .await;
    }
    // Revalidate inventory/ownership fingerprint.
    let project_count = bound_project_count(db, &plan.profile_id)
        .await
        .unwrap_or(-1);
    if destructive_fingerprint(&profile, project_count) != plan.fingerprint {
        return reject_destructive(
            db,
            kind,
            &plan.profile_id,
            store,
            &claimed.plan_id,
            CommitRejected::new(
                "Runtime ownership or inventory changed since preview. Preview the action again.",
                "rejected_stale",
                "stale_preview",
            ),
        )
        .await;
    }
    // Revalidate active jobs/watch.
    let (running_jobs, running_watch) = active_work(db, &plan.profile_id).await.unwrap_or((0, 0));
    if running_jobs > 0 || running_watch > 0 {
        return reject_destructive(
            db,
            kind,
            &plan.profile_id,
            store,
            &claimed.plan_id,
            CommitRejected::new(
                "A job or watch session is active on this runtime. Stop it and preview again.",
                "rejected_active_work",
                "active_work",
            ),
        )
        .await;
    }

    // Gate passed. The engine-mutating provider command is Phase 14b, so nothing
    // is executed here. Record the deferral and return a non-success terminal.
    store.finish(&claimed.plan_id, crate::action_plans::PlanState::Succeeded);
    let _ = action_audit::record(
        db,
        AuditEntry {
            kind,
            profile_id: Some(plan.profile_id.clone()),
            runtime_class: Some(profile.runtime_class.clone()),
            ownership_result: "authorized".to_owned(),
            command_kind: Some(action.deferred_command_kind().to_owned()),
            elevation_mode: Some("os_mediated_consent".to_owned()),
            terminal_status: action_audit::STATUS_DEFERRED_14B.to_owned(),
            affected: vec![AffectedCount {
                category: "project_bindings".to_owned(),
                count: project_count.max(0),
            }],
            failure_code: None,
            correlation_token: None,
            started_at_ms: started,
            completed_at_ms: Some(now_ms()),
        },
    )
    .await;
    Ok(DestructiveCommitResult {
        profile_id: plan.profile_id,
        action: plan.action,
        status: "deferred_to_phase_14b",
        revalidated: true,
        message: "Ownership and state verified. Engine execution arrives in a later phase; nothing was changed.".to_owned(),
        next_steps: vec![
            "The built-in runtime engine operation is not enabled in this build.".to_owned(),
        ],
    })
}

async fn reject_destructive(
    db: &Database,
    kind: ActionKind,
    profile_id: &str,
    store: &ActionPlanStore,
    plan_id: &str,
    rejection: CommitRejected,
) -> Result<DestructiveCommitResult, CommitRejected> {
    store.finish(plan_id, crate::action_plans::PlanState::Failed);
    let _ = action_audit::record_rejection(
        db,
        kind,
        Some(profile_id.to_owned()),
        rejection.ownership_result,
        rejection.audit_code,
    )
    .await;
    Err(rejection)
}

pub fn uninstall_policy() -> UninstallPolicy {
    UninstallPolicy {
        default_choice: "app_binaries_only",
        choices: vec![
            UninstallChoice {
                id: "app_binaries_only",
                label: "Remove app only",
                mutates_external_runtimes: false,
                selected_by_default: true,
            },
            UninstallChoice {
                id: "studio_metadata",
                label: "Delete Studio metadata",
                mutates_external_runtimes: false,
                selected_by_default: false,
            },
            UninstallChoice {
                id: "built_in_runtime",
                label: "Delete Susun Runtime",
                mutates_external_runtimes: false,
                selected_by_default: false,
            },
            UninstallChoice {
                id: "credentials",
                label: "Delete Studio credentials",
                mutates_external_runtimes: false,
                selected_by_default: false,
            },
        ],
        unattended_behavior: "remove_app_binaries_only_unless_explicit_flags_are_documented",
        reinstall_rule: "preserved built-in ownership must be proven by stored evidence, never inferred from its name",
    }
}

// --------------------------------------------------------------------------
// Shared helpers
// --------------------------------------------------------------------------

fn engine_category(category: &'static str, effect: &'static str) -> AffectedCategory {
    AffectedCategory {
        category,
        count: None,
        exactness: "unknown_until_engine_inspection",
        effect,
    }
}

fn failed_result(plan: &MigrationCommitPlan, failures: Vec<String>) -> MigrationResult {
    MigrationResult {
        migration_id: format!("rtm_{}", uuid::Uuid::new_v4().simple()),
        status: "failed",
        source_profile_id: plan.source_profile_id.clone(),
        target_profile_id: plan.target_profile_id.clone(),
        project_count: 0,
        skipped_items: Vec::new(),
        failures,
        rollback_available: true,
    }
}

async fn fail_commit(
    db: &Database,
    kind: ActionKind,
    profile_id: Option<String>,
    audit_code: &'static str,
    message: &str,
    started: i64,
) -> Result<MigrationResult, CommitRejected> {
    let _ = action_audit::record(
        db,
        AuditEntry {
            kind,
            profile_id,
            runtime_class: None,
            ownership_result: "rejected".to_owned(),
            command_kind: None,
            elevation_mode: None,
            terminal_status: action_audit::STATUS_REJECTED.to_owned(),
            affected: Vec::new(),
            failure_code: Some(audit_code.to_owned()),
            correlation_token: None,
            started_at_ms: started,
            completed_at_ms: Some(now_ms()),
        },
    )
    .await;
    Err(CommitRejected::new(
        message.to_owned(),
        "rejected_stale",
        audit_code,
    ))
}

async fn reject_and_audit(
    db: &Database,
    kind: ActionKind,
    profile_id: Option<String>,
    code: &'static str,
) {
    let _ = action_audit::record_rejection(db, kind, profile_id, "rejected", code).await;
}

async fn persist_failed_migration(
    db: &Database,
    plan: &MigrationCommitPlan,
    result: &MigrationResult,
) -> Result<(), turso::Error> {
    let conn = db.connect()?;
    conn.execute(
        "INSERT INTO runtime_migrations (
            id, source_profile_id, target_profile_id, status, project_count,
            project_ids_json, skipped_items_json, failures_json,
            rollback_available, created_at_ms, completed_at_ms
         ) VALUES (?1, ?2, ?3, 'failed', 0, ?4, '[]', ?5, 1, ?6, ?6)",
        params![
            result.migration_id.clone(),
            plan.source_profile_id.clone(),
            plan.target_profile_id.clone(),
            serde_json::to_string(&plan.project_ids).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&result.failures).unwrap_or_else(|_| "[]".into()),
            now_ms()
        ],
    )
    .await?;
    Ok(())
}

async fn migration_projects(
    db: &Database,
    ids: &[String],
    source_profile_id: &str,
) -> Result<Vec<MigrationProject>, turso::Error> {
    let conn = db.connect()?;
    let mut projects = Vec::new();
    for id in ids {
        let mut rows = conn
            .query(
                "SELECT id, name, runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
                params![id.clone()],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let bound: Option<String> = row.get(2)?;
            projects.push(MigrationProject {
                id: row.get(0)?,
                name: row.get(1)?,
                currently_bound_to_source: bound.as_deref() == Some(source_profile_id),
            });
        }
    }
    Ok(projects)
}

async fn bound_project_count(db: &Database, profile_id: &str) -> Result<i64, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM projects WHERE runtime_profile_id = ?1",
            params![profile_id.to_owned()],
        )
        .await?;
    match rows.next().await? {
        Some(row) => row.get(0),
        None => Ok(0),
    }
}

async fn load_profile(
    db: &Database,
    profile_id: &str,
) -> Result<Option<RuntimeProfile>, turso::Error> {
    Ok(list_all_profiles(db)
        .await?
        .into_iter()
        .find(|profile| profile.id == profile_id))
}

async fn load_migration(
    db: &Database,
    migration_id: &str,
) -> Result<Option<(String, String, String, String, i64)>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT source_profile_id, target_profile_id, project_ids_json, status,
                    rollback_available
             FROM runtime_migrations WHERE id = ?1 LIMIT 1",
            params![migration_id.to_owned()],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(Some((
            row.get::<String>(0)?,
            row.get::<String>(1)?,
            row.get::<String>(2)?,
            row.get::<String>(3)?,
            row.get::<i64>(4)?,
        ))),
        None => Ok(None),
    }
}

/// Count of running jobs and watch sessions attributed to a runtime profile.
/// Watch sessions attribute through their project's runtime binding.
async fn active_work(db: &Database, profile_id: &str) -> Result<(i64, i64), turso::Error> {
    let conn = db.connect()?;
    let mut job_rows = conn
        .query(
            "SELECT COUNT(*) FROM jobs WHERE status = 'running' AND runtime_profile_id = ?1",
            params![profile_id.to_owned()],
        )
        .await?;
    let jobs: i64 = match job_rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    };
    let mut watch_rows = conn
        .query(
            "SELECT COUNT(*) FROM watch_sessions w
             JOIN projects p ON p.id = w.project_id
             WHERE w.status = 'running' AND p.runtime_profile_id = ?1",
            params![profile_id.to_owned()],
        )
        .await?;
    let watch: i64 = match watch_rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    };
    Ok((jobs, watch))
}

/// Stable fingerprint of the source bindings + target selectability, so a commit
/// can detect any inventory/state change since preview.
async fn migration_fingerprint(
    db: &Database,
    source_id: &str,
    target: &RuntimeProfile,
    project_ids: &[String],
) -> Result<String, turso::Error> {
    let conn = db.connect()?;
    let mut sorted = project_ids.to_vec();
    sorted.sort();
    let mut canonical = format!(
        "src={source_id};tgt={};tavail={};tsel={};tconn={};",
        target.id, target.availability_state, target.management.can_select, target.connection.state
    );
    for project_id in &sorted {
        let mut rows = conn
            .query(
                "SELECT runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
                params![project_id.clone()],
            )
            .await?;
        let bound: Option<String> = match rows.next().await? {
            Some(row) => row.get(0)?,
            None => None,
        };
        canonical.push_str(&format!("{project_id}={};", bound.unwrap_or_default()));
    }
    Ok(stable_suffix(&canonical))
}

async fn rollback_fingerprint(
    db: &Database,
    migration_id: &str,
    source: &str,
    target: &str,
    project_ids: &[String],
) -> Result<String, turso::Error> {
    let conn = db.connect()?;
    let mut sorted = project_ids.to_vec();
    sorted.sort();
    let mut canonical = format!("mig={migration_id};src={source};tgt={target};");
    for project_id in &sorted {
        let mut rows = conn
            .query(
                "SELECT runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
                params![project_id.clone()],
            )
            .await?;
        let bound: Option<String> = match rows.next().await? {
            Some(row) => row.get(0)?,
            None => None,
        };
        canonical.push_str(&format!("{project_id}={};", bound.unwrap_or_default()));
    }
    Ok(stable_suffix(&canonical))
}

/// Ownership + inventory fingerprint for a destructive target. Binds the plan to
/// the exact ownership state, observation revision, and binding count seen at
/// preview, so any change forces a fresh preview.
fn destructive_fingerprint(profile: &RuntimeProfile, project_count: i64) -> String {
    stable_suffix(&format!(
        "id={};class={};own={};avail={};rev={};bindings={};",
        profile.id,
        profile.runtime_class,
        profile.ownership_state,
        profile.availability_state,
        profile.observation_revision,
        project_count
    ))
}
