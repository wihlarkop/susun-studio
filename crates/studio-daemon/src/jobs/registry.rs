use std::sync::Arc;

use dashmap::DashMap;
use susun::{CancellationToken, RuntimeEvent};
use tokio::sync::{Notify, broadcast};

struct JobHandle {
    cancellation: CancellationToken,
    sender: broadcast::Sender<RuntimeEvent>,
    cancel_notify: Arc<Notify>,
}

/// In-memory registry of running jobs: a cancellation token, a live event
/// broadcast, and a hard-cancel notifier per job ID. Daemon-process-scoped.
#[derive(Default)]
pub struct JobRegistry {
    jobs: DashMap<String, JobHandle>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a job and returns its cancellation token, event sender, and a
    /// hard-cancel notifier the runner races the execution against.
    pub fn register(
        &self,
        job_id: String,
    ) -> (
        CancellationToken,
        broadcast::Sender<RuntimeEvent>,
        Arc<Notify>,
    ) {
        let cancellation = CancellationToken::new();
        let (sender, _receiver) = broadcast::channel(256);
        let cancel_notify = Arc::new(Notify::new());
        self.jobs.insert(
            job_id,
            JobHandle {
                cancellation: cancellation.clone(),
                sender: sender.clone(),
                cancel_notify: cancel_notify.clone(),
            },
        );
        (cancellation, sender, cancel_notify)
    }

    /// Requests cancellation. Flips the cooperative token (clean stop between
    /// actions) and fires the notifier (drops an in-flight action immediately).
    /// Returns whether the job was found.
    pub fn cancel(&self, job_id: &str) -> bool {
        match self.jobs.get(job_id) {
            Some(handle) => {
                handle.cancellation.cancel();
                handle.cancel_notify.notify_one();
                true
            }
            None => false,
        }
    }

    /// Subscribes to a running job's live event stream.
    pub fn subscribe(&self, job_id: &str) -> Option<broadcast::Receiver<RuntimeEvent>> {
        self.jobs
            .get(job_id)
            .map(|handle| handle.sender.subscribe())
    }

    /// Removes a finished job from the registry.
    pub fn unregister(&self, job_id: &str) {
        self.jobs.remove(job_id);
    }
}
