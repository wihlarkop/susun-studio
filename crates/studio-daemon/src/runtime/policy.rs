use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use susun::EngineEndpoint;
use turso::transaction::Transaction;
use turso::{Connection, Database, params};

use super::{ManagementCapabilities, find_provider, now_ms};

const MAX_PROJECT_SUMMARY_BATCH: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBindingSource {
    ProjectPin,
    GlobalPreference,
    PlatformDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBindingState {
    Ready,
    Unavailable,
    Missing,
    Unconfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeBindingSummary {
    pub source: RuntimeBindingSource,
    pub state: RuntimeBindingState,
    pub profile_id: Option<String>,
    pub runtime_class: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimePreference {
    pub preferred_profile_id: Option<String>,
    pub binding: RuntimeBindingSummary,
}

/// Display-safe impact of changing the global runtime preference. The opaque
/// fingerprint is the daemon's proof that the commit still describes this
/// exact observed policy, binding, and active-work state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimePreferenceImpactPreview {
    pub current: RuntimeBindingSummary,
    pub target: RuntimeBindingSummary,
    pub inheriting_project_count: i64,
    pub explicitly_pinned_project_count: i64,
    pub affected_active_jobs: i64,
    pub affected_active_watch_sessions: i64,
    pub change_allowed: bool,
    pub reason_code: Option<String>,
    pub is_noop: bool,
    pub impact_fingerprint: String,
}

/// Display-safe impact of changing exactly one project's explicit runtime pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectRuntimeImpactPreview {
    pub project_id: String,
    pub current: RuntimeBindingSummary,
    pub target: RuntimeBindingSummary,
    pub affected_active_jobs: i64,
    pub affected_active_watch_sessions: i64,
    pub change_allowed: bool,
    pub reason_code: Option<String>,
    pub is_noop: bool,
    pub impact_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextChangeRejection {
    TargetMissing,
    TargetUnavailable,
    ActiveWork,
    StalePreview,
}

impl ContextChangeRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::TargetMissing => "target_missing",
            Self::TargetUnavailable => "target_unavailable",
            Self::ActiveWork => "active_work",
            Self::StalePreview => "stale_preview",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::TargetMissing => {
                "The selected runtime is no longer available. Refresh and choose a runtime again."
            }
            Self::TargetUnavailable => {
                "The selected runtime is not ready. Recheck it and preview the change again."
            }
            Self::ActiveWork => {
                "A running job or watch session would be redirected. Stop it and preview the change again."
            }
            Self::StalePreview => {
                "Runtime context changed since preview. Review the current impact and preview again."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextChangeCommit<T> {
    Updated(T),
    Rejected(ContextChangeRejection),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeAttribution {
    pub runtime_profile_id: Option<String>,
    pub runtime_class: Option<String>,
    pub binding_source: RuntimeBindingSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetPreferredOutcome {
    Updated,
    NotFound,
    Unavailable,
}

/// A request-local runtime decision. The endpoint is deliberately internal:
/// routes and persisted reports consume the redacted summary or attribution.
#[derive(Clone)]
pub(crate) struct ResolvedRuntime {
    summary: RuntimeBindingSummary,
    endpoint: Option<EngineEndpoint>,
}

impl ResolvedRuntime {
    pub fn summary(&self) -> &RuntimeBindingSummary {
        &self.summary
    }

    pub(crate) fn endpoint(&self) -> Option<&EngineEndpoint> {
        self.endpoint.as_ref()
    }

    pub fn attribution(&self) -> RuntimeAttribution {
        RuntimeAttribution {
            runtime_profile_id: self.summary.profile_id.clone(),
            runtime_class: self.summary.runtime_class.clone(),
            binding_source: self.summary.source,
        }
    }
}

struct ProfileContext {
    id: String,
    provider_id: String,
    provider_runtime_key: String,
    display_name: String,
    runtime_class: String,
    ownership_state: String,
    connection_state: String,
    availability_state: String,
    observation_revision: i64,
}

impl ProfileContext {
    fn is_ready(&self) -> bool {
        self.connection_state == "summarized"
            && ManagementCapabilities::derive(
                &self.runtime_class,
                &self.ownership_state,
                &self.availability_state,
            )
            .can_select
    }

    fn endpoint(&self) -> Option<EngineEndpoint> {
        self.is_ready().then(|| {
            find_provider(&self.provider_id)
                .and_then(|provider| provider.endpoint_for_runtime_key(&self.provider_runtime_key))
        })?
    }
}

pub async fn read_preference(db: &Database) -> Result<RuntimePreference, turso::Error> {
    let resolved = resolve_global(db).await?;
    let binding = resolved.summary;
    let preferred_profile_id = match binding.source {
        RuntimeBindingSource::GlobalPreference => binding.profile_id.clone(),
        RuntimeBindingSource::PlatformDefault => None,
        RuntimeBindingSource::ProjectPin => {
            unreachable!("global resolution cannot return a project pin")
        }
    };
    Ok(RuntimePreference {
        preferred_profile_id,
        binding,
    })
}

pub async fn set_preferred(
    db: &Database,
    preferred_profile_id: Option<&str>,
) -> Result<SetPreferredOutcome, turso::Error> {
    if let Some(profile_id) = preferred_profile_id {
        let outcome = validate_profile_for_binding(db, profile_id).await?;
        if outcome != SetPreferredOutcome::Updated {
            return Ok(outcome);
        }
    }

    let conn = db.connect()?;
    conn.execute(
        "UPDATE runtime_policy SET preferred_profile_id = ?1, updated_at_ms = ?2
         WHERE singleton = 1",
        params![preferred_profile_id.map(str::to_owned), now_ms()],
    )
    .await?;
    Ok(SetPreferredOutcome::Updated)
}

/// Preview a guarded global-preference change. This only reads persisted
/// policy/observation state; it never probes, discovers, or selects a runtime.
pub async fn preview_preference_change(
    db: &Database,
    target_profile_id: Option<&str>,
) -> Result<RuntimePreferenceImpactPreview, turso::Error> {
    let current_preference = preferred_profile_id(db).await?;
    let current = summarize_global(db).await?;
    let target = summarize_preference_target(db, target_profile_id).await?;
    let is_noop = current_preference.as_deref() == target_profile_id;
    let (inheriting_project_count, explicitly_pinned_project_count, project_bindings) =
        project_binding_inventory(db).await?;
    let active = if is_noop {
        ActiveRuntimeWork::default()
    } else {
        active_work_for_global_binding(db, &current).await?
    };
    let rejection = if is_noop {
        None
    } else {
        binding_rejection(&target).or_else(|| {
            active
                .has_work()
                .then_some(ContextChangeRejection::ActiveWork)
        })
    };
    let target_revision = profile_observation_revision(db, target.profile_id.as_deref()).await?;
    let fingerprint = stable_impact_fingerprint(&[
        "global-preference",
        current_preference.as_deref().unwrap_or("<none>"),
        target_profile_id.unwrap_or("<none>"),
        &binding_fingerprint(&current),
        &binding_fingerprint(&target),
        &target_revision
            .map(|revision| revision.to_string())
            .unwrap_or_else(|| "<none>".to_owned()),
        &project_bindings,
        &active.fingerprint,
    ]);

    Ok(RuntimePreferenceImpactPreview {
        current,
        target,
        inheriting_project_count,
        explicitly_pinned_project_count,
        affected_active_jobs: active.jobs,
        affected_active_watch_sessions: active.watch_sessions,
        change_allowed: rejection.is_none(),
        reason_code: rejection.map(|rejection| rejection.code().to_owned()),
        is_noop,
        impact_fingerprint: fingerprint,
    })
}

/// Recompute the exact daemon-owned global impact immediately before the
/// metadata write. The expected fingerprint is not a capability: it is only a
/// stale-preview guard and all target/active-work checks are repeated here.
pub async fn commit_preference_change(
    db: &Database,
    target_profile_id: Option<&str>,
    expected_impact_fingerprint: &str,
) -> Result<ContextChangeCommit<RuntimePreference>, turso::Error> {
    let preview = preview_preference_change(db, target_profile_id).await?;
    if preview.impact_fingerprint != expected_impact_fingerprint {
        return Ok(ContextChangeCommit::Rejected(
            ContextChangeRejection::StalePreview,
        ));
    }
    if let Some(rejection) = preview
        .reason_code
        .as_deref()
        .and_then(context_rejection_from_code)
    {
        return Ok(ContextChangeCommit::Rejected(rejection));
    }

    if !preview.is_noop {
        if let Some(profile_id) = target_profile_id {
            match validate_profile_for_binding(db, profile_id).await? {
                SetPreferredOutcome::Updated => {}
                SetPreferredOutcome::NotFound => {
                    return Ok(ContextChangeCommit::Rejected(
                        ContextChangeRejection::TargetMissing,
                    ));
                }
                SetPreferredOutcome::Unavailable => {
                    return Ok(ContextChangeCommit::Rejected(
                        ContextChangeRejection::TargetUnavailable,
                    ));
                }
            }
        }

        let conn = db.connect()?;
        let changed = match target_profile_id {
            Some(profile_id) => {
                let Some(observation_revision) =
                    profile_observation_revision(db, Some(profile_id)).await?
                else {
                    return Ok(ContextChangeCommit::Rejected(
                        ContextChangeRejection::TargetMissing,
                    ));
                };
                conn.execute(
                    "UPDATE runtime_policy
                     SET preferred_profile_id = ?1, updated_at_ms = ?2
                     WHERE singleton = 1
                       AND preferred_profile_id IS ?3
                       AND EXISTS (
                           SELECT 1 FROM runtime_profiles
                           WHERE id = ?1 AND observation_revision = ?4
                       )",
                    params![
                        profile_id.to_owned(),
                        now_ms(),
                        read_preference_value_from_preview(&preview.current),
                        observation_revision,
                    ],
                )
                .await?
            }
            None => {
                conn.execute(
                    "UPDATE runtime_policy
                     SET preferred_profile_id = NULL, updated_at_ms = ?1
                     WHERE singleton = 1 AND preferred_profile_id IS ?2",
                    params![
                        now_ms(),
                        read_preference_value_from_preview(&preview.current)
                    ],
                )
                .await?
            }
        };
        if changed == 0 {
            return Ok(ContextChangeCommit::Rejected(
                ContextChangeRejection::StalePreview,
            ));
        }
    }

    Ok(ContextChangeCommit::Updated(read_preference(db).await?))
}

/// Preview a guarded explicit project binding change. `None` represents a
/// missing project; an existing unpinned project is represented by a preview
/// whose current source is global preference or platform default.
pub async fn preview_project_binding_change(
    db: &Database,
    project_id: &str,
    target_profile_id: Option<&str>,
) -> Result<Option<ProjectRuntimeImpactPreview>, turso::Error> {
    let Some(current_pin) = project_binding_if_present(db, project_id).await? else {
        return Ok(None);
    };
    let current = resolve_project(db, project_id).await?.summary;
    let target = match target_profile_id {
        Some(profile_id) => {
            resolve_profile(db, RuntimeBindingSource::ProjectPin, profile_id.to_owned())
                .await?
                .summary
        }
        None => summarize_global(db).await?,
    };
    let is_noop = current_pin.as_deref() == target_profile_id;
    let active = if is_noop {
        ActiveRuntimeWork::default()
    } else {
        active_work_for_project(db, project_id).await?
    };
    let rejection = if is_noop {
        None
    } else {
        binding_rejection(&target).or_else(|| {
            active
                .has_work()
                .then_some(ContextChangeRejection::ActiveWork)
        })
    };
    let current_preference = preferred_profile_id(db).await?;
    let target_revision = profile_observation_revision(db, target.profile_id.as_deref()).await?;
    let fingerprint = stable_impact_fingerprint(&[
        "project-binding",
        project_id,
        current_pin.as_deref().unwrap_or("<none>"),
        target_profile_id.unwrap_or("<none>"),
        current_preference.as_deref().unwrap_or("<none>"),
        &binding_fingerprint(&current),
        &binding_fingerprint(&target),
        &target_revision
            .map(|revision| revision.to_string())
            .unwrap_or_else(|| "<none>".to_owned()),
        &active.fingerprint,
    ]);

    Ok(Some(ProjectRuntimeImpactPreview {
        project_id: project_id.to_owned(),
        current,
        target,
        affected_active_jobs: active.jobs,
        affected_active_watch_sessions: active.watch_sessions,
        change_allowed: rejection.is_none(),
        reason_code: rejection.map(|rejection| rejection.code().to_owned()),
        is_noop,
        impact_fingerprint: fingerprint,
    }))
}

/// Apply one project pin only after recomputing the exact impact. The compare
/// and swap predicate prevents a concurrent pin edit from being overwritten.
pub async fn commit_project_binding_change(
    db: &Database,
    project_id: &str,
    target_profile_id: Option<&str>,
    expected_impact_fingerprint: &str,
) -> Result<Option<ContextChangeCommit<()>>, turso::Error> {
    let Some(preview) = preview_project_binding_change(db, project_id, target_profile_id).await?
    else {
        return Ok(None);
    };
    if preview.impact_fingerprint != expected_impact_fingerprint {
        return Ok(Some(ContextChangeCommit::Rejected(
            ContextChangeRejection::StalePreview,
        )));
    }
    if let Some(rejection) = preview
        .reason_code
        .as_deref()
        .and_then(context_rejection_from_code)
    {
        return Ok(Some(ContextChangeCommit::Rejected(rejection)));
    }

    if !preview.is_noop {
        if let Some(profile_id) = target_profile_id {
            match validate_profile_for_binding(db, profile_id).await? {
                SetPreferredOutcome::Updated => {}
                SetPreferredOutcome::NotFound => {
                    return Ok(Some(ContextChangeCommit::Rejected(
                        ContextChangeRejection::TargetMissing,
                    )));
                }
                SetPreferredOutcome::Unavailable => {
                    return Ok(Some(ContextChangeCommit::Rejected(
                        ContextChangeRejection::TargetUnavailable,
                    )));
                }
            }
        }

        let expected_current_pin = match preview.current.source {
            RuntimeBindingSource::ProjectPin => preview.current.profile_id.clone(),
            RuntimeBindingSource::GlobalPreference | RuntimeBindingSource::PlatformDefault => None,
        };
        let conn = db.connect()?;
        let changed = match target_profile_id {
            Some(profile_id) => {
                let Some(observation_revision) =
                    profile_observation_revision(db, Some(profile_id)).await?
                else {
                    return Ok(Some(ContextChangeCommit::Rejected(
                        ContextChangeRejection::TargetMissing,
                    )));
                };
                conn.execute(
                    "UPDATE projects
                     SET runtime_profile_id = ?1
                     WHERE id = ?2
                       AND runtime_profile_id IS ?3
                       AND EXISTS (
                           SELECT 1 FROM runtime_profiles
                           WHERE id = ?1 AND observation_revision = ?4
                       )",
                    params![
                        profile_id.to_owned(),
                        project_id.to_owned(),
                        expected_current_pin,
                        observation_revision,
                    ],
                )
                .await?
            }
            None => match preview.target.source {
                RuntimeBindingSource::GlobalPreference => {
                    let Some(profile_id) = preview.target.profile_id.as_deref() else {
                        return Ok(Some(ContextChangeCommit::Rejected(
                            ContextChangeRejection::StalePreview,
                        )));
                    };
                    let Some(observation_revision) =
                        profile_observation_revision(db, Some(profile_id)).await?
                    else {
                        return Ok(Some(ContextChangeCommit::Rejected(
                            ContextChangeRejection::TargetMissing,
                        )));
                    };
                    conn.execute(
                        "UPDATE projects
                         SET runtime_profile_id = NULL
                         WHERE id = ?1
                           AND runtime_profile_id IS ?2
                           AND EXISTS (
                               SELECT 1 FROM runtime_policy
                               WHERE singleton = 1 AND preferred_profile_id IS ?3
                           )
                           AND EXISTS (
                               SELECT 1 FROM runtime_profiles
                               WHERE id = ?3 AND observation_revision = ?4
                           )",
                        params![
                            project_id.to_owned(),
                            expected_current_pin,
                            profile_id.to_owned(),
                            observation_revision,
                        ],
                    )
                    .await?
                }
                RuntimeBindingSource::PlatformDefault => {
                    conn.execute(
                        "UPDATE projects
                         SET runtime_profile_id = NULL
                         WHERE id = ?1
                           AND runtime_profile_id IS ?2
                           AND EXISTS (
                               SELECT 1 FROM runtime_policy
                               WHERE singleton = 1 AND preferred_profile_id IS NULL
                           )",
                        params![project_id.to_owned(), expected_current_pin],
                    )
                    .await?
                }
                RuntimeBindingSource::ProjectPin => {
                    unreachable!("clearing a pin cannot target a pin")
                }
            },
        };
        if changed == 0 {
            return Ok(Some(ContextChangeCommit::Rejected(
                ContextChangeRejection::StalePreview,
            )));
        }
    }

    Ok(Some(ContextChangeCommit::Updated(())))
}

/// Validate a user-selected profile before it becomes either a global
/// preference or a project pin. Missing, unavailable, and
/// ownership-conflicted profiles are rejected; callers must never substitute
/// another runtime on their behalf.
pub async fn validate_profile_for_binding(
    db: &Database,
    profile_id: &str,
) -> Result<SetPreferredOutcome, turso::Error> {
    let Some(profile) = load_profile(db, profile_id).await? else {
        return Ok(SetPreferredOutcome::NotFound);
    };
    if profile.ownership_state == "ownership_conflict"
        || !profile.is_ready()
        || profile.endpoint().is_none()
    {
        return Ok(SetPreferredOutcome::Unavailable);
    }
    Ok(SetPreferredOutcome::Updated)
}

#[derive(Default)]
struct ActiveRuntimeWork {
    jobs: i64,
    watch_sessions: i64,
    fingerprint: String,
}

impl ActiveRuntimeWork {
    fn has_work(&self) -> bool {
        self.jobs > 0 || self.watch_sessions > 0
    }
}

fn binding_rejection(binding: &RuntimeBindingSummary) -> Option<ContextChangeRejection> {
    match binding.state {
        RuntimeBindingState::Missing => Some(ContextChangeRejection::TargetMissing),
        RuntimeBindingState::Unavailable => Some(ContextChangeRejection::TargetUnavailable),
        RuntimeBindingState::Ready | RuntimeBindingState::Unconfigured => None,
    }
}

fn context_rejection_from_code(code: &str) -> Option<ContextChangeRejection> {
    match code {
        "target_missing" => Some(ContextChangeRejection::TargetMissing),
        "target_unavailable" => Some(ContextChangeRejection::TargetUnavailable),
        "active_work" => Some(ContextChangeRejection::ActiveWork),
        _ => None,
    }
}

fn read_preference_value_from_preview(current: &RuntimeBindingSummary) -> Option<String> {
    match current.source {
        RuntimeBindingSource::GlobalPreference => current.profile_id.clone(),
        RuntimeBindingSource::PlatformDefault => None,
        RuntimeBindingSource::ProjectPin => unreachable!("global impact cannot have a project pin"),
    }
}

async fn summarize_preference_target(
    db: &Database,
    preferred_profile_id: Option<&str>,
) -> Result<RuntimeBindingSummary, turso::Error> {
    match preferred_profile_id {
        Some(profile_id) => Ok(resolve_profile(
            db,
            RuntimeBindingSource::GlobalPreference,
            profile_id.to_owned(),
        )
        .await?
        .summary),
        None => Ok(platform_default().summary),
    }
}

async fn project_binding_inventory(db: &Database) -> Result<(i64, i64, String), turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT id, runtime_profile_id FROM projects ORDER BY id ASC",
            (),
        )
        .await?;
    let mut inheriting = 0_i64;
    let mut pinned = 0_i64;
    let mut fingerprint = String::new();
    while let Some(row) = rows.next().await? {
        let id: String = row.get(0)?;
        let profile_id: Option<String> = row.get(1)?;
        match profile_id {
            Some(profile_id) => {
                pinned += 1;
                fingerprint.push_str(&format!("{id}=p:{profile_id};"));
            }
            None => {
                inheriting += 1;
                fingerprint.push_str(&format!("{id}=i;"));
            }
        }
    }
    Ok((inheriting, pinned, fingerprint))
}

async fn active_work_for_global_binding(
    db: &Database,
    current: &RuntimeBindingSummary,
) -> Result<ActiveRuntimeWork, turso::Error> {
    let source = binding_source_value(current.source);
    let profile_id = current.profile_id.as_deref();
    let conn = db.connect()?;
    let (jobs, jobs_fingerprint) = active_rows(
        &conn,
        "jobs",
        "id, project_id, runtime_profile_id, runtime_binding_source",
        "status = 'running' AND (
             runtime_binding_source IS NULL OR
             (runtime_binding_source = ?1 AND runtime_profile_id IS ?2)
         )",
        params![source.to_owned(), profile_id.map(str::to_owned)],
    )
    .await?;
    let (watch_sessions, watches_fingerprint) = active_rows(
        &conn,
        "watch_sessions",
        "id, project_id, runtime_profile_id, runtime_binding_source",
        "status = 'running' AND (
             runtime_binding_source IS NULL OR
             (runtime_binding_source = ?1 AND runtime_profile_id IS ?2)
         )",
        params![source.to_owned(), profile_id.map(str::to_owned)],
    )
    .await?;
    Ok(ActiveRuntimeWork {
        jobs,
        watch_sessions,
        fingerprint: format!("jobs:{jobs_fingerprint}|watch:{watches_fingerprint}"),
    })
}

async fn active_work_for_project(
    db: &Database,
    project_id: &str,
) -> Result<ActiveRuntimeWork, turso::Error> {
    let conn = db.connect()?;
    let (jobs, jobs_fingerprint) = active_rows(
        &conn,
        "jobs",
        "id, project_id, runtime_profile_id, runtime_binding_source",
        "status = 'running' AND project_id = ?1",
        params![project_id.to_owned()],
    )
    .await?;
    let (watch_sessions, watches_fingerprint) = active_rows(
        &conn,
        "watch_sessions",
        "id, project_id, runtime_profile_id, runtime_binding_source",
        "status = 'running' AND project_id = ?1",
        params![project_id.to_owned()],
    )
    .await?;
    Ok(ActiveRuntimeWork {
        jobs,
        watch_sessions,
        fingerprint: format!("jobs:{jobs_fingerprint}|watch:{watches_fingerprint}"),
    })
}

async fn active_rows(
    conn: &Connection,
    table: &str,
    columns: &str,
    predicate: &str,
    params: impl turso::params::IntoParams,
) -> Result<(i64, String), turso::Error> {
    let sql = format!("SELECT {columns} FROM {table} WHERE {predicate} ORDER BY id ASC");
    let mut rows = conn.query(&sql, params).await?;
    let mut count = 0_i64;
    let mut fingerprint = String::new();
    while let Some(row) = rows.next().await? {
        count += 1;
        let id: String = row.get(0)?;
        let project_id: String = row.get(1)?;
        let runtime_profile_id: Option<String> = row.get(2)?;
        let source: Option<String> = row.get(3)?;
        fingerprint.push_str(&format!(
            "{id}:{project_id}:{}:{};",
            runtime_profile_id.unwrap_or_else(|| "<none>".to_owned()),
            source.unwrap_or_else(|| "<legacy>".to_owned())
        ));
    }
    Ok((count, fingerprint))
}

fn binding_source_value(source: RuntimeBindingSource) -> &'static str {
    match source {
        RuntimeBindingSource::ProjectPin => "project_pin",
        RuntimeBindingSource::GlobalPreference => "global_preference",
        RuntimeBindingSource::PlatformDefault => "platform_default",
    }
}

