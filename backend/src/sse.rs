use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use uuid::Uuid;

/// One broadcast channel per job that has had a dashboard subscriber. Channels
/// are created on first subscribe and dropped when a later publish or
/// subscriber check finds nobody listening, so a job nobody has watched since
/// its last subscriber left keeps a small channel until then (bounded by the
/// jobs there are).
#[derive(Clone, Default)]
pub struct SseBroadcaster {
    /// Payloads are shared, not copied: a receiver clones what it receives,
    /// and a `String` was a whole payload per subscriber per push.
    channels: Arc<Mutex<HashMap<Uuid, broadcast::Sender<Arc<str>>>>>,
    /// Jobs with a stats push in flight -- building, or in the cool-down after
    /// a build -- and whether a further one has been asked for meanwhile.
    /// Building the payload is several aggregates over a job's history, so it
    /// runs on a spawned task rather than on the submission that triggered it
    /// -- and this is what stops a busy job spawning one of those per
    /// submission, all reading the same rows and racing each other to publish
    /// out of order.
    pushes: Arc<Mutex<HashMap<Uuid, Push>>>,
}

struct Push {
    pending: bool,
    /// Wakes the loop from its cool-down: an admin's change should reach open
    /// pages now, not after the interval that spaces submissions' pushes.
    urgent: Arc<tokio::sync::Notify>,
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
        self.ask_for_push(job_id, false)
    }

    /// As [`Self::begin_push`], and cuts a running loop's cool-down short.
    pub fn begin_urgent_push(&self, job_id: Uuid) -> bool {
        self.ask_for_push(job_id, true)
    }

    fn ask_for_push(&self, job_id: Uuid, urgent: bool) -> bool {
        let mut pushes = self.pushes.lock().expect("sse push map poisoned");
        match pushes.get_mut(&job_id) {
            Some(push) => {
                push.pending = true;
                if urgent {
                    push.urgent.notify_one();
                }
                false
            }
            None => {
                let push = Push { pending: false, urgent: Arc::new(tokio::sync::Notify::new()) };
                pushes.insert(job_id, push);
                true
            }
        }
    }

    /// What wakes a push loop's cool-down early, while one is in flight.
    pub fn urgent(&self, job_id: Uuid) -> Option<Arc<tokio::sync::Notify>> {
        let pushes = self.pushes.lock().expect("sse push map poisoned");
        pushes.get(&job_id).map(|push| push.urgent.clone())
    }

    /// Forget a push for good: its job is gone.
    pub fn abandon_push(&self, job_id: Uuid) {
        self.pushes.lock().expect("sse push map poisoned").remove(&job_id);
    }

    /// Finish a push, returning whether another round was asked for meanwhile.
    pub fn end_push(&self, job_id: Uuid) -> bool {
        let mut pushes = self.pushes.lock().expect("sse push map poisoned");
        match pushes.get_mut(&job_id) {
            Some(push) if push.pending => {
                push.pending = false;
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

    /// An urgent ask wakes a loop's cool-down; a plain one only marks it.
    #[tokio::test]
    async fn an_urgent_push_cuts_the_cool_down_short() {
        let sse = SseBroadcaster::new();
        let job = Uuid::new_v4();
        assert!(sse.begin_push(job));
        let urgent = sse.urgent(job).expect("in flight");
        assert!(!sse.begin_push(job));
        let slept = tokio::time::timeout(std::time::Duration::from_millis(50), urgent.notified()).await;
        assert!(slept.is_err(), "a submission does not wake it");
        assert!(!sse.begin_urgent_push(job));
        tokio::time::timeout(std::time::Duration::from_secs(1), urgent.notified())
            .await
            .expect("an admin's change does");
        assert!(sse.end_push(job));
        assert!(!sse.end_push(job));
        assert!(sse.urgent(job).is_none(), "idle again");
    }

    /// A push for a deleted job is forgotten even with a round pending, where
    /// returning without it left the entry for good.
    #[test]
    fn an_abandoned_push_leaves_nothing_behind() {
        let sse = SseBroadcaster::new();
        let job = Uuid::new_v4();
        assert!(sse.begin_push(job));
        assert!(!sse.begin_push(job), "a round pending");
        sse.abandon_push(job);
        assert!(sse.urgent(job).is_none());
        assert!(sse.begin_push(job), "the next ask owns a fresh loop");
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
