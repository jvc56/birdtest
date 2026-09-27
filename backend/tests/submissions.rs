//! Result submission against a real database (`I-SUBMIT-*`): what each job
//! type's result writes, what the schema refuses, the task counters an
//! accepted result moves, and how captured positions are stored.

mod common;

use axum::http::StatusCode;
use axum::Router;
use common::*;
use serde_json::{json, Value};
use uuid::Uuid;

/// Claims with no identity, returning the assignment and the minted UUID.
async fn first_claim(app: &Router) -> (Value, String) {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().expect("a minted worker_uuid").to_string();
    (body, uuid)
}

async fn submit(app: &Router, assignment: &Value, uuid: &str, result: Value) -> (StatusCode, Value) {
    send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": assignment["claim_token"], "result": result }),
        ),
    )
    .await
}

/// Claims one task and submits `result` for it, asserting it was accepted.
async fn claim_and_submit(app: &Router, result: Value) -> Value {
    let (assignment, uuid) = first_claim(app).await;
    let (status, body) = submit(app, &assignment, &uuid, result).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));
    assignment
}

/// The job's one `game_results` row, as JSON keyed by column.
async fn game_result_row(db: &TestDb, job: Uuid) -> serde_json::Map<String, Value> {
    let rows: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM game_results r WHERE job_id = $1")
            .bind(job)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1, "one row per accepted result: {rows:?}");
    rows[0].as_object().unwrap().clone()
}

const PENTANOMIAL: [&str; 5] = ["pent_0", "pent_1", "pent_2", "pent_3", "pent_4"];
const DIVERGENT: [&str; 4] = ["divergent_games", "divergent_wins", "divergent_losses", "divergent_ties"];

/// I-SUBMIT-1: a `games` result inserts exactly one `game_results` row with
/// its counts, and NULL in every pentanomial and divergent column -- even when
/// the worker sends those fields, since a plain games job plays no pairs.
#[tokio::test]
async fn a_games_result_stores_one_row_with_no_pair_columns() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let app = birdtest::app(db.state().await);

    let mut result = games_result(2, 1);
    result["pentanomial"] = json!([0, 0, 1, 0, 0]);
    result["divergent_games"] = json!({
        "games": 2, "wins": 1, "losses": 1, "ties": 0,
        "p1_score_mean": 420.0, "p1_score_sd": 60.0, "p2_score_mean": 410.0, "p2_score_sd": 58.0,
    });
    let assignment = claim_and_submit(&app, result).await;

    let row = game_result_row(&db, job).await;
    for (column, expected) in [("games", 2), ("wins", 1), ("losses", 1), ("ties", 0)] {
        assert_eq!(row[column], expected, "{column}: {row:?}");
    }
    for column in PENTANOMIAL.iter().chain(DIVERGENT.iter()) {
        assert!(row[*column].is_null(), "{column} is NULL for a games job: {row:?}");
    }
    let task: Uuid = sqlx::query_scalar(
        "SELECT task_id FROM task_claims WHERE claim_token = $1",
    )
    .bind(assignment["claim_token"].as_str().unwrap().parse::<Uuid>().unwrap())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(row["task_id"], json!(task), "the row names the claimed task");
}

/// An active `game_pairs` job at two pairs a batch, between two static
/// players.
async fn pairs_job(db: &TestDb) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let p1 = db.static_player(&format!("p1{}", Uuid::new_v4().simple()), admin).await;
    let p2 = db.static_player(&format!("p2{}", Uuid::new_v4().simple()), admin).await;
    let job = db.bare_job("game_pairs", 1, admin).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 2, 1000000, 1000000)",
    )
    .bind(job)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

/// Two pairs, one split and one won both by player 1. Four games, 3 wins and 1 loss: buckets 2 (a split) and 4 (won both), which
/// agree with the counts on pairs (2 = 4/2) and half-points (2 + 4 = 2*3 + 0).
fn pairs_result() -> Value {
    let mut result = games_result(4, 3);
    result["pentanomial"] = json!([0, 0, 1, 0, 1]);
    result["divergent_games"] = json!({
        "games": 2, "wins": 2, "losses": 0, "ties": 0,
        "p1_score_mean": 420.0, "p1_score_sd": 60.0, "p2_score_mean": 410.0, "p2_score_sd": 58.0,
    });
    result
}

/// The name of the constraint an update of the job's result violates.
async fn violated_constraint(db: &TestDb, job: Uuid, set: &str) -> String {
    let err = sqlx::query(&format!("UPDATE game_results SET {set} WHERE job_id = $1"))
        .bind(job)
        .execute(&db.pool)
        .await
        .expect_err("the schema refuses a pentanomial that contradicts its counts");
    let db_err = err.as_database_error().expect("a database error");
    assert_eq!(db_err.code().as_deref(), Some("23514"), "a CHECK violation: {db_err}");
    db_err.constraint().unwrap().to_string()
}