fn binding_fingerprint(binding: &RuntimeBindingSummary) -> String {
    format!(
        "{}:{}:{}:{}",
        binding_source_value(binding.source),
        binding_state_value(binding.state),
        binding.profile_id.as_deref().unwrap_or("<none>"),
        binding.runtime_class.as_deref().unwrap_or("<none>"),
    )
}

fn binding_state_value(state: RuntimeBindingState) -> &'static str {
    match state {
        RuntimeBindingState::Ready => "ready",
        RuntimeBindingState::Unavailable => "unavailable",
        RuntimeBindingState::Missing => "missing",
        RuntimeBindingState::Unconfigured => "unconfigured",
    }
}

fn stable_impact_fingerprint(parts: &[&str]) -> String {
    super::stable_suffix(&parts.join("\u{1f}"))
}

async fn profile_observation_revision(
    db: &Database,
    profile_id: Option<&str>,
) -> Result<Option<i64>, turso::Error> {
    let Some(profile_id) = profile_id else {
        return Ok(None);
    };
    Ok(load_profile(db, profile_id)
        .await?
        .map(|profile| profile.observation_revision))
}

pub async fn summarize_global(db: &Database) -> Result<RuntimeBindingSummary, turso::Error> {
    Ok(resolve_global(db).await?.summary)
}

