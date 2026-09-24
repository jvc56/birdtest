//! Dashboard statistics against a real database (`jobstats`): what a job's
//! stats sum and what SPRT is run over, leave-generation progress, who
//! contributed, the ETA, and the finish check a submission runs.
//!
//! Results are written with plain SQL -- a task, a claim and its result -- for
//! everything but the finish check, which only a submission runs.

mod common;

use axum::http::StatusCode;
use birdtest::jobstats;
use birdtest::stats::sprt::{self, Pentanomial, Sample, SprtStatus, Tally};
use common::*;
use serde_json::json;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Who a claim belongs to.
#[derive(Clone, Copy)]
enum Owner {
    User(Uuid),
    Anon(Uuid),
}

async fn anon(db: &TestDb) -> Uuid {
    let uuid = Uuid::new_v4();
    sqlx::query("INSERT INTO anonymous_workers (uuid) VALUES ($1)")
        .bind(uuid)
        .execute(&db.pool)
        .await
        .unwrap();
    uuid
}

/// A task of its own for `job` and one claim of it by `owner` in `state`,
/// completed `minutes_ago` when the state is `completed`. Returns the task and
/// the claim.
async fn claim(
    db: &TestDb,
    job: Uuid,
    owner: Owner,
    state: &str,
    minutes_ago: i32,
) -> (Uuid, Uuid) {
    let task_state = if state == "completed" { "completed" } else { "available" };
    let task: Uuid = sqlx::query_scalar(
        "INSERT INTO tasks (job_id, seed, state, accepted_count)
         SELECT $1, COALESCE(MAX(seed), 0) + 1, $2::task_state, ($2 = 'completed')::int
         FROM tasks WHERE job_id = $1
         RETURNING id",
    )
    .bind(job)
    .bind(task_state)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let (user, anon) = match owner {
        Owner::User(id) => (Some(id), None),
        Owner::Anon(uuid) => (None, Some(uuid)),
    };
    let claim: Uuid = sqlx::query_scalar(
        "INSERT INTO task_claims
             (task_id, claim_token, state, claimed_by_user_id, claimed_by_anon_uuid, completed_at)
         VALUES ($1, gen_random_uuid(), $2::claim_state, $3, $4,
                 CASE WHEN $2 = 'completed'
                      THEN now() - make_interval(mins => $5) END)
         RETURNING id",
    )
    .bind(task)
    .bind(state)
    .bind(user)
    .bind(anon)
    .bind(minutes_ago)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    (task, claim)
}

/// A completed batch for `job`: `(wins, losses, ties)` over every game, and
/// for a paired batch its pentanomial and the divergent subset's
/// `(wins, losses, ties)`.
async fn result(
    db: &TestDb,
    job: Uuid,
    tally: (i32, i32, i32),
    pent: Option<[i32; 5]>,
    divergent: Option<(i32, i32, i32)>,
) {
    let owner = Owner::Anon(anon(db).await);
    let (task, claim) = claim(db, job, owner, "completed", 0).await;
    let (wins, losses, ties) = tally;
    let divergent = divergent.map(|(w, l, t)| vec![w + l + t, w, l, t]);
    sqlx::query(
        "INSERT INTO game_results
             (task_claim_id, task_id, job_id, games, wins, losses, ties,
              p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd,
              pent_0, pent_1, pent_2, pent_3, pent_4,
              divergent_games, divergent_wins, divergent_losses, divergent_ties)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 420, 60, 410, 58,
                 $8[1], $8[2], $8[3], $8[4], $8[5], $9[1], $9[2], $9[3], $9[4])",
    )
    .bind(claim)
    .bind(task)
    .bind(job)
    .bind(wins + losses + ties)
    .bind(wins)
    .bind(losses)
    .bind(ties)
    .bind(pent.map(|p| p.to_vec()))
    .bind(divergent)
    .execute(&db.pool)
    .await
    .unwrap();
}

