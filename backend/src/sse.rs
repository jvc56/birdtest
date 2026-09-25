use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

/// One broadcast channel per job that currently has at least one dashboard
/// subscriber. Channels are created on first subscribe and dropped once nobody
/// is listening, so an idle server holds no per-job state.
#[derive(Clone, Default)]
pub struct SseBroadcaster {
    /// Payloads are shared, not copied: a receiver clones what it receives,
    /// and a `String` was a whole payload per subscriber per push.
    channels: Arc<Mutex<HashMap<Uuid, broadcast::Sender<Arc<str>>>>>,
    /// Jobs with a stats push in flight, and whether a further one has been
    /// asked for while it ran. Building the payload is several aggregates over
    /// a job's history, so it runs on a spawned task rather than on the
    /// submission that triggered it -- and this is what stops a busy job
    /// spawning one of those per submission, all reading the same rows and
    /// racing each other to publish out of order.
    pushes: Arc<Mutex<HashMap<Uuid, bool>>>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, job_id: Uuid) -> broadcast::Receiver<Arc<str>> {
        let mut channels = self.channels.lock().expect("sse channel map poisoned");
        let sender = channels
            .entry(job_id)
            .or_insert_with(|| broadcast::channel(64).0);
        sender.subscribe()
    }

    /// Ends every open stream of `job_id`: its channel's sender is dropped,
    /// so each subscriber's stream finishes. For a deleted job, whose open
    /// pages otherwise stayed "live" on keep-alives for as long as they were
    /// open; reconnecting, they are answered 404 and stop.
    pub fn close(&self, job_id: Uuid) {
        self.channels.lock().expect("sse channel map poisoned").remove(&job_id);
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

    /// Ask for a stats push, and say whether the caller is the one to do it.
    ///
    /// `true` means no push is running for this job and the caller owns the
    /// loop; `false` means one is already running and has been told to go
    /// round again, so the caller has nothing to do. Coalescing rather than
    /// spawning per submission keeps a busy job to one in-flight payload plus
    /// one queued, and keeps the pushes ordered, since a single task issues
    /// them.
    pub fn begin_push(&self, job_id: Uuid) -> bool {
        let mut pushes = self.pushes.lock().expect("sse push map poisoned");
        match pushes.get_mut(&job_id) {
            Some(pending) => {
                *pending = true;
                false
            }
            None => {
                pushes.insert(job_id, false);
                true
            }
        }
    }

    /// Finish a push, returning whether another round was asked for meanwhile.
    pub fn end_push(&self, job_id: Uuid) -> bool {
        let mut pushes = self.pushes.lock().expect("sse push map poisoned");
        match pushes.get_mut(&job_id) {
            Some(pending) if *pending => {
                *pending = false;
                true
            }
            _ => {
                pushes.remove(&job_id);
                false
            }
        }
    }

    /// Push a serialized stats payload to everyone watching `job_id`. A send with
    /// no receivers is not an error — nobody has the dashboard open.
    pub fn publish(&self, job_id: Uuid, payload: Arc<str>) {
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

    /// Closing a job's channel ends its open streams: a deleted job's pages
    /// otherwise stayed "live" on keep-alives.
    #[tokio::test]
    async fn closing_a_job_ends_its_streams() {
        let sse = SseBroadcaster::new();
        let job = Uuid::new_v4();
        let mut receiver = sse.subscribe(job);
        sse.close(job);
        assert!(matches!(
            receiver.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ));
        assert!(!sse.has_subscribers(job));
    }

    /// A burst of submissions must not spawn a payload build each: the first
    /// owns the loop, the rest only mark it to go round once more.
    #[test]
    fn pushes_coalesce_into_one_in_flight_and_one_pending() {
        let sse = SseBroadcaster::new();
        let job = Uuid::new_v4();

        assert!(sse.begin_push(job), "the first caller owns the push");
        assert!(!sse.begin_push(job), "a second caller defers to it");
        assert!(!sse.begin_push(job), "and so does a third");

        assert!(sse.end_push(job), "one more round was asked for");
        assert!(!sse.end_push(job), "and nothing was asked for during that one");

        // Back to idle, so the next submission owns the loop again.
        assert!(sse.begin_push(job));
        assert!(!sse.end_push(job));
    }

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