/// Resolve a bounded set of persisted project pins with bulk project and
/// profile reads. Callers with larger lists are chunked instead of issuing one
/// query per project.
pub async fn summarize_projects(
    db: &Database,
    project_ids: &[String],
) -> Result<HashMap<String, RuntimeBindingSummary>, turso::Error> {
    let global = summarize_global(db).await?;
    let mut summaries = HashMap::with_capacity(project_ids.len());

    for project_ids in project_ids.chunks(MAX_PROJECT_SUMMARY_BATCH) {
        let bindings = load_project_bindings(db, project_ids).await?;
        let profile_ids = bindings
            .values()
            .filter_map(Clone::clone)
            .collect::<HashSet<_>>();
        let profiles = load_profiles(db, &profile_ids).await?;

        for project_id in project_ids {
            let summary = match bindings.get(project_id).and_then(Clone::clone) {
                Some(profile_id) => {
                    let profile = profiles.get(&profile_id);
                    profile_summary(RuntimeBindingSource::ProjectPin, profile_id, profile)
                }
                None => global.clone(),
            };
            summaries.insert(project_id.clone(), summary);
        }
    }

    Ok(summaries)
}

pub(crate) async fn resolve_global(db: &Database) -> Result<ResolvedRuntime, turso::Error> {
    match preferred_profile_id(db).await? {
        Some(profile_id) => {
            resolve_profile(db, RuntimeBindingSource::GlobalPreference, profile_id).await
        }
        None => Ok(platform_default()),
    }
}

