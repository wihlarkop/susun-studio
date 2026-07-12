use serde::{Deserialize, Serialize};
use turso::{Database, params};

use super::{RuntimeProfile, list_all_profiles, now_ms};

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
pub struct MigrationRollbackResult {
    pub migration_id: String,
    pub status: &'static str,
    pub restored_project_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestructiveAction {
    Repair,
    ResetEngineData,
    RemoveBuiltInRuntime,
}

impl DestructiveAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Repair => "repair",
            Self::ResetEngineData => "reset_engine_data",
            Self::RemoveBuiltInRuntime => "remove_built_in_runtime",
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
}

#[derive(Debug, Serialize)]
pub struct AffectedCategory {
    pub category: &'static str,
    pub count: Option<i64>,
    pub exactness: &'static str,
    pub effect: &'static str,
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

pub async fn preview_migration(
    db: &Database,
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

    Ok(Some(MigrationPreview {
        source,
        target,
        projects,
        can_migrate: blockers.is_empty(),
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
    }))
}

pub async fn execute_migration(
    db: &Database,
    request: &MigrationRequest,
) -> Result<Option<MigrationResult>, turso::Error> {
    let Some(preview) = preview_migration(db, request).await? else {
        return Ok(None);
    };
    if !preview.can_migrate {
        let result = failed_result(request, preview.blockers);
        persist_failed_migration(db, request, &result).await?;
        return Ok(Some(result));
    }

    let migration_id = format!("rtm_{}", uuid::Uuid::new_v4().simple());
    let now = now_ms();
    let project_ids_json =
        serde_json::to_string(&request.project_ids).unwrap_or_else(|_| "[]".into());
    let skipped = vec![
        "volume data".to_owned(),
        "registry credentials".to_owned(),
        "runtime ownership".to_owned(),
    ];
    let skipped_json = serde_json::to_string(&skipped).unwrap_or_else(|_| "[]".into());
    let mut conn = db.connect()?;
    let tx = conn.transaction().await?;
    for project_id in &request.project_ids {
        let changed = tx
            .execute(
                "UPDATE projects SET runtime_profile_id = ?1
                 WHERE id = ?2 AND runtime_profile_id = ?3",
                params![
                    request.target_profile_id.clone(),
                    project_id.clone(),
                    request.source_profile_id.clone()
                ],
            )
            .await?;
        if changed != 1 {
            tx.rollback().await?;
            let result = failed_result(
                request,
                vec![
                    "Project bindings changed after preview; no migration was applied.".to_owned(),
                ],
            );
            persist_failed_migration(db, request, &result).await?;
            return Ok(Some(result));
        }
    }
    tx.execute(
        "INSERT INTO runtime_migrations (
            id, source_profile_id, target_profile_id, status, project_count,
            project_ids_json, skipped_items_json, failures_json,
            rollback_available, created_at_ms, completed_at_ms
         ) VALUES (?1, ?2, ?3, 'completed', ?4, ?5, ?6, '[]', 1, ?7, ?7)",
        params![
            migration_id.clone(),
            request.source_profile_id.clone(),
            request.target_profile_id.clone(),
            request.project_ids.len() as i64,
            project_ids_json,
            skipped_json,
            now
        ],
    )
    .await?;
    tx.commit().await?;

    Ok(Some(MigrationResult {
        migration_id,
        status: "completed",
        source_profile_id: request.source_profile_id.clone(),
        target_profile_id: request.target_profile_id.clone(),
        project_count: request.project_ids.len(),
        skipped_items: skipped,
        failures: Vec::new(),
        rollback_available: true,
    }))
}

pub async fn rollback_migration(
    db: &Database,
    migration_id: &str,
) -> Result<Option<MigrationRollbackResult>, turso::Error> {
    let mut conn = db.connect()?;
    let record = {
        let mut rows = conn
            .query(
                "SELECT source_profile_id, target_profile_id, project_ids_json, status,
                        rollback_available
                 FROM runtime_migrations WHERE id = ?1 LIMIT 1",
                params![migration_id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Some((
                row.get::<String>(0)?,
                row.get::<String>(1)?,
                row.get::<String>(2)?,
                row.get::<String>(3)?,
                row.get::<i64>(4)?,
            )),
            None => None,
        }
    };
    let Some((source, target, project_ids_json, status, rollback_available)) = record else {
        return Ok(None);
    };
    if status != "completed" || rollback_available != 1 {
        return Ok(Some(MigrationRollbackResult {
            migration_id: migration_id.to_owned(),
            status: "unavailable",
            restored_project_count: 0,
        }));
    }
    let project_ids: Vec<String> = serde_json::from_str(&project_ids_json).unwrap_or_default();
    let tx = conn.transaction().await?;
    for project_id in &project_ids {
        let changed = tx
            .execute(
                "UPDATE projects SET runtime_profile_id = ?1
                 WHERE id = ?2 AND runtime_profile_id = ?3",
                params![source.clone(), project_id.clone(), target.clone()],
            )
            .await?;
        if changed != 1 {
            tx.rollback().await?;
            return Ok(Some(MigrationRollbackResult {
                migration_id: migration_id.to_owned(),
                status: "failed",
                restored_project_count: 0,
            }));
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
    Ok(Some(MigrationRollbackResult {
        migration_id: migration_id.to_owned(),
        status: "rolled_back",
        restored_project_count: project_ids.len(),
    }))
}

pub async fn preview_destructive_operation(
    db: &Database,
    profile_id: &str,
    request: &DestructivePreviewRequest,
) -> Result<Option<DestructivePreview>, turso::Error> {
    let profiles = list_all_profiles(db).await?;
    let Some(profile) = profiles
        .into_iter()
        .find(|profile| profile.id == profile_id)
    else {
        return Ok(None);
    };
    let allowed = profile.runtime_class == "built_in"
        && profile.ownership_state == "studio_managed"
        && profile.availability_state == "available";
    let blocker = (!allowed).then(|| {
        "Reset and removal require an available built-in runtime with verified Studio ownership."
            .to_owned()
    });
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
        AffectedCategory {
            category: "containers",
            count: None,
            exactness: "unknown_until_engine_inspection",
            effect: engine_effect,
        },
        AffectedCategory {
            category: "images",
            count: None,
            exactness: "unknown_until_engine_inspection",
            effect: engine_effect,
        },
        AffectedCategory {
            category: "volumes",
            count: None,
            exactness: "unknown_until_engine_inspection",
            effect: volume_effect,
        },
        AffectedCategory {
            category: "networks",
            count: None,
            exactness: "unknown_until_engine_inspection",
            effect: engine_effect,
        },
        AffectedCategory {
            category: "build_cache",
            count: None,
            exactness: "unknown_until_engine_inspection",
            effect: engine_effect,
        },
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
    }))
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

fn failed_result(request: &MigrationRequest, failures: Vec<String>) -> MigrationResult {
    MigrationResult {
        migration_id: format!("rtm_{}", uuid::Uuid::new_v4().simple()),
        status: "failed",
        source_profile_id: request.source_profile_id.clone(),
        target_profile_id: request.target_profile_id.clone(),
        project_count: 0,
        skipped_items: Vec::new(),
        failures,
        rollback_available: true,
    }
}

async fn persist_failed_migration(
    db: &Database,
    request: &MigrationRequest,
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
            request.source_profile_id.clone(),
            request.target_profile_id.clone(),
            serde_json::to_string(&request.project_ids).unwrap_or_else(|_| "[]".into()),
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
