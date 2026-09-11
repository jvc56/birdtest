use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

/// One broadcast channel per job that currently has at least one dashboard
/// subscriber. Channels are created on first subscribe and dropped once nobody
/// is listening, so an idle server holds no per-job state.
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

    /// Whether anyone is watching `job_id`. The submission path asks before
    /// computing a stats payload: building one costs several aggregates over
    /// the job's results, and paying that on every submission to a job nobody
    /// has open is the most expensive way to do nothing.
    pub fn has_subscribers(&self, job_id: Uuid) -> bool {
        let mut channels = self.channels.lock().expect("sse channel map poisoned");
        match channels.get(&job_id) {
            Some(sender) if sender.receiver_count() > 0 => true,
            Some(_) => {
                channels.remove(&job_id);
                false
            }
            None => false,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribers_are_counted_and_forgotten_when_they_leave() {
        let sse = SseBroadcaster::new();
        let job = Uuid::new_v4();
        assert!(!sse.has_subscribers(job));
        let receiver = sse.subscribe(job);
        assert!(sse.has_subscribers(job));
        drop(receiver);
        assert!(!sse.has_subscribers(job));
    }
}
