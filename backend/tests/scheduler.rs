//! The scheduler and capability negotiation, driven through
//! `birdtest::scheduler` and `birdtest::jobs::expected_data` directly: which
//! job a claim goes to, what a claim writes, what a worker that can run
//! nothing is told, and which files an assignment names. States the claim path
//! would not produce on its own -- a task already at redundancy, a claim a
//! second inside its timeout -- are written with plain SQL.

mod common;

use birdtest::auth::WorkerIdentity;
use birdtest::scheduler::{self, ClaimOutcome, ShutdownDirective, TaskClaim, WorkerCapabilities};
use birdtest::state::AppState;
use birdtest::version::Version;
use common::*;
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn caps(version: &str, unsupported: &[Uuid]) -> WorkerCapabilities {
    WorkerCapabilities {
        magpie_version: Version::parse_or_zero(version),
        unsupported_jobs: unsupported.to_vec(),
    }
}

/// An anonymous worker the server has already issued a UUID to.
async fn anon(db: &TestDb) -> WorkerIdentity {
    let uuid = Uuid::new_v4();
    sqlx::query("INSERT INTO anonymous_workers (uuid) VALUES ($1)")
        .bind(uuid)
        .execute(&db.pool)
        .await
        .unwrap();
    WorkerIdentity::Anonymous { uuid }
}

/// A worker authenticated by an API key, i.e. by its account.
async fn registered(db: &TestDb) -> WorkerIdentity {
    let user_id = db.user(&format!("w{}", Uuid::new_v4().simple()), false).await;
    WorkerIdentity::User { user_id, key_id: Uuid::nil() }
}

fn outcome_kind(outcome: &ClaimOutcome) -> &'static str {
    match outcome {
        ClaimOutcome::Task(_) => "task",
        ClaimOutcome::Idle => "idle",
        ClaimOutcome::NoWorkExists => "no_work_exists",
        ClaimOutcome::Shutdown(_) => "shutdown",
    }
}

async fn claim(state: &AppState, identity: &WorkerIdentity, caps: &WorkerCapabilities) -> ClaimOutcome {
    scheduler::claim(state, identity, caps).await.unwrap()
}

async fn claim_task(state: &AppState, identity: &WorkerIdentity, caps: &WorkerCapabilities) -> TaskClaim {
    match claim(state, identity, caps).await {
        ClaimOutcome::Task(task) => *task,
        other => panic!("expected a task, got {}", outcome_kind(&other)),
    }
}

async fn claim_shutdown(
    state: &AppState,
    identity: &WorkerIdentity,
    caps: &WorkerCapabilities,
) -> ShutdownDirective {
    match claim(state, identity, caps).await {
        ClaimOutcome::Shutdown(directive) => directive,
        other => panic!("expected a shutdown, got {}", outcome_kind(&other)),
    }
}

