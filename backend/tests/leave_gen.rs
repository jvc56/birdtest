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
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 0, 'leaves/test/generation-0.klv2', repeat('0', 64), 'klv-1')",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    // A leave job's bot plays with a wordmap by default, and a job whose
    // derived files are not built is not dispatched. Creation through the API
    // queues those builds; this job was assembled with plain SQL, so the gate
    // is satisfied here the same way the generation-0 artifact row above is.
    assert_eq!(db.derived_ready(job).await, 1, "a leave job needs one wordmap");

    let row = sqlx::query_as::<_, birdtest::models::job::Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let job_data = birdtest::jobs::load_job_data(&mut conn, row.id).await.unwrap();
    let seeded = birdtest::jobs::leave_gen::seed_generation(&mut conn, job, 1, &job_data.letterdist)
        .await
        .unwrap();
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

/// A submission is staged, and a merge adds it to its racks' rows; a rack that
/// is not a full rack of the distribution neither counts nor creates a row. The
/// submission itself touches no per-rack row: folding in the submit transaction
/// was seconds of random-access writes and hundreds of megabytes of WAL.
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

    let progress = || async {
        sqlx::query_as::<_, (i64, f64)>(
            "SELECT occurrence_count, equity_sum FROM leave_rack_progress
             WHERE job_id = $1 AND generation = 1 AND rack = $2",
        )
        .bind(job)
        .bind(&racks[0])
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    // Accepted and staged, with the generation's live counters moved -- and
    // no per-rack row written by the submission.
    assert_eq!(progress().await, (0, 0.0));
    let staged: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(staged, 1);
    let live: (i64, i64) = sqlx::query_as(
        "SELECT tasks_completed, games_played FROM leave_generation_progress
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(live, (1, 100), "one task of the job's 100 games");

    let merged = birdtest::jobs::leave_gen::merge_staged(&db.pool, job, 1, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((merged.folds_merged, merged.racks_updated), (1, 2), "ZZZZZZZ updates nothing");
    assert_eq!(progress().await, (3, 30.0));
    let staged: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(staged, 0, "a merged result is no longer staged");
    // A second merge finds nothing and changes nothing.
    let again = birdtest::jobs::leave_gen::merge_staged(&db.pool, job, 1, true).await.unwrap().unwrap();
    assert_eq!(again.folds_merged, 0);
    assert_eq!(progress().await, (3, 30.0));

    // The summary the dashboard reads was refreshed by the merge.
    let summary: (i64, i64, Option<i64>, bool) = sqlx::query_as(
        "SELECT racks_total, racks_at_target, min_rack_count, merged_at IS NOT NULL
         FROM leave_generation_progress WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(summary, (seeded, 0, Some(0), true));

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
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 1, 'leaves/test/generation-1.klv2', repeat('1', 64), 'klv-1')",
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
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 1, 'leaves/test/generation-1.klv2', repeat('1', 64), 'klv-1')",
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
    // Not by waiting for a merge, either: nothing was staged for it.
    let staged: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = $1")
            .bind(job)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(staged, 0, "a result for a closed generation is not staged");

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
    NeedsMerge,
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
    let lexicon = leave_gen::lexicon_name(&mut tx, config.kwg_id).await.unwrap();
    leave_gen::lock_claim_decisions(&mut tx, job).await.unwrap();
    let step = leave_gen::next_step(&mut tx, job, &config, &job_data, &lexicon).await.unwrap();
    let step = match step {
        LeaveGenStep::Transition { .. } => Step::Transition,
        LeaveGenStep::TransitionInProgress { .. } => Step::InProgress,
        LeaveGenStep::NeedsMerge { .. } => Step::NeedsMerge,
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
            "klv-1",
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

/// Closing a generation does not write the next one's universe, and neither
/// does the claim that finds it missing: that claim starts the seeding on its
/// own task and answers straight away.
///
/// It used to be part of the transition's transaction, which made closing a
/// generation a millions-of-rows write every worker on the job waited out. It
/// then moved into the first claim for the new generation -- still inside a
/// request, where a client that gave up rolled it back. MAGPIE gives up after
/// 120 seconds, so on a database slower than that at seeding 3.2 million rows
/// every claim restarted the seeding and none finished.
#[tokio::test]
async fn the_next_generations_universe_is_seeded_off_the_claim_path() {
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
        "klv-1",
        &config,
    )
    .await
    .unwrap();

    assert_eq!(universe(2).await.unwrap(), 0, "the transition wrote no rows");

    // The next claim finds generation 2 current and its universe missing. It
    // is answered at once with nothing to do rather than held for the seeding.
    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The seeding it started finishes on its own, in full.
    let seeded_in_full = wait_for(|| async { universe(2).await.unwrap() == seeded }).await;
    assert!(seeded_in_full, "the detached seeding wrote the whole universe");

    // Then work flows from generation 2, and nothing seeds it a second time.
    let (status, assignment) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    assert_eq!(assignment["task_request"]["generation"], json!(2));
    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(universe(2).await.unwrap(), seeded);
}

/// A leave task carries a seed, chosen when the task is created and replayed
/// when the task is reissued.
///
/// Before, the request had none and MAGPIE seeded the task's games from its own
/// process state, so what a task played depended on which machine ran it.
#[tokio::test]
async fn a_leave_task_carries_its_seed_and_a_reissue_replays_it() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let (status, first) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let seed = first["task_request"]["seed"].as_str().expect("a decimal-string seed");
    seed.parse::<u64>().expect("a uint64");

    sqlx::query("UPDATE task_claims SET claimed_at = now() - interval '1 hour'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(birdtest::scheduler::reclaim_expired(&db.pool, job, 300.0).await.unwrap(), 1);

    let (status, again) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["task_request"]["seed"], first["task_request"]["seed"]);

    // A new task of the same generation gets a seed of its own.
    let (status, other) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{other}");
    assert_ne!(other["task_request"]["seed"], first["task_request"]["seed"]);
}

/// With redundancy above 1 every claim of a leave task replays the same seed,
/// so only the first accepted result folds into the generation; the others are
/// credited and nothing more.
#[tokio::test]
async fn only_the_first_result_for_a_leave_task_is_folded() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    sqlx::query("UPDATE jobs SET redundancy = 2 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let mut claims = Vec::new();
    for _ in 0..2 {
        let (status, body) =
            send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        claims.push(body);
    }
    assert_eq!(
        claims[0]["task_request"]["seed"], claims[1]["task_request"]["seed"],
        "both slots of one task: {claims:?}"
    );
    let rack = forced_racks(&claims[0])[0].clone();

    for claim in &claims {
        let (status, body) = send(
            &app,
            post_json(
                "/api/worker/result",
                &[("x-worker-uuid", claim["worker_uuid"].as_str().unwrap())],
                json!({ "claim_token": claim["claim_token"], "result": { "racks": [
                    { "rack": rack, "count": 3, "mean": 10.0 }
                ]}}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["accepted"], json!(true));
    }

    birdtest::jobs::leave_gen::merge_staged(&db.pool, job, 1, true).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT occurrence_count FROM leave_rack_progress
         WHERE job_id = $1 AND generation = 1 AND rack = $2",
    )
    .bind(job)
    .bind(&rack)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(count, 3, "the same games are counted once, not once per claim");

    let credited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leave_records r JOIN tasks t ON t.id = r.task_id WHERE t.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(credited, 2, "both claims are credited");
}

/// Generation 1's universe is seeded by the first claim, like every later
/// generation's: creating or purging a job writes none of it, so neither holds
/// its request, or a purge its locks, for the tens of seconds 3.2 million rows
/// take.
#[tokio::test]
async fn generation_ones_universe_is_seeded_by_the_first_claim_too() {
    let db = TestDb::new().await;
    let (job, seeded) = leave_job(&db, 2).await;
    // What creation and purge now leave behind.
    sqlx::query("DELETE FROM leave_rack_progress WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let app = birdtest::app(db.state().await);

    let (status, _) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let universe = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = $1 AND generation = 1",
        )
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    assert!(wait_for(|| async { universe().await == seeded }).await, "seeded in full");

    let (status, assignment) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    assert_eq!(assignment["task_request"]["generation"], json!(1));
}

/// Submissions for different tasks of one generation overlap on every commonly
/// drawn rack. When each folded itself into `leave_rack_progress` they
/// contended for those rows -- first by deadlocking (an UPDATE locks rows in
/// whatever order its plan visits them), then, once the rows were locked in
/// rack order, by queueing behind each other for the seconds a fold took. A
/// submission now appends one staged row and touches no per-rack row at all.
/// The one thing two of them share is the generation's counter row, a
/// single-row update the second waits for exactly as it waits for the job's own
/// counters a statement later -- so with both transactions open at once and
/// the racks named in opposite orders, neither fails, and the merge that
/// follows counts both.
#[tokio::test]
async fn overlapping_leave_submissions_do_not_wait_on_each_other() {
    use birdtest::jobs::handler::{JobHandler, LeaveRecord, RackOccurrence};
    use birdtest::jobs::leave_gen::LeaveGenHandler;

    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let mut claims = Vec::new();
    for _ in 0..2 {
        let (status, body) =
            send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let token: Uuid = body["claim_token"].as_str().unwrap().parse().unwrap();
        let ids: (Uuid, Uuid) =
            sqlx::query_as("SELECT id, task_id FROM task_claims WHERE claim_token = $1")
                .bind(token)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        claims.push(ids);
    }
    let racks: Vec<String> = sqlx::query_scalar(
        "SELECT rack FROM leave_rack_progress WHERE job_id = $1 AND generation = 1
         ORDER BY rack LIMIT 2",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let (low, high) = (racks[0].clone(), racks[1].clone());
    let occurrence = |rack: &str| RackOccurrence { rack: rack.to_string(), count: 1, mean: 2.0 };

    let job_row = sqlx::query_as::<_, birdtest::models::job::Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let template = birdtest::jobs::dispatch::JobTemplate::load(&mut conn, &job_row).await.unwrap();
    drop(conn);

    // Both transactions open at once, naming the same racks in opposite
    // orders -- the shape that used to be a lock cycle.
    let mut a = db.pool.begin().await.unwrap();
    LeaveGenHandler::insert_record(
        &mut a, &template, claims[0].1, claims[0].0,
        &LeaveRecord { racks: vec![occurrence(&low), occurrence(&high)] },
    )
    .await
    .unwrap();

    let mut b = db.pool.begin().await.unwrap();
    let b_record = LeaveRecord { racks: vec![occurrence(&high), occurrence(&low)] };
    let b_stage = async {
        LeaveGenHandler::insert_record(&mut b, &template, claims[1].1, claims[1].0, &b_record).await
    };
    let a_finish = async move {
        // Long enough for B to have reached whatever it is going to wait on.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        a.commit().await
    };
    let (b_result, a_result) = tokio::join!(b_stage, a_finish);
    a_result.expect("A must not be chosen as a deadlock victim");
    b_result.expect("B must not be chosen as a deadlock victim");
    b.commit().await.unwrap();

    let merged = birdtest::jobs::leave_gen::merge_staged(&db.pool, job, 1, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(merged.folds_merged, 2);

    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT rack, occurrence_count FROM leave_rack_progress
         WHERE job_id = $1 AND generation = 1 AND rack = ANY($2) ORDER BY rack",
    )
    .bind(job)
    .bind(vec![low.clone(), high.clone()])
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(counts, vec![(low, 2), (high, 2)]);
}

/// Claims one leave task and submits `count` occurrences of each rack it was
/// forced, returning those racks.
async fn play_one_task(app: &axum::Router, count: i64) -> Vec<String> {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let racks = forced_racks(&body);
    let result = json!({ "racks": racks.iter().map(|rack| json!({
        "rack": rack, "count": count, "mean": 1.0,
    })).collect::<Vec<_>>() });
    let (status, accepted) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", body["worker_uuid"].as_str().unwrap())],
            json!({ "claim_token": body["claim_token"], "result": result }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    racks
}

/// Until a merge, `leave_rack_progress` still shows a finished task's racks at
/// the counts they had before it played -- so, ordered on those counts, they
/// are the lowest in the generation the moment their claim completes. Handed
/// out on that basis they would go to every claim until the next merge, while
/// racks nobody has forced yet waited. They are held out of selection while
/// their task's result is staged.
#[tokio::test]
async fn racks_of_a_staged_result_are_not_handed_out_again_before_the_merge() {
    let db = TestDb::new().await;
    let (_job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    let first = play_one_task(&app, 5).await;
    let second = play_one_task(&app, 5).await;
    assert!(
        first.iter().all(|rack| !second.contains(rack)),
        "{first:?} were forced again as {second:?} with their result still staged"
    );
}

/// A generation must not close on totals that are missing staged results: the
/// racks those tasks forced are held out of selection, so "nothing left to hand
/// out" does not yet mean "every rack is at target". The claim asks for a merge
/// instead, and the decision is made on exact figures afterwards.
#[tokio::test]
async fn a_generation_does_not_close_with_results_still_staged() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let app = birdtest::app(db.state().await);

    // Every rack at target except the two the first task forces, which its
    // result then leaves short.
    let racks = {
        let (status, body) =
            send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let racks = forced_racks(&body);
        sqlx::query(
            "UPDATE leave_rack_progress SET occurrence_count = 1000
             WHERE job_id = $1 AND generation = 1 AND NOT (rack = ANY($2))",
        )
        .bind(job)
        .bind(&racks)
        .execute(&db.pool)
        .await
        .unwrap();
        let (status, accepted) = send(
            &app,
            post_json(
                "/api/worker/result",
                &[("x-worker-uuid", body["worker_uuid"].as_str().unwrap())],
                json!({ "claim_token": body["claim_token"], "result": { "racks": [
                    { "rack": racks[0], "count": 1000, "mean": 1.0 },
                    { "rack": racks[1], "count": 10, "mean": 1.0 },
                ]}}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{accepted}");
        racks
    };

    // Nothing to hand out and nothing in flight -- and not complete.
    let step = next_step(&db, job).await;
    assert!(matches!(step, Step::NeedsMerge), "{step:?}");
    let owned: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM leave_generation_transitions WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(owned, 0, "no transition was started on stale totals");

    // Through the worker API the same claim is a `204`, and starts the merge.
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let pool = db.pool.clone();
    assert!(
        wait_for(|| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = $1")
                    .bind(job)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
                    == 0
            }
        })
        .await,
        "the claim's merge never landed"
    );

    // On exact figures the second rack is still short, so it is forced again
    // rather than the generation closing around it.
    let (status, body) =
        send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(forced_racks(&body), vec![racks[1].clone()]);
}

/// A purge takes the staged results and the generations' summaries with
/// everything else, so nothing accepted for the old run is folded into the new
/// one.
#[tokio::test]
async fn a_purge_discards_staged_results() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2).await;
    let cfg = db.config();
    let admin = db.user("root", true).await;
    let app = birdtest::app(db.state().await);
    play_one_task(&app, 5).await;

    let mut request = axum::http::Request::post(format!("/api/admin/jobs/{job}/purge"));
    for (name, value) in admin_headers(&cfg, admin) {
        request = request.header(name, value);
    }
    let (status, body) = send(&app, request.body(axum::body::Body::empty()).unwrap()).await;
    // The purge itself commits; rebuilding the generation-0 KLV afterwards
    // needs an object store, which these tests do not have.
    assert!(status == StatusCode::OK || status.is_server_error(), "{status}: {body}");

    let left: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM leave_rack_staging WHERE job_id = $1),
                (SELECT COUNT(*) FROM leave_generation_progress WHERE job_id = $1)",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(left, (0, 0));
}
