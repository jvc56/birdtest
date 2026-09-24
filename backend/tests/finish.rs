//! The finish check a submission runs, through the worker API: the debounced
//! SPRT check under steady load, and an opening-rack job completing once its
//! rack space is used up and every task is accepted.
//!
//! The check runs on every `SPRT_CHECK_EVERY`th submission for a job, or on
//! any submission that leaves the job nothing in flight. Tests that submit with
//! no other claim open only ever reach the second branch; these hold claims
//! open so the first one is what decides.

mod common;

use axum::http::StatusCode;
use birdtest::jobstats;
use birdtest::models::job::JobStatus;
use birdtest::stats::sprt::SprtStatus;
use birdtest::state::SPRT_CHECK_EVERY;
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
            json!({ "claim_token": assignment["claim_token"], "result": result }),
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
// The debounced SPRT check
// ---------------------------------------------------------------------------

/// I-STATS-9 (debounced): under steady load -- one worker's claim open the
/// whole time, so the job is never idle -- the finish check runs on every
/// `SPRT_CHECK_EVERY`th submission and not before. Every batch is 90-10, so
/// the LLR is past the bound from the first submission on (at `min_games`
/// 100); the job is nonetheless still `active` right before the
/// `SPRT_CHECK_EVERY`th submission, and `completed` by it. The open claim is
/// untouched, and its result is still accepted afterwards: the submit path
/// validates the claim, not the job's status.
#[tokio::test]
async fn under_steady_load_the_finish_check_runs_on_every_nth_submission() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 100).await;
    sqlx::query("UPDATE job_game_config SET min_games = 100 WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let (held, held_uuid) = first_claim(&app).await;

    for n in 1..=SPRT_CHECK_EVERY {
        let (assignment, uuid) = first_claim(&app).await;
        assert_eq!(job_status(&db, job).await, "active", "before submission {n}");
        submit(&app, &uuid, &assignment, games_result(100, 90)).await;

        let row = jobstats::load_job(&db.pool, job).await.unwrap();
        let games = jobstats::game_stats(&db.pool, &row).await.unwrap().unwrap();
        assert_eq!(games.units_completed, 100 * n);
        assert_eq!(games.sprt.status, SprtStatus::Passed, "the verdict is there at {n}");
        if n < SPRT_CHECK_EVERY {
            assert_eq!(row.status, JobStatus::Active, "unchecked after submission {n}");
        }
    }
    assert_eq!(job_status(&db, job).await, "completed");
    assert_eq!(claim_state(&db, &held).await, "claimed", "the held claim is still open");

    submit(&app, &held_uuid, &held, games_result(100, 90)).await;
    assert_eq!(claim_state(&db, &held).await, "completed");
    assert_eq!(job_status(&db, job).await, "completed");
}

// ---------------------------------------------------------------------------
// Opening racks
// ---------------------------------------------------------------------------

/// An active opening-rack job over a rack space of `total_racks`, handed out
/// `racks_per_batch` at a time.
async fn opening_rack_job(db: &TestDb, racks_per_batch: i32, total_racks: i64) -> Uuid {
    let admin = db.user("admin", true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", 1, admin).await;
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