/// The task a claim token was issued for.
async fn task_of(db: &TestDb, claim_token: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT task_id FROM task_claims WHERE claim_token = $1")
        .bind(claim_token)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn claim_id_of(db: &TestDb, claim_token: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM task_claims WHERE claim_token = $1")
        .bind(claim_token)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn claims_issued(db: &TestDb, job: Uuid) -> i64 {
    sqlx::query_scalar("SELECT claims_issued FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn exec(db: &TestDb, sql: &str, id: Uuid) {
    sqlx::query(sql).bind(id).execute(&db.pool).await.unwrap();
}

async fn set_floor(db: &TestDb, job: Uuid, major: i32, minor: i32, patch: i32) {
    sqlx::query(
        "UPDATE jobs SET min_magpie_major = $2, min_magpie_minor = $3, min_magpie_patch = $4
         WHERE id = $1",
    )
    .bind(job)
    .bind(major)
    .bind(minor)
    .bind(patch)
    .execute(&db.pool)
    .await
    .unwrap();
}

async fn set_created(db: &TestDb, job: Uuid, minutes_ago: i32) {
    sqlx::query("UPDATE jobs SET created_at = now() - make_interval(mins => $2) WHERE id = $1")
        .bind(job)
        .bind(minutes_ago)
        .execute(&db.pool)
        .await
        .unwrap();
}

/// Every job but `job`, for an unsupported set that steers a claim to it.
async fn every_job_but(db: &TestDb, job: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM jobs WHERE id <> $1")
        .bind(job)
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

async fn load_job(db: &TestDb, job: Uuid) -> birdtest::models::job::Job {
    sqlx::query_as("SELECT * FROM jobs WHERE id = $1").bind(job).fetch_one(&db.pool).await.unwrap()
}

// ---------------------------------------------------------------------------
// I-SCHED
// ---------------------------------------------------------------------------

/// I-SCHED-1: a claim against one active job returns a task, writes a
/// `task_claims` row naming the worker and the MAGPIE it reported, and counts
/// it on the task -- the second worker filling a redundancy-2 task's other slot
/// takes it to capacity.
#[tokio::test]
async fn a_claim_writes_its_row_and_counts_itself_on_the_task() {
    let db = TestDb::new().await;
    let job = db.games_job(2, 2).await;
    let state = db.state().await;

    let worker = anon(&db).await;
    let task = claim_task(&state, &worker, &caps("1.2.3", &[])).await;
    assert_eq!(task.job_id, job);

    type ClaimRow = (Uuid, String, Option<Uuid>, Option<Uuid>, Option<String>);
    let rows: Vec<ClaimRow> = sqlx::query_as(
        "SELECT claim_token, state::text, claimed_by_user_id, claimed_by_anon_uuid, magpie_version
         FROM task_claims",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![(task.claim_token, "claimed".into(), None, worker.anon_uuid(), Some("1.2.3".into()))],
        "exactly one claim row, for this worker, in state claimed"
    );

    let task_id = task_of(&db, task.claim_token).await;
    let (active, state_text): (i32, String) =
        sqlx::query_as("SELECT active_claim_count, state::text FROM tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((active, state_text.as_str()), (1, "available"), "one of two slots taken");
    assert_eq!(claims_issued(&db, job).await, 1);

    let second = claim_task(&state, &anon(&db).await, &caps("1.2.3", &[])).await;
    assert_eq!(task_of(&db, second.claim_token).await, task_id, "the other slot of the same task");
    let (active, state_text): (i32, String) =
        sqlx::query_as("SELECT active_claim_count, state::text FROM tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((active, state_text.as_str()), (2, "claimed"), "at redundancy the task is claimed");
    assert_eq!(claims_issued(&db, job).await, 2);
}

/// I-SCHED-4: an abandoned claim still counts toward a job's share. Every claim
/// that lands on one of two 50/50 jobs is abandoned (it lapses and the next
/// claim reclaims it); the split stays 10/10. Were abandoned claims left out of
/// the count -- the scheduler's `claims_issued`, TESTING.md's
/// `tasks_dispatched` -- that job's deficit would never move and it would take
/// all twenty.
#[tokio::test]
async fn abandoned_claims_still_count_against_a_jobs_share() {
    let db = TestDb::new().await;
    let flaky = db.games_job(1, 1).await;
    let steady = db.games_job(1, 1).await;
    // The flaky job wins every tie, so it is the one that would run away.
    set_created(&db, flaky, 60).await;
    let state = db.state().await;

    let mut by_job: HashMap<Uuid, usize> = HashMap::new();
    for _ in 0..20 {
        let task = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[])).await;
        *by_job.entry(task.job_id).or_default() += 1;
        if task.job_id == flaky {
            sqlx::query(
                "UPDATE task_claims SET claimed_at = now() - interval '1 hour'
                 WHERE claim_token = $1",
            )
            .bind(task.claim_token)
            .execute(&db.pool)
            .await
            .unwrap();
        }
    }
    assert_eq!(by_job.get(&flaky), Some(&10), "{by_job:?}");
    assert_eq!(by_job.get(&steady), Some(&10), "{by_job:?}");

    // Every flaky claim but the last has been reclaimed by a later claim, and
    // each of them is still in the count.
    let abandoned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE t.job_id = $1 AND c.state = 'abandoned'",
    )
    .bind(flaky)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(abandoned >= 9, "the flaky job's claims were abandoned ({abandoned})");
    assert_eq!(claims_issued(&db, flaky).await, 10);
    assert_eq!(claims_issued(&db, steady).await, 10);
}

/// I-SCHED-5: between jobs equally far behind their share, the older one is
/// served first -- by `created_at`, not by the order the rows were written.
#[tokio::test]
async fn equal_deficits_go_to_the_older_job() {
    let db = TestDb::new().await;
    let newer = db.games_job(1, 1).await;
    let older = db.games_job(1, 1).await;
    set_created(&db, older, 60).await;
    let state = db.state().await;

    let task = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[])).await;
    assert_eq!(task.job_id, older);

    // And the other way round, from level again.
    sqlx::query("UPDATE jobs SET claims_issued = 0, created_at = now() - interval '2 hours' WHERE id = $1")
        .bind(newer)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE jobs SET claims_issued = 0 WHERE id = $1")
        .bind(older)
        .execute(&db.pool)
        .await
        .unwrap();
    let task = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[])).await;
    assert_eq!(task.job_id, newer, "now the older of the two");
}