/// A paired batch: the per-game tally is derived from the pentanomial, a split
/// pair booked as a win and a loss and a 1.5-0.5 pair as a win and a tie.
async fn pair_result(db: &TestDb, job: Uuid, pent: [i32; 5], divergent: (i32, i32, i32)) {
    let [p0, p1, p2, p3, p4] = pent;
    let tally = (p2 + p3 + 2 * p4, 2 * p0 + p1 + p2, p1 + p3);
    result(db, job, tally, Some(pent), Some(divergent)).await;
}

/// An active `game_pairs` job at the schema's default SPRT settings.
async fn pairs_job(db: &TestDb) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let p1 = db.static_player("p1", admin).await;
    let p2 = db.static_player("p2", admin).await;
    let job = db.bare_job("game_pairs", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 8, 1000000, 1000000)",
    )
    .bind(job)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

async fn game_stats(db: &TestDb, job: Uuid) -> jobstats::GameStats {
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    jobstats::game_stats(&db.pool, &row).await.unwrap().expect("a games or pairs job")
}

/// `got` equals a value computed independently of the code under test.
fn close(got: f64, want: f64) {
    assert!((got - want).abs() < 1e-12, "{got} != {want}");
}

async fn stats(db: &TestDb, job: Uuid) -> jobstats::JobStats {
    let row = jobstats::load_job(&db.pool, job).await.unwrap();
    jobstats::compute(&db.pool, &row).await.unwrap()
}

// ---------------------------------------------------------------------------
// What the stats sum, and what SPRT reads
// ---------------------------------------------------------------------------

/// I-STATS-1: a `games` job's stats sum every task's result, and SPRT runs
/// over those games -- one observation each, at the job's own bounds.
#[tokio::test]
async fn a_games_jobs_stats_sum_every_result_and_test_the_games() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 10).await;
    result(&db, job, (7, 2, 1), None, None).await;
    result(&db, job, (4, 5, 1), None, None).await;
    result(&db, job, (10, 0, 0), None, None).await;

    let games = game_stats(&db, job).await;
    assert_eq!(games.unit, "game");
    assert_eq!((games.wins, games.losses, games.draws), (21, 7, 2));
    assert_eq!(games.units_completed, 30);
    assert_eq!((games.pentanomial, games.divergent_pairs), (None, None));
    assert_eq!((games.min_units, games.max_units), (1_000_000, 1_000_000));
    assert_eq!(games.win_pct, 70.0);

    let tally = Tally { wins: 21, losses: 7, draws: 2 };
    let expected = sprt::llr(&Sample::from_games(&tally), -10.0, 10.0);
    assert!(expected > 0.0, "a lopsided sample has something to say");
    assert_eq!(games.sprt.llr, expected);
    assert_eq!((games.sprt.lower_bound, games.sprt.upper_bound), sprt::bounds(0.05, 0.05));
    assert_eq!(games.sprt.status, SprtStatus::Running, "below min_games");
    // And the numbers themselves, computed outside the code from PLAN.md's
    // formulas, so a job that read the right rows through the wrong maths
    // fails here too.
    close(games.sprt.llr, 1.125_953_543_425_981_8);
    close(games.sprt.upper_bound, 2.944_438_979_166_440_5);
    close(games.sprt.lower_bound, -2.944_438_979_166_440_5);
}

/// Two paired batches: 16 pairs, 7 of which diverged.
async fn two_paired_batches(db: &TestDb) -> Uuid {
    let job = pairs_job(db).await;
    pair_result(db, job, [1, 2, 3, 1, 1], (5, 1, 0)).await;
    pair_result(db, job, [0, 1, 4, 2, 1], (6, 2, 0)).await;
    job
}

/// I-STATS-2: a `game_pairs` job's stats sum the pentanomial, and its unit
/// count is the pairs played -- every pair, not the seven that diverged.
#[tokio::test]
async fn a_pairs_jobs_stats_sum_the_pentanomial_and_count_every_pair() {
    let db = TestDb::new().await;
    let job = two_paired_batches(&db).await;

    let games = game_stats(&db, job).await;
    assert_eq!(games.unit, "pair");
    assert_eq!(games.pentanomial, Some([1, 3, 7, 3, 2]));
    assert_eq!(games.units_completed, 16, "every pair, not the divergent ones");
    assert_eq!(games.wins + games.losses + games.draws, 32, "two games a pair");
}

