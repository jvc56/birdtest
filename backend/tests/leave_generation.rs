//! Leave generation's claim-time decisions and generation bookkeeping, against
//! a real database and nothing else (`I-LEAVE-3`, `-5`, `-7`, `-9`, `-10`, and
//! the `seed_generation` half of `I-DATA-2`).
//!
//! What needs a real MAGPIE or an object store -- building, uploading and
//! rebuilding a KLV -- is in `magpie_leave.rs`. Here a generation is closed
//! with `close_generation`, the half of a transition that runs after the
//! upload, under a made-up key and digest.

mod common;

use axum::http::StatusCode;
use birdtest::jobs::leave_gen::{self, LeaveGenStep};
use birdtest::models::job::LeaveConfig;
use common::*;
use serde_json::json;
use uuid::Uuid;

const TESTDIST: &[u8] = include_bytes!("../src/jobs/testdata/testdist.csv");

/// The generation-0 key `leave_job` records.
const GEN0_KEY: &str = "leaves/test/generation-0.klv2";

/// A leave-generation job over the test distribution (13 tiles, 149 full
/// racks), with its generation-1 universe seeded and a generation-0 artifact
/// row so it can dispatch. Copied from `leave_gen.rs`, with the settings these
/// tests vary as parameters.
async fn leave_job(
    db: &TestDb,
    racks_per_task: i32,
    target: i32,
    generation_count: i32,
    num_iterations: i32,
) -> (Uuid, i64) {
    let admin = db
        .user(&format!("admin{}", Uuid::new_v4().simple()), true)
        .await;
    let job = db.bare_job("leave_generation", 1, admin).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(job)
    .bind(kwg)
    .bind(num_iterations)
    .bind(generation_count)
    .bind(target)
    .bind(racks_per_task)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, 0, $2, repeat('0', 64), 'klv-1')",
    )
    .bind(job)
    .bind(GEN0_KEY)
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        db.derived_ready(job).await,
        1,
        "a leave job needs one wordmap"
    );
    let seeded = seed(db, job, 1).await;
    (job, seeded)
}

/// Seeds `generation`'s universe from the job's pinned distribution.
async fn seed(db: &TestDb, job: Uuid, generation: i32) -> i64 {
    let mut conn = db.pool.acquire().await.unwrap();
    let data = birdtest::jobs::load_job_data(&mut conn, job).await.unwrap();
    leave_gen::seed_generation(&mut conn, job, generation, &data.letterdist)
        .await
        .unwrap()
}