/// I-SCHED-6: both capability filters are part of candidate selection. A worker
/// that cannot run the job furthest behind its share -- because it listed it
/// as unsupported, or because the job needs a newer MAGPIE -- is offered the
/// next job in deficit order, not the one after it and not a shutdown.
#[tokio::test]
async fn a_worker_that_cannot_run_the_job_furthest_behind_gets_the_next_one() {
    let db = TestDb::new().await;
    let behind = db.games_job(1, 1).await;
    let next = db.games_job(1, 1).await;
    let ahead = db.games_job(1, 1).await;
    for (job, issued) in [(behind, 0i64), (next, 5), (ahead, 10)] {
        sqlx::query("UPDATE jobs SET claims_issued = $2 WHERE id = $1")
            .bind(job)
            .bind(issued)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    // The job furthest ahead is the oldest, so no tie-break explains the answer.
    set_created(&db, ahead, 60).await;
    let state = db.state().await;

    let task = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[behind])).await;
    assert_eq!(task.job_id, next, "unsupported: the next job in deficit order");

    // Back to the same standings, and the version filter instead.
    sqlx::query("UPDATE jobs SET claims_issued = 5 WHERE id = $1")
        .bind(next)
        .execute(&db.pool)
        .await
        .unwrap();
    set_floor(&db, behind, 2, 0, 0).await;
    let task = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[])).await;
    assert_eq!(task.job_id, next, "too new: the next job in deficit order");
}

/// I-SCHED-7: versions compare as numbers. A 1.9.0 worker is offered a job
/// requiring 1.9.0 and never one requiring 1.10.0 -- which as text sorts below
/// "1.9.0" -- even with the 1.10.0 job furthest behind; a 1.10.0 worker takes
/// that job; and with only the 1.10.0 job on offer the 1.9.0 worker is told to
/// upgrade to exactly that.
#[tokio::test]
async fn a_1_9_worker_gets_a_1_9_job_and_never_a_1_10_one() {
    let db = TestDb::new().await;
    let needs_1_10 = db.games_job(1, 1).await;
    let needs_1_9 = db.games_job(1, 1).await;
    set_floor(&db, needs_1_10, 1, 10, 0).await;
    set_floor(&db, needs_1_9, 1, 9, 0).await;
    set_created(&db, needs_1_10, 60).await;
    let state = db.state().await;

    for _ in 0..3 {
        let task = claim_task(&state, &anon(&db).await, &caps("1.9.0", &[])).await;
        assert_eq!(task.job_id, needs_1_9);
        assert_eq!(task.min_magpie_version, "1.9.0");
    }
    let task = claim_task(&state, &anon(&db).await, &caps("1.10.0", &[])).await;
    assert_eq!(task.job_id, needs_1_10, "the job furthest behind, for a worker that can run it");
    assert_eq!(task.min_magpie_version, "1.10.0");

    exec(&db, "UPDATE jobs SET status = 'inactive' WHERE id = $1", needs_1_9).await;
    let directive = claim_shutdown(&state, &anon(&db).await, &caps("1.9.0", &[])).await;
    assert_eq!(directive.reason, "magpie_too_old");
    assert_eq!(directive.required_magpie_version.as_deref(), Some("1.10.0"));
}

/// I-SCHED-8: `Idle` and `NoWorkExists` are different answers. No job, or only
/// an inactive one, is `NoWorkExists`; an active job the worker could run that
/// has nothing to hand out this instant -- its only batch is out and its cap is
/// reached -- is `Idle`.
#[tokio::test]
async fn idle_means_work_exists_and_no_work_exists_means_none_is_offered() {
    let db = TestDb::new().await;
    let state = db.state().await;

    let worker = anon(&db).await;
    assert_eq!(outcome_kind(&claim(&state, &worker, &caps("1.0.0", &[])).await), "no_work_exists");

    let job = db.games_job(1, 2).await;
    exec(&db, "UPDATE jobs SET status = 'inactive' WHERE id = $1", job).await;
    assert_eq!(
        outcome_kind(&claim(&state, &worker, &caps("1.0.0", &[])).await),
        "no_work_exists",
        "an inactive job offers nothing"
    );

    exec(&db, "UPDATE jobs SET status = 'active' WHERE id = $1", job).await;
    exec(&db, "UPDATE job_game_config SET max_games = 2 WHERE job_id = $1", job).await;
    claim_task(&state, &worker, &caps("1.0.0", &[])).await;
    assert_eq!(
        outcome_kind(&claim(&state, &anon(&db).await, &caps("1.0.0", &[])).await),
        "idle",
        "the job is on offer and runnable, with nothing to hand out right now"
    );
}