/// I-STATS-3: the divergent pairs are reported, and are not the sample: the
/// LLR is the pentanomial's, which differs from what the divergent games alone
/// would give.
#[tokio::test]
async fn divergent_pairs_are_reported_but_not_tested() {
    let db = TestDb::new().await;
    let job = two_paired_batches(&db).await;

    let games = game_stats(&db, job).await;
    assert_eq!(games.divergent_pairs, Some(7), "fourteen divergent games");
    let over_pairs =
        sprt::llr(&Sample::from_pentanomial(&Pentanomial { counts: [1, 3, 7, 3, 2] }), -10.0, 10.0);
    let over_divergent =
        sprt::llr(&Sample::from_games(&Tally { wins: 11, losses: 3, draws: 0 }), -10.0, 10.0);
    assert_eq!(games.sprt.llr, over_pairs);
    assert!(
        (over_pairs - over_divergent).abs() > 0.1,
        "the two samples must disagree for this to prove anything: {over_pairs} vs {over_divergent}"
    );
    // Computed outside the code: the 16 pairs scored i/4 give 0.2074996702…,
    // the 14 divergent games alone would have given 0.6836092355…
    close(games.sprt.llr, 0.207_499_670_225_107_38);
    close(over_divergent, 0.683_609_235_523_814_9);
}

/// I-STATS-4: a job with no results reports zeros and an LLR of 0 -- finite,
/// with every percentage 0 -- for both job types, and the whole payload
/// serializes, rather than an error or a NaN.
#[tokio::test]
async fn a_job_with_no_results_reports_zeros_not_nan() {
    let db = TestDb::new().await;
    for job in [db.games_job(1, 10).await, pairs_job(&db).await] {
        let games = game_stats(&db, job).await;
        assert_eq!((games.wins, games.losses, games.draws, games.units_completed), (0, 0, 0, 0));
        assert_eq!(games.sprt.llr, 0.0);
        assert_eq!(games.sprt.status, SprtStatus::Running);
        for pct in [games.win_pct, games.loss_pct, games.draw_pct] {
            assert_eq!(pct, 0.0);
        }
        if games.unit == "pair" {
            assert_eq!(games.pentanomial, Some([0; 5]));
            assert_eq!(games.divergent_pairs, Some(0));
        }

        let payload = serde_json::to_value(stats(&db, job).await).unwrap();
        assert_eq!(payload["games"]["sprt"]["llr"], json!(0.0), "{payload}");
        assert_eq!(payload["games"]["win_pct"], json!(0.0), "{payload}");
        assert_eq!(payload["tasks_total"], 0);
        assert_eq!(payload["eta_seconds"], json!(null));
    }
}

// ---------------------------------------------------------------------------
// Leave generation
// ---------------------------------------------------------------------------

/// A leave-generation job of `generations` generations at a target of 1000,
/// with the generation-0 artifact every job starts from.
async fn leave_job(db: &TestDb, generations: i32) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let job = db.bare_job("leave_generation", 1, admin).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, 100, $3, 1000, 50)",
    )
    .bind(job)
    .bind(kwg)
    .bind(generations)
    .execute(&db.pool)
    .await
    .unwrap();
    close_generation(db, job, 0).await;
    job
}

async fn close_generation(db: &TestDb, job: Uuid, generation: i32) {
    sqlx::query(
        "INSERT INTO leave_generation_artifacts
             (job_id, generation, artifact_key, sha256, builder)
         VALUES ($1, $2, 'leaves/test/g.klv2', repeat('0', 64), 'klv-1')",
    )
    .bind(job)
    .bind(generation)
    .execute(&db.pool)
    .await
    .unwrap();
}