async fn config(db: &TestDb, job: Uuid) -> LeaveConfig {
    sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// Polls `condition` until it holds or a generous deadline passes, for work a
/// claim deliberately leaves to a spawned task (seeding a universe, a merge).
async fn wait_for<F, Fut>(mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..1200 {
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

/// One claim through the worker API: the status and the body.
async fn claim(app: &axum::Router) -> (StatusCode, serde_json::Value) {
    send(
        app,
        post_json("/api/worker/task", &[], claim_body("1.0.0", &[])),
    )
    .await
}

async fn claim_one(app: &axum::Router) -> serde_json::Value {
    let (status, body) = claim(app).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Submits `racks` as the result of `assignment`, each `(rack, count)`.
async fn submit(app: &axum::Router, assignment: &serde_json::Value, racks: &[(&str, i64)]) {
    let result = json!({ "racks": racks.iter().map(|(rack, count)| json!({
        "rack": rack, "count": count, "mean": 1.0,
    })).collect::<Vec<_>>() });
    let (status, accepted) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", assignment["worker_uuid"].as_str().unwrap())],
            json!({ "claim_token": assignment["claim_token"], "result": result }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["accepted"], true, "{accepted}");
}

/// What `next_step` decided.
#[derive(Debug, PartialEq)]
enum Step {
    Dispatch {
        generation: i32,
        racks: Vec<String>,
        previous_artifact_key: String,
    },
    Transition(i32),
    InProgress(i32),
    NeedsMerge(i32),
    NoWorkYet,
    Finished,
}

/// One claim decision for a leave job, through the same path a worker's
/// request takes -- the job's advisory lock, then `next_step`, in one
/// transaction that commits -- but without inserting a task for a dispatch.
async fn try_next_step(db: &TestDb, job: Uuid) -> birdtest::error::AppResult<Step> {
    let mut tx = db.pool.begin().await.unwrap();
    let config =
        sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
            .bind(job)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let job_data = birdtest::jobs::load_job_data(&mut tx, job).await.unwrap();
    let lexicon = leave_gen::lexicon_name(&mut tx, config.kwg_id)
        .await
        .unwrap();
    assert!(leave_gen::lock_claim_decisions(&mut tx, job).await.unwrap());
    let step = match leave_gen::next_step(&mut tx, job, &config, &job_data, &lexicon).await? {
        LeaveGenStep::Dispatch(request) => Step::Dispatch {
            generation: request.generation,
            racks: request.forced_racks,
            previous_artifact_key: request.previous_artifact_key,
        },
        LeaveGenStep::Transition { generation } => Step::Transition(generation),
        LeaveGenStep::TransitionInProgress { generation } => Step::InProgress(generation),
        LeaveGenStep::NeedsMerge { generation } => Step::NeedsMerge(generation),
        LeaveGenStep::NoWorkYet => Step::NoWorkYet,
        LeaveGenStep::Finished => Step::Finished,
    };
    tx.commit().await.unwrap();
    Ok(step)
}

async fn next_step(db: &TestDb, job: Uuid) -> Step {
    try_next_step(db, job).await.unwrap()
}

/// The generation's racks in primary-key order.
async fn racks_in_order(db: &TestDb, job: Uuid, generation: i32) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT rack FROM leave_rack_progress WHERE job_id = $1 AND generation = $2 ORDER BY rack",
    )
    .bind(job)
    .bind(generation)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn set_count(db: &TestDb, job: Uuid, generation: i32, rack: &str, count: i64) {
    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = $4
         WHERE job_id = $1 AND generation = $2 AND rack = $3",
    )
    .bind(job)
    .bind(generation)
    .bind(rack)
    .bind(count)
    .execute(&db.pool)
    .await
    .unwrap();
}

async fn set_all_counts(db: &TestDb, job: Uuid, generation: i32, count: i64) {
    sqlx::query(
        "UPDATE leave_rack_progress SET occurrence_count = $3 WHERE job_id = $1 AND generation = $2",
    )
    .bind(job)
    .bind(generation)
    .bind(count)
    .execute(&db.pool)
    .await
    .unwrap();
}

async fn count_of(db: &TestDb, job: Uuid, generation: i32, rack: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT occurrence_count FROM leave_rack_progress
         WHERE job_id = $1 AND generation = $2 AND rack = $3",
    )
    .bind(job)
    .bind(generation)
    .bind(rack)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

async fn transitions(db: &TestDb, job: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM leave_generation_transitions WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn job_status(db: &TestDb, job: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// Closes `generation` the way a transition that owns it does once its KLV is
/// uploaded: the owner row, then `close_generation`.
async fn close(db: &TestDb, job: Uuid, generation: i32, sha256: &str) {
    sqlx::query(
        "INSERT INTO leave_generation_transitions (job_id, generation) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(job)
    .bind(generation)
    .execute(&db.pool)
    .await
    .unwrap();
    leave_gen::close_generation(
        &db.pool,
        job,
        generation,
        &leave_gen::artifact_key(job, generation),
        sha256,
        "klv-1",
        &config(db, job).await,
    )
    .await
    .unwrap();
}

/// I-LEAVE-3: selection hands out the racks furthest below target first, and
/// never one that has reached it.
///
/// Every rack is at its 1000 target except eight: 5, 10, 20, four at 500, and
/// one a single occurrence short at 999. They are scattered through the
/// universe rather than first in primary-key order, so the order they come
/// out in is the counts' and not the table's. Two racks a task keeps the
/// 149-rack universe under the sweep threshold, which is the selection this is
/// about.
#[tokio::test]
async fn selection_hands_out_the_racks_furthest_below_target_first() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2, 1000, 2, 100).await;
    let order = racks_in_order(&db, job, 1).await;
    set_all_counts(&db, job, 1, 1000).await;
    let scattered = [
        (100, 5),
        (40, 10),
        (140, 20),
        (20, 500),
        (50, 500),
        (80, 500),
        (120, 500),
        (7, 999),
    ];
    for (i, count) in scattered {
        set_count(&db, job, 1, &order[i], count).await;
    }
    let app = birdtest::app(db.state().await);

    // Each claim is left open, so its racks are out when the next one selects.
    let mut by_claim = Vec::new();
    let mut handed_out = Vec::new();
    loop {
        let (status, body) = claim(&app).await;
        if status == StatusCode::NO_CONTENT {
            break;
        }
        assert_eq!(status, StatusCode::OK, "{body}");
        let racks = forced_racks(&body);
        let mut counts = Vec::new();
        for rack in &racks {
            counts.push(count_of(&db, job, 1, rack).await);
        }
        counts.sort();
        by_claim.push(counts);
        handed_out.extend(racks);
    }
    assert_eq!(
        by_claim,
        vec![vec![5, 10], vec![20, 500], vec![500, 500], vec![500, 999]],
        "lowest count first, claim after claim"
    );

    // Every rack below target, once, and none of the 141 at target.
    handed_out.sort();
    let mut below: Vec<String> = scattered.iter().map(|&(i, _)| order[i].clone()).collect();
    below.sort();
    assert_eq!(handed_out, below);
}

/// I-LEAVE-3: once every rack is at target, selection hands out nothing --
/// while a claim is in flight it waits, and with none in flight (and nothing
/// staged) the decision is the generation's transition, never a dispatch.
#[tokio::test]
async fn nothing_is_handed_out_once_every_rack_is_at_target() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2, 1000, 2, 100).await;
    let order = racks_in_order(&db, job, 1).await;
    set_all_counts(&db, job, 1, 1000).await;
    let short = &order[60];
    set_count(&db, job, 1, short, 999).await;
    // A claim handed fewer racks than a task holds asks for a merge on a
    // spawned task, which could land after the submission below and take the
    // decision this test makes by hand. Marking one as just started holds
    // those off for the minute they are rate-limited to.
    let state = db.state().await;
    assert!(state.leave_merges.due(job));
    let app = birdtest::app(state);

    // The one rack below target, alone: a task is not padded out with racks
    // that have reached it.
    let out = claim_one(&app).await;
    assert_eq!(forced_racks(&out), vec![short.clone()]);

    // Its claim is in flight: nothing to hand out, and no decision about the
    // generation.
    let (status, body) = claim(&app).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(next_step(&db, job).await, Step::NoWorkYet);
    assert_eq!(transitions(&db, job).await, 0);

    // Its result brings the rack to target. Staged, it is not yet in the
    // counts, so the claim asks for a merge rather than closing the generation.
    submit(&app, &out, &[(short, 1)]).await;
    assert_eq!(next_step(&db, job).await, Step::NeedsMerge(1));
    leave_gen::merge_staged(&db.pool, job, 1, true)
        .await
        .unwrap();
    assert_eq!(count_of(&db, job, 1, short).await, 1000);

    // Every rack at target, nothing in flight, nothing staged: nothing is
    // selected, and the generation closes.
    assert_eq!(next_step(&db, job).await, Step::Transition(1));
    assert_eq!(next_step(&db, job).await, Step::InProgress(1));
    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(tasks, 1, "no task beyond the one that was needed");
}

/// I-LEAVE-7: every dispatched generation carries the key of the KLV its
/// predecessor closed with -- generation 1 included, which reads the zeroed
/// generation-0 KLV rather than having no key -- and a job with no
/// predecessor KLV to name dispatches nothing rather than a task without one.
#[tokio::test]
async fn every_dispatched_generation_carries_its_predecessors_klv() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2, 1000, 3, 100).await;
    let app = birdtest::app(db.state().await);

    let first = claim_one(&app).await;
    assert_eq!(first["task_request"]["generation"], json!(1));
    assert_eq!(
        first["task_request"]["previous_artifact_key"],
        json!(GEN0_KEY)
    );
    submit(&app, &first, &[(&forced_racks(&first)[0], 3)]).await;
    leave_gen::merge_staged(&db.pool, job, 1, true)
        .await
        .unwrap();

    // Generation 1 closes; generation 2's tasks name its KLV.
    set_all_counts(&db, job, 1, 1000).await;
    assert_eq!(next_step(&db, job).await, Step::Transition(1));
    close(&db, job, 1, &"1".repeat(64)).await;
    seed(&db, job, 2).await;
    let second = claim_one(&app).await;
    assert_eq!(second["task_request"]["generation"], json!(2));
    assert_eq!(
        second["task_request"]["previous_artifact_key"],
        json!(leave_gen::artifact_key(job, 1))
    );

    // Every stored request names exactly its predecessor's recorded key.
    let (requests, wrong): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),
                COUNT(*) FILTER (WHERE a.artifact_key IS DISTINCT FROM r.previous_artifact_key)
         FROM leave_requests r
         JOIN tasks t ON t.id = r.task_id
         LEFT JOIN leave_generation_artifacts a
                ON a.job_id = t.job_id AND a.generation = r.generation - 1
         WHERE t.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((requests, wrong), (2, 0));

    // A job whose generation-0 KLV is missing has nothing to play generation 1
    // with: the decision is an error naming that, and a claim gets nothing.
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2, 1000, 3, 100).await;
    sqlx::query("DELETE FROM leave_generation_artifacts WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let err = try_next_step(&db, job)
        .await
        .expect_err("dispatched with no generation-0 KLV");
    assert!(
        err.message.contains("no generation-0 KLV"),
        "{}",
        err.message
    );
    let (status, body) = claim(&birdtest::app(db.state().await)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let tasks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(tasks, 0, "no task was written without a KLV to name");
}

/// I-LEAVE-9: a job of two generations closes the first and goes on to the
/// second -- still active, dispatching generation-2 tasks once its universe is
/// seeded -- and closing the second, its last, completes the job, after which
/// there is nothing to claim.
#[tokio::test]
async fn a_two_generation_job_advances_and_finishes_after_its_last() {
    let db = TestDb::new().await;
    let (job, seeded) = leave_job(&db, 2, 1000, 2, 100).await;
    let app = birdtest::app(db.state().await);
    let current = || async {
        let mut conn = db.pool.acquire().await.unwrap();
        leave_gen::current_generation(&mut conn, job, &config(&db, job).await)
            .await
            .unwrap()
    };
    assert_eq!(current().await, Some(1));

    set_all_counts(&db, job, 1, 1000).await;
    assert_eq!(next_step(&db, job).await, Step::Transition(1));
    close(&db, job, 1, &"1".repeat(64)).await;
    assert_eq!(current().await, Some(2), "generation 2 is open");
    assert_eq!(
        job_status(&db, job).await,
        "active",
        "one generation of two is not the job"
    );

    // Generation 2's universe is seeded by the first claim to find it missing,
    // and then its tasks flow.
    let (status, body) = claim(&app).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let universe = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM leave_rack_progress WHERE job_id = $1 AND generation = 2",
        )
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    assert!(
        wait_for(|| async { universe().await == seeded }).await,
        "generation 2 was seeded"
    );
    let task = claim_one(&app).await;
    assert_eq!(task["task_request"]["generation"], json!(2));
    let racks = forced_racks(&task);
    submit(&app, &task, &[(&racks[0], 1000), (&racks[1], 1000)]).await;
    leave_gen::merge_staged(&db.pool, job, 2, true)
        .await
        .unwrap();

    set_all_counts(&db, job, 2, 1000).await;
    assert_eq!(next_step(&db, job).await, Step::Transition(2));
    close(&db, job, 2, &"2".repeat(64)).await;
    assert_eq!(
        job_status(&db, job).await,
        "completed",
        "the last generation completes the job"
    );
    assert_eq!(current().await, None);
    assert_eq!(next_step(&db, job).await, Step::Finished);

    let (status, body) = claim(&app).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a finished job hands out nothing: {body}"
    );
    let generations: Vec<i32> = sqlx::query_scalar(
        "SELECT generation FROM leave_generation_artifacts WHERE job_id = $1 ORDER BY 1",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        generations,
        vec![0, 1, 2],
        "no third generation was started"
    );
}

