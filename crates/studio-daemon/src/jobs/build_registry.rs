use std::sync::Arc;

use dashmap::DashMap;
use susun::BuildCancellationToken;
use tokio::sync::Notify;

struct BuildJobHandle {
    cancellation: BuildCancellationToken,
    cancel_notify: Arc<Notify>,
}

/// In-memory registry of running image-build jobs — a cancellation token and
/// a hard-cancel notifier per job id, daemon-process-scoped like
/// [`super::registry::JobRegistry`]. Kept as a separate, smaller type rather
/// than generalizing `JobRegistry` over its event type: build progress is
/// read back from the persisted, bounded `build_job_progress` table (see
/// `susun_integration::persist_build_progress`) instead of pushed live over
/// SSE, so there is no `BuildEvent` broadcast channel to carry here the way
/// `JobRegistry` carries one for `RuntimeEvent` — `BuildxProcessBuildEngine`
/// blocks on the whole `docker buildx build` process and only hands back
/// events in one batch when it exits, so a live push channel would sit idle
/// for the entire build and then dump everything at once anyway; polling the
/// job's persisted progress is both simpler and equally truthful here.
#[derive(Default)]
pub struct BuildJobRegistry {
    jobs: DashMap<String, BuildJobHandle>,
}

impl BuildJobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a build job and returns its cancellation token and a
    /// hard-cancel notifier the runner races execution against.
    pub fn register(&self, job_id: String) -> (BuildCancellationToken, Arc<Notify>) {
        let cancellation = BuildCancellationToken::new();
        let cancel_notify = Arc::new(Notify::new());
        self.jobs.insert(
            job_id,
            BuildJobHandle {
                cancellation: cancellation.clone(),
                cancel_notify: cancel_notify.clone(),
            },
        );
        (cancellation, cancel_notify)
    }

    /// Requests cancellation. Flips the cooperative token (checked by
    /// `BuildxProcessBuildEngine` before starting the build process, and
    /// again immediately after — see its own docs for why this cannot stop
    /// an already-running `docker buildx build` subprocess) and fires the
    /// hard-cancel notifier, which drops Studio's own await on the build
    /// future — the same "we stop waiting; the underlying operation may
    /// keep running detached" semantics `JobRegistry::cancel` already has
    /// for up/down jobs. Returns whether the job was found.
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

    /// Removes a finished build job from the registry.
    pub fn unregister(&self, job_id: &str) {
        self.jobs.remove(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_flips_the_token_and_wakes_a_registered_job_waiting_on_the_notifier()
    -> Result<(), Box<dyn std::error::Error>> {
        let registry = BuildJobRegistry::new();
        let (cancellation, cancel_notify) = registry.register("job-1".to_owned());
        assert!(!cancellation.is_cancelled());

        assert!(registry.cancel("job-1"));
        assert!(cancellation.is_cancelled());

        // A runner racing `cancel_notify.notified()` (as `run_image_build`
        // does) must observe the notification `cancel` just fired, not hang
        // waiting for a future one — `Notify::notify_one` stores a permit
        // when called before anyone is waiting.
        tokio::time::timeout(std::time::Duration::from_secs(1), cancel_notify.notified()).await?;
        Ok(())
    }

    #[test]
    fn cancel_reports_false_for_an_unknown_job_id() {
        let registry = BuildJobRegistry::new();
        assert!(!registry.cancel("does-not-exist"));
    }

    #[test]
    fn unregister_removes_the_job_so_a_later_cancel_reports_false() {
        let registry = BuildJobRegistry::new();
        let (_cancellation, _cancel_notify) = registry.register("job-1".to_owned());
        registry.unregister("job-1");
        assert!(!registry.cancel("job-1"));
    }
}