/// A generation's progress row: tasks, games, racks at target of the total,
/// and the rack furthest from target.
async fn progress(db: &TestDb, job: Uuid, generation: i32, figures: (i64, i64, i64, i64)) {
    let (tasks, games, at_target, total) = figures;
    sqlx::query(
        "INSERT INTO leave_generation_progress
             (job_id, generation, tasks_completed, games_played, racks_total, racks_at_target,
              min_rack, min_rack_count, merged_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'AEINRST', 12, '2026-09-01T12:00:00Z')",
    )
    .bind(job)
    .bind(generation)
    .bind(tasks)
    .bind(games)
    .bind(total)
    .bind(at_target)
    .execute(&db.pool)
    .await
    .unwrap();
}

/// I-STATS-6: a leave job's stats are the in-progress generation's: the one
/// after the last closed (generation 0, the zeroed starting point, is not a
/// closed generation), its racks at target against its universe, and its live
/// counters -- not the finished generation before it. A job whose last
/// generation has closed reports that one rather than one past the end, and a
/// generation with no universe yet reports zeros rather than failing.
#[tokio::test]
async fn leave_stats_report_the_current_generations_racks_against_its_universe() {
    let db = TestDb::new().await;
    let job = leave_job(&db, 2).await;

    let fresh = stats(&db, job).await.leave_generation.expect("a leave job's block");
    assert_eq!(fresh.current_generation, 1, "generation 0 is not a closed generation");
    assert_eq!((fresh.racks_at_target, fresh.racks_total, fresh.tasks_completed), (0, 0, 0));
    assert_eq!(fresh.progress_as_of, None);

    progress(&db, job, 1, (40, 4000, 100, 100)).await;
    close_generation(&db, job, 1).await;
    progress(&db, job, 2, (4, 400, 37, 100)).await;

    let current = stats(&db, job).await;
    assert!(current.games.is_none() && current.opening_racks.is_none());
    let leave = current.leave_generation.expect("a leave job's block");
    assert_eq!((leave.current_generation, leave.generation_count), (2, 2));
    assert_eq!(leave.target_rack_count, 1000);
    assert_eq!((leave.racks_at_target, leave.racks_total), (37, 100), "generation 2's, not 1's");
    assert_eq!((leave.tasks_completed, leave.games_played), (4, 400));
    assert_eq!((leave.min_rack.as_deref(), leave.min_rack_count), (Some("AEINRST"), Some(12)));
    assert_eq!(leave.progress_as_of, Some("2026-09-01T12:00:00Z".parse().unwrap()));

    close_generation(&db, job, 2).await;
    let done = stats(&db, job).await.leave_generation.expect("a leave job's block");
    assert_eq!(done.current_generation, 2, "the last generation, not one past it");
    assert_eq!(done.racks_at_target, 37);
}

// ---------------------------------------------------------------------------
// Contributors and the ETA
// ---------------------------------------------------------------------------

