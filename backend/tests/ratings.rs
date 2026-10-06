//! Rating pools against a real database: what `build_matrix` counts as
//! evidence, what a fit stores, and the rating endpoints over HTTP.
//!
//! Evidence is written with plain SQL -- a task, a completed claim and a
//! `game_results` row per batch -- because what is under test is the fit's
//! reading of results, not the submission path that produced them.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdtest::ratings::{self, Trigger};
use common::*;
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// The `(letterdist, layout)` half of a pool's scope; the variant is passed
/// alongside, since it is a column rather than a row.
#[derive(Clone, Copy)]
struct Scope {
    letterdist: Uuid,
    layout: Uuid,
}

async fn scope(db: &TestDb) -> Scope {
    Scope {
        letterdist: db.input_data("letterdist", "english").await,
        layout: db.input_data("layout", "standard15").await,
    }
}

/// A `jobs` row of `job_type` in `variant` and `scope`, and nothing else.
async fn job_in(db: &TestDb, job_type: &str, variant: &str, scope: Scope) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO jobs (job_type, allocation, status, variant,
                           letterdist_id, layout_id, bingo_bonus, sim_cutoff)
         VALUES ($1::job_type, 50, 'active', $2, $3, $4, 50, 0.005)
         RETURNING id",
    )
    .bind(job_type)
    .bind(variant)
    .bind(scope.letterdist)
    .bind(scope.layout)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// A `game_pairs` job between `p1` and `p2`.
async fn pairs_job(db: &TestDb, variant: &str, scope: Scope, p1: Uuid, p2: Uuid) -> Uuid {
    let job = job_in(db, "game_pairs", variant, scope).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 1, 1000000, 1000000)",
    )
    .bind(job)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

/// One accepted batch of pairs for `job`, as a completed task with one
/// completed claim and its result. `pent` is player 1's pentanomial; the
/// per-game counts are derived from it so the row satisfies the table's
/// cross-checks (a split pair is booked as a win and a loss, a 1.5-0.5 pair as
/// a win and a tie).
async fn pair_result(db: &TestDb, job: Uuid, pent: [i32; 5]) {
    let [p0, p1, p2, p3, p4] = pent;
    let wins = p2 + p3 + 2 * p4;
    let ties = p1 + p3;
    let losses = 2 * p0 + p1 + p2;
    game_result(db, job, (wins, losses, ties), Some(pent)).await;
}

/// A completed task, claim and `game_results` row: the per-game tally and, for
/// a paired batch, the pentanomial.
async fn game_result(db: &TestDb, job: Uuid, tally: (i32, i32, i32), pent: Option<[i32; 5]>) {
    let (wins, losses, ties) = tally;
    let worker = Uuid::new_v4();
    sqlx::query("INSERT INTO anonymous_workers (uuid) VALUES ($1)")
        .bind(worker)
        .execute(&db.pool)
        .await
        .unwrap();
    let task: Uuid = sqlx::query_scalar(
        "INSERT INTO tasks (job_id, seed, state, accepted_count, completed_at)
         SELECT $1, COALESCE(MAX(seed), 0) + 1, 'completed'::task_state, 1, now()
         FROM tasks WHERE job_id = $1
         RETURNING id",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let claim: Uuid = sqlx::query_scalar(
        "INSERT INTO task_claims (task_id, job_id, claim_token, state, claimed_by_anon_uuid, completed_at)
         VALUES ($1, (SELECT job_id FROM tasks WHERE id = $1), gen_random_uuid(), 'completed', $2, now()) RETURNING id",
    )
    .bind(task)
    .bind(worker)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let pent = pent.map(|p| p.to_vec());
    sqlx::query(
        "INSERT INTO game_results
             (task_claim_id, task_id, job_id, games, wins, losses, ties,
              p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd,
              pent_0, pent_1, pent_2, pent_3, pent_4)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 420, 60, 410, 58,
                 $8[1], $8[2], $8[3], $8[4], $8[5])",
    )
    .bind(claim)
    .bind(task)
    .bind(job)
    .bind(wins + losses + ties)
    .bind(wins)
    .bind(losses)
    .bind(ties)
    .bind(pent)
    .execute(&db.pool)
    .await
    .unwrap();
    // As the submission that stores a first result does: the sweep reads this
    // running total to decide whether a pool's evidence moved.
    sqlx::query("UPDATE jobs SET games_completed = games_completed + $2 WHERE id = $1")
        .bind(job)
        .bind(i64::from(wins + losses + ties))
        .execute(&db.pool)
        .await
        .unwrap();
}

/// A pool over `scope` and `variant` whose members are `anchor` and `others`.
async fn pool(
    db: &TestDb,
    name: &str,
    variant: &str,
    scope: Scope,
    anchor: Uuid,
    others: &[Uuid],
    anchor_rating: f64,
) -> Uuid {
    let pool: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_pools
             (name, variant, letterdist_id, layout_id, anchor_player_config_id, anchor_rating)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(name)
    .bind(variant)
    .bind(scope.letterdist)
    .bind(scope.layout)
    .bind(anchor)
    .bind(anchor_rating)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    for member in std::iter::once(&anchor).chain(others) {
        add_member(db, pool, *member).await;
    }
    pool
}

async fn add_member(db: &TestDb, pool: Uuid, member: Uuid) {
    sqlx::query("INSERT INTO rating_pool_members (pool_id, player_config_id) VALUES ($1, $2)")
        .bind(pool)
        .bind(member)
        .execute(&db.pool)
        .await
        .unwrap();
}