/// I-SUBMIT-2: a `game_pairs` result stores its pentanomial and divergent
/// summary, and the database's CHECK refuses a row whose buckets disagree
/// with its counts -- on the pair count and, separately, on player 1's
/// half-points with the pair count still right. Both halves are clauses of the
/// one named constraint, `game_results_pentanomial_all_or_nothing`, and each is
/// shown to fire on its own. The endpoint refuses the same contradictions
/// before they reach the schema, with a message that says which.
#[tokio::test]
async fn a_pairs_result_stores_its_pentanomial_and_the_schema_refuses_a_contradiction() {
    let db = TestDb::new().await;
    let job = pairs_job(&db).await;
    let app = birdtest::app(db.state().await);
    claim_and_submit(&app, pairs_result()).await;

    let row = game_result_row(&db, job).await;
    for (column, expected) in PENTANOMIAL.iter().zip([0, 0, 1, 0, 1]) {
        assert_eq!(row[*column], expected, "{column}: {row:?}");
    }
    for (column, expected) in DIVERGENT.iter().zip([2, 2, 0, 0]) {
        assert_eq!(row[*column], expected, "{column}: {row:?}");
    }

    // One extra pair in a bucket: three pairs for four games.
    assert_eq!(
        violated_constraint(&db, job, "pent_0 = pent_0 + 1").await,
        "game_results_pentanomial_all_or_nothing",
        "the pair-count clause"
    );
    // Still two pairs, but a split moved to a loss-both: 0 + 4 half-points
    // where the counts say 2*3 + 0 = 6.
    assert_eq!(
        violated_constraint(&db, job, "pent_2 = pent_2 - 1, pent_0 = pent_0 + 1").await,
        "game_results_pentanomial_all_or_nothing",
        "the half-point clause"
    );
    // The right pair count and half-points, but two win-and-draw pairs where
    // the counts hold no draw.
    assert_eq!(
        violated_constraint(&db, job, "pent_2 = 0, pent_3 = 2, pent_4 = 0").await,
        "game_results_pentanomial_all_or_nothing",
        "the draw clause"
    );
    // And a partial pentanomial is refused by the same constraint.
    assert_eq!(
        violated_constraint(&db, job, "pent_4 = NULL").await,
        "game_results_pentanomial_all_or_nothing"
    );

    for (pentanomial, message) in [
        (json!([0, 0, 2, 0, 1]), "pentanomial pair count must be exactly half the games played"),
        (json!([1, 0, 0, 0, 1]), "pentanomial disagrees with the game counts about player 1's score"),
        (json!([0, 0, 0, 2, 0]), "pentanomial disagrees with the game counts about the draws"),
    ] {
        let mut result = pairs_result();
        result["pentanomial"] = pentanomial;
        let (assignment, uuid) = first_claim(&app).await;
        let (status, body) = submit(&app, &assignment, &uuid, result).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["message"], message, "{body}");
    }
}

