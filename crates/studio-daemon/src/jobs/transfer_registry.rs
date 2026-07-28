use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Notify;

#[derive(Default)]
pub struct ArtifactTransferJobRegistry {
    jobs: DashMap<String, Arc<Notify>>,
}

impl ArtifactTransferJobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, job_id: String) -> Arc<Notify> {
        let notification = Arc::new(Notify::new());
        self.jobs.insert(job_id, notification.clone());
        notification
    }

    pub fn cancel(&self, job_id: &str) -> bool {
        match self.jobs.get(job_id) {
            Some(notification) => {
                notification.notify_one();
                true
            }
            None => false,
        }
    }

    pub fn finish(&self, job_id: &str) {
        self.jobs.remove(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_wakes_a_registered_transfer_once() -> Result<(), Box<dyn std::error::Error>> {
        let registry = ArtifactTransferJobRegistry::new();
        let notification = registry.register("job-1".to_owned());
        assert!(registry.cancel("job-1"));
        tokio::time::timeout(std::time::Duration::from_secs(1), notification.notified()).await?;
        registry.finish("job-1");
        assert!(!registry.cancel("job-1"));
        Ok(())
    }
}
