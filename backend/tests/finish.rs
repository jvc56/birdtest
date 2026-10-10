//! The finish check a submission runs, through the worker API: the debounced
//! match-test check under steady load, and an opening-rack job completing once its
//! rack space is used up and every task is accepted.
//!
//! The check runs on every `TEST_CHECK_EVERY`th submission for a job, or on
//! any submission that leaves the job nothing in flight. Tests that submit with
//! no other claim open only ever reach the second branch; these hold claims
//! open so the first one is what decides.

mod common;

use axum::http::StatusCode;
use birdtest::jobstats;
use birdtest::models::job::JobStatus;
use birdtest::stats::match_test::TestStatus;
use birdtest::state::TEST_CHECK_EVERY;
use common::*;
use serde_json::{json, Value};
use uuid::Uuid;

/// Claims once with no identity, returning the assignment and the UUID the
/// server minted for it.
async fn first_claim(app: &axum::Router) -> (Value, String) {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().expect("a minted worker_uuid").to_string();
    (body, uuid)
}

async fn submit(app: &axum::Router, uuid: &str, assignment: &Value, result: Value) {
    let (status, body) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": assignment["claim_token"], "movegens": 1000, "result": result }),
        ),
    )
    .await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));
}

async fn job_status(db: &TestDb, job: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn claim_state(db: &TestDb, assignment: &Value) -> String {
    sqlx::query_scalar("SELECT state::text FROM task_claims WHERE claim_token = $1::uuid")
        .bind(assignment["claim_token"].as_str().unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// The debounced match-test check
// ---------------------------------------------------------------------------

/// I-STATS-9 (debounced): under steady load -- one worker's claim open the
/// whole time, so the job is never idle -- the finish check runs on every
/// `TEST_CHECK_EVERY`th submission and not before. Every batch is 90-10, so
/// player 1's interval is above an even score from the first submission on
/// (at `min_games` 100); the job is nonetheless still `active` right before the
/// `TEST_CHECK_EVERY`th submission, and `completed` by it. The open claim is
/// untouched, and its result is still accepted afterwards: the submit path
/// validates the claim, not the job's status.
#[tokio::test]
async fn under_steady_load_the_finish_check_runs_on_every_nth_submission() {
    let db = TestDb::new().await;
    let job = db.games_job(100).await;
    sqlx::query("UPDATE job_game_config SET min_games = 100, max_games = 10000 WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let (held, held_uuid) = first_claim(&app).await;

    for n in 1..=TEST_CHECK_EVERY {
        let (assignment, uuid) = first_claim(&app).await;
        assert_eq!(job_status(&db, job).await, "active", "before submission {n}");
        submit(&app, &uuid, &assignment, games_result(100, 90)).await;

        let row = jobstats::load_job(&db.pool, job).await.unwrap();
        let games = jobstats::game_stats(&db.pool, &row).await.unwrap().unwrap();
        assert_eq!(games.units_completed, 100 * n);
        let test = games.test.expect("a job with a test");
        assert_eq!(test.status, TestStatus::Player1Better, "the verdict is there at {n}");
        if n < TEST_CHECK_EVERY {
            assert_eq!(row.status, JobStatus::Active, "unchecked after submission {n}");
        }
    }
    assert_eq!(job_status(&db, job).await, "completed");
    assert_eq!(claim_state(&db, &held).await, "claimed", "the held claim is still open");
    let decided = jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(decided.test_decided_status.as_deref(), Some("player1_better"));
    assert_eq!(decided.test_decided_units, Some(100 * TEST_CHECK_EVERY as i64));
    let decided_lower = decided.test_decided_lower.unwrap();

    // I-STATS-9e: the result in flight at completion lands and moves the live
    // figures -- a losing batch -- and the verdict the job stopped on stays.
    submit(&app, &held_uuid, &held, games_result(100, 10)).await;
    assert_eq!(claim_state(&db, &held).await, "completed");
    assert_eq!(job_status(&db, job).await, "completed");
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    let games = jobstats::game_stats(&db.pool, &row).await.unwrap().unwrap();
    let live = games.test.expect("a job with a test").lower;
    assert!(live < decided_lower, "the live interval moved: {live} vs {decided_lower}");
    let stored = games.decided.expect("the stored verdict is reported");
    assert_eq!((stored.status.as_str(), stored.lower), ("player1_better", decided_lower));
    // I-STATS-9f: and the page is told why it finished -- its test, not an
    // admin and not its cap.
    let completion = jobstats::compute(&db.pool, &row).await.unwrap().completion.expect("completed");
    assert!(!completion.forced);
    assert_eq!(completion.reason.as_deref(), Some("player1_better"));
}

// ---------------------------------------------------------------------------
// Opening racks
// ---------------------------------------------------------------------------

/// An active opening-rack job over a rack space of `total_racks`, handed out
/// `racks_per_batch` at a time.
async fn opening_rack_job(db: &TestDb, racks_per_batch: i32, total_racks: i64) -> Uuid {
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, $3, 7, $4)",
    )
    .bind(job)
    .bind(player)
    .bind(racks_per_batch)
    .bind(total_racks)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

/// An analysis of exactly the racks `assignment` handed out.
fn analysis(assignment: &Value) -> Value {
    let racks = assignment["task_request"]["racks"].as_array().expect("an opening-rack task");
    json!({ "racks": racks.iter().map(|rack| json!({
        "rack": rack,
        "num_moves": 1,
        "moves": [{ "move": "8G WUZ", "score": 30, "equity": 32.5 }],
    })).collect::<Vec<_>>() })
}

/// The job's tasks as `(seed, state)`, by seed.
async fn tasks(db: &TestDb, job: Uuid) -> Vec<(i64, String)> {
    sqlx::query_as("SELECT seed, state::text FROM tasks WHERE job_id = $1 ORDER BY seed")
        .bind(job)
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

/// I-STATS-9 (opening racks): a rack space of 6 in batches of 3 is two tasks.
/// Once both are handed out there is nothing left to generate, but the job
/// stays `active` while one is still open, and completes when the second is
/// accepted -- on its own, with nobody asking.
#[tokio::test]
async fn an_opening_rack_job_completes_once_its_racks_are_handed_out_and_all_accepted() {
    let db = TestDb::new().await;
    let job = opening_rack_job(&db, 3, 6).await;
    let app = birdtest::app(db.state().await);

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the rack space is used up: {body}");

    submit(&app, &uuid_a, &a, analysis(&a)).await;
    assert_eq!(job_status(&db, job).await, "active", "one task is still open");

    submit(&app, &uuid_b, &b, analysis(&b)).await;
    assert_eq!(
        tasks(&db, job).await,
        vec![(0, "completed".to_string()), (3, "completed".to_string())]
    );
    assert_eq!(job_status(&db, job).await, "completed");
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(row.racks_analyzed, 6);
    // The server completed it, and says so: only an admin's completion was on
    // record, and `jobs` keeps no completion time (the audit's pass 22).
    let logged: Vec<(Option<uuid::Uuid>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT actor_user_id, old_status, new_status FROM audit_log
         WHERE action = 'job.completed' AND job_id = $1",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(logged, vec![(None, Some("active".to_string()), Some("completed".to_string()))]);
    // I-STATS-9f: its racks ran out -- no verdict, no admin.
    let completion = jobstats::compute(&db.pool, &row).await.unwrap().completion.expect("completed");
    assert_eq!((completion.forced, completion.reason), (false, None));
}

/// I-STATS-9f: an admin's force-complete is reported as one, and a job that
/// is not completed reports no completion.
#[tokio::test]
async fn a_forced_completion_is_reported_as_forced() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let job = db.games_job(10).await;
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    assert!(jobstats::compute(&db.pool, &row).await.unwrap().completion.is_none());

    let headers = admin_headers(&db.config(), admin);
    let (status, body) = send(&app, post_json(&format!("/api/admin/jobs/{job}/complete"), &headers
        .iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>(), json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    let completion = jobstats::compute(&db.pool, &row).await.unwrap().completion.expect("completed");
    assert_eq!((completion.forced, completion.reason), (true, None));
}

/// I-STATS-9 (opening racks): a declined task goes back to `available`, and a
/// job whose rack space is used up but which still has that task to do is not
/// finished. Here the check certainly runs -- the other task's submission
/// leaves nothing in flight -- and finds the rack space exhausted and the
/// declined task undone, so the job stays `active`; the task is handed out
/// again, and its acceptance completes the job.
#[tokio::test]
async fn a_declined_opening_rack_task_keeps_its_job_active_until_it_is_done() {
    let db = TestDb::new().await;
    let job = opening_rack_job(&db, 3, 6).await;
    let app = birdtest::app(db.state().await);

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", uuid_b.as_str())],
            json!({ "claim_token": b["claim_token"], "reason": "task_failed" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    submit(&app, &uuid_a, &a, analysis(&a)).await;
    assert_eq!(
        tasks(&db, job).await,
        vec![(0, "completed".to_string()), (3, "available".to_string())]
    );
    assert_eq!(job_status(&db, job).await, "active", "the declined task is still to do");

    let (c, uuid_c) = first_claim(&app).await;
    assert_eq!(c["task_request"]["seed"], b["task_request"]["seed"], "the declined task, again");
    submit(&app, &uuid_c, &c, analysis(&c)).await;
    assert_eq!(job_status(&db, job).await, "completed");
}

/// I-STATS-9j: the opening-rack finish check reads indexes, not every job's
/// tasks. It runs on the submission that settles a job's last rack, before
/// its worker is answered; asked as `EXISTS` over the job's tasks and
/// `NOT EXISTS` of one not completed, both were sequential scans of `tasks`,
/// whose rows every job keeps. Thirty thousand of an older job's tasks laid
/// down first, a completed claim each, analysed: the check is index scans
/// only, in a custom plan and a generic one, and still answers -- done with
/// every task completed, not done with a task available or claimed, and not
/// done with no task at all.
#[tokio::test]
async fn the_opening_rack_finish_check_reads_indexes_not_every_jobs_tasks() {
    use birdtest::routes::worker::OPENING_RACK_FINISHED;
    let db = TestDb::new().await;
    let admin = db.user("planner", true).await;
    let older = db.bare_job("games", admin).await;
    sqlx::query(
        "INSERT INTO tasks (job_id, seed, state, accepted_count, completed_at)
         SELECT $1, s, 'completed', 1, now() FROM generate_series(0, 29999) s",
    )
    .bind(older)
    .execute(&db.pool)
    .await
    .unwrap();
    let job = opening_rack_job(&db, 10, 2000).await;
    let empty = db.bare_job("opening_rack", admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         SELECT $1, player_config_id, racks_per_batch, rack_size, total_racks
         FROM job_opening_rack_config WHERE job_id = $2",
    )
    .bind(empty)
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET racks_settled = 2000 WHERE id = ANY($1)")
        .bind(vec![job, empty])
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO tasks (job_id, seed, state, accepted_count, completed_at)
         SELECT $1, s * 10, 'completed', 1, now() FROM generate_series(0, 199) s",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    // A completed claim for every task, as the jobs left them.
    sqlx::query(
        "INSERT INTO task_claims (task_id, job_id, claim_token, state, claimed_by_user_id, completed_at)
         SELECT id, job_id, gen_random_uuid(), 'completed', $1, now() FROM tasks",
    )
    .bind(admin)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE tasks").execute(&db.pool).await.unwrap();
    sqlx::query("ANALYZE task_claims").execute(&db.pool).await.unwrap();

    // The data is what made the old form scan the table, so the assertions
    // below are not vacuous.
    let old: Vec<String> = sqlx::query_scalar(&format!(
        "EXPLAIN SELECT j.racks_settled >= c.total_racks
                AND EXISTS (SELECT 1 FROM tasks t WHERE t.job_id = j.id)
                AND NOT EXISTS (SELECT 1 FROM tasks t
                                WHERE t.job_id = j.id AND t.state <> 'completed')
         FROM jobs j JOIN job_opening_rack_config c ON c.job_id = j.id
         WHERE j.id = '{job}'"
    ))
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(old.join("\n").contains("Seq Scan on tasks"), "{old:?}");

    for mode in ["auto", "force_generic_plan"] {
        let mut tx = db.pool.begin().await.unwrap();
        for statement in [
            format!("SET LOCAL plan_cache_mode = {mode}"),
            format!("PREPARE finished AS {OPENING_RACK_FINISHED}"),
        ] {
            sqlx::raw_sql(&statement).execute(&mut *tx).await.unwrap();
        }
        let plan: Vec<String> = sqlx::query_scalar(&format!("EXPLAIN EXECUTE finished('{job}')"))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        let plan = plan.join("\n");
        assert!(!plan.contains("Seq Scan"), "{mode}: {plan}");
        for index in ["tasks_seed_unique_idx", "tasks_queue_idx", "task_claims_open_idx"] {
            assert!(plan.contains(index), "{mode}: {plan}");
        }
        tx.rollback().await.unwrap();
    }

    let finished = |job: Uuid| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(OPENING_RACK_FINISHED)
                .bind(job)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert!(finished(job).await, "every rack settled, every task completed");
    assert!(!finished(empty).await, "no task was ever made");
    // A reissue, waiting to go out, then out on a claim.
    let last: Uuid =
        sqlx::query_scalar("INSERT INTO tasks (job_id, seed) VALUES ($1, 2000) RETURNING id")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(!finished(job).await, "a task waiting to go out");
    sqlx::query("UPDATE tasks SET state = 'claimed', active_claim_count = 1 WHERE id = $1")
        .bind(last)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO task_claims (task_id, job_id, claim_token, claimed_by_user_id)
         VALUES ($1, $2, gen_random_uuid(), $3)",
    )
    .bind(last)
    .bind(job)
    .bind(admin)
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(!finished(job).await, "a task out on a claim");
}

/// I-STATS-9k: the "anything still in flight" probe reads the open-claims
/// index, not the job's tasks. The finish checks run it inline on most
/// submissions and an export asks it of a completed job; joined to `tasks` for
/// the job, it walked every open claim in the fleet with a `tasks` probe each,
/// and without statistics -- after a restore, before ANALYZE -- read every
/// task the job ever had. Thirty thousand of a job's tasks laid down, a
/// completed claim each, and another job's two hundred claims open, with the
/// two tables' statistics removed (where the old form scans the job's tasks):
/// [`JOB_HAS_OPEN_CLAIM`] reads `task_claims_open_idx` and no `tasks`, custom
/// and generic, with statistics and without, and answers for each job.
#[tokio::test]
async fn the_in_flight_probe_reads_the_open_claims_not_the_jobs_tasks() {
    use birdtest::routes::worker::JOB_HAS_OPEN_CLAIM;
    let db = TestDb::new().await;
    let admin = db.user("planner", true).await;
    let done = db.bare_job("games", admin).await;
    let busy = db.bare_job("games", admin).await;
    sqlx::query(
        "INSERT INTO tasks (job_id, seed, state, accepted_count, completed_at)
         SELECT $1, s, 'completed', 1, now() FROM generate_series(0, 29999) s",
    )
    .bind(done)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tasks (job_id, seed, state, active_claim_count)
         SELECT $1, s, 'claimed', 1 FROM generate_series(0, 199) s",
    )
    .bind(busy)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO task_claims (task_id, job_id, claim_token, state, claimed_by_user_id, completed_at)
         SELECT id, job_id, gen_random_uuid(),
                CASE WHEN state = 'completed' THEN 'completed' ELSE 'claimed' END::claim_state,
                $1, completed_at
         FROM tasks",
    )
    .bind(admin)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE tasks").execute(&db.pool).await.unwrap();
    sqlx::query("ANALYZE task_claims").execute(&db.pool).await.unwrap();

    for statistics in [true, false] {
        let mut tx = db.pool.begin().await.unwrap();
        if !statistics {
            // As after a restore, before ANALYZE. TestDb connects as a
            // superuser; the rollback puts them back.
            sqlx::raw_sql(
                "DELETE FROM pg_statistic
                 WHERE starelid IN ('tasks'::regclass, 'task_claims'::regclass);
                 UPDATE pg_class SET reltuples = -1
                 WHERE oid IN ('tasks'::regclass, 'task_claims'::regclass)",
            )
            .execute(&mut *tx)
            .await
            .unwrap();
            // The data is what made the old form read the job's tasks, so the
            // assertions below are not vacuous.
            let old: Vec<String> = sqlx::query_scalar(&format!(
                "EXPLAIN SELECT EXISTS (SELECT 1 FROM task_claims c JOIN tasks t ON t.id = c.task_id
                                        WHERE t.job_id = '{done}' AND c.state = 'claimed')"
            ))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
            assert!(old.join("\n").contains("on tasks t"), "{old:?}");
        }
        for mode in ["auto", "force_generic_plan"] {
            for statement in [
                format!("SET LOCAL plan_cache_mode = {mode}"),
                format!("PREPARE in_flight AS {JOB_HAS_OPEN_CLAIM}"),
            ] {
                sqlx::raw_sql(&statement).execute(&mut *tx).await.unwrap();
            }
            let plan: Vec<String> =
                sqlx::query_scalar(&format!("EXPLAIN EXECUTE in_flight('{done}')"))
                    .fetch_all(&mut *tx)
                    .await
                    .unwrap();
            let plan = plan.join("\n");
            assert!(plan.contains("task_claims_open_idx"), "{statistics} {mode}: {plan}");
            assert!(!plan.contains("on tasks"), "{statistics} {mode}: {plan}");
            assert!(!plan.contains("Seq Scan"), "{statistics} {mode}: {plan}");
            sqlx::raw_sql("DEALLOCATE in_flight").execute(&mut *tx).await.unwrap();
        }
        for (job, open) in [(done, false), (busy, true)] {
            let answer: bool = sqlx::query_scalar(JOB_HAS_OPEN_CLAIM)
                .bind(job)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            assert_eq!(answer, open, "{statistics}");
        }
        tx.rollback().await.unwrap();
    }
}

// ---------------------------------------------------------------------------
// A job whose last results landed with nobody to check them
// ---------------------------------------------------------------------------

/// Deactivates (0%) or reactivates (above 0%) the job through the
/// allocation page's request.
async fn set_allocation(app: &axum::Router, headers: &[(String, String)], job: Uuid, allocation: i32) {
    let refs: Vec<(&str, &str)> = headers.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (status, body) = send(app, allocate(job, allocation, &refs)).await;
    assert_eq!(status, StatusCode::OK, "{job} at {allocation}%: {body}");
}

/// The idle check runs off the claim request, so its result is waited for.
async fn eventually_completed(db: &TestDb, job: Uuid) -> String {
    for _ in 0..100 {
        let status = job_status(db, job).await;
        if status == "completed" {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    job_status(db, job).await
}

/// I-STATS-9i: an opening-rack job deactivated while its last two tasks are
/// out. Their results land while it is inactive and are accepted, with no
/// finish check; reactivated, it has nothing to hand out and nothing will
/// submit again. It used to stay `active` at its allocation for good; the
/// first claim to find it empty now completes it.
#[tokio::test]
async fn a_job_whose_last_results_landed_while_inactive_completes_once_reactivated() {
    let db = TestDb::new().await;
    let job = opening_rack_job(&db, 3, 6).await;
    let boss = db.user("boss", true).await;
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, boss);
    let app = birdtest::app(state);

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    set_allocation(&app, &headers, job, 0).await;
    submit(&app, &uuid_a, &a, analysis(&a)).await;
    submit(&app, &uuid_b, &b, analysis(&b)).await;
    assert_eq!(job_status(&db, job).await, "inactive");

    set_allocation(&app, &headers, job, 50).await;
    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(eventually_completed(&db, job).await, "completed");
}

/// A games job at its `max_games` of 200, with or without its test, whose
/// last two batches, both even, landed while it was inactive; reactivated,
/// the first claim to find it empty completes it.
async fn a_games_job_whose_cap_landed_while_inactive(test_enabled: bool) -> (TestDb, Uuid) {
    let db = TestDb::new().await;
    let job = db.games_job(100).await;
    sqlx::query(
        "UPDATE job_game_config SET test_enabled = $2, min_games = 100, max_games = 200
         WHERE job_id = $1",
    )
    .bind(job)
    .bind(test_enabled)
    .execute(&db.pool)
    .await
    .unwrap();
    let boss = db.user("boss", true).await;
    let state = db.state().await;
    let headers = admin_headers(&state.cfg, boss);
    let app = birdtest::app(state);

    let (a, uuid_a) = first_claim(&app).await;
    let (b, uuid_b) = first_claim(&app).await;
    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the cap is handed out");
    // That claim found nothing with two in flight: no completion, and the
    // pacing must not stop the check after reactivation from running.
    assert_eq!(job_status(&db, job).await, "active");

    set_allocation(&app, &headers, job, 0).await;
    submit(&app, &uuid_a, &a, games_result(100, 50)).await;
    submit(&app, &uuid_b, &b, games_result(100, 50)).await;
    set_allocation(&app, &headers, job, 50).await;
    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(eventually_completed(&db, job).await, "completed");
    (db, job)
}

/// I-STATS-9i (games): the same for a games job at its `max_games` cap.
#[tokio::test]
async fn a_games_job_at_its_cap_whose_results_landed_while_inactive_completes() {
    let (db, job) = a_games_job_whose_cap_landed_while_inactive(true).await;
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(row.test_decided_status.as_deref(), Some("inconclusive"));
}

/// I-STATS-9g (games, no test): and for one without a test, at its target --
/// with no verdict stored, and `reached_target` as the reason.
#[tokio::test]
async fn a_games_job_without_a_test_completes_at_its_target_once_reactivated() {
    let (db, job) = a_games_job_whose_cap_landed_while_inactive(false).await;
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    assert_eq!(row.test_decided_status, None);
    let completion = jobstats::compute(&db.pool, &row).await.unwrap().completion.expect("completed");
    assert!(!completion.forced);
    assert_eq!(completion.reason.as_deref(), Some("reached_target"));
}
