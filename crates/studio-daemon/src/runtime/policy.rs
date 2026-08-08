use std::collections::{HashMap, HashSet};

use serde::Serialize;
use susun::EngineEndpoint;
use turso::transaction::Transaction;
use turso::{Connection, Database, params};

use super::{ManagementCapabilities, find_provider, now_ms};

const MAX_PROJECT_SUMMARY_BATCH: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
        let Some(profile) = load_profile(db, profile_id).await? else {
            return Ok(SetPreferredOutcome::NotFound);
        };
        if profile.ownership_state == "ownership_conflict"
            || !profile.is_ready()
            || profile.endpoint().is_none()
        {
            return Ok(SetPreferredOutcome::Unavailable);
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
    let conn = db.connect()?;
    let mut rows = conn
        .query(
            "SELECT runtime_profile_id FROM projects WHERE id = ?1 LIMIT 1",
            params![project_id.to_owned()],
        )
        .await?;
    match rows.next().await? {
        Some(row) => row.get(0),
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
                    ownership_state, connection_state, availability_state
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
                ownership_state, connection_state, availability_state
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
    })
}

#[cfg(test)]
mod tests;