/// Every key anywhere in `value`, and every number or string it holds, for
/// searching an assignment for something it must not carry.
fn walk(value: &serde_json::Value, keys: &mut Vec<String>, leaves: &mut Vec<serde_json::Value>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                keys.push(key.clone());
                walk(value, keys, leaves);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, keys, leaves)),
        other => leaves.push(other.clone()),
    }
}

/// I-LEAVE-10: a task stops after its `num_games` and nothing else -- the
/// generation's rack target is server-only state and is not in the assignment
/// at all, under any name or as any value.
///
/// The target (7919) and game count (37) are numbers nothing else in an
/// assignment would be, so finding either anywhere would mean it was sent.
#[tokio::test]
async fn the_rack_target_is_not_sent_to_the_worker() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2, 7919, 2, 37).await;
    let app = birdtest::app(db.state().await);
    let assignment = claim_one(&app).await;

    let request = assignment["task_request"].as_object().unwrap();
    let mut fields: Vec<&str> = request.keys().map(String::as_str).collect();
    fields.sort();
    assert_eq!(
        fields,
        vec![
            "bingo_bonus",
            "board_layout",
            "forced_racks",
            "generation",
            "job_type",
            "letter_distribution",
            "lexicon",
            "num_games",
            "previous_artifact_key",
            "previous_artifact_sha256",
            "seed",
            "use_wordmap",
            "variant",
        ],
        "a leave task's request is exactly these fields"
    );
    assert_eq!(request["num_games"], json!(37), "the job's num_iterations");

    let (mut keys, mut leaves) = (Vec::new(), Vec::new());
    walk(&assignment, &mut keys, &mut leaves);
    // `build_target` is the CPU target a derived file was built for, which
    // every assignment with a wordmap carries; nothing else may say "target".
    assert!(
        !keys
            .iter()
            .any(|key| key.contains("target") && key != "build_target"),
        "the assignment names a target: {keys:?}"
    );
    assert!(
        !leaves
            .iter()
            .any(|v| *v == json!(7919) || *v == json!("7919")),
        "the rack target reached the worker: {assignment}"
    );

    let stored: i32 = sqlx::query_scalar(
        "SELECT r.num_games FROM leave_requests r JOIN tasks t ON t.id = r.task_id WHERE t.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        stored, 37,
        "the stored request, reissued as it stands, says the same"
    );
}