/// I-SCHED-9: the three shutdown reasons. A worker too old for every job is
/// told the version to get and where; one missing every job's data is told the
/// tarballs; one blocked on both is told `both`, and the message leads with the
/// version -- updating MAGPIE is the step that also brings the data.
#[tokio::test]
async fn each_shutdown_reason_names_what_the_worker_must_change() {
    let db = TestDb::new().await;
    let too_new = db.games_job(1, 1).await;
    let data = db.games_job(1, 1).await;
    set_floor(&db, too_new, 2, 0, 0).await;
    let state = db.state().await;
    let worker = anon(&db).await;

    exec(&db, "UPDATE jobs SET status = 'inactive' WHERE id = $1", data).await;
    let directive = claim_shutdown(&state, &worker, &caps("1.0.0", &[])).await;
    assert_eq!(directive.reason, "magpie_too_old");
    assert_eq!(directive.required_magpie_version.as_deref(), Some("2.0.0"));
    assert_eq!(directive.download_url.as_deref(), Some(state.cfg.magpie_download_url.as_str()));
    assert!(directive.required_tarball_dates.is_empty());
    assert!(directive.message.contains("2.0.0") && directive.message.contains("1.0.0"), "{}", directive.message);

    exec(&db, "UPDATE jobs SET status = 'inactive' WHERE id = $1", too_new).await;
    exec(&db, "UPDATE jobs SET status = 'active' WHERE id = $1", data).await;
    let directive = claim_shutdown(&state, &worker, &caps("1.0.0", &[data])).await;
    assert_eq!(directive.reason, "data_out_of_date");
    assert_eq!(directive.required_magpie_version, None);
    assert_eq!(directive.download_url, None, "no MAGPIE to download");
    assert_eq!(directive.required_tarball_dates, vec!["20251004".to_string()]);

    exec(&db, "UPDATE jobs SET status = 'active' WHERE id = $1", too_new).await;
    let directive = claim_shutdown(&state, &worker, &caps("1.0.0", &[data])).await;
    assert_eq!(directive.reason, "both");
    assert_eq!(directive.required_magpie_version.as_deref(), Some("2.0.0"));
    assert!(directive.download_url.is_some());
    assert_eq!(directive.required_tarball_dates, vec!["20251004".to_string()]);
    let version_at = directive.message.find("MAGPIE 2.0.0").expect(&directive.message);
    let data_at = directive.message.find("input data").expect(&directive.message);
    assert!(version_at < data_at, "the version comes first: {}", directive.message);
}

/// I-SCHED-10: a shutdown directive's version and tarball dates are read from
/// the active jobs that rule the worker out, not written in. The version is the
/// smallest upgrade that unblocks anything, compared as numbers (2.9.5, not
/// 2.10.0, which sorts first as text); the dates are those of the unsupported
/// active jobs' distributions and boards, newest first, and nothing from an
/// inactive or parked job; the download link is the configured one.
#[tokio::test]
async fn a_shutdown_names_what_the_active_jobs_actually_require() {
    let db = TestDb::new().await;
    let v_small = db.games_job(1, 1).await;
    let v_big = db.games_job(1, 1).await;
    set_floor(&db, v_small, 2, 9, 5).await;
    set_floor(&db, v_big, 2, 10, 0).await;
    let d1 = db.games_job(1, 1).await;
    let d2 = db.games_job(1, 1).await;
    let inactive = db.games_job(1, 1).await;
    let parked = db.games_job(1, 1).await;
    exec(&db, "UPDATE jobs SET status = 'inactive' WHERE id = $1", inactive).await;
    exec(&db, "UPDATE jobs SET allocation = 0 WHERE id = $1", parked).await;

    for (job, ld_date, layout_date) in [
        (d1, "20260101", "20250505"),
        (d2, "20240303", "20250505"),
        (inactive, "19990101", "19990101"),
        (parked, "19980101", "19980101"),
    ] {
        sqlx::query(
            "UPDATE input_data d SET tarball_date = CASE WHEN d.id = j.letterdist_id THEN $2 ELSE $3 END
             FROM jobs j WHERE j.id = $1 AND d.id IN (j.letterdist_id, j.layout_id)",
        )
        .bind(job)
        .bind(ld_date)
        .bind(layout_date)
        .execute(&db.pool)
        .await
        .unwrap();
    }

    let mut cfg = db.config();
    cfg.magpie_download_url = "https://downloads.example.invalid/magpie-2.9.5".into();
    let state = db.state_with(cfg).await;

    let directive = claim_shutdown(
        &state,
        &anon(&db).await,
        &caps("2.9.0", &[d1, d2, inactive, parked]),
    )
    .await;
    assert_eq!(directive.reason, "both");
    assert_eq!(directive.required_magpie_version.as_deref(), Some("2.9.5"));
    assert_eq!(
        directive.required_tarball_dates,
        vec!["20260101".to_string(), "20250505".to_string(), "20240303".to_string()]
    );
    assert_eq!(
        directive.download_url.as_deref(),
        Some("https://downloads.example.invalid/magpie-2.9.5")
    );
    assert!(directive.message.contains("2.9.5") && directive.message.contains("2.9.0"), "{}", directive.message);
}