pub(crate) async fn resolve_project(
    db: &Database,
    project_id: &str,
) -> Result<ResolvedRuntime, turso::Error> {
    match project_binding(db, project_id).await? {
        Some(profile_id) => resolve_profile(db, RuntimeBindingSource::ProjectPin, profile_id).await,
        None => resolve_global(db).await,
    }
}

pub(super) async fn set_preferred_in_transaction(
    tx: &Transaction<'_>,
    preferred_profile_id: &str,
) -> Result<(), turso::Error> {
    tx.execute(
        "UPDATE runtime_policy SET preferred_profile_id = ?1, updated_at_ms = ?2
         WHERE singleton = 1",
        params![preferred_profile_id.to_owned(), now_ms()],
    )
    .await?;
    Ok(())
}

pub(super) async fn resolve_profile(
    db: &Database,
    source: RuntimeBindingSource,
    profile_id: String,
) -> Result<ResolvedRuntime, turso::Error> {
    let Some(profile) = load_profile(db, &profile_id).await? else {
        return Ok(ResolvedRuntime {
            summary: RuntimeBindingSummary {
                source,
                state: RuntimeBindingState::Missing,
                profile_id: Some(profile_id.clone()),
                runtime_class: None,
                display_name: format!("Missing runtime ({profile_id})"),
            },
            endpoint: None,
        });
    };
    let endpoint = profile.endpoint();
    let state = if endpoint.is_some() {
        RuntimeBindingState::Ready
    } else {
        RuntimeBindingState::Unavailable
    };
    Ok(ResolvedRuntime {
        summary: RuntimeBindingSummary {
            source,
            state,
            profile_id: Some(profile.id),
            runtime_class: Some(profile.runtime_class),
            display_name: profile.display_name,
        },
        endpoint,
    })
}

