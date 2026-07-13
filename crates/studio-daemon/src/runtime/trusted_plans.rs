//! In-memory, owner-bound, single-use trusted runtime plans.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Serialize;

use super::command::{CommandKind, ExecutableCommand, ProcessElevation, SoftwareProvenance};

const PLAN_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustedPlanState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustedPlanPreview {
    pub plan_id: String,
    pub provider_id: String,
    pub action: String,
    pub label: String,
    pub destructive: bool,
    pub consequence: String,
    pub elevation: String,
    pub command_summary: String,
    pub software_provenance: Option<SoftwareProvenance>,
    pub expires_in_seconds: u64,
    pub state: TrustedPlanState,
}

#[derive(Clone)]
struct StoredPlan {
    owner: String,
    provider_id: String,
    action: String,
    command: ExecutableCommand,
    expires_at: Instant,
    state: TrustedPlanState,
}

pub struct ClaimedPlan {
    pub plan_id: String,
    pub provider_id: String,
    pub action: String,
    pub command: ExecutableCommand,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TrustedPlanError {
    #[error("trusted runtime plan was not found")]
    NotFound,
    #[error("trusted runtime plan belongs to another session")]
    WrongOwner,
    #[error("trusted runtime plan expired")]
    Expired,
    #[error("trusted runtime plan was already consumed")]
    AlreadyConsumed,
}

pub struct TrustedPlanStore {
    plans: Mutex<HashMap<String, StoredPlan>>,
    ttl: Duration,
}

pub struct TrustedPlanMetadata<'a> {
    pub provider_id: &'a str,
    pub action: &'a str,
    pub label: &'a str,
    pub destructive: bool,
    pub consequence: &'a str,
}

impl Default for TrustedPlanStore {
    fn default() -> Self {
        Self {
            plans: Mutex::new(HashMap::new()),
            ttl: PLAN_TTL,
        }
    }
}

impl TrustedPlanStore {
    pub fn prepare(
        &self,
        owner: &str,
        metadata: TrustedPlanMetadata<'_>,
        command: ExecutableCommand,
    ) -> TrustedPlanPreview {
        let plan_id = format!("rtp_{}", uuid::Uuid::new_v4().simple());
        let elevation = match command.elevation {
            ProcessElevation::CurrentUser => "current_user",
            ProcessElevation::OneShotOsMediated => "os_mediated_consent",
        };
        let command_summary = match command.kind {
            CommandKind::PackageManager => "Verified package manager operation",
            CommandKind::VendorInstaller => "Verified vendor installer operation",
            CommandKind::OsConfigTool => "Verified operating-system configuration operation",
            CommandKind::RuntimeCli => "Verified runtime lifecycle operation",
        };
        let preview = TrustedPlanPreview {
            plan_id: plan_id.clone(),
            provider_id: metadata.provider_id.to_owned(),
            action: metadata.action.to_owned(),
            label: metadata.label.to_owned(),
            destructive: metadata.destructive,
            consequence: metadata.consequence.to_owned(),
            elevation: elevation.to_owned(),
            command_summary: command_summary.to_owned(),
            software_provenance: command.software_provenance.clone(),
            expires_in_seconds: self.ttl.as_secs(),
            state: TrustedPlanState::Pending,
        };
        let mut plans = self.plans.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        plans.retain(|_, plan| {
            matches!(plan.state, TrustedPlanState::Running)
                || (matches!(plan.state, TrustedPlanState::Pending) && plan.expires_at > now)
        });
        plans.insert(
            plan_id,
            StoredPlan {
                owner: owner.to_owned(),
                provider_id: metadata.provider_id.to_owned(),
                action: metadata.action.to_owned(),
                command,
                expires_at: now + self.ttl,
                state: TrustedPlanState::Pending,
            },
        );
        preview
    }

    pub fn claim(&self, plan_id: &str, owner: &str) -> Result<ClaimedPlan, TrustedPlanError> {
        let mut plans = self.plans.lock().unwrap_or_else(|error| error.into_inner());
        let plan = plans.get_mut(plan_id).ok_or(TrustedPlanError::NotFound)?;
        if plan.owner != owner {
            return Err(TrustedPlanError::WrongOwner);
        }
        if Instant::now() >= plan.expires_at {
            plan.state = TrustedPlanState::Cancelled;
            return Err(TrustedPlanError::Expired);
        }
        if !matches!(plan.state, TrustedPlanState::Pending) {
            return Err(TrustedPlanError::AlreadyConsumed);
        }
        plan.state = TrustedPlanState::Running;
        Ok(ClaimedPlan {
            plan_id: plan_id.to_owned(),
            provider_id: plan.provider_id.clone(),
            action: plan.action.clone(),
            command: plan.command.clone(),
        })
    }