async fn remove_member(db: &TestDb, pool: Uuid, member: Uuid) {
    sqlx::query("DELETE FROM rating_pool_members WHERE pool_id = $1 AND player_config_id = $2")
        .bind(pool)
        .bind(member)
        .execute(&db.pool)
        .await
        .unwrap();
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stored {
    rating: f64,
    pairs_played: i64,
    connected: bool,
    is_anchor: bool,
}

/// Every rating a run stored, by player config.
async fn stored_ratings(db: &TestDb, run: Uuid) -> HashMap<Uuid, Stored> {
    sqlx::query_as::<_, (Uuid, f64, i64, bool, bool)>(
        "SELECT player_config_id, rating, pairs_played, connected_to_anchor, is_anchor
         FROM player_config_ratings WHERE run_id = $1",
    )
    .bind(run)
    .fetch_all(&db.pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(id, rating, pairs_played, connected, is_anchor)| {
        (id, Stored { rating, pairs_played, connected, is_anchor })
    })
    .collect()
}

/// A run's evidence totals: `(pairs_used, jobs_used)`.
async fn evidence(db: &TestDb, run: Uuid) -> (i64, i32) {
    sqlx::query_as("SELECT pairs_used, jobs_used FROM rating_runs WHERE id = $1")
        .bind(run)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// A run's residuals: `(row, col, pairs, actual, predicted)`.
async fn residuals(db: &TestDb, run: Uuid) -> Vec<(Uuid, Uuid, f64, f64, f64)> {
    sqlx::query_as(
        "SELECT row_player_config_id, col_player_config_id, pairs, actual, predicted
         FROM rating_run_residuals WHERE run_id = $1
         ORDER BY row_player_config_id, col_player_config_id",
    )
    .bind(run)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn run_count(db: &TestDb, pool: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM rating_runs WHERE pool_id = $1")
        .bind(pool)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// A two-member pool -- `a-anchor` and `b-rival`, named so the anchor orders
/// first and is the residual's row -- with a control job in scope whose two
/// pairs are the only evidence it should see. Each exclusion test adds one job
/// that differs from the control in a single way.
struct Fixture {
    admin: Uuid,
    scope: Scope,
    anchor: Uuid,
    rival: Uuid,
    pool: Uuid,
}

async fn fixture(db: &TestDb) -> Fixture {
    let admin = db.user("root", true).await;
    let scope = scope(db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let control = pairs_job(db, "classic", scope, anchor, rival).await;
    // One split pair and one won by the anchor: 3 of 4 points.
    pair_result(db, control, [0, 0, 1, 0, 1]).await;
    let pool = pool(db, "pool", "classic", scope, anchor, &[rival], 2000.0).await;
    Fixture { admin, scope, anchor, rival, pool }
}

/// Fits the fixture's pool and asserts only the control job was read: had the
/// excluded job's five lopsided pairs counted, every one of these would move.
async fn assert_only_the_control_counted(db: &TestDb, f: &Fixture) {
    let run = ratings::recompute(&db.pool, f.pool, Trigger::Manual).await.unwrap();
    assert_eq!(evidence(db, run).await, (2, 1), "two pairs from one job");
    let stored = stored_ratings(db, run).await;
    assert_eq!(stored[&f.rival].pairs_played, 2);
    let residuals = residuals(db, run).await;
    assert_eq!(residuals.len(), 1, "{residuals:?}");
    let (row, col, pairs, actual, _) = residuals[0];
    assert_eq!((row, col), (f.anchor, f.rival));
    assert_eq!(pairs, 2.0);
    assert_eq!(actual, 0.75, "the control's score, undiluted");
}

/// Five pairs the anchor lost outright. Lopsided on purpose: counted, they
/// would drag the control's 0.75 below a half.
const EXCLUDED: [i32; 5] = [5, 0, 0, 0, 0];

// ---------------------------------------------------------------------------
// I-RATE: what counts, and what a fit stores
// ---------------------------------------------------------------------------

/// I-RATE-2 (variant): a `game_pairs` job between two members, in the pool's
/// letter distribution and layout but another variant, is not evidence. A
/// wordsmog game and a classic one are different games.
#[tokio::test]
async fn a_pairs_job_in_another_variant_is_not_evidence() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let other = pairs_job(&db, "wordsmog", f.scope, f.anchor, f.rival).await;
    pair_result(&db, other, EXCLUDED).await;
    assert_only_the_control_counted(&db, &f).await;
}

/// I-RATE-2 (letterdist_id): another letter distribution is another game.
#[tokio::test]
async fn a_pairs_job_on_another_letter_distribution_is_not_evidence() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let scope = Scope { letterdist: db.input_data("letterdist", "english").await, ..f.scope };
    let other = pairs_job(&db, "classic", scope, f.anchor, f.rival).await;
    pair_result(&db, other, EXCLUDED).await;
    assert_only_the_control_counted(&db, &f).await;
}

/// I-RATE-2 (layout_id): another board is another game.
#[tokio::test]
async fn a_pairs_job_on_another_board_layout_is_not_evidence() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let scope = Scope { layout: db.input_data("layout", "standard15").await, ..f.scope };
    let other = pairs_job(&db, "classic", scope, f.anchor, f.rival).await;
    pair_result(&db, other, EXCLUDED).await;
    assert_only_the_control_counted(&db, &f).await;
}

/// I-RATE-2 (membership): a job whose other player is not in the pool is not
/// evidence, even though one of its players is -- in either seat.
#[tokio::test]
async fn a_pairs_job_against_a_non_member_is_not_evidence() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let outsider = db.static_player("c-outsider", f.admin).await;
    let as_p2 = pairs_job(&db, "classic", f.scope, f.anchor, outsider).await;
    pair_result(&db, as_p2, EXCLUDED).await;
    let as_p1 = pairs_job(&db, "classic", f.scope, outsider, f.rival).await;
    pair_result(&db, as_p1, EXCLUDED).await;
    assert_only_the_control_counted(&db, &f).await;
}

/// I-RATE-2 (job type): a plain `games` job between two members is not
/// evidence -- it is not side-balanced, and going first is worth real Elo.
///
/// Twice: once as the app builds one (a `job_game_config` row, results with no
/// pentanomial), and once in a state the app forbids -- a `games` job that
/// also has a pair config and pentanomial results -- so the job-type clause
/// itself is what excludes it, not the missing join.
#[tokio::test]
async fn a_plain_games_job_is_not_evidence() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;

    let games = job_in(&db, "games", "classic", f.scope).await;
    sqlx::query(
        "INSERT INTO job_game_config
             (job_id, player1_config_id, player2_config_id, games_per_batch, min_games, max_games)
         VALUES ($1, $2, $3, 10, 1000000, 1000000)",
    )
    .bind(games)
    .bind(f.anchor)
    .bind(f.rival)
    .execute(&db.pool)
    .await
    .unwrap();
    game_result(&db, games, (0, 10, 0), None).await;
    assert_only_the_control_counted(&db, &f).await;

    let disguised = job_in(&db, "games", "classic", f.scope).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 1, 1000000, 1000000)",
    )
    .bind(disguised)
    .bind(f.anchor)
    .bind(f.rival)
    .execute(&db.pool)
    .await
    .unwrap();
    pair_result(&db, disguised, EXCLUDED).await;
    assert_only_the_control_counted(&db, &f).await;
}

/// I-RATE-3: the pair is the unit. Four pairs over two batches -- eight games
/// -- with half points in them: a 1-3 pair, a split and two 3-1 pairs, nine
/// half-points of sixteen. The head-to-head is four observations, not eight,
/// scoring 9/4 = 2.25 of them, a rate of 0.5625.
#[tokio::test]
async fn a_head_to_head_counts_pairs_and_scores_half_points_over_four() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let job = pairs_job(&db, "classic", scope, anchor, rival).await;
    pair_result(&db, job, [0, 1, 0, 1, 0]).await;
    pair_result(&db, job, [0, 0, 1, 1, 0]).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[rival], 2000.0).await;

    let run = ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();
    assert_eq!(evidence(&db, run).await, (4, 1), "pairs_used counts pairs");
    let stored = stored_ratings(&db, run).await;
    assert_eq!(stored[&anchor].pairs_played, 4);
    assert_eq!(stored[&rival].pairs_played, 4);
    let residuals = residuals(&db, run).await;
    assert_eq!(residuals.len(), 1, "{residuals:?}");
    let (row, col, pairs, actual, _) = residuals[0];
    assert_eq!((row, col), (anchor, rival));
    assert_eq!(pairs, 4.0, "four pairs, not eight games");
    assert_eq!(actual, 0.5625, "nine half-points over four, per pair");
}

