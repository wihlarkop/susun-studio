//! Shared security envelope for destructive data operations.
//!
//! This is the single, opaque, owner-bound, single-use, expiring plan store that
//! gates every destructive Runtime Data 3 action (migration commit/rollback,
//! runtime reset/remove/repair, engine prune, metadata restore). It mirrors the
//! ephemeral [`crate::runtime::trusted_plans::TrustedPlanStore`] used for trusted
//! command execution, but carries a *domain-agnostic* payload: the executable
//! target list is captured here at prepare time and revealed only to the matching
//! domain's commit path.
//!
//! Deliberate boundaries:
//! - It holds **no domain execution logic**. Each domain (migration, destructive,
//!   prune, restore) keeps its own prepare/commit code and simply asks this store
//!   to mint, claim, or cancel a plan.
//! - Plans live only in memory, so a daemon restart invalidates every outstanding
//!   plan ID (no forged/replayed ID survives a restart).
//! - A claimed plan is spent: replay, wrong owner, expiry, and post-consumption
//!   reuse are all rejected. The frontend never supplies the executable target
//!   list at commit — it supplies only the opaque `plan_id`.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::backup::RestoreManifestSummary;

/// How long a prepared destructive plan stays valid before it must be re-previewed.
const PLAN_TTL: Duration = Duration::from_secs(5 * 60);

/// The stable set of destructive actions gated by this envelope. `as_str` matches
/// the `runtime_action_audit.action_kind` vocabulary; `domain` matches
/// `runtime_action_audit.domain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    MigrationCommit,
    MigrationRollback,
    DestructiveRepair,
    DestructiveResetEngineData,
    DestructiveRemoveBuiltInRuntime,
    EnginePrune,
    MetadataRestore,
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MigrationCommit => "migration_commit",
            Self::MigrationRollback => "migration_rollback",
            Self::DestructiveRepair => "destructive_repair",
            Self::DestructiveResetEngineData => "destructive_reset_engine_data",
            Self::DestructiveRemoveBuiltInRuntime => "destructive_remove_built_in_runtime",
            Self::EnginePrune => "engine_prune",
            Self::MetadataRestore => "metadata_restore",
        }
    }

    pub fn domain(self) -> &'static str {
        match self {
            Self::MigrationCommit | Self::MigrationRollback => "migration",
            Self::DestructiveRepair
            | Self::DestructiveResetEngineData
            | Self::DestructiveRemoveBuiltInRuntime => "destructive",
            Self::EnginePrune => "prune",
            Self::MetadataRestore => "restore",
        }
    }
}

/// Server-held executable details for a prepared plan. Plain data only — the
/// store never interprets these; the owning domain does at commit time.
#[derive(Debug, Clone)]
pub enum ActionPlanPayload {
    MigrationCommit(MigrationCommitPlan),
    MigrationRollback(MigrationRollbackPlan),
    Destructive(DestructivePlan),
    EnginePrune(EnginePrunePlan),
    MetadataRestore(MetadataRestorePlan),
}

