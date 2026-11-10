//! A runtime watchdog: notices when the async runtime has stopped running
//! tasks, says so, and ends the process.
//!
//! On 2026-11-10 production's only backend task stopped answering at 10:18
//! UTC with its CPU idle and its memory flat, and logged nothing at all. Even
//! `/health`, which touches no lock, pool or database, timed out, so every
//! runtime worker was stuck waiting on something. The load balancer's health
//! check took about thirty seconds to notice. ECS then waited out the ten-second
//! drain and the 120-second stop timeout (a stuck process does not act on
//! SIGTERM) before it would start a replacement, and the site answered 503 for
//! eight minutes. Nothing recorded why.
//!
//! This watchdog shortens that and leaves a record. A task on the runtime
//! stamps a heartbeat every second; a plain OS thread, which no stuck runtime
//! can hold up, checks it. Once the heartbeat is older than the stall limit,
//! the thread logs what every thread of the process is doing (from `/proc`)
//! and exits the process. ECS sees the essential container exit and starts a
//! replacement at once, without the health checks, drain and stop timeout
//! in between.
//!
//! The limit (`STALL_LIMIT`) sits below the thirty-odd seconds the load
//! balancer takes to give up on a target, so this acts first, and far above
//! anything a working runtime does: the heartbeat is one timer tick, and every
//! slow thing the server does (argon2, MAGPIE runs, fits) runs on a blocking
//! thread or a child process, not on a runtime worker.
//!
//! A process that was suspended as a whole (a debugger, `SIGSTOP`, a laptop
//! sleeping) stops the watchdog thread too. When it resumes, the watchdog's own
//! sleep has overshot by about as much as the heartbeat is stale, and that is
//! read as a suspension and forgiven, not as a stall.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the runtime may go without running the heartbeat task before the
/// process is ended. See the module comment for why twenty seconds.
pub const STALL_LIMIT: Duration = Duration::from_secs(20);

/// How often the heartbeat task runs.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

/// What a stall looked like, for the report and for tests.
#[derive(Debug, Clone)]
pub struct Stall {
    /// How long it had been since the runtime last ran the heartbeat task.
    pub since_heartbeat: Duration,
}

/// Starts the watchdog on the current runtime: the heartbeat as a task on it,
/// the check on a thread of its own. `on_stall` is called once, on the
/// watchdog thread, when the heartbeat is older than `limit`; the thread then
/// ends.
///
/// Must be called from inside a Tokio runtime.
pub fn start<F>(limit: Duration, on_stall: F)
where
    F: FnOnce(Stall) + Send + 'static,
{
    let started = Instant::now();
    let beat = Arc::new(AtomicU64::new(0));

    let heartbeat = beat.clone();
    let interval = HEARTBEAT_INTERVAL.min(limit / 4);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            heartbeat.store(millis_since(started), Ordering::Relaxed);
        }
    });

    let check = interval;
    let spawned = std::thread::Builder::new()
        .name("runtime-watchdog".into())
        .spawn(move || {
            let mut last_check = Instant::now();
            loop {
                std::thread::sleep(check);
                let now = Instant::now();
                // This thread overslept by about as long as the heartbeat is
                // stale: the whole process was stopped, not the runtime.
                // Forgive it and start counting again from here.
                let overslept = now.duration_since(last_check).saturating_sub(check);
                last_check = now;
                if overslept > limit / 2 {
                    beat.store(millis_since(started), Ordering::Relaxed);
                    continue;
                }
                let since = Duration::from_millis(
                    millis_since(started).saturating_sub(beat.load(Ordering::Relaxed)),
                );
                if since > limit {
                    on_stall(Stall { since_heartbeat: since });
                    return;
                }
            }
        });
    if let Err(err) = spawned {
        tracing::error!(%err, "could not start the runtime watchdog; a stalled runtime will go unnoticed");
    }
}

/// What production does on a stall: log it, with what every thread is doing,
/// and exit so the container is replaced.
pub fn exit_on_stall(stall: Stall) {
    tracing::error!(
        since_heartbeat_ms = stall.since_heartbeat.as_millis() as u64,
        limit_ms = STALL_LIMIT.as_millis() as u64,
        threads = %thread_report(),
        "the async runtime has run no task for longer than the stall limit; \
         exiting so the container is replaced"
    );
    // Nothing more can be done in-process: the runtime that would run a
    // graceful shutdown is the thing that is stuck.
    std::process::exit(70);
}

fn millis_since(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

/// One line per thread of this process: its id, name, scheduler state, and
/// the kernel function it is waiting in. A thread blocked on a lock shows
/// `futex_wait_queue`, one waiting for I/O shows `do_epoll_wait` or a read,
/// and one spinning shows `R` (running). Empty where `/proc` is not there.
pub fn thread_report() -> String {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return String::new();
    };
    let mut lines: Vec<String> = tasks
        .filter_map(Result::ok)
        .map(|task| {
            let path = task.path();
            let read = |file: &str| {
                std::fs::read_to_string(path.join(file))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "?".into())
            };
            let stat = read("stat");
            // The state is the first field after the parenthesised name,
            // which may itself contain spaces or parentheses.
            let state = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .unwrap_or("?")
                .to_string();
            format!(
                "{} {} {} {}",
                task.file_name().to_string_lossy(),
                read("comm"),
                state,
                read("wchan")
            )
        })
        .collect();
    lines.sort();
    lines.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// A runtime whose only worker is blocked is reported, and soon after the
    /// limit -- not before.
    #[test]
    fn a_blocked_runtime_is_reported() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let limit = Duration::from_millis(400);
        runtime.block_on(async {
            start(limit, move |stall| {
                let _ = sender.send(stall);
            });
            // Let the heartbeat run once, then hold the only worker.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let blocked = Instant::now();
            std::thread::sleep(Duration::from_millis(1500));
            let stall = receiver
                .try_recv()
                .expect("the watchdog did not report a runtime blocked for 1.5 s");
            assert!(stall.since_heartbeat > limit, "{stall:?}");
            assert!(blocked.elapsed() >= limit);
        });
    }

    /// A runtime that is idle, but free to run tasks, is never reported.
    #[test]
    fn an_idle_runtime_is_not_reported() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        runtime.block_on(async {
            start(Duration::from_millis(400), move |stall| {
                let _ = sender.send(stall);
            });
            tokio::time::sleep(Duration::from_millis(1500)).await;
        });
        assert!(receiver.try_recv().is_err(), "an idle runtime was reported as stalled");
    }

    /// The report names this thread, which is not running anything else.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_thread_report_lists_threads() {
        let report = thread_report();
        assert!(!report.is_empty());
        // SAFETY: gettid takes no arguments and cannot fail.
        let me = unsafe { libc::gettid() };
        assert!(
            report.split("; ").any(|line| line.starts_with(&format!("{me} "))),
            "{report}"
        );
    }
}