/// I-RATE-4: a fit stores one run and one rating per member, the anchor
/// flagged on exactly one of them; a second fit adds a second run rather than
/// changing the first.
#[tokio::test]
async fn a_fit_stores_one_run_and_one_rating_per_member() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let b = db.static_player("b", admin).await;
    let c = db.static_player("c", admin).await;
    let ab = pairs_job(&db, "classic", scope, anchor, b).await;
    pair_result(&db, ab, [1, 1, 2, 0, 0]).await;
    let bc = pairs_job(&db, "classic", scope, b, c).await;
    pair_result(&db, bc, [0, 1, 1, 1, 1]).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[b, c], 2000.0).await;

    let run = ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();
    assert_eq!(run_count(&db, pool).await, 1);
    let (trigger, method): (String, String) =
        sqlx::query_as("SELECT trigger, method FROM rating_runs WHERE id = $1")
            .bind(run)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((trigger.as_str(), method.as_str()), ("manual", "bradley_terry_newton"));

    let stored = stored_ratings(&db, run).await;
    let mut rated: Vec<Uuid> = stored.keys().copied().collect();
    rated.sort();
    let mut members = vec![anchor, b, c];
    members.sort();
    assert_eq!(rated, members, "one rating per member");
    let anchors: Vec<Uuid> =
        stored.iter().filter(|(_, s)| s.is_anchor).map(|(id, _)| *id).collect();
    assert_eq!(anchors, vec![anchor], "is_anchor on exactly the anchor");

    let second = ratings::recompute(&db.pool, pool, Trigger::Evidence).await.unwrap();
    assert_ne!(second, run);
    assert_eq!(run_count(&db, pool).await, 2);
    assert_eq!(stored_ratings(&db, run).await, stored, "the first run is a snapshot");
    assert_eq!(stored_ratings(&db, second).await.len(), 3);
}

/// I-RATE-5: the anchor is stored at exactly `anchor_rating`, whatever its
/// results -- here it loses most of its pairs, and an unanchored fit would put
/// it well below.
#[tokio::test]
async fn the_anchor_is_stored_at_exactly_its_anchor_rating() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let job = pairs_job(&db, "classic", scope, anchor, rival).await;
    pair_result(&db, job, [6, 2, 1, 1, 0]).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[rival], 1837.25).await;

    let run = ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();
    let stored = stored_ratings(&db, run).await;
    assert_eq!(stored[&anchor].rating, 1837.25, "exactly, not approximately");
    assert!(stored[&rival].rating > 1837.25 + 50.0, "{:?}", stored[&rival]);
}

/// I-RATE-6: a member with no chain of games to the anchor is stored as not
/// connected -- one that played only another unconnected member, and one that
/// played nobody -- while the member that played the anchor is connected.
#[tokio::test]
async fn a_member_with_no_path_to_the_anchor_is_stored_unconnected() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let b = db.static_player("b", admin).await;
    let c = db.static_player("c", admin).await;
    let d = db.static_player("d", admin).await;
    let idle = db.static_player("e-idle", admin).await;
    let ab = pairs_job(&db, "classic", scope, anchor, b).await;
    pair_result(&db, ab, [0, 1, 2, 1, 0]).await;
    let cd = pairs_job(&db, "classic", scope, c, d).await;
    pair_result(&db, cd, [0, 0, 1, 2, 1]).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[b, c, d, idle], 2000.0).await;

    let run = ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();
    let stored = stored_ratings(&db, run).await;
    assert!(stored[&anchor].connected);
    assert!(stored[&b].connected, "b played the anchor");
    assert!(!stored[&c].connected, "c played only d");
    assert!(!stored[&d].connected, "d played only c");
    assert!(!stored[&idle].connected, "e played nobody");
    assert_eq!(stored[&c].pairs_played, 4, "c's games are still counted, just not rated");
}

/// I-RATE-7: adding a member moves the other members' ratings -- its games
/// are new evidence about them -- and removing it moves them back exactly,
/// since a fit is a pure function of membership and evidence. The newcomer
/// plays two members and contradicts their order, so it is not a leaf a fit
/// could absorb on its own.
#[tokio::test]
async fn membership_changes_move_everyone_and_removal_moves_them_back() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let b = db.static_player("b", admin).await;
    let c = db.static_player("c", admin).await;
    let d = db.static_player("d-newcomer", admin).await;
    for (p1, p2, pent) in [
        (anchor, b, [1, 1, 2, 0, 0]),
        (b, c, [0, 1, 2, 1, 0]),
        (anchor, c, [0, 0, 2, 2, 0]),
        (d, b, [0, 0, 0, 1, 3]),
        (c, d, [0, 0, 0, 2, 2]),
    ] {
        let job = pairs_job(&db, "classic", scope, p1, p2).await;
        pair_result(&db, job, pent).await;
    }
    let pool = pool(&db, "pool", "classic", scope, anchor, &[b, c], 2000.0).await;

    let fit = |trigger| ratings::recompute(&db.pool, pool, trigger);
    let before = stored_ratings(&db, fit(Trigger::Manual).await.unwrap()).await;

    add_member(&db, pool, d).await;
    let with = stored_ratings(&db, fit(Trigger::Membership).await.unwrap()).await;
    assert_eq!(with.len(), 4);
    assert_eq!(with[&anchor].rating, before[&anchor].rating, "the anchor does not move");
    for member in [b, c] {
        assert!(
            (with[&member].rating - before[&member].rating).abs() > 1.0,
            "adding d did not move {member}: {} -> {}",
            before[&member].rating,
            with[&member].rating
        );
    }

    remove_member(&db, pool, d).await;
    let after = stored_ratings(&db, fit(Trigger::Membership).await.unwrap()).await;
    assert_eq!(after, before, "removing d restores the fit it replaced");
}

/// I-RATE-8: removing the anchor is refused. The route refuses it outright
/// (A-RATE-5, below); here the membership row is deleted behind the route's
/// back, and the fit refuses to rate a pool with no fixed point rather than
/// storing a run measured against nothing.
#[tokio::test]
async fn a_pool_whose_anchor_is_not_a_member_is_refused_a_fit() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    remove_member(&db, f.pool, f.anchor).await;

    let err = ratings::recompute(&db.pool, f.pool, Trigger::Manual).await.unwrap_err();
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert!(err.message.contains("anchor is not a member"), "{}", err.message);
    assert_eq!(run_count(&db, f.pool).await, 0, "nothing is stored");
}

/// I-RATE-9: the sweep refits a pool whose evidence grew since its last run
/// and leaves one whose `pairs_used` is unchanged alone -- then, with nothing
/// new anywhere, refits nothing.
#[tokio::test]
async fn the_sweep_refits_only_the_pools_whose_evidence_grew() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let (busy_scope, quiet_scope) = (scope(&db).await, scope(&db).await);
    let busy_job = pairs_job(&db, "classic", busy_scope, anchor, rival).await;
    pair_result(&db, busy_job, [0, 1, 1, 0, 0]).await;
    let quiet_job = pairs_job(&db, "classic", quiet_scope, anchor, rival).await;
    pair_result(&db, quiet_job, [0, 0, 1, 1, 0]).await;
    let busy = pool(&db, "busy", "classic", busy_scope, anchor, &[rival], 2000.0).await;
    let quiet = pool(&db, "quiet", "classic", quiet_scope, anchor, &[rival], 2000.0).await;
    let busy_first = ratings::recompute(&db.pool, busy, Trigger::Manual).await.unwrap();
    ratings::recompute(&db.pool, quiet, Trigger::Manual).await.unwrap();

    pair_result(&db, busy_job, [0, 0, 0, 1, 2]).await;
    assert_eq!(ratings::recompute_stale(&db.pool).await.unwrap(), 1);
    assert_eq!((run_count(&db, busy).await, run_count(&db, quiet).await), (2, 1));
    let (newest, trigger, pairs_used): (Uuid, String, i64) = sqlx::query_as(
        "SELECT id, trigger, pairs_used FROM rating_runs
         WHERE pool_id = $1 ORDER BY computed_at DESC LIMIT 1",
    )
    .bind(busy)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_ne!(newest, busy_first);
    assert_eq!((trigger.as_str(), pairs_used), ("evidence", 5));

    assert_eq!(ratings::recompute_stale(&db.pool).await.unwrap(), 0, "nothing grew");
    assert_eq!((run_count(&db, busy).await, run_count(&db, quiet).await), (2, 1));
}