fn profile_summary(
    source: RuntimeBindingSource,
    profile_id: String,
    profile: Option<&ProfileContext>,
) -> RuntimeBindingSummary {
    let Some(profile) = profile else {
        return RuntimeBindingSummary {
            source,
            state: RuntimeBindingState::Missing,
            profile_id: Some(profile_id.clone()),
            runtime_class: None,
            display_name: format!("Missing runtime ({profile_id})"),
        };
    };
    RuntimeBindingSummary {
        source,
        state: if profile.is_ready() && profile.endpoint().is_some() {
            RuntimeBindingState::Ready
        } else {
            RuntimeBindingState::Unavailable
        },
        profile_id: Some(profile.id.clone()),
        runtime_class: Some(profile.runtime_class.clone()),
        display_name: profile.display_name.clone(),
    }
}

fn platform_default() -> ResolvedRuntime {
    ResolvedRuntime {
        summary: RuntimeBindingSummary {
            source: RuntimeBindingSource::PlatformDefault,
            state: RuntimeBindingState::Unconfigured,
            profile_id: None,
            runtime_class: None,
            display_name: "Platform default runtime".to_owned(),
        },
        endpoint: None,
    }
}

#[cfg(test)]
pub(crate) fn resolved_runtime_for_test(attribution: RuntimeAttribution) -> ResolvedRuntime {
    let state = match attribution.binding_source {
        RuntimeBindingSource::PlatformDefault => RuntimeBindingState::Unconfigured,
        RuntimeBindingSource::ProjectPin | RuntimeBindingSource::GlobalPreference => {
            RuntimeBindingState::Ready
        }
    };
    ResolvedRuntime {
        summary: RuntimeBindingSummary {
            source: attribution.binding_source,
            state,
            profile_id: attribution.runtime_profile_id,
            runtime_class: attribution.runtime_class,
            display_name: "Test runtime".to_owned(),
        },
        endpoint: None,
    }
}

