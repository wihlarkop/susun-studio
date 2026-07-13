use std::{path::PathBuf, sync::Arc};

use turso::Database;

use crate::{
    jobs::{registry::JobRegistry, tickets::StreamTickets},
    restore::RestoreCoordinator,
    watch::registry::WatchRegistry,
};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub db_path: PathBuf,
    pub auth_token: Arc<str>,
    pub jobs: Arc<JobRegistry>,
    pub stream_tickets: Arc<StreamTickets>,
    pub watch: Arc<WatchRegistry>,
    pub restore: Arc<RestoreCoordinator>,
    pub trusted_plans: Arc<crate::runtime::trusted_plans::TrustedPlanStore>,
    /// Shared security envelope for destructive data operations (migration,
    /// reset/remove/repair, engine prune, metadata restore).
    pub action_plans: Arc<crate::action_plans::ActionPlanStore>,
}
