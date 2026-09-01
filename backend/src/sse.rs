use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

/// One broadcast channel per job that currently has at least one dashboard
/// subscriber. Channels are created on first subscribe and dropped when the last
/// receiver goes away, so an idle server holds no per-job state.
#[derive(Clone, Default)]
pub struct SseBroadcaster {
    channels: Arc<Mutex<HashMap<Uuid, broadcast::Sender<String>>>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, job_id: Uuid) -> broadcast::Receiver<String> {
        let mut channels = self.channels.lock().expect("sse channel map poisoned");
        let sender = channels
            .entry(job_id)
            .or_insert_with(|| broadcast::channel(64).0);
        sender.subscribe()
    }

    /// Push a serialized stats payload to everyone watching `job_id`. A send with
    /// no receivers is not an error — nobody has the dashboard open.
    pub fn publish(&self, job_id: Uuid, payload: String) {
        let mut channels = self.channels.lock().expect("sse channel map poisoned");
        if let Some(sender) = channels.get(&job_id) {
            if sender.send(payload).is_err() {
                channels.remove(&job_id);
            }
        }
    }
}