/// I-SCHED-12: reclamation is lazy and exact. A claim whose last heartbeat is
/// one second inside the timeout is left alone while one a second past it is
/// reclaimed -- and only when a claim next considers that job: a claim that
/// goes to another job leaves it claimed.
#[tokio::test]
async fn a_lapsed_claim_is_reclaimed_by_the_next_claim_for_its_job_only() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 1).await;
    let other = db.games_job(1, 1).await;
    let state = db.state().await;
    let timeout = state.cfg.heartbeat_timeout.as_secs() as i32;

    let inside = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[other])).await;
    let past = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[other])).await;
    assert_eq!((inside.job_id, past.job_id), (job, job));
    // The heartbeat is what counts, not the claim's age.
    sqlx::query(
        "UPDATE task_claims SET claimed_at = now() - interval '1 hour',
                                last_heartbeat_at = now() - make_interval(secs => $2 - 1)
         WHERE claim_token = $1",
    )
    .bind(inside.claim_token)
    .bind(timeout)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE task_claims SET claimed_at = now() - make_interval(secs => $2 + 1)
         WHERE claim_token = $1",
    )
    .bind(past.claim_token)
    .bind(timeout)
    .execute(&db.pool)
    .await
    .unwrap();

    let claim_state = |token: Uuid| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT state::text FROM task_claims WHERE claim_token = $1")
                .bind(token)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    // A claim that never considers the job reclaims nothing of it.
    let elsewhere = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[job])).await;
    assert_eq!(elsewhere.job_id, other);
    assert_eq!(claim_state(past.claim_token).await, "claimed", "nothing runs in the background");

    // The next claim for the job does, and takes the reopened task.
    let next = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[other])).await;
    assert_eq!(claim_state(past.claim_token).await, "abandoned");
    assert_eq!(claim_state(inside.claim_token).await, "claimed", "one second inside is not lapsed");
    let past_task = task_of(&db, past.claim_token).await;
    assert_eq!(task_of(&db, next.claim_token).await, past_task, "the reopened task goes out again");
    let counts: Vec<i32> =
        sqlx::query_scalar("SELECT active_claim_count FROM tasks WHERE job_id = $1 ORDER BY seed")
            .bind(job)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(counts, vec![1, 1]);
}

/// I-SCHED-13: many workers claiming at once from one redundancy-2 job all get
/// work, and nothing is lost in the counters: every task's live count is the
/// number of claims on it, the job's dispatch counter is the number of claims,
/// its task total is the number of tasks, and the tasks' seeds tile the space
/// with neither a duplicate nor a gap.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claimers_neither_collide_nor_lose_a_count() {
    const WORKERS: usize = 8;
    const BATCH: i64 = 3;
    let db = TestDb::new().await;
    let job = db.games_job(2, BATCH as i32).await;
    let state = db.state().await;

    let mut identities = Vec::new();
    for _ in 0..WORKERS {
        identities.push(anon(&db).await);
    }
    let handles: Vec<_> = identities
        .into_iter()
        .map(|identity| {
            let state = state.clone();
            tokio::spawn(async move {
                scheduler::claim(&state, &identity, &caps("1.0.0", &[])).await.unwrap()
            })
        })
        .collect();
    for handle in handles {
        let outcome = handle.await.unwrap();
        assert_eq!(outcome_kind(&outcome), "task", "a worker was sent away empty-handed");
    }

    let tasks: Vec<(i64, i32, i64, String)> = sqlx::query_as(
        "SELECT t.seed, t.active_claim_count,
                (SELECT COUNT(*) FROM task_claims c WHERE c.task_id = t.id AND c.state = 'claimed'),
                t.state::text
         FROM tasks t WHERE t.job_id = $1 ORDER BY t.seed",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    for (seed, active, claims, task_state) in &tasks {
        assert_eq!(i64::from(*active), *claims, "task {seed}: live count matches its claims");
        assert!(*claims <= 2, "task {seed} is over redundancy");
        assert_eq!(task_state == "claimed", *claims == 2, "task {seed}: {task_state} with {claims}");
    }
    let total: i64 = tasks.iter().map(|t| t.2).sum();
    assert_eq!(total, WORKERS as i64);
    let seeds: Vec<i64> = tasks.iter().map(|t| t.0).collect();
    let tiled: Vec<i64> = (0..seeds.len() as i64).map(|i| 1 + i * BATCH).collect();
    assert_eq!(seeds, tiled, "seeds tile the space");

    let (issued, tasks_total): (i64, i64) =
        sqlx::query_as("SELECT claims_issued, tasks_total FROM jobs WHERE id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(issued, WORKERS as i64, "no lost dispatch count");
    assert_eq!(tasks_total, tasks.len() as i64, "no lost task count");

    let doubled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (
             SELECT task_id, claimed_by_anon_uuid FROM task_claims
             GROUP BY 1, 2 HAVING COUNT(*) > 1) d",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(doubled, 0, "no worker holds two slots on one task");
}