/// I-LEAVE-5: the artifact row keeps the **first** digest written for a
/// generation. A second close of the same generation -- a transition replayed
/// after a restore that reopened its owner row, against fewer results and so
/// different bytes under the same key -- commits, but leaves the recorded
/// digest and builder as they were, so the mismatch stays visible to a rebuild
/// rather than being quietly agreed with.
#[tokio::test]
async fn a_generations_first_digest_is_the_one_kept() {
    let db = TestDb::new().await;
    let (job, _) = leave_job(&db, 2, 1000, 2, 100).await;
    let first = "a".repeat(64);
    close(&db, job, 1, &first).await;

    // The owner row reopened, as a restore of a mid-transition backup leaves
    // it; the replay owns it and closes with different bytes and builder.
    sqlx::query(
        "UPDATE leave_generation_transitions SET completed_at = NULL
         WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    leave_gen::close_generation(
        &db.pool,
        job,
        1,
        &leave_gen::artifact_key(job, 1),
        &"b".repeat(64),
        "klv-2",
        &config(&db, job).await,
    )
    .await
    .expect("the replay's close commits");

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT sha256, builder FROM leave_generation_artifacts WHERE job_id = $1 AND generation = 1",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![(first, "klv-1".to_string())],
        "the first digest and builder stand"
    );
}