/// I-RATE-9b: the sweep also repairs a membership change whose own refit never
/// ran -- the route commits the membership and then fits in a separate step,
/// and a dropped request or a failed fit left the pool's newest run describing
/// the old membership. When the config added had no pairs in the pool, the
/// evidence count did not move and nothing ever noticed.
#[tokio::test]
async fn the_sweep_refits_a_pool_whose_membership_changed_without_a_refit() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let newcomer = db.static_player("c-newcomer", admin).await;
    let scope = scope(&db).await;
    let job = pairs_job(&db, "classic", scope, anchor, rival).await;
    pair_result(&db, job, [0, 1, 1, 0, 0]).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[rival], 2000.0).await;
    ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();

    // A member with no pairs, added behind the route's back: no refit ran.
    add_member(&db, pool, newcomer).await;
    assert_eq!(ratings::recompute_stale(&db.pool).await.unwrap(), 1);
    let newest: Uuid = sqlx::query_scalar(
        "SELECT id FROM rating_runs WHERE pool_id = $1 ORDER BY computed_at DESC LIMIT 1",
    )
    .bind(pool)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(stored_ratings(&db, newest).await.contains_key(&newcomer), "the newcomer is rated");
    assert_eq!(ratings::recompute_stale(&db.pool).await.unwrap(), 0, "and then it is current");

    remove_member(&db, pool, newcomer).await;
    assert_eq!(ratings::recompute_stale(&db.pool).await.unwrap(), 1, "a removal likewise");
}

/// I-RATE-10: two pools over the same configs and the same set of jobs, scoped
/// to different variants, each fit only their own jobs -- and so disagree,
/// the rival above the anchor in one and below it in the other -- while each
/// is internally consistent: its evidence totals are its own jobs', its
/// residuals' actual scores are its own results, and their predictions are
/// the ones its own stored ratings imply.
#[tokio::test]
async fn pools_of_different_scopes_fit_different_consistent_ratings() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let third = db.static_player("c-third", admin).await;

    // Classic: the rival wins; wordsmog: the anchor does.
    let classic = pairs_job(&db, "classic", scope, anchor, rival).await;
    pair_result(&db, classic, [3, 1, 1, 0, 0]).await;
    let classic_third = pairs_job(&db, "classic", scope, rival, third).await;
    pair_result(&db, classic_third, [0, 1, 1, 0, 0]).await;
    let wordsmog = pairs_job(&db, "wordsmog", scope, anchor, rival).await;
    pair_result(&db, wordsmog, [0, 0, 1, 1, 3]).await;

    let classic_pool =
        pool(&db, "classic", "classic", scope, anchor, &[rival, third], 2000.0).await;
    let wordsmog_pool =
        pool(&db, "wordsmog", "wordsmog", scope, anchor, &[rival, third], 2000.0).await;
    let classic_run = ratings::recompute(&db.pool, classic_pool, Trigger::Manual).await.unwrap();
    let wordsmog_run = ratings::recompute(&db.pool, wordsmog_pool, Trigger::Manual).await.unwrap();

    assert_eq!(evidence(&db, classic_run).await, (7, 2));
    assert_eq!(evidence(&db, wordsmog_run).await, (5, 1));

    let classic_ratings = stored_ratings(&db, classic_run).await;
    let wordsmog_ratings = stored_ratings(&db, wordsmog_run).await;
    assert!(classic_ratings[&rival].rating > 2000.0, "{classic_ratings:?}");
    assert!(wordsmog_ratings[&rival].rating < 2000.0, "{wordsmog_ratings:?}");
    assert!(classic_ratings[&third].connected);
    assert!(!wordsmog_ratings[&third].connected, "third played no wordsmog");

    for (run, stored, expected) in [
        (
            classic_run,
            &classic_ratings,
            vec![(anchor, rival, 5.0, 3.0 / 20.0), (rival, third, 2.0, 3.0 / 8.0)],
        ),
        (wordsmog_run, &wordsmog_ratings, vec![(anchor, rival, 5.0, 17.0 / 20.0)]),
    ] {
        let residuals = residuals(&db, run).await;
        let mut got: Vec<(Uuid, Uuid, f64, f64)> =
            residuals.iter().map(|&(r, c, pairs, actual, _)| (r, c, pairs, actual)).collect();
        got.sort_by_key(|&(r, c, _, _)| (r, c));
        let mut expected = expected;
        expected.sort_by_key(|&(r, c, _, _)| (r, c));
        assert_eq!(got, expected);
        for (row, col, _, _, predicted) in residuals {
            let diff = stored[&row].rating - stored[&col].rating;
            let implied = 1.0 / (1.0 + 10f64.powf(-diff / 400.0));
            assert!((predicted - implied).abs() < 1e-12, "{predicted} vs {implied}");
        }
    }
}

/// I-RATE-11: a pool of only its anchor, with no games anywhere, fits to a
/// run -- the anchor at its rating, no evidence, no residuals -- rather than
/// an error. It is every new pool's first state.
#[tokio::test]
async fn a_pool_of_only_its_anchor_fits_to_an_empty_run() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[], 1500.0).await;

    let run = ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();
    assert_eq!(evidence(&db, run).await, (0, 0));
    let stored = stored_ratings(&db, run).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[&anchor],
        Stored { rating: 1500.0, pairs_played: 0, connected: true, is_anchor: true }
    );
    assert!(residuals(&db, run).await.is_empty());
}

// ---------------------------------------------------------------------------
// A-RATE: the endpoints
// ---------------------------------------------------------------------------