/// I-STATS-7: contributions are counted per identity across both kinds -- an
/// account by id and name, an anonymous worker by pseudonym only -- over this
/// job's completed claims alone, ranked, and capped with the rest counted.
#[tokio::test]
async fn contributions_are_attributed_to_each_identity_across_both_kinds() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 10).await;
    let other_job = db.games_job(1, 10).await;
    let alice = db.user("alice", false).await;
    let bob = db.user("bob", false).await;
    let worker = anon(&db).await;
    let decliner = anon(&db).await;

    for _ in 0..3 {
        claim(&db, job, Owner::User(alice), "completed", 5).await;
    }
    for _ in 0..2 {
        claim(&db, job, Owner::Anon(worker), "completed", 5).await;
    }
    claim(&db, job, Owner::User(bob), "completed", 5).await;
    // None of these are this job's contributions.
    claim(&db, job, Owner::User(bob), "abandoned", 0).await;
    claim(&db, job, Owner::User(bob), "claimed", 0).await;
    claim(&db, job, Owner::Anon(decliner), "declined", 0).await;
    for _ in 0..5 {
        claim(&db, other_job, Owner::Anon(worker), "completed", 5).await;
    }

    let (workers, others) = jobstats::worker_contributions(&db.pool, job).await.unwrap();
    let got: Vec<_> = workers
        .iter()
        .map(|w| (w.user_id, w.username.as_deref(), w.anon_id.clone(), w.tasks_completed))
        .collect();
    let pseudonym = birdtest::auth::public_anon_id(worker);
    assert_eq!(
        got,
        vec![
            (Some(alice), Some("alice"), None, 3),
            (None, None, Some(pseudonym), 2),
            (Some(bob), Some("bob"), None, 1),
        ]
    );
    assert_eq!(others, 0);

    // Fifty more contributors of one task each: the list stops at the cap and
    // says how many it left out.
    sqlx::query(
        "WITH workers AS (
             INSERT INTO anonymous_workers (uuid)
             SELECT gen_random_uuid() FROM generate_series(1, 50) RETURNING uuid
         ),
         numbered AS (SELECT uuid, row_number() OVER () AS n FROM workers),
         tasks AS (
             INSERT INTO tasks (job_id, seed, state, accepted_count)
             SELECT $1, 1000 + n, 'completed'::task_state, 1 FROM numbered
             RETURNING id, seed
         )
         INSERT INTO task_claims
             (task_id, claim_token, state, claimed_by_anon_uuid, completed_at)
         SELECT t.id, gen_random_uuid(), 'completed'::claim_state, w.uuid, now()
         FROM tasks t JOIN numbered w ON t.seed = 1000 + w.n",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();
    let (workers, others) = jobstats::worker_contributions(&db.pool, job).await.unwrap();
    assert_eq!(workers.len() as i64, jobstats::MAX_WORKER_CONTRIBUTIONS);
    assert_eq!(others, 53 - jobstats::MAX_WORKER_CONTRIBUTIONS);
    assert_eq!(workers[0].username.as_deref(), Some("alice"), "still ranked");
}

/// I-STATS-8: with no task completed in the last hour there is no throughput
/// to extrapolate, and the ETA is `None` rather than infinity -- as it is for
/// a job that is not active. One recent completion gives a finite estimate.
#[tokio::test]
async fn the_eta_is_none_without_recent_throughput() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 10).await;
    let worker = Owner::Anon(anon(&db).await);

    assert_eq!(stats(&db, job).await.eta_seconds, None, "nothing done at all");
    claim(&db, job, worker, "completed", 120).await;
    assert_eq!(stats(&db, job).await.eta_seconds, None, "nothing done in the last hour");

    claim(&db, job, worker, "completed", 10).await;
    let eta = stats(&db, job).await.eta_seconds.expect("recent throughput");
    assert!(eta.is_finite() && eta > 0.0, "{eta}");

    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(stats(&db, job).await.eta_seconds, None, "an inactive job has no ETA");
}

// ---------------------------------------------------------------------------
// The finish check
// ---------------------------------------------------------------------------

/// A games job of one 100-game batch per task, with the given gates.
async fn gated_games_job(db: &TestDb, min_games: i32, max_games: i32) -> Uuid {
    let job = db.games_job(1, 100).await;
    sqlx::query("UPDATE job_game_config SET min_games = $2, max_games = $3 WHERE job_id = $1")
        .bind(job)
        .bind(min_games)
        .bind(max_games)
        .execute(&db.pool)
        .await
        .unwrap();
    job
}