/// I-SCHED-14: a task already at `redundancy` -- two live claims, or one
/// accepted result and one live claim -- is not handed to another worker; the
/// third worker gets a fresh task and the full ones are untouched.
#[tokio::test]
async fn a_task_at_redundancy_is_not_handed_to_a_third_worker() {
    let db = TestDb::new().await;
    let job = db.games_job(2, 2).await;
    let state = db.state().await;

    // Two full tasks, written directly.
    let mut full = Vec::new();
    for (seed, accepted, active, claim_states) in
        [(1i64, 0, 2, ["claimed", "claimed"]), (3, 1, 1, ["completed", "claimed"])]
    {
        let task: Uuid = sqlx::query_scalar(
            "INSERT INTO tasks (job_id, seed, state, accepted_count, active_claim_count)
             VALUES ($1, $2, 'claimed', $3, $4) RETURNING id",
        )
        .bind(job)
        .bind(seed)
        .bind(accepted)
        .bind(active)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        for claim_state in claim_states {
            let holder = anon(&db).await;
            sqlx::query(
                "INSERT INTO task_claims (task_id, claim_token, state, claimed_by_anon_uuid)
                 VALUES ($1, $2, $3::claim_state, $4)",
            )
            .bind(task)
            .bind(Uuid::new_v4())
            .bind(claim_state)
            .bind(holder.anon_uuid())
            .execute(&db.pool)
            .await
            .unwrap();
        }
        full.push(task);
    }

    let third = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[])).await;
    let got = task_of(&db, third.claim_token).await;
    assert!(!full.contains(&got), "a full task was handed out again");
    let seed: i64 = sqlx::query_scalar("SELECT seed FROM tasks WHERE id = $1")
        .bind(got)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(seed, 5, "a new batch after the full ones");

    for task in full {
        let (active, claims): (i32, i64) = sqlx::query_as(
            "SELECT t.active_claim_count, (SELECT COUNT(*) FROM task_claims c WHERE c.task_id = t.id)
             FROM tasks t WHERE t.id = $1",
        )
        .bind(task)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(claims, 2, "no third claim on a full task");
        assert!(active <= 2);
    }
}

