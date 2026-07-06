//! In-memory registry of running watch sessions: a cancellation token and a
//! live event broadcast per session. Daemon-process-scoped, mirrors
//! `jobs::registry::JobRegistry`.

use dashmap::DashMap;
use serde::Serialize;
use susun::WatchCancellationToken;
use tokio::sync::broadcast;

/// One entry in a watch session's live event stream (SSE payload shape and
/// the persisted `watch_events.payload_json` shape are the same).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatchStreamEvent {
    /// A file changed under a watched path.
    FileEvent { kind: String, path: String },
    /// The configured action started running for the event above.
    ActionStarted { action: String },
    /// The configured action finished successfully.
    ActionSucceeded { action: String },
    /// The configured action failed.
    ActionFailed { action: String, error: String },
    /// The native watcher itself failed; the session is no longer running.
    SessionFailed { error: String },
}

struct WatchHandle {
    cancellation: WatchCancellationToken,
    sender: broadcast::Sender<WatchStreamEvent>,
}

#[derive(Default)]
pub struct WatchRegistry {
    sessions: DashMap<String, WatchHandle>,
}

impl WatchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an already-started session's cancellation token and
    /// returns its event broadcast sender.
    pub fn register(
        &self,
        watch_id: String,
        cancellation: WatchCancellationToken,
    ) -> broadcast::Sender<WatchStreamEvent> {
        let (sender, _receiver) = broadcast::channel(256);
        self.sessions.insert(
            watch_id,
            WatchHandle {
                cancellation,
                sender: sender.clone(),
            },
        );
        sender
    }

    /// Requests cancellation. Returns whether the session was found.
    pub fn stop(&self, watch_id: &str) -> bool {
        match self.sessions.get(watch_id) {
            Some(handle) => {
                handle.cancellation.cancel();
                true
            }
            None => false,
        }
    }

    /// Subscribes to a running session's live event stream.
    pub fn subscribe(&self, watch_id: &str) -> Option<broadcast::Receiver<WatchStreamEvent>> {
        self.sessions
            .get(watch_id)
            .map(|handle| handle.sender.subscribe())
    }

    /// Removes a finished session from the registry.
    pub fn unregister(&self, watch_id: &str) {
        self.sessions.remove(watch_id);
    }
}