/// Claims the job's next batch and submits `wins` of its 100 games, through
/// the worker API -- the only caller of the finish check. With no other claim
/// in flight the submission always runs it.
async fn play_batch(app: &axum::Router, wins: i32) {
    let (status, assignment) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let uuid = assignment["worker_uuid"].as_str().unwrap();
    let (status, body) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": assignment["claim_token"], "result": games_result(100, wins) }),
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

/// I-STATS-9 (significance): at `min_games`, a 90-10 batch crosses the upper
/// bound, and the submission that brought it there completes the job.
#[tokio::test]
async fn a_job_completes_when_its_llr_crosses_at_min_games() {
    let db = TestDb::new().await;
    let job = gated_games_job(&db, 100, 1_000_000).await;
    let app = birdtest::app(db.state().await);

    play_batch(&app, 90).await;
    assert_eq!(game_stats(&db, job).await.sprt.status, SprtStatus::Passed);
    assert_eq!(job_status(&db, job).await, "completed");
}

/// I-STATS-9 (hard cap): an even batch that reaches `max_games` completes the
/// job with no verdict either way.
#[tokio::test]
async fn a_job_completes_at_its_hard_cap_without_a_verdict() {
    let db = TestDb::new().await;
    let job = gated_games_job(&db, 100, 100).await;
    let app = birdtest::app(db.state().await);

    play_batch(&app, 50).await;
    let games = game_stats(&db, job).await;
    assert_eq!(games.sprt.status, SprtStatus::TerminatedAtMax);
    assert!(games.sprt.llr < games.sprt.upper_bound && games.sprt.llr > games.sprt.lower_bound);
    assert_eq!(job_status(&db, job).await, "completed");
}

/// I-STATS-9 (the floor): the same 90-10 batch below `min_games` crosses the
/// upper bound and still does not complete the job -- the floor is what stops
/// an early streak from ending it.
#[tokio::test]
async fn a_crossed_llr_below_min_games_does_not_complete_the_job() {
    let db = TestDb::new().await;
    let job = gated_games_job(&db, 1000, 1_000_000).await;
    let app = birdtest::app(db.state().await);

    play_batch(&app, 90).await;
    let games = game_stats(&db, job).await;
    assert!(games.sprt.llr > games.sprt.upper_bound, "the LLR has crossed: {:?}", games.sprt);
    assert_eq!(games.sprt.status, SprtStatus::Running);
    assert_eq!(job_status(&db, job).await, "active");
}

/// I-STATS-9 (at the bound): the job completes on the submission that takes
/// its LLR past the upper bound, and not on one that leaves it just short.
/// 71-29 over 100 games is 2.9347 against a bound of 2.9444: at `min_games`,
/// checked, and still running. Another 54-46 makes 125-75 over 200, 3.0693:
/// past it, and the job completes. (Both computed outside the code.)
#[tokio::test]
async fn a_job_completes_on_the_batch_that_crosses_the_bound_and_not_before() {
    let db = TestDb::new().await;
    let job = gated_games_job(&db, 100, 1_000_000).await;
    let app = birdtest::app(db.state().await);

    play_batch(&app, 71).await;
    let games = game_stats(&db, job).await;
    close(games.sprt.llr, 2.934_734_021_233_334_5);
    assert_eq!(games.sprt.status, SprtStatus::Running, "just inside the bound");
    assert_eq!(job_status(&db, job).await, "active");

    play_batch(&app, 54).await;
    let games = game_stats(&db, job).await;
    assert_eq!((games.wins, games.losses), (125, 75));
    close(games.sprt.llr, 3.069_265_955_413_046_7);
    assert_eq!(games.sprt.status, SprtStatus::Passed);
    assert_eq!(job_status(&db, job).await, "completed");
}

/// I-STATS-9 (H0): a job whose player 1 is losing completes too, with its
/// verdict failed (H0 accepted) rather than passed. 28-72 over 100 games is
/// an LLR of -3.1401, past the lower bound of -2.9444 (computed outside the
/// code).
#[tokio::test]
async fn a_job_driven_to_h0_completes_with_its_sprt_failed() {
    let db = TestDb::new().await;
    let job = gated_games_job(&db, 100, 1_000_000).await;
    let app = birdtest::app(db.state().await);

    play_batch(&app, 28).await;
    let games = game_stats(&db, job).await;
    close(games.sprt.llr, -3.140_060_036_229_865_5);
    assert_eq!(games.sprt.status, SprtStatus::Failed);
    assert_eq!(job_status(&db, job).await, "completed");

    let (_, body) = send(&app, get_request(&format!("/api/jobs/{job}"), &[])).await;
    assert_eq!(body["games"]["sprt"]["status"], json!("failed"), "{body}");
}