/// The resolved, server-owned project binding move. Captured at preview so the
/// frontend cannot substitute a different target list at commit.
#[derive(Debug, Clone)]
pub struct MigrationCommitPlan {
    pub source_profile_id: String,
    pub target_profile_id: String,
    pub project_ids: Vec<String>,
    /// Fingerprint of the source bindings + target selectability at preview time.
    /// Recomputed at commit; a mismatch means the inventory changed → stale.
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct MigrationRollbackPlan {
    pub migration_id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct DestructivePlan {
    pub operation_id: String,
    pub profile_id: String,
    /// Serialized destructive action kind ("repair" | "reset_engine_data" |
    /// "remove_built_in_runtime").
    pub action: String,
    /// Ownership/inventory fingerprint captured at preview; recomputed at commit
    /// to reject a runtime whose ownership or binding count changed.
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct EnginePrunePlan {
    /// Server-validated at preview time (`resolve_and_validate_engine`),
    /// never a raw, unchecked path segment. Commit trusts this stored value
    /// and never re-reads an id from a request.
    pub engine_id: String,
    /// Exact runtime profile selected at preview. `None` means the platform
    /// default was selected because no profile existed.
    pub runtime_profile_id: Option<String>,
    /// Prune *policy* (which resource classes) — never a resource target list.
    /// The server derives the exact resources to remove at commit.
    pub scopes: Vec<String>,
    pub all_images: bool,
    /// Fingerprint of the engine identity the preview ran against: selected
    /// runtime profile id/class/endpoint/state/observation revision + engine API
    /// version. Recomputed at commit; a mismatch means selection/endpoint/provider
    /// state changed and the plan must be rejected (no silent default fallback).
    pub identity_fingerprint: String,
    /// Fingerprint of the server-derived cleanup inventory (per-scope support,
    /// candidate counts, reclaimable bytes). Recomputed at commit to reject stale
    /// inventory.
    pub inventory_fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct MetadataRestorePlan {
    /// The validated archive's database sha256. Prepare rejects any archive whose
    /// bytes do not hash to this identity, so no replacement archive can be swapped
    /// in between preview and prepare.
    pub archive_sha256: String,
    pub manifest: RestoreManifestSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// A successfully claimed plan, ready for its domain to revalidate and execute.
/// The domain matches on `payload` to recover its typed inputs.
pub struct ClaimedActionPlan {
    pub plan_id: String,
    pub payload: ActionPlanPayload,
}

/// The opaque handle returned to the frontend. Carries no executable detail.
#[derive(Debug, Clone)]
pub struct ActionPlanTicket {
    pub plan_id: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ActionPlanError {
    #[error("destructive plan was not found")]
    NotFound,
    #[error("destructive plan belongs to another session")]
    WrongOwner,
    #[error("destructive plan expired")]
    Expired,
    #[error("destructive plan was already consumed")]
    AlreadyConsumed,
    #[error("destructive plan is for a different action")]
    KindMismatch,
}

impl ActionPlanError {
    /// Short, secret-free code for audit rows.
    pub fn audit_code(&self) -> &'static str {
        match self {
            Self::NotFound => "plan_not_found",
            Self::WrongOwner => "wrong_owner",
            Self::Expired => "plan_expired",
            Self::AlreadyConsumed => "plan_replayed",
            Self::KindMismatch => "plan_kind_mismatch",
        }
    }
}

struct StoredPlan {
    owner: String,
    kind: ActionKind,
    payload: ActionPlanPayload,
    expires_at: Instant,
    state: PlanState,
}

pub struct ActionPlanStore {
    plans: Mutex<HashMap<String, StoredPlan>>,
    ttl: Duration,
}

impl Default for ActionPlanStore {
    fn default() -> Self {
        Self {
            plans: Mutex::new(HashMap::new()),
            ttl: PLAN_TTL,
        }
    }
}

impl ActionPlanStore {
    /// Mint an opaque, owner-bound, single-use, expiring plan and return only its
    /// handle. Also sweeps expired/terminal plans so the map stays bounded.
    pub fn prepare(
        &self,
        owner: &str,
        kind: ActionKind,
        payload: ActionPlanPayload,
    ) -> ActionPlanTicket {
        let plan_id = format!("rap_{}", uuid::Uuid::new_v4().simple());
        let mut plans = self.plans.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        plans.retain(|_, plan| {
            matches!(plan.state, PlanState::Running)
                || (matches!(plan.state, PlanState::Pending) && plan.expires_at > now)
        });
        plans.insert(
            plan_id.clone(),
            StoredPlan {
                owner: owner.to_owned(),
                kind,
                payload,
                expires_at: now + self.ttl,
                state: PlanState::Pending,
            },
        );
        ActionPlanTicket {
            plan_id,
            expires_in_seconds: self.ttl.as_secs(),
        }
    }

    /// Consume a plan exactly once, but only if its action is one of `allowed`.
    ///
    /// This is the atomic domain gate: owner binding, expiry, single-use, and the
    /// allowed-kind check all happen under one lock, and a **kind mismatch never
    /// consumes the plan** — it stays `Pending` for its rightful commit endpoint.
    /// A commit endpoint must pass exactly the kinds it is allowed to execute, so
    /// a prune/restore/migration plan can never be spent through the destructive
    /// endpoint (or vice-versa).
    pub fn claim(
        &self,
        plan_id: &str,
        owner: &str,
        allowed: &[ActionKind],
    ) -> Result<ClaimedActionPlan, ActionPlanError> {
        let mut plans = self.plans.lock().unwrap_or_else(|error| error.into_inner());
        let plan = plans.get_mut(plan_id).ok_or(ActionPlanError::NotFound)?;
        if plan.owner != owner {
            return Err(ActionPlanError::WrongOwner);
        }
        if Instant::now() >= plan.expires_at {
            plan.state = PlanState::Cancelled;
            return Err(ActionPlanError::Expired);
        }
        if !matches!(plan.state, PlanState::Pending) {
            return Err(ActionPlanError::AlreadyConsumed);
        }
        // Reject before consuming: an unrelated plan is left untouched.
        if !allowed.contains(&plan.kind) {
            return Err(ActionPlanError::KindMismatch);
        }
        plan.state = PlanState::Running;
        Ok(ClaimedActionPlan {
            plan_id: plan_id.to_owned(),
            payload: plan.payload.clone(),
        })
    }

    /// Record the terminal outcome of a claimed plan (kept briefly so a swept map
    /// still reflects the last state; the entry is removed on the next prepare).
    pub fn finish(&self, plan_id: &str, state: PlanState) {
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
    use std::{sync::Arc, thread};

    use super::*;

    fn payload() -> ActionPlanPayload {
        ActionPlanPayload::MigrationCommit(MigrationCommitPlan {
            source_profile_id: "src".to_owned(),
            target_profile_id: "dst".to_owned(),
            project_ids: vec!["p1".to_owned()],
            fingerprint: "fp".to_owned(),
        })
    }

    fn prepare(store: &ActionPlanStore) -> ActionPlanTicket {
        store.prepare("owner-a", ActionKind::MigrationCommit, payload())
    }

    const MIGRATION: &[ActionKind] = &[ActionKind::MigrationCommit];

    #[test]
    fn concurrent_claim_allows_exactly_one_runner() {
        let store = Arc::new(ActionPlanStore::default());
        let ticket = prepare(&store);
        let handles = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let id = ticket.plan_id.clone();
                thread::spawn(move || store.claim(&id, "owner-a", MIGRATION).is_ok())
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
    fn wrong_owner_and_replay_are_rejected() {
        let store = ActionPlanStore::default();
        let ticket = prepare(&store);
        assert_eq!(
            store.claim(&ticket.plan_id, "owner-b", MIGRATION).err(),
            Some(ActionPlanError::WrongOwner)
        );
        assert!(store.claim(&ticket.plan_id, "owner-a", MIGRATION).is_ok());
        assert_eq!(
            store.claim(&ticket.plan_id, "owner-a", MIGRATION).err(),
            Some(ActionPlanError::AlreadyConsumed)
        );
    }

    #[test]
    fn expired_and_post_restart_ids_are_rejected() {
        let store = ActionPlanStore::with_ttl(Duration::ZERO);
        let ticket = prepare(&store);
        assert_eq!(
            store.claim(&ticket.plan_id, "owner-a", MIGRATION).err(),
            Some(ActionPlanError::Expired)
        );
        // A fresh store models a daemon restart: the old ID no longer exists.
        let restarted = ActionPlanStore::default();
        assert_eq!(
            restarted.claim(&ticket.plan_id, "owner-a", MIGRATION).err(),
            Some(ActionPlanError::NotFound)
        );
    }

    #[test]
    fn cross_domain_claim_is_rejected_without_consuming_the_plan() {
        let store = ActionPlanStore::default();
        let ticket = prepare(&store); // a MigrationCommit plan

        // Attempt to spend it through the destructive/prune/restore domains.
        for wrong in [
            &[ActionKind::EnginePrune][..],
            &[ActionKind::MetadataRestore][..],
            &[
                ActionKind::DestructiveRepair,
                ActionKind::DestructiveResetEngineData,
                ActionKind::DestructiveRemoveBuiltInRuntime,
            ][..],
        ] {
            assert_eq!(
                store.claim(&ticket.plan_id, "owner-a", wrong).err(),
                Some(ActionPlanError::KindMismatch)
            );
        }

        // The plan was never consumed: its rightful domain can still claim it.
        assert!(store.claim(&ticket.plan_id, "owner-a", MIGRATION).is_ok());
    }
}