/// I-SCHED-15: the per-identity unique indexes are partial on
/// `state NOT IN ('abandoned', 'declined')`. A worker that declined a task --
/// it was missing a file -- and then fixed its data claims that same task
/// again, by anonymous UUID and by account alike. With `'declined'` dropped
/// from either index, the second claim collides with the first and the worker
/// is barred from that task for good.
#[tokio::test]
async fn a_worker_that_declined_a_task_can_claim_the_same_task_again() {
    let db = TestDb::new().await;

    let defs: Vec<(String, String)> = sqlx::query_as(
        "SELECT indexname::text, indexdef FROM pg_indexes
         WHERE indexname IN ('task_claims_user_unique_idx', 'task_claims_anon_unique_idx')
         ORDER BY indexname",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(defs.len(), 2, "{defs:?}");
    for (name, def) in &defs {
        assert!(def.contains("UNIQUE"), "{name}: {def}");
        assert!(def.contains("'abandoned'") && def.contains("'declined'"), "{name}: {def}");
    }

    let state = db.state().await;
    for worker in [anon(&db).await, registered(&db).await] {
        let job = db.games_job(1, 2).await;
        let only_this = caps("1.0.0", &every_job_but(&db, job).await);

        let first = claim_task(&state, &worker, &only_this).await;
        let task = task_of(&db, first.claim_token).await;
        let mut tx = db.pool.begin().await.unwrap();
        let released =
            scheduler::release_claim(&mut tx, claim_id_of(&db, first.claim_token).await, "declined")
                .await
                .unwrap();
        tx.commit().await.unwrap();
        assert!(released);

        let again = claim_task(&state, &worker, &only_this).await;
        assert_eq!(task_of(&db, again.claim_token).await, task, "{worker:?}: the same task, again");
        let states: Vec<String> = sqlx::query_scalar(
            "SELECT state::text FROM task_claims WHERE task_id = $1
               AND (claimed_by_user_id = $2 OR claimed_by_anon_uuid = $3)
             ORDER BY state::text",
        )
        .bind(task)
        .bind(worker.user_id())
        .bind(worker.anon_uuid())
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(states, vec!["claimed".to_string(), "declined".to_string()], "{worker:?}");
    }
}

/// I-SCHED-16: one worker never holds two live claims on one task, by account
/// or by anonymous UUID. Claiming again from a redundancy-2 job hands the
/// worker the next task rather than the other slot of its own, and the unique
/// index refuses a second live row even written directly.
#[tokio::test]
async fn one_worker_never_holds_two_live_claims_on_one_task() {
    let db = TestDb::new().await;
    let state = db.state().await;

    for (worker, index) in [
        (registered(&db).await, "task_claims_user_unique_idx"),
        (anon(&db).await, "task_claims_anon_unique_idx"),
    ] {
        let job = db.games_job(2, 2).await;
        let others = every_job_but(&db, job).await;
        let first = claim_task(&state, &worker, &caps("1.0.0", &others)).await;
        let second = claim_task(&state, &worker, &caps("1.0.0", &others)).await;
        let task = task_of(&db, first.claim_token).await;
        assert_ne!(task_of(&db, second.claim_token).await, task, "{worker:?} got its own task's other slot");

        let err = sqlx::query(
            "INSERT INTO task_claims (task_id, claim_token, claimed_by_user_id, claimed_by_anon_uuid)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(task)
        .bind(Uuid::new_v4())
        .bind(worker.user_id())
        .bind(worker.anon_uuid())
        .execute(&db.pool)
        .await
        .expect_err("a second live claim on one task");
        let db_err = err.as_database_error().expect("a database error");
        assert_eq!(db_err.code().as_deref(), Some("23505"), "{db_err}");
        assert_eq!(db_err.constraint(), Some(index), "{db_err}");
    }
}

/// I-SCHED-19: an inactive job is never selected, however far behind its share
/// and however old; every claim goes to the active one.
#[tokio::test]
async fn an_inactive_job_is_never_selected() {
    let db = TestDb::new().await;
    let inactive = db.games_job(1, 1).await;
    let active = db.games_job(1, 1).await;
    exec(&db, "UPDATE jobs SET status = 'inactive' WHERE id = $1", inactive).await;
    set_created(&db, inactive, 60).await;
    sqlx::query("UPDATE jobs SET claims_issued = 1000 WHERE id = $1")
        .bind(active)
        .execute(&db.pool)
        .await
        .unwrap();
    let state = db.state().await;

    for _ in 0..5 {
        let task = claim_task(&state, &anon(&db).await, &caps("1.0.0", &[])).await;
        assert_eq!(task.job_id, active);
    }
    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE job_id = $1")
        .bind(inactive)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(tasks, 0);
}

// ---------------------------------------------------------------------------
// I-EXPECT
// ---------------------------------------------------------------------------

async fn expected(db: &TestDb, job: Uuid) -> Vec<birdtest::jobs::ExpectedFile> {
    let job = load_job(db, job).await;
    let mut conn = db.pool.acquire().await.unwrap();
    birdtest::jobs::expected_data(&mut conn, &job).await.unwrap()
}

fn names_of<'a>(files: &'a [birdtest::jobs::ExpectedFile], role: &str) -> Vec<&'a str> {
    files.iter().filter(|f| f.role == role).map(|f| f.name.as_str()).collect()
}

async fn player_files(db: &TestDb, player: Uuid) -> (String, String) {
    sqlx::query_as(
        "SELECT kwg.name, klv.name FROM player_configs pc
         JOIN input_data kwg ON kwg.id = pc.kwg_id
         JOIN input_data klv ON klv.id = pc.klv_id
         WHERE pc.id = $1",
    )
    .bind(player)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// I-EXPECT-1: two players on different lexicons need both lexicons and both
/// sets of leaves, plus the job's one distribution and one board.
#[tokio::test]
async fn players_on_different_lexicons_each_contribute_a_kwg_and_a_klv() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 1).await;
    let (p1, p2): (Uuid, Uuid) = sqlx::query_as(
        "SELECT player1_config_id, player2_config_id FROM job_game_config WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let (kwg1, klv1) = player_files(&db, p1).await;
    let (kwg2, klv2) = player_files(&db, p2).await;
    assert_ne!(kwg1, kwg2);

    let files = expected(&db, job).await;
    let mut kwgs = names_of(&files, "kwg");
    kwgs.sort();
    let mut want = vec![kwg1.as_str(), kwg2.as_str()];
    want.sort();
    assert_eq!(kwgs, want);
    let mut klvs = names_of(&files, "klv");
    klvs.sort();
    let mut want = vec![klv1.as_str(), klv2.as_str()];
    want.sort();
    assert_eq!(klvs, want);
    assert_eq!(names_of(&files, "letterdist"), vec!["english"]);
    assert_eq!(names_of(&files, "layout"), vec!["standard15"]);
    assert_eq!(files.len(), 6, "{files:?}");
}