    pub fn cancel(&self, plan_id: &str, owner: &str) -> Result<(), TrustedPlanError> {
        let mut plans = self.plans.lock().unwrap_or_else(|error| error.into_inner());
        let plan = plans.get_mut(plan_id).ok_or(TrustedPlanError::NotFound)?;
        if plan.owner != owner {
            return Err(TrustedPlanError::WrongOwner);
        }
        if !matches!(plan.state, TrustedPlanState::Pending) {
            return Err(TrustedPlanError::AlreadyConsumed);
        }
        plan.state = TrustedPlanState::Cancelled;
        Ok(())
    }

    pub fn finish(&self, plan_id: &str, state: TrustedPlanState) {
        if let Some(plan) = self
            .plans
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(plan_id)
        {
            plan.state = state;
        }
    }

    #[cfg(test)]
    fn with_ttl(ttl: Duration) -> Self {
        Self {
            plans: Mutex::new(HashMap::new()),
            ttl,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, sync::Arc, thread};

    use super::*;
    use crate::runtime::command::{CommandKind, ProcessElevation, TrustedProgram};

    fn command() -> ExecutableCommand {
        ExecutableCommand {
            program: TrustedProgram::Podman,
            args: vec![OsString::from("machine"), OsString::from("start")],
            env_allowlist: Vec::new(),
            working_dir: None,
            timeout: Duration::from_secs(1),
            kind: CommandKind::RuntimeCli,
            elevation: ProcessElevation::CurrentUser,
            software_provenance: None,
            success_message: "started".to_owned(),
        }
    }

    fn prepare(store: &TrustedPlanStore) -> TrustedPlanPreview {
        store.prepare(
            "owner-a",
            TrustedPlanMetadata {
                provider_id: "podman",
                action: "start",
                label: "Start",
                destructive: false,
                consequence: "Starts runtime",
            },
            command(),
        )
    }

    #[test]
    fn concurrent_claim_allows_exactly_one_runner() {
        let store = Arc::new(TrustedPlanStore::default());
        let preview = prepare(&store);
        let handles = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let id = preview.plan_id.clone();
                thread::spawn(move || store.claim(&id, "owner-a").is_ok())
            })
            .collect::<Vec<_>>();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(false))
            .filter(|won| *won)
            .count();
        assert_eq!(winners, 1);
    }

    #[test]
    fn wrong_owner_cancel_and_replay_are_rejected() {
        let store = TrustedPlanStore::default();
        let preview = prepare(&store);
        assert_eq!(
            store.claim(&preview.plan_id, "owner-b").err(),
            Some(TrustedPlanError::WrongOwner)
        );
        assert!(store.cancel(&preview.plan_id, "owner-a").is_ok());
        assert_eq!(
            store.claim(&preview.plan_id, "owner-a").err(),
            Some(TrustedPlanError::AlreadyConsumed)
        );
    }

    #[test]
    fn expired_and_post_restart_ids_are_rejected() {
        let store = TrustedPlanStore::with_ttl(Duration::ZERO);
        let preview = prepare(&store);
        assert_eq!(
            store.claim(&preview.plan_id, "owner-a").err(),
            Some(TrustedPlanError::Expired)
        );
        let restarted = TrustedPlanStore::default();
        assert_eq!(
            restarted.claim(&preview.plan_id, "owner-a").err(),
            Some(TrustedPlanError::NotFound)
        );
    }

    #[test]
    fn preview_never_exposes_executable_arguments_or_environment() {
        let store = TrustedPlanStore::default();
        let preview = prepare(&store);
        let json = serde_json::to_string(&preview).unwrap_or_default();
        assert!(!json.contains("podman.exe"));
        assert!(!json.contains(r#""args""#));
        assert!(!json.contains(r#""env""#));
        assert!(!json.contains("machine"));
        assert!(json.contains("Verified runtime lifecycle operation"));
    }
}
