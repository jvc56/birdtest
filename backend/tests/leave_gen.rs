//! Leave generation against a real database: the full-rack universe, forced
//! racks that are full racks, submissions folding into it, and claim-time
//! selection skipping racks already out.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;

/// A leave-generation job over the test distribution (13 tiles, so a small
/// full-rack universe), with its generation-1 universe seeded and a
/// generation-0 artifact row so it can dispatch.
async fn leave_job(db: &TestDb, racks_per_task: i32) -> (Uuid, i64) {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let job = db.bare_job("leave_generation", 1, admin).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, 100, 2, 1000, $3)",
    )
    .bind(job)
    .bind(kwg)
    .bind(racks_per_task)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256)
         VALUES ($1, 0, 'leaves/test/generation-0.klv2', repeat('0', 64))",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let row = sqlx::query_as::<_, birdtest::models::job::Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let seeded = birdtest::jobs::registry::initialize_job_state(&mut conn, &row).await.unwrap();
    (job, seeded)
}

/// Polls `condition` until it holds or a generous deadline passes, for the one
/// thing in this file that is deliberately not finished when the request that
/// started it returns: a generation transition, which runs on its own task so
/// the claiming worker is not held for it.
async fn wait_for<F, Fut>(mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if condition().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    false
}

fn forced_racks(assignment: &serde_json::Value) -> Vec<String> {
    assignment["task_request"]["forced_racks"]
        .as_array()
        .expect("a leave assignment carries forced_racks")
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect()
}

/// Bug: the server enumerated leaves of 1..6 tiles and forced them, and MAGPIE
/// refused every task ("forced racks must all be full racks of 7 tiles").
#[tokio::test]
async fn the_universe_and_the_forced_racks_are_full_racks() {
    let db = TestDb::new().await;
    let (job, seeded) = leave_job(&db, 5).await;

    let rows: Vec<String> =
        sqlx::query_scalar("SELECT rack FROM leave_rack_progress WHERE job_id = $1 AND generation = 1")
            .bind(job)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(rows.len() as i64, seeded);
    assert!(rows.iter().all(|r| r.chars().count() == 7), "every row is a full rack");
    let ld = birdtest::jobs::racks::LetterDistribution::parse(
        include_bytes!("../src/jobs/testdata/testdist.csv"),
        "testdist",
    )
    .unwrap();
    assert_eq!(seeded as usize, ld.enumerate_racks(7).len());

    let app = birdtest::app(db.state().await);
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let racks = forced_racks(&body);
    assert_eq!(racks.len(), 5);
    assert!(racks.iter().all(|r| r.chars().count() == 7), "{racks:?}");
}

/// Bug: two workers claiming at once were both handed the same lowest-count
/// racks.
#[tokio::test]
async fn racks_out_with_an_open_claim_are_not_handed_out_again() {
    let db = TestDb::new().await;
    let (_, seeded) = leave_job(&db, 4).await;
    let app = birdtest::app(db.state().await);

    let mut seen = HashSet::new();
    let claims = (seeded / 4).min(6);
    for _ in 0..claims {
        let (status, body) =
            send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for rack in forced_racks(&body) {
            assert!(seen.insert(rack.clone()), "rack {rack} handed to two open claims");
        }
    }
}

/// A submission adds to its racks' rows, and a rack that is not a full rack of
/// the distribution neither counts nor creates a row.
#[tokio::test]
async fn a_result_folds_into_the_generation_and_creates_no_rows() {
    let db = TestDb::new().await;
    let (job, seeded) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().unwrap();
    let token = body["claim_token"].as_str().unwrap();
    let racks = forced_racks(&body);

    let result = json!({ "racks": [
        { "rack": racks[0], "count": 3, "mean": 10.0 },
        { "rack": racks[1], "count": 1, "mean": -2.0 },
        // Seven tiles, but ZZZZZZZ is not drawable from this distribution.
        { "rack": "ZZZZZZZ", "count": 5, "mean": 1.0 },
    ]});
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": token, "result": result }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (count, sum): (i64, f64) = sqlx::query_as(
        "SELECT occurrence_count, equity_sum FROM leave_rack_progress
         WHERE job_id = $1 AND generation = 1 AND rack = $2",
    )
    .bind(job)
    .bind(&racks[0])
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((count, sum), (3, 30.0));

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, seeded, "no row for a rack outside the universe");
}