/// I-EXPECT-4: a leave-generation job needs its lexicon, distribution and board
/// and never a `klv` -- its leaves are the server-built artifact -- even with
/// leaves of the lexicon's name imported.
#[tokio::test]
async fn a_leave_job_needs_its_lexicon_bag_and_board_and_never_leaves() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let job = db.bare_job("leave_generation", 1, admin).await;
    let kwg = db.input_data("kwg", "CSW24").await;
    db.input_data("klv", "CSW24").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, 100, 2, 1000, 10)",
    )
    .bind(job)
    .bind(kwg)
    .execute(&db.pool)
    .await
    .unwrap();

    let files = expected(&db, job).await;
    let got: Vec<(&str, &str)> = files.iter().map(|f| (f.role.as_str(), f.name.as_str())).collect();
    assert_eq!(got, vec![("kwg", "CSW24"), ("layout", "standard15"), ("letterdist", "english")]);
}

/// I-EXPECT-5: every entry carries the digest of the row the job pins, not of
/// some row with the same name. Each pinned file has a namesake with other
/// bytes imported beside it, so a lookup by name would pick the wrong digest.
#[tokio::test]
async fn every_expected_file_carries_the_pinned_rows_digest() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 1).await;

    let pinned: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT d.id, d.role, d.name, d.sha256 FROM input_data d
         WHERE d.id IN (
             SELECT letterdist_id FROM jobs WHERE id = $1
             UNION SELECT layout_id FROM jobs WHERE id = $1
             UNION SELECT pc.kwg_id FROM player_configs pc JOIN job_game_config g
                   ON pc.id IN (g.player1_config_id, g.player2_config_id) WHERE g.job_id = $1
             UNION SELECT pc.klv_id FROM player_configs pc JOIN job_game_config g
                   ON pc.id IN (g.player1_config_id, g.player2_config_id) WHERE g.job_id = $1)",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(pinned.len(), 6);
    for (_, role, name, _) in &pinned {
        // Same role, name and path; different bytes.
        db.input_data(role, name).await;
    }

    let files = expected(&db, job).await;
    let mut got: Vec<(String, String, String)> =
        files.iter().map(|f| (f.role.clone(), f.name.clone(), f.sha256.clone())).collect();
    got.sort();
    let mut want: Vec<(String, String, String)> =
        pinned.into_iter().map(|(_, role, name, sha)| (role, name, sha)).collect();
    want.sort();
    assert_eq!(got, want);
    for file in &files {
        assert_eq!(file.sha256.len(), 64, "{file:?}");
        assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{file:?}");
    }
}

/// I-EXPECT-6: an opening-rack job needs its one player's files -- lexicon,
/// leaves and win% model -- and the job's distribution and board, and nothing
/// of any other player.
#[tokio::test]
async fn an_opening_rack_job_needs_its_players_files_and_the_jobs_bag_and_board() {
    let db = TestDb::new().await;
    let admin = db.user("admin", true).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
    let player = db.static_player("solo", admin).await;
    db.static_player("bystander", admin).await;
    let winpct = db.input_data("winpct", "winpct").await;
    // A simmer, since only a simming player loads a win% model.
    sqlx::query(
        "UPDATE player_configs
         SET num_plies = 2, winpct_id = $2, max_iterations = 1000, stopping_pct = 99,
             use_inference = false, time_limit_secs = 0, min_play_iterations = 100,
             threshold = 'none', sampling_rule = 'round_robin', inference_margin = 0,
             utility_w_winpct = 1, utility_w_spread = 0, utility_spread_scale = 1
         WHERE id = $1",
    )
        .bind(player)
        .bind(winpct)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, 10, 2, 100)",
    )
    .bind(job)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();

    let files = expected(&db, job).await;
    let got: Vec<(&str, &str)> = files.iter().map(|f| (f.role.as_str(), f.name.as_str())).collect();
    assert_eq!(
        got,
        vec![
            ("klv", "NWLsolo"),
            ("kwg", "NWLsolo"),
            ("layout", "standard15"),
            ("letterdist", "english"),
            ("winpct", "winpct"),
        ]
    );
}