fn request(
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// A-RATE-1: the list names each pool with its member count and latest fit
/// time, and the detail shows the latest run -- not the first -- with its
/// ratings best first and its residuals.
#[tokio::test]
async fn the_pool_pages_show_the_latest_run_with_its_residuals() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let job = pairs_job(&db, "classic", scope, anchor, rival).await;
    pair_result(&db, job, [0, 0, 1, 0, 1]).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[rival], 2000.0).await;
    ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();
    pair_result(&db, job, [2, 0, 0, 0, 0]).await;
    let latest = ratings::recompute(&db.pool, pool, Trigger::Evidence).await.unwrap();
    let latest_at: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT computed_at FROM rating_runs WHERE id = $1")
            .bind(latest)
            .fetch_one(&db.pool)
            .await
            .unwrap();

    let (status, list) = send(&app, get_request("/api/rating-pools", &[])).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list[0]["id"], json!(pool));
    assert_eq!(list[0]["members"], 2);
    assert_eq!(list[0]["letter_distribution"], "english");
    assert_eq!(list[0]["layout"], "standard15");
    assert_eq!(list[0]["last_computed_at"], json!(latest_at));

    let (status, detail) = send(&app, get_request(&format!("/api/rating-pools/{pool}"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["run"]["id"], json!(latest), "{detail}");
    assert_eq!(detail["run"]["trigger"], "evidence");
    assert_eq!(detail["run"]["pairs_used"], 4);
    let ratings = detail["ratings"].as_array().unwrap();
    assert_eq!(ratings.len(), 2, "{detail}");
    assert!(ratings[0]["rating"].as_f64() >= ratings[1]["rating"].as_f64(), "best first");
    let anchor_row = ratings.iter().find(|r| r["player_config_id"] == json!(anchor)).unwrap();
    assert_eq!(anchor_row["rating"], 2000.0);
    assert_eq!(anchor_row["is_anchor"], true);
    assert_eq!(anchor_row["name"], "a-anchor");
    let residuals = detail["residuals"].as_array().unwrap();
    assert_eq!(residuals.len(), 1, "{detail}");
    assert_eq!(residuals[0]["row"], json!(anchor));
    assert_eq!(residuals[0]["col"], json!(rival));
    assert_eq!(residuals[0]["pairs"], 4.0, "the latest run's evidence");
    assert_eq!(residuals[0]["actual"], 3.0 / 8.0);
    assert!(residuals[0]["predicted"].as_f64().unwrap() > 0.0);
}

/// A-RATE-2: a pool that has never been fit renders as a pool with no run --
/// no ratings, no residuals, no history -- rather than an error; and a pool
/// that does not exist is a 404.
#[tokio::test]
async fn a_pool_with_no_run_renders_empty() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[], 2000.0).await;

    let (status, list) = send(&app, get_request("/api/rating-pools", &[])).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list[0]["last_computed_at"], json!(null), "{list}");

    let (status, detail) = send(&app, get_request(&format!("/api/rating-pools/{pool}"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["run"], json!(null), "{detail}");
    assert_eq!(detail["ratings"], json!([]));
    assert_eq!(detail["residuals"], json!([]));
    assert_eq!(detail["anchor_player_config_id"], json!(anchor));

    let (status, history) =
        send(&app, get_request(&format!("/api/rating-pools/{pool}/history"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history, json!([]));

    let (status, body) =
        send(&app, get_request(&format!("/api/rating-pools/{}", Uuid::new_v4()), &[])).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["message"], "rating pool not found");
}

/// A-RATE-3: creating a pool makes its anchor a member, with nothing else to
/// do -- a pool whose fixed point is not in it has nothing to fix.
#[tokio::test]
async fn creating_a_pool_makes_its_anchor_a_member() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;

    let (status, body) = send(
        &app,
        request(
            "POST",
            "/api/admin/rating-pools",
            &admin_headers(&state.cfg, admin),
            Some(json!({
                "name": "classic english",
                "variant": "classic",
                "letterdist_id": scope.letterdist,
                "layout_id": scope.layout,
                "anchor_player_config_id": anchor,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let pool: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    let members: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT player_config_id, added_by FROM rating_pool_members WHERE pool_id = $1",
    )
    .bind(pool)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(members, vec![(anchor, Some(admin))]);

    let (_, detail) = send(&app, get_request(&format!("/api/rating-pools/{pool}"), &[])).await;
    assert_eq!(detail["anchor_rating"], 2000.0, "the default anchor rating");
    let (_, list) = send(&app, get_request("/api/rating-pools", &[])).await;
    assert_eq!(list[0]["members"], 1, "{list}");
}

/// A-RATE-3b: a pool is validated the way a job is. A variant no job can have,
/// a distribution row that is really a layout, or an anchor rating whose
/// logistic scale overflows each made a pool that rated no one, or rated
/// everyone at infinity -- and a pool cannot be deleted. Each is refused, and
/// nothing is created.
#[tokio::test]
async fn a_pool_that_could_rate_no_one_is_refused() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let body = |variant: &str, letterdist: Uuid, rating: f64| {
        json!({
            "name": "pool",
            "variant": variant,
            "letterdist_id": letterdist,
            "layout_id": scope.layout,
            "anchor_player_config_id": anchor,
            "anchor_rating": rating,
        })
    };

    for (what, bad) in [
        ("a variant no job has", body("scrabble", scope.letterdist, 2000.0)),
        ("a layout as the distribution", body("classic", scope.layout, 2000.0)),
        ("an overflowing anchor rating", body("classic", scope.letterdist, 200_000.0)),
        ("an anchor that does not exist", {
            let mut b = body("classic", scope.letterdist, 2000.0);
            b["anchor_player_config_id"] = json!(Uuid::new_v4());
            b
        }),
        ("a board no job can be created on", {
            let mut board = b"10, 10\n".to_vec();
            for _ in 0..21 {
                board.extend_from_slice(&[b' '; 21]);
                board.push(b'\n');
            }
            let super21: Uuid = sqlx::query_scalar(
                "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
                 VALUES ('layouts/standard21.txt', 'layout', 'standard21', repeat('2', 64), $1,
                         '20260101', $2)
                 RETURNING id",
            )
            .bind(board.len() as i64)
            .bind(&board)
            .fetch_one(&db.pool)
            .await
            .unwrap();
            let mut b = body("classic", scope.letterdist, 2000.0);
            b["layout_id"] = json!(super21);
            b
        }),
        ("a letter distribution no job can be created on", {
            let garbled = b"this is not a letter distribution\n".to_vec();
            let garbled_id: Uuid = sqlx::query_scalar(
                "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
                 VALUES ('letterdists/garbled.csv', 'letterdist', 'garbled', repeat('3', 64), $1,
                         '20260101', $2)
                 RETURNING id",
            )
            .bind(garbled.len() as i64)
            .bind(&garbled)
            .fetch_one(&db.pool)
            .await
            .unwrap();
            body("classic", garbled_id, 2000.0)
        }),
    ] {
        let (status, response) = send(
            &app,
            request("POST", "/api/admin/rating-pools", &admin_headers(&state.cfg, admin), Some(bad)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {response}");
    }
    let pools: i64 = sqlx::query_scalar("SELECT count(*) FROM rating_pools")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(pools, 0);
}

/// A-RATE-4: adding a member and removing one each refit the pool and return
/// the new run, which includes the newcomer and then no longer does.
#[tokio::test]
async fn adding_and_removing_a_member_each_refit_the_pool() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let headers = admin_headers(&state.cfg, f.admin);
    let first = ratings::recompute(&db.pool, f.pool, Trigger::Manual).await.unwrap();
    let newcomer = db.static_player("c-newcomer", f.admin).await;
    let job = pairs_job(&db, "classic", f.scope, f.rival, newcomer).await;
    pair_result(&db, job, [0, 0, 1, 1, 0]).await;

    let members = format!("/api/admin/rating-pools/{}/members", f.pool);
    let (status, body) = send(
        &app,
        request("POST", &members, &headers, Some(json!({ "player_config_id": newcomer }))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let added: Uuid = body["run_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(added, first);
    let stored = stored_ratings(&db, added).await;
    assert!(stored.contains_key(&newcomer), "{stored:?}");
    assert_eq!(evidence(&db, added).await, (4, 2), "and its games with it");

    let (status, body) =
        send(&app, request("DELETE", &format!("{members}/{newcomer}"), &headers, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let removed: Uuid = body["run_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(removed, added);
    assert!(!stored_ratings(&db, removed).await.contains_key(&newcomer));
    assert_eq!(evidence(&db, removed).await, (2, 1));

    let triggers: Vec<String> = sqlx::query_scalar(
        "SELECT trigger FROM rating_runs WHERE id = ANY($1) ORDER BY computed_at",
    )
    .bind(vec![added, removed])
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(triggers, ["membership", "membership"]);
    assert_eq!(run_count(&db, f.pool).await, 3);
}

/// A-RATE-4c: adding a config that is already a member -- a second click, or
/// the anchor, which the pool was created with -- adds nothing, and is
/// answered `run_id: null` with no audit row and no refit. It was logged as an
/// addition and refitted, a `membership` run the ratings page showed as the
/// reason for a change that did not happen.
#[tokio::test]
async fn adding_a_config_that_is_already_a_member_changes_nothing() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let headers = admin_headers(&state.cfg, f.admin);
    ratings::recompute(&db.pool, f.pool, Trigger::Manual).await.unwrap();
    let runs = run_count(&db, f.pool).await;

    let members = format!("/api/admin/rating-pools/{}/members", f.pool);
    for member in [f.rival, f.anchor] {
        let (status, body) = send(
            &app,
            request("POST", &members, &headers, Some(json!({ "player_config_id": member }))),
        )
        .await;
        assert_eq!((status, &body), (StatusCode::OK, &json!({ "run_id": null })));
    }
    assert_eq!(run_count(&db, f.pool).await, runs, "nothing refitted");
    let logged: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action = 'rating_pool.member_added'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(logged, 0, "nothing logged");
}

/// A-RATE-10: the detail lists the pool's members as they are now, not as
/// the latest fit rated them. A never-fitted pool lists its anchor; a member
/// added since the fit (or whose refit failed -- the membership commits
/// first) is listed with no rating; and one removed since is rated but not
/// listed. The page built its membership from the ratings, so it offered such
/// a member under "Add" and gave it no Remove button.
#[tokio::test]
async fn the_detail_lists_members_the_latest_fit_has_not_rated() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let f = fixture(&db).await;
    let path = format!("/api/rating-pools/{}", f.pool);
    let member = |id: Uuid, name: &str| json!({ "player_config_id": id, "name": name });

    let (status, never_fitted) = send(&app, get_request(&path, &[])).await;
    assert_eq!(status, StatusCode::OK, "{never_fitted}");
    assert_eq!(never_fitted["ratings"], json!([]), "{never_fitted}");
    assert_eq!(
        never_fitted["members"],
        json!([member(f.anchor, "a-anchor"), member(f.rival, "b-rival")]),
        "by name, the anchor among them"
    );

    ratings::recompute(&db.pool, f.pool, Trigger::Manual).await.unwrap();
    let newcomer = db.static_player("c-newcomer", f.admin).await;
    add_member(&db, f.pool, newcomer).await;
    remove_member(&db, f.pool, f.rival).await;

    let (status, after) = send(&app, get_request(&path, &[])).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(
        after["members"],
        json!([member(f.anchor, "a-anchor"), member(newcomer, "c-newcomer")]),
        "{after}"
    );
    let rated: Vec<&serde_json::Value> =
        after["ratings"].as_array().unwrap().iter().map(|r| &r["player_config_id"]).collect();
    assert_eq!(rated.len(), 2, "{after}");
    assert!(rated.contains(&&json!(f.rival)), "still rated by the latest fit");
    assert!(!rated.contains(&&json!(newcomer)), "not rated until the next fit");
}

/// A-RATE-4b: adding a config that does not exist is a 400 on its field, and
/// to a pool that does not exist a 404 -- not the 409 "still referenced" a
/// bare foreign-key failure maps to. Neither refits anything.
#[tokio::test]
async fn adding_an_unknown_member_or_to_an_unknown_pool_says_which() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let headers = admin_headers(&state.cfg, f.admin);
    let runs = run_count(&db, f.pool).await;

    let ghost = Uuid::new_v4();
    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/rating-pools/{}/members", f.pool),
            &headers,
            Some(json!({ "player_config_id": ghost })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["fields"][0]["field"], "player_config_id", "{body}");

    let (status, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/admin/rating-pools/{}/members", Uuid::new_v4()),
            &headers,
            Some(json!({ "player_config_id": f.rival })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(run_count(&db, f.pool).await, runs);
}

/// A config that is not a member is not removed: a second click was logged
/// as a removal and refitted the pool (the audit's pass 22).
#[tokio::test]
async fn removing_a_config_that_is_not_a_member_is_a_404() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let stranger = Uuid::new_v4();
    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/rating-pools/{}/members/{stranger}", f.pool),
            &admin_headers(&state.cfg, f.admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let logged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action = 'rating_pool.member_removed'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(logged, 0);
}

/// A-RATE-5: removing the anchor is refused, with a message that says what to
/// do instead -- move the anchor (A-RATE-8) -- and changes nothing.
#[tokio::test]
async fn removing_the_anchor_is_refused_with_the_fix_named() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;

    let (status, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/admin/rating-pools/{}/members/{}", f.pool, f.anchor),
            &admin_headers(&state.cfg, f.admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["message"].as_str().unwrap();
    assert!(message.contains("cannot remove the pool's anchor"), "{message}");
    assert!(message.contains("Make another member the anchor first"), "{message}");

    let still: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM rating_pool_members
                        WHERE pool_id = $1 AND player_config_id = $2)",
    )
    .bind(f.pool)
    .bind(f.anchor)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(still, "the anchor is still a member");
    assert_eq!(run_count(&db, f.pool).await, 0, "and nothing was refit");
}

/// A-RATE-6: history comes back oldest first whatever order the runs were
/// written in, and a config the run could not rate (no path to the anchor) is
/// left out rather than charted at a meaningless number.
#[tokio::test]
async fn history_is_in_time_order_and_leaves_out_unrated_configs() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rated = db.static_player("b-rated", admin).await;
    let unrated = db.static_player("c-unrated", admin).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[rated, unrated], 2000.0).await;

    // Written newest first, so an unordered read would come back backwards.
    for (at, rating) in [("2026-03-03", 2030.0), ("2026-03-01", 2010.0), ("2026-03-02", 2020.0)] {
        let run: Uuid = sqlx::query_scalar(
            "INSERT INTO rating_runs (pool_id, computed_at, trigger, iterations, converged,
                                      pairs_used, jobs_used)
             VALUES ($1, $2::timestamptz, 'evidence', 1, true, 1, 1) RETURNING id",
        )
        .bind(pool)
        .bind(at)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO player_config_ratings
                 (run_id, player_config_id, rating, stderr, pairs_played,
                  connected_to_anchor, is_anchor)
             VALUES ($1, $2, 2000, 0, 1, true, true),
                    ($1, $3, $5, 10, 1, true, false),
                    ($1, $4, 1500, 1e300, 0, false, false)",
        )
        .bind(run)
        .bind(anchor)
        .bind(rated)
        .bind(unrated)
        .bind(rating)
        .execute(&db.pool)
        .await
        .unwrap();
    }

    let (status, body) =
        send(&app, get_request(&format!("/api/rating-pools/{pool}/history"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let points: Vec<(String, String, f64)> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["computed_at"].as_str().unwrap().to_string(),
                p["name"].as_str().unwrap().to_string(),
                p["rating"].as_f64().unwrap(),
            )
        })
        .collect();
    let expected: Vec<(String, String, f64)> = [
        ("2026-03-01T00:00:00Z", "a-anchor", 2000.0),
        ("2026-03-01T00:00:00Z", "b-rated", 2010.0),
        ("2026-03-02T00:00:00Z", "a-anchor", 2000.0),
        ("2026-03-02T00:00:00Z", "b-rated", 2020.0),
        ("2026-03-03T00:00:00Z", "a-anchor", 2000.0),
        ("2026-03-03T00:00:00Z", "b-rated", 2030.0),
    ]
    .into_iter()
    .map(|(at, name, rating)| (at.to_string(), name.to_string(), rating))
    .collect();
    assert_eq!(points, expected);
}

/// A-RATE-6b: history carries only what the chart draws -- the six current
/// members rated highest in the newest run -- where it sent every member's
/// every point on each view of a public page; and a config removed from the
/// pool takes none of the six, however high it was rated.
#[tokio::test]
async fn history_carries_the_six_highest_current_members() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("anchor", admin).await;
    let mut others = Vec::new();
    for i in 0..8 {
        others.push(db.static_player(&format!("m{i}"), admin).await);
    }
    let removed = db.static_player("removed", admin).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &others, 2000.0).await;

    // Two runs; member i rated 1900 + 10 i, the removed config above them all.
    for at in ["2026-03-01", "2026-03-02"] {
        let run: Uuid = sqlx::query_scalar(
            "INSERT INTO rating_runs (pool_id, computed_at, trigger, iterations, converged,
                                      pairs_used, jobs_used)
             VALUES ($1, $2::timestamptz, 'evidence', 1, true, 1, 1) RETURNING id",
        )
        .bind(pool)
        .bind(at)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let mut rows = vec![(anchor, 2000.0), (removed, 2500.0)];
        rows.extend(others.iter().enumerate().map(|(i, c)| (*c, 1900.0 + 10.0 * i as f64)));
        for (config, rating) in rows {
            sqlx::query(
                "INSERT INTO player_config_ratings
                     (run_id, player_config_id, rating, stderr, pairs_played,
                      connected_to_anchor, is_anchor)
                 VALUES ($1, $2, $3, 10, 1, true, $4)",
            )
            .bind(run)
            .bind(config)
            .bind(rating)
            .bind(config == anchor)
            .execute(&db.pool)
            .await
            .unwrap();
        }
    }

    let (status, body) =
        send(&app, get_request(&format!("/api/rating-pools/{pool}/history"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut names: Vec<String> =
        body.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names.len(), 12, "six configs at two runs: {names:?}");
    names.sort();
    names.dedup();
    assert_eq!(names, ["anchor", "m3", "m4", "m5", "m6", "m7"]);
}

/// A-RATE-7: recompute is an admin action -- refused without a session and
/// with a non-admin one, before anything is fit -- and an admin's stores a
/// new manual run and returns it.
#[tokio::test]
async fn only_an_admin_can_recompute_and_it_stores_a_new_run() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let path = format!("/api/admin/rating-pools/{}/recompute", f.pool);

    let (status, body) = send(&app, request("POST", &path, &[], None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let member = db.user("member", false).await;
    let token = birdtest::auth::session::issue(&state.cfg, member, "member", false, 0).unwrap();
    let not_admin = vec![
        ("cookie".to_string(), format!("birdtest_session={token}; birdtest_csrf=testcsrf")),
        ("x-csrf-token".to_string(), "testcsrf".to_string()),
    ];
    let (status, body) = send(&app, request("POST", &path, &not_admin, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["message"], "admin privileges required");
    assert_eq!(run_count(&db, f.pool).await, 0, "a refused request fits nothing");

    let (status, body) =
        send(&app, request("POST", &path, &admin_headers(&state.cfg, f.admin), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let run: Uuid = body["run_id"].as_str().unwrap().parse().unwrap();
    let trigger: String = sqlx::query_scalar("SELECT trigger FROM rating_runs WHERE id = $1")
        .bind(run)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(trigger, "manual");
    assert_eq!(evidence(&db, run).await, (2, 1));
    assert_eq!(run_count(&db, f.pool).await, 1);
}

/// A self-play job (both seats one config, which creation allows) is not
/// counted as evidence the fit used: the fit ignores it, and the page's "over
/// N pairs from M jobs" said otherwise.
#[tokio::test]
async fn a_self_play_job_is_not_counted_as_evidence() {
    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let scope = scope(&db).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let real = pairs_job(&db, "classic", scope, anchor, rival).await;
    pair_result(&db, real, [1, 2, 3, 2, 1]).await;
    let mirror = pairs_job(&db, "classic", scope, rival, rival).await;
    pair_result(&db, mirror, [0, 5, 10, 5, 0]).await;
    let pool = pool(&db, "pool", "classic", scope, anchor, &[rival], 2000.0).await;

    let run = ratings::recompute(&db.pool, pool, Trigger::Manual).await.unwrap();
    assert_eq!(evidence(&db, run).await, (9, 1), "the real job's nine pairs, and it alone");
}

// ---------------------------------------------------------------------------
// A-RATE: editing the anchor, and deleting a pool
// ---------------------------------------------------------------------------

async fn pool_row(db: &TestDb, pool: Uuid) -> (Uuid, f64) {
    sqlx::query_as("SELECT anchor_player_config_id, anchor_rating FROM rating_pools WHERE id = $1")
        .bind(pool)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn is_member(db: &TestDb, pool: Uuid, config: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM rating_pool_members
                        WHERE pool_id = $1 AND player_config_id = $2)",
    )
    .bind(pool)
    .bind(config)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// A-RATE-8: moving the anchor to a config that is not yet a member adds it,
/// pins it at the new rating, and refits in the same request as an `anchor`
/// run; the old anchor is then an ordinary member that can be removed (the fix
/// A-RATE-5 names). Moving only the rating rescales everyone by the same
/// amount, and sending what is already there changes nothing.
#[tokio::test]
async fn an_anchor_change_refits_sets_the_anchor_and_adds_it_as_a_member() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let headers = admin_headers(&state.cfg, f.admin);
    let newcomer = db.static_player("c-newcomer", f.admin).await;
    let job = pairs_job(&db, "classic", f.scope, f.rival, newcomer).await;
    pair_result(&db, job, [0, 0, 1, 1, 0]).await;
    let path = format!("/api/admin/rating-pools/{}", f.pool);

    let (status, body) = send(
        &app,
        request(
            "PATCH",
            &path,
            &headers,
            Some(json!({ "anchor_player_config_id": newcomer, "anchor_rating": 1500.0 })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let run: Uuid = body["run_id"].as_str().unwrap().parse().unwrap();
    assert!(is_member(&db, f.pool, newcomer).await, "the new anchor joined the pool");
    assert_eq!(pool_row(&db, f.pool).await, (newcomer, 1500.0));
    let stored = stored_ratings(&db, run).await;
    assert_eq!(stored.len(), 3, "{stored:?}");
    assert!(stored[&newcomer].is_anchor && !stored[&f.anchor].is_anchor, "{stored:?}");
    assert_eq!(stored[&newcomer].rating, 1500.0);
    assert_eq!(evidence(&db, run).await, (4, 2), "the newcomer's games came in with it");
    let trigger: String = sqlx::query_scalar("SELECT trigger FROM rating_runs WHERE id = $1")
        .bind(run)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(trigger, "anchor");
    let logged: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT action, reason FROM audit_log WHERE action LIKE 'rating_pool.%' ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        logged,
        [
            ("rating_pool.member_added".to_string(), None),
            (
                "rating_pool.anchor_changed".to_string(),
                Some(format!("anchor={}@2000 -> {newcomer}@1500", f.anchor)),
            ),
        ]
    );

    // The rating alone: every rating moves by the same 100.
    let (status, body) =
        send(&app, request("PATCH", &path, &headers, Some(json!({ "anchor_rating": 1600.0 })))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shifted = stored_ratings(&db, body["run_id"].as_str().unwrap().parse().unwrap()).await;
    for (config, before) in &stored {
        assert!((shifted[config].rating - before.rating - 100.0).abs() < 1e-6, "{config}");
    }

    // Nothing to change: no run, no log.
    let runs = run_count(&db, f.pool).await;
    let (status, body) = send(
        &app,
        request("PATCH", &path, &headers, Some(json!({ "anchor_player_config_id": newcomer }))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["run_id"], serde_json::Value::Null);
    assert_eq!(run_count(&db, f.pool).await, runs);

    // The old anchor is now removable.
    let (status, body) = send(
        &app,
        request("DELETE", &format!("{path}/members/{}", f.anchor), &headers, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!is_member(&db, f.pool, f.anchor).await);
}

/// A-RATE-8b: an anchor that does not exist is a 400 on its field, an anchor
/// rating out of range or an empty body a 400, and an unknown pool a 404.
/// None of them changes the pool, its members, or its runs.
#[tokio::test]
async fn a_bad_anchor_change_is_refused_and_changes_nothing() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let headers = admin_headers(&state.cfg, f.admin);
    let path = format!("/api/admin/rating-pools/{}", f.pool);
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rating_pool_members")
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let ghost = Uuid::new_v4();
    for (body, field) in [
        (json!({ "anchor_player_config_id": ghost }), Some("anchor_player_config_id")),
        (json!({ "anchor_rating": 1.0e6 }), Some("anchor_rating")),
        (json!({}), None),
    ] {
        let (status, response) = send(&app, request("PATCH", &path, &headers, Some(body.clone()))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        if let Some(field) = field {
            assert_eq!(response["fields"][0]["field"], field, "{response}");
        }
    }
    let (status, response) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/admin/rating-pools/{}", Uuid::new_v4()),
            &headers,
            Some(json!({ "anchor_rating": 1500.0 })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{response}");

    assert_eq!(pool_row(&db, f.pool).await, (f.anchor, 2000.0));
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rating_pool_members")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(after, members);
    assert_eq!(run_count(&db, f.pool).await, 0);
}

/// A-RATE-9: deleting a pool takes its members, runs, ratings and residuals
/// with it, writes its census first, and is a 404 afterwards -- to read and to
/// delete again. It frees the configs it anchored and rated, which could not
/// be deleted while it stood; the games stay with their job.
#[tokio::test]
async fn deleting_a_pool_cascades_frees_its_configs_and_is_a_404_after() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let f = fixture(&db).await;
    let headers = admin_headers(&state.cfg, f.admin);
    ratings::recompute(&db.pool, f.pool, Trigger::Manual).await.unwrap();
    let path = format!("/api/admin/rating-pools/{}", f.pool);

    // A second pool anchored on a config nothing else uses: its delete is
    // what frees that config.
    let idle = db.static_player("c-idle", f.admin).await;
    let idle_pool = pool(&db, "idle", "classic", f.scope, idle, &[], 2000.0).await;
    let delete_idle = format!("/api/admin/player-configs/{idle}");
    let (status, body) = send(&app, request("DELETE", &delete_idle, &headers, None)).await;
    assert_eq!(status, StatusCode::CONFLICT, "the pool pins its anchor: {body}");

    let (status, body) = send(&app, request("DELETE", &path, &headers, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    for table in ["rating_runs", "player_config_ratings", "rating_run_residuals"] {
        let left: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "{table}");
    }
    assert!(!is_member(&db, f.pool, f.anchor).await && !is_member(&db, f.pool, f.rival).await);
    let games: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_results")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(games > 0, "the games are the job's, not the pool's");
    let census: String = sqlx::query_scalar(
        "SELECT reason FROM audit_log WHERE action = 'rating_pool.deleted.census'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(census, "name=pool members=2 runs=1");

    let (status, body) = send(&app, request("GET", &format!("/api/rating-pools/{}", f.pool), &[], None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = send(&app, request("DELETE", &path, &headers, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = send(
        &app,
        request("POST", &format!("{path}/members"), &headers, Some(json!({ "player_config_id": f.rival }))),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) =
        send(&app, request("DELETE", &format!("/api/admin/rating-pools/{idle_pool}"), &headers, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = send(&app, request("DELETE", &delete_idle, &headers, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the anchor is free: {body}");
}

/// Counts the ERROR events of the future it is attached to.
struct CountErrors(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CountErrors {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// I-RATE-12: a pool deleted after the sweep listed the pools -- here while
/// the sweep waits on its fit lock, which the delete holds -- is skipped
/// without an error logged, and the sweep goes on to the next pool.
#[tokio::test]
async fn the_sweep_skips_a_pool_deleted_mid_sweep_quietly() {
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::layer::SubscriberExt;

    let db = TestDb::new().await;
    let admin = db.user("root", true).await;
    let anchor = db.static_player("a-anchor", admin).await;
    let rival = db.static_player("b-rival", admin).await;
    let scope = scope(&db).await;
    let job = pairs_job(&db, "classic", scope, anchor, rival).await;
    pair_result(&db, job, [0, 1, 1, 0, 0]).await;
    let doomed = pool(&db, "doomed", "classic", scope, anchor, &[rival], 2000.0).await;
    let kept = pool(&db, "kept", "classic", scope, anchor, &[rival], 2000.0).await;

    // The delete's own first step: the doomed pool's fit lock.
    let mut deleting = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(2, hashtext($1::text))")
        .bind(doomed)
        .execute(&mut *deleting)
        .await
        .unwrap();
    let errors = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(CountErrors(errors.clone()));
    let sweeping = db.pool.clone();
    let sweep = tokio::spawn(
        async move { ratings::recompute_stale(&sweeping).await }.with_subscriber(subscriber),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        if waiting > 0 || sweep.is_finished() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the sweep never reached the doomed pool");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    sqlx::query("DELETE FROM rating_pools WHERE id = $1")
        .bind(doomed)
        .execute(&mut *deleting)
        .await
        .unwrap();
    deleting.commit().await.unwrap();

    let fitted = sweep.await.unwrap().unwrap();
    assert_eq!(errors.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing logged as a failure");
    assert_eq!(run_count(&db, kept).await, 1, "the other pool was still fitted");
    // The kept pool may have been fitted before the doomed one or after it,
    // but the doomed one never was.
    assert_eq!(fitted, 1);
}