/// A leave result naming anything but full racks is refused outright.
#[tokio::test]
async fn a_leave_result_of_partial_racks_is_rejected() {
    let db = TestDb::new().await;
    leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let (_, body) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    let uuid = body["worker_uuid"].as_str().unwrap();
    let token = body["claim_token"].as_str().unwrap();
    let (status, _) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": token, "result": { "racks": [
                { "rack": "AB", "count": 3, "mean": 10.0 }
            ]}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Bug: a leave task whose claim timed out went back to `available`, and every
/// claim re-dispatched available tasks before reaching the leave-generation
/// path -- so before the job's claim lock was taken, and whatever generation the
/// task belonged to. A task from a generation that had since closed was handed
/// out again: the worker played a finished generation with an outdated KLV,
/// and the result could only be discarded. A reopened task is now reissued only
/// while its own generation is the current one, and under the lock.
#[tokio::test]
async fn a_reclaimed_task_is_reissued_only_while_its_generation_is_open() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let expire_all = || async {
        sqlx::query("UPDATE task_claims SET claimed_at = now() - interval '1 hour'")
            .execute(&db.pool)
            .await
            .unwrap();
        birdtest::scheduler::reclaim_expired(&db.pool, job, 300.0).await.unwrap()
    };

    let (status, first) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(expire_all().await, 1);

    // While generation 1 is open, the reopened task is what the next worker
    // gets: the same racks, not a fresh task beside it.
    let (status, again) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(forced_racks(&again), forced_racks(&first));
    assert_eq!(expire_all().await, 1);

    // Generation 1 closes, and generation 2's universe exists.
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256)
         VALUES ($1, 1, 'leaves/test/generation-1.klv2', repeat('1', 64))",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO leave_rack_progress (job_id, generation, rack)
         SELECT job_id, 2, rack FROM leave_rack_progress WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let (status, next) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{next}");
    assert_eq!(
        next["task_request"]["generation"],
        json!(2),
        "a closed generation's task must not be handed out again: {next}"
    );
}

/// The generation whose KLV is already built must not take more results.
///
/// The claim flow no longer produces this state (a generation closes only with
/// nothing in flight, and closed generations' tasks are not reissued), so the
/// test forces it by hand, as a partial restore could. The submission is
/// accepted -- the worker did the work, and failing it would only make it
/// retry -- but folding it in would leave the rows disagreeing with the
/// artifact built from them, which is the signal reserved for a corrupted
/// object.
#[tokio::test]
async fn a_result_for_a_closed_generation_is_credited_but_not_folded() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let (_, body) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    let uuid = body["worker_uuid"].as_str().unwrap().to_string();
    let token = body["claim_token"].as_str().unwrap().to_string();
    let racks = forced_racks(&body);

    // Generation 1 closes while the task is out, as a transition started just
    // before the claim committed would have closed it.
    sqlx::query(
        "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256)
         VALUES ($1, 1, 'leaves/test/generation-1.klv2', repeat('1', 64))",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid.as_str())],
            json!({ "claim_token": token, "result": { "racks": [
                { "rack": racks[0], "count": 3, "mean": 10.0 }
            ]}}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], json!(true), "the worker is not told to retry");

    let count: i64 = sqlx::query_scalar(
        "SELECT occurrence_count FROM leave_rack_progress
         WHERE job_id = $1 AND generation = 1 AND rack = $2",
    )
    .bind(job)
    .bind(&racks[0])
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "a closed generation's totals are frozen");

    let credited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leave_records r JOIN tasks t ON t.id = r.task_id
         WHERE t.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(credited, 1, "the claim is still recorded as completed work");
}

/// One transition per generation, however many claims find it complete.
///
/// The transition runs outside the claim transaction, so without the marker row
/// every claim arriving during it would start another one -- each streaming
/// every rack of the generation and uploading a KLV.
#[tokio::test]
async fn only_one_claim_starts_a_generations_transition() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;

    // Every rack at target and nothing in flight: the generation is complete.
    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = 1000
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let step = next_step(&db, job).await;
    assert!(matches!(step, Step::Transition), "{step:?}");
    let step = next_step(&db, job).await;
    assert!(matches!(step, Step::InProgress), "a second claim waits instead: {step:?}");

    let (attempts, completed): (i32, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT attempts, completed_at FROM leave_generation_transitions
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((attempts, completed), (1, None));
}