async fn preferred_profile_id(db: &Database) -> Result<Option<String>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT preferred_profile_id FROM runtime_policy WHERE singleton = 1",
            (),
        )
        .await?;
    match rows.next().await? {
        Some(row) => row.get(0),
        None => Ok(None),
    }
}

async fn project_binding(db: &Database, project_id: &str) -> Result<Option<String>, turso::Error> {
    Ok(project_binding_if_present(db, project_id).await?.flatten())
}

async fn project_binding_if_present(
    db: &Database,
    project_id: &str,
) -> Result<Option<Option<String>>, turso::Error> {
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
            params![project_id.to_owned()],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

async fn load_profile(db: &Database, id: &str) -> Result<Option<ProfileContext>, turso::Error> {
    let conn = db.connect()?;
    load_profile_from_connection(&conn, id).await
}

async fn load_profile_from_connection(
    conn: &Connection,
    id: &str,
) -> Result<Option<ProfileContext>, turso::Error> {
    let mut rows = conn
        .query(
            "SELECT id, provider_id, provider_runtime_key, display_name, runtime_class,
                    ownership_state, connection_state, availability_state, observation_revision
             FROM runtime_profiles WHERE id = ?1 LIMIT 1",
            params![id.to_owned()],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(Some(profile_context_from_row(&row)?)),
        None => Ok(None),
    }
}

async fn load_project_bindings(
    db: &Database,
    project_ids: &[String],
) -> Result<HashMap<String, Option<String>>, turso::Error> {
    if project_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let conn = db.connect()?;
    let placeholders = std::iter::repeat_n("?", project_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT id, runtime_profile_id FROM projects WHERE id IN ({placeholders})");
    let params = project_ids
        .iter()
        .cloned()
        .map(turso::Value::Text)
        .collect::<Vec<_>>();
    let mut rows = conn.query(&sql, params).await?;
    let mut bindings = HashMap::with_capacity(project_ids.len());
    while let Some(row) = rows.next().await? {
        bindings.insert(row.get(0)?, row.get(1)?);
    }
    Ok(bindings)
}

async fn load_profiles(
    db: &Database,
    profile_ids: &HashSet<String>,
) -> Result<HashMap<String, ProfileContext>, turso::Error> {
    if profile_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let conn = db.connect()?;
    let ids = profile_ids.iter().collect::<Vec<_>>();
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id, provider_id, provider_runtime_key, display_name, runtime_class,
                ownership_state, connection_state, availability_state, observation_revision
         FROM runtime_profiles WHERE id IN ({placeholders})"
    );
    let params = ids
        .into_iter()
        .map(|id| turso::Value::Text(id.clone()))
        .collect::<Vec<_>>();
    let mut rows = conn.query(&sql, params).await?;
    let mut profiles = HashMap::with_capacity(profile_ids.len());
    while let Some(row) = rows.next().await? {
        let profile = profile_context_from_row(&row)?;
        profiles.insert(profile.id.clone(), profile);
    }
    Ok(profiles)
}

fn profile_context_from_row(row: &turso::Row) -> Result<ProfileContext, turso::Error> {
    Ok(ProfileContext {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        provider_runtime_key: row.get(2)?,
        display_name: row.get(3)?,
        runtime_class: row.get(4)?,
        ownership_state: row.get(5)?,
        connection_state: row.get(6)?,
        availability_state: row.get(7)?,
        observation_revision: row.get(8)?,
    })
}

#[cfg(test)]
mod tests;