/// A letter distribution row named `name` with exactly `content`.
async fn letterdist(db: &TestDb, name: &str, content: &[u8]) -> Uuid {
    use sha2::Digest;
    sqlx::query_scalar(
        "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
         VALUES ($1, 'letterdist', $2, $3, $4, '20251004', $5) RETURNING id",
    )
    .bind(format!("letterdistributions/{name}.csv"))
    .bind(name)
    .bind(hex::encode(sha2::Sha256::digest(content)))
    .bind(content.len() as i64)
    .bind(content)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// I-DATA-2 (the `seed_generation` half; the MAGPIE conversions are in
/// `magpie_leave.rs`): two jobs pinning two different distributions that share
/// one name seed two different universes, each exactly its own distribution's
/// full racks. Seeding reads the pinned row's bytes, not the name.
#[tokio::test]
async fn seeding_reads_the_jobs_pinned_distribution_not_its_name() {
    let db = TestDb::new().await;
    let (job_a, _) = leave_job(&db, 2, 1000, 2, 100).await;
    let (job_b, _) = leave_job(&db, 2, 1000, 2, 100).await;
    // One more A: the same name, a larger bag.
    let text = std::str::from_utf8(TESTDIST)
        .unwrap()
        .replacen("A,a,3,", "A,a,4,", 1);
    assert_ne!(text.as_bytes(), TESTDIST);
    let (ld_a, ld_b) = (
        letterdist(&db, "english", TESTDIST).await,
        letterdist(&db, "english", text.as_bytes()).await,
    );
    for (job, ld) in [(job_a, ld_a), (job_b, ld_b)] {
        sqlx::query("UPDATE jobs SET letterdist_id = $2 WHERE id = $1")
            .bind(job)
            .bind(ld)
            .execute(&db.pool)
            .await
            .unwrap();
        // Reseeded under the pin just set.
        sqlx::query("DELETE FROM leave_rack_progress WHERE job_id = $1")
            .bind(job)
            .execute(&db.pool)
            .await
            .unwrap();
        seed(&db, job, 1).await;
    }

    let expected = |bytes: &[u8]| {
        let mut racks = birdtest::jobs::racks::LetterDistribution::parse(bytes, "english")
            .unwrap()
            .enumerate_racks(7);
        racks.sort();
        racks
    };
    let (mut a, mut b) = (
        racks_in_order(&db, job_a, 1).await,
        racks_in_order(&db, job_b, 1).await,
    );
    a.sort();
    b.sort();
    assert_eq!(a, expected(TESTDIST));
    assert_eq!(b, expected(text.as_bytes()));
    assert_ne!(a.len(), b.len(), "one name, two universes");
    assert!(b.contains(&"AAAABBC".to_string()) && !a.contains(&"AAAABBC".to_string()));
}