/// A transition the process died in the middle of is taken over, or the job
/// would wait on a transition nobody is running.
#[tokio::test]
async fn a_transition_that_never_finished_is_taken_over() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;

    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = 1000
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    assert!(matches!(next_step(&db, job).await, Step::Transition));
    sqlx::query(
        "UPDATE leave_generation_transitions SET started_at = now() - interval '2 hours'
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let step = next_step(&db, job).await;
    assert!(matches!(step, Step::Transition), "past the takeover timeout: {step:?}");
    let attempts: i32 = sqlx::query_scalar(
        "SELECT attempts FROM leave_generation_transitions WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(attempts, 2, "a takeover is recorded rather than silent");

    // A finished transition is never restarted, timeout or not.
    sqlx::query(
        "UPDATE leave_generation_transitions
         SET completed_at = now(), started_at = now() - interval '2 hours'
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let step = next_step(&db, job).await;
    assert!(matches!(step, Step::InProgress), "{step:?}");
}

/// What `next_step` decided, without the request payloads.
#[derive(Debug)]
enum Step {
    Transition,
    InProgress,
    Other,
}

/// One claim decision for a leave job, through the same path a worker's request
/// takes: the job's advisory lock, then `next_step`, in one transaction.
async fn next_step(db: &TestDb, job: Uuid) -> Step {
    use birdtest::jobs::leave_gen::{self, LeaveGenStep};
    let mut tx = db.pool.begin().await.unwrap();
    let job_row = sqlx::query_as::<_, birdtest::models::job::Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let config = sqlx::query_as::<_, birdtest::models::job::LeaveConfig>(
        "SELECT * FROM job_leave_config WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let job_data = birdtest::jobs::load_job_data(&mut tx, job_row.id).await.unwrap();
    leave_gen::lock_claim_decisions(&mut tx, job).await.unwrap();
    let step = leave_gen::next_step(&mut tx, job, &config, &job_data).await.unwrap();
    let step = match step {
        LeaveGenStep::Transition { .. } => Step::Transition,
        LeaveGenStep::TransitionInProgress { .. } => Step::InProgress,
        _ => Step::Other,
    };
    tx.commit().await.unwrap();
    step
}

/// The marker row that says who owns a transition has to *commit*, or it
/// stops nobody.
///
/// The claim transaction that decides a generation is complete writes nothing
/// else, and the transition runs after it ends -- so rolling it back, as the
/// path once did, would leave every claim arriving during the transition free
/// to start another one. Driven through the HTTP claim to exercise the real
/// commit: the transition itself then fails here (the test config points the
/// object store at a closed port), which also exercises the failure path handing
/// ownership straight back rather than waiting out the takeover timeout.
#[tokio::test]
async fn the_transition_owner_is_committed_before_the_transition_runs() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = 1000
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    // 204 immediately: the transition runs on its own task rather than on this
    // request, so the claim is not held for the tens of seconds a real one
    // takes. The upload cannot succeed against a closed port, and a job that
    // cannot dispatch is logged and skipped rather than failing the claim.
    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The owner row is committed by the claim itself, so it is there the
    // moment the claim answers -- that is the property under test.
    let row: Option<(i32, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT attempts, completed_at
         FROM leave_generation_transitions WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    assert_eq!(row.expect("the owner row survives the claim"), (1, None));

    // Ownership comes back when the detached transition fails, which is a
    // moment later rather than before the claim answered.
    let released = wait_for(|| async {
        sqlx::query_scalar::<_, bool>(
            "SELECT started_at <= to_timestamp(0) FROM leave_generation_transitions
             WHERE job_id = $1 AND generation = 1",
        )
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    })
    .await;
    assert!(released, "a failed transition hands ownership back immediately");

    // And the next claim decision picks it up rather than waiting out the
    // takeover timeout.
    let step = next_step(&db, job).await;
    assert!(matches!(step, Step::Transition), "{step:?}");
}

/// Bug: a task left `available` by a lapsed claim was reissued *during* its
/// generation's transition.
///
/// A generation does not read as closed until the transition commits its
/// artifact row, and the transition takes tens of seconds -- streaming every
/// rack, deriving the leave values, uploading the KLV. Throughout that window
/// `current_generation` still named the closing generation, so the reissue that
/// runs before `next_step` handed the task straight back out. The worker played
/// it and its occurrences were folded into the very rows the transition was
/// reading, so the KLV it uploaded no longer reproduced from the database --
/// and a hash mismatch is the one signal `rebuild_artifacts` reserves for a
/// corrupted object.
#[tokio::test]
async fn no_task_is_issued_while_a_generations_transition_runs() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    // One task, then let its claim lapse, so a reissuable `available` task
    // exists for generation 1.
    let (status, first) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    sqlx::query("UPDATE task_claims SET claimed_at = now() - interval '1 hour'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(birdtest::scheduler::reclaim_expired(&db.pool, job, 300.0).await.unwrap(), 1);

    // The generation is complete and a transition owns it, exactly as the
    // claim that found it complete leaves things.
    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = 1000
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO leave_generation_transitions (job_id, generation) VALUES ($1, 1)",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a task of the closing generation was reissued mid-transition: {body}"
    );

    // A transition whose process died still gets taken over rather than
    // stalling the job forever, which is why the check uses the same bound the
    // takeover does.
    sqlx::query(
        "UPDATE leave_generation_transitions SET started_at = now() - interval '2 hours'
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let step = next_step(&db, job).await;
    assert!(matches!(step, Step::Transition), "past the takeover timeout: {step:?}");
}

/// A transition that outlived the job it was closing writes nothing.
///
/// Purge deletes the transitions row, the artifacts and the progress rows and
/// reseeds generation 1 -- all while a transition spawned before it may still
/// be streaming. Closing anyway would hand the purged job a generation-1 KLV
/// derived from results it no longer has, and copy a freshly zeroed universe
/// into generation 2, so the job would never do generation 1's work again.
#[tokio::test]
async fn a_transition_whose_job_was_purged_meanwhile_closes_nothing() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let config = sqlx::query_as::<_, birdtest::models::job::LeaveConfig>(
        "SELECT * FROM job_leave_config WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let sha256 = "1".repeat(64);
    let close = || {
        birdtest::jobs::leave_gen::close_generation(
            &db.pool,
            job,
            1,
            "leaves/test/generation-1.klv2",
            &sha256,
            &config,
        )
    };

    // With no ownership row at all -- what a purge leaves behind -- the close
    // is refused rather than inventing a closed generation.
    let err = close().await.expect_err("a transition with no owner row must not close");
    assert!(err.message.contains("purged"), "{}", err.message);
    let artifacts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leave_generation_artifacts WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(artifacts, 0, "nothing was written");
    let generations: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT generation FROM leave_rack_progress WHERE job_id = $1 ORDER BY 1",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(generations, vec![1], "nothing was written");

    // With the row this request owns, the same call closes the generation.
    sqlx::query("INSERT INTO leave_generation_transitions (job_id, generation) VALUES ($1, 1)")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    close().await.expect("the owner closes its own generation");
    let closed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leave_generation_artifacts WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(closed, 1);

    // And a second close of the same generation is refused too, so a taken-over
    // transition that turns out to have been finished cannot rewrite it.
    assert!(close().await.is_err());
}

/// Closing a generation does not write the next one's universe; the first claim
/// for that generation does.
///
/// It used to be part of the same transaction, which made closing a generation
/// a millions-of-rows write that every worker on the job waited out -- and one
/// the transition had to redo in full if anything failed, because the close and
/// the copy stood or fell together. Seeded from the claim path it happens while
/// workers are busy, under the job's lock so two claims cannot both do it, and
/// a failure costs a retry of the seeding alone.
#[tokio::test]
async fn the_next_generations_universe_is_seeded_by_a_claim_not_by_the_transition() {
    let db = TestDb::new().await;
    let (job, seeded) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);
    let config = sqlx::query_as::<_, birdtest::models::job::LeaveConfig>(
        "SELECT * FROM job_leave_config WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let universe = |generation: i32| {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = $1 AND generation = $2",
        )
        .bind(job)
        .bind(generation)
        .fetch_one(&db.pool)
    };

    sqlx::query("INSERT INTO leave_generation_transitions (job_id, generation) VALUES ($1, 1)")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    birdtest::jobs::leave_gen::close_generation(
        &db.pool,
        job,
        1,
        "leaves/test/generation-1.klv2",
        &"1".repeat(64),
        &config,
    )
    .await
    .unwrap();

    assert_eq!(universe(2).await.unwrap(), 0, "the transition wrote no rows");

    // The next claim finds generation 2 current, seeds its universe, and hands
    // out work from it.
    let (status, assignment) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    assert_eq!(assignment["task_request"]["generation"], json!(2));
    assert_eq!(universe(2).await.unwrap(), seeded, "the claim seeded it, in full");

    // And a second claim does not seed it again.
    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(universe(2).await.unwrap(), seeded);
}