/// `(accepted_count, active_claim_count, state, completed_at IS NOT NULL)`.
async fn task_counters(db: &TestDb, task: Uuid) -> (i32, i32, String, bool) {
    sqlx::query_as(
        "SELECT accepted_count, active_claim_count, state::text, completed_at IS NOT NULL
         FROM tasks WHERE id = $1",
    )
    .bind(task)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

async fn tasks_completed(db: &TestDb, job: Uuid) -> i64 {
    sqlx::query_scalar("SELECT tasks_completed FROM jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// I-SUBMIT-3: each accepted result increments the task's `accepted_count`
/// and gives back its live claim; the task stays on offer while redundancy is
/// unfilled, and completes -- with `completed_at`, and the job's
/// `tasks_completed` bumped once -- at the result that reaches `redundancy`.
#[tokio::test]
async fn accepted_results_move_the_task_counters_and_complete_it_at_redundancy() {
    let db = TestDb::new().await;
    let job = db.games_job(3, 2).await;
    let app = birdtest::app(db.state().await);

    let (first, first_uuid) = first_claim(&app).await;
    let (second, second_uuid) = first_claim(&app).await;
    assert_eq!(first["task_request"]["seed"], second["task_request"]["seed"], "one task");
    let task: Uuid = sqlx::query_scalar("SELECT id FROM tasks WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(task_counters(&db, task).await, (0, 2, "available".into(), false));

    let (_, body) = submit(&app, &first, &first_uuid, games_result(2, 1)).await;
    assert_eq!(body, json!({ "accepted": true }));
    assert_eq!(task_counters(&db, task).await, (1, 1, "available".into(), false));

    let (_, body) = submit(&app, &second, &second_uuid, games_result(2, 1)).await;
    assert_eq!(body, json!({ "accepted": true }));
    assert_eq!(task_counters(&db, task).await, (2, 0, "available".into(), false));
    assert_eq!(tasks_completed(&db, job).await, 0);

    // The third slot: claimed, the task is at capacity; accepted, it is done.
    let (third, third_uuid) = first_claim(&app).await;
    assert_eq!(third["task_request"]["seed"], first["task_request"]["seed"], "the same task");
    assert_eq!(task_counters(&db, task).await, (2, 1, "claimed".into(), false));
    let (_, body) = submit(&app, &third, &third_uuid, games_result(2, 1)).await;
    assert_eq!(body, json!({ "accepted": true }));
    assert_eq!(task_counters(&db, task).await, (3, 0, "completed".into(), true));
    assert_eq!(tasks_completed(&db, job).await, 1, "counted once, on the transition");
}

/// A games job capturing positions, whose player 1 keeps `keep` moves per
/// position.
async fn capturing_job(db: &TestDb, keep: i32) -> Uuid {
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE player_configs SET num_plays_recorded = $2
         WHERE id = (SELECT player1_config_id FROM job_game_config WHERE job_id = $1)",
    )
    .bind(job)
    .bind(keep)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

/// A captured position with `moves` ranked moves, best (highest equity) first.
fn position(game_index: i32, num_moves: i32, moves: usize) -> Value {
    let moves: Vec<Value> = (0..moves)
        .map(|i| json!({ "move": format!("move-{i}"), "score": 60 - i as i32, "equity": 70.0 - i as f64 }))
        .collect();
    json!({
        "game_index": game_index, "turn_number": 0, "rack": "AEINRST", "position": "cgp",
        "num_moves": num_moves, "moves": moves,
    })
}

/// I-SUBMIT-6: a captured position keeps only player 1's
/// `num_plays_recorded` moves -- the best ones, in rank order -- however many
/// the worker sent, and `num_moves` keeps the count ranked before that
/// truncation.
#[tokio::test]
async fn captured_positions_keep_the_configured_moves_and_the_full_count() {
    let db = TestDb::new().await;
    let job = capturing_job(&db, 3).await;
    let app = birdtest::app(db.state().await);

    let mut result = games_result(2, 1);
    result["positions"] = json!([position(0, 40, 5), position(1, 2, 2)]);
    claim_and_submit(&app, result).await;

    let records: Vec<(i16, i32, Vec<String>)> = sqlx::query_as(
        "SELECT r.game_index, r.num_moves,
                array_agg(m.move ORDER BY m.rank) FILTER (WHERE m.id IS NOT NULL)
         FROM position_analysis_records r
         LEFT JOIN position_analysis_moves m ON m.record_id = r.id
         WHERE r.job_id = $1
         GROUP BY r.id ORDER BY r.game_index",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        records,
        vec![
            (0, 40, vec!["move-0".into(), "move-1".into(), "move-2".into()]),
            (1, 2, vec!["move-0".into(), "move-1".into()]),
        ],
        "five sent, three kept, forty ranked; a shorter list is kept whole"
    );
}

/// I-SUBMIT-7: rank 1 is the best move -- the first of the worker's
/// best-first list -- and ranks follow the list; and the rank-1 lookup the
/// results feed makes through a record (`m.record_id = r.id AND m.rank = 1`)
/// is served by the `(record_id, rank)` index.
///
/// TESTING.md asks for a *partial index on rank 1 used by a dashboard
/// aggregate*. The schema removed both on purpose (the migration's comment on
/// `position_analysis_moves_record_idx`): the aggregate is gone and every
/// rank-1 read goes through its record. This pins that design instead: the
/// index that exists, what it covers, and that the planner uses it.
#[tokio::test]
async fn the_best_move_is_rank_one_and_is_read_through_the_record_index() {
    let db = TestDb::new().await;
    let job = capturing_job(&db, 10).await;
    let app = birdtest::app(db.state().await);
    let mut result = games_result(2, 1);
    result["positions"] = json!([position(0, 30, 4), position(1, 30, 4)]);
    claim_and_submit(&app, result).await;

    let ranks: Vec<(i16, String, f64, f64)> = sqlx::query_as(
        "SELECT m.rank, m.move, m.equity,
                MAX(m.equity) OVER (PARTITION BY m.record_id)
         FROM position_analysis_moves m
         JOIN position_analysis_records r ON r.id = m.record_id
         WHERE r.job_id = $1 ORDER BY r.game_index, m.rank",
    )
    .bind(job)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(ranks.len(), 8);
    for (rank, play, equity, best) in &ranks {
        assert_eq!(play, &format!("move-{}", rank - 1), "ranks follow the list");
        assert_eq!(*rank == 1, equity == best, "rank 1 is exactly the best move: {ranks:?}");
    }

    let definition: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE indexname = 'position_analysis_moves_record_idx'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(definition.ends_with("(record_id, rank)"), "{definition}");

    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL enable_seqscan = off").execute(&mut *tx).await.unwrap();
    let plan: Vec<String> = sqlx::query_scalar(&format!(
        "EXPLAIN SELECT r.id, m.move FROM position_analysis_records r
         LEFT JOIN position_analysis_moves m ON m.record_id = r.id AND m.rank = 1
         WHERE r.job_id = '{job}'"
    ))
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    let plan = plan.join("\n");
    assert!(plan.contains("position_analysis_moves_record_idx"), "{plan}");
}
