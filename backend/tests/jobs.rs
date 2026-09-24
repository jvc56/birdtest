//! Job creation and lifecycle against a real database (`I-JOB-*`): every SQL
//! string job creation touches, the validators that read real rows, and the
//! destructive paths that must leave nothing -- or exactly the audit record --
//! behind.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use common::*;
use serde_json::{json, Value};
use uuid::Uuid;

/// A request with an optional JSON body and the given headers.
fn request(method: &str, path: &str, headers: &[(String, String)], body: Option<Value>) -> Request<Body> {
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

/// An app and an admin session on it.
struct Admin {
    app: Router,
    headers: Vec<(String, String)>,
}

impl Admin {
    async fn new(db: &TestDb, state: birdtest::state::AppState) -> Self {
        let admin = db.user(&format!("root{}", Uuid::new_v4().simple()), true).await;
        let headers = admin_headers(&state.cfg, admin);
        Admin { app: birdtest::app(state), headers }
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        send(&self.app, request(method, path, &self.headers, body)).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.call("POST", path, Some(body)).await
    }

    /// A player config created through the API, returning its id.
    async fn player(&self, body: Value) -> Uuid {
        let (status, created) = self.post("/api/admin/player-configs", body).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        created["id"].as_str().unwrap().parse().unwrap()
    }

    /// A static player on the given files.
    async fn static_player(&self, name: &str, kwg: Uuid, klv: Uuid, extra: Value) -> Uuid {
        let mut body = json!({
            "name": name, "recorder_type": "all", "kwg_id": kwg, "klv_id": klv,
            "num_plays_recorded": 3,
        });
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        self.player(body).await
    }

    /// A simming player on the given files and win% model.
    async fn simmer(&self, name: &str, kwg: Uuid, klv: Uuid, winpct: Uuid) -> Uuid {
        self.player(json!({
            "name": name, "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
            "winpct_id": winpct, "num_plies": 2, "num_plays": 10, "max_iterations": 100,
            "time_limit_secs": 0, "num_plays_recorded": 1,
        }))
        .await
    }

    async fn create_job(&self, body: Value) -> (StatusCode, Value) {
        self.post("/api/admin/jobs", body).await
    }
}

/// The job's letter distribution and board.
async fn board(db: &TestDb) -> (Uuid, Uuid) {
    (db.input_data("letterdist", "english").await, db.input_data("layout", "standard15").await)
}

fn games_body(ld: Uuid, layout: Uuid, p1: Uuid, p2: Uuid) -> Value {
    json!({
        "job_type": "games", "variant": "classic", "letterdist_id": ld, "layout_id": layout,
        "player1_config_id": p1, "player2_config_id": p2, "min_games": 1, "max_games": 10,
    })
}

fn pairs_body(ld: Uuid, layout: Uuid, p1: Uuid, p2: Uuid) -> Value {
    json!({
        "job_type": "game_pairs", "variant": "classic", "letterdist_id": ld, "layout_id": layout,
        "player1_config_id": p1, "player2_config_id": p2, "min_pairs": 1, "max_pairs": 10,
    })
}

/// The whole row of a per-type config table, as JSON keyed by column.
async fn config_row(db: &TestDb, table: &str, job: Uuid) -> serde_json::Map<String, Value> {
    let row: Value = sqlx::query_scalar(&format!("SELECT to_jsonb(c) FROM {table} c WHERE job_id = $1"))
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("no {table} row for {job}: {e}"));
    row.as_object().unwrap().clone()
}

/// Every column of `row` is populated, and every one is exactly what
/// `expected` names -- no column left over on either side. Numbers compare as
/// numbers, since `to_jsonb` writes a whole double without its `.0`.
fn assert_reads_back(table: &str, row: &serde_json::Map<String, Value>, expected: &Value) {
    let expected = expected.as_object().unwrap();
    let mut columns: Vec<&String> = row.keys().collect();
    columns.sort();
    let mut named: Vec<&String> = expected.keys().collect();
    named.sort();
    assert_eq!(columns, named, "{table}: every column is accounted for");
    for (column, value) in row {
        assert!(!value.is_null(), "{table}.{column} is populated");
        let want = &expected[column];
        match (value.as_f64(), want.as_f64()) {
            (Some(got), Some(want)) => assert_eq!(got, want, "{table}.{column}"),
            _ => assert_eq!(value, want, "{table}.{column}"),
        }
    }
}

/// A job row as JSON.
async fn job_row(db: &TestDb, job: Uuid) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(j) FROM jobs j WHERE id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn job_count(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM jobs").fetch_one(&db.pool).await.unwrap()
}

/// I-JOB-1: a `games`, a `game_pairs` and an `opening_rack` job created through
/// the admin API each write their config row with every column populated, and
/// the row reads back exactly as requested -- including the settings that
/// differ from the column defaults, which is what catches a bind in the wrong
/// position.
#[tokio::test]
async fn each_job_type_stores_every_setting_it_was_created_with() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let (ld, layout) = board(&db).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = admin.static_player("p1", kwg, klv, json!({})).await;
    let p2 = admin.static_player("p2", kwg, klv, json!({})).await;

    // games, with every optional setting stated and none at its default.
    let (status, created) = admin
        .create_job(json!({
            "job_type": "games", "redundancy": 2, "variant": "wordsmog",
            "letterdist_id": ld, "layout_id": layout, "min_magpie_version": "1.2.3",
            "player1_config_id": p1, "player2_config_id": p2, "games_per_batch": 4,
            "min_games": 100, "max_games": 2000, "sprt_alpha": 0.07, "sprt_beta": 0.03,
            "elo_low": -5.0, "elo_high": 15.0, "capture_positions": true,
        }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let games: Uuid = created["job"]["id"].as_str().unwrap().parse().unwrap();
    let row = config_row(&db, "job_game_config", games).await;
    assert_reads_back(
        "job_game_config",
        &row,
        &json!({
            "job_id": games, "player1_config_id": p1, "player2_config_id": p2,
            "games_per_batch": 4, "min_games": 100, "max_games": 2000, "sprt_alpha": 0.07,
            "sprt_beta": 0.03, "elo_low": -5.0, "elo_high": 15.0, "capture_positions": true,
        }),
    );
    let job = job_row(&db, games).await;
    for (column, expected) in [
        ("job_type", json!("games")),
        ("redundancy", json!(2)),
        ("variant", json!("wordsmog")),
        ("letterdist_id", json!(ld)),
        ("layout_id", json!(layout)),
        ("min_magpie_major", json!(1)),
        ("min_magpie_minor", json!(2)),
        ("min_magpie_patch", json!(3)),
        ("status", json!("inactive")),
        ("allocation", Value::Null),
    ] {
        assert_eq!(job[column], expected, "jobs.{column}: {job}");
    }

    // game_pairs, the same way; the pair order is swapped so a transposed bind
    // would show.
    let (status, created) = admin
        .create_job(json!({
            "job_type": "game_pairs", "variant": "classic",
            "letterdist_id": ld, "layout_id": layout,
            "player1_config_id": p2, "player2_config_id": p1, "pairs_per_batch": 3,
            "min_pairs": 10, "max_pairs": 500, "sprt_alpha": 0.02, "sprt_beta": 0.08,
            "elo_low": -3.5, "elo_high": 6.5, "capture_positions": true,
        }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let pairs: Uuid = created["job"]["id"].as_str().unwrap().parse().unwrap();
    let row = config_row(&db, "job_game_pair_config", pairs).await;
    assert_reads_back(
        "job_game_pair_config",
        &row,
        &json!({
            "job_id": pairs, "player1_config_id": p2, "player2_config_id": p1,
            "pairs_per_batch": 3, "min_pairs": 10, "max_pairs": 500, "sprt_alpha": 0.02,
            "sprt_beta": 0.08, "elo_low": -3.5, "elo_high": 6.5, "capture_positions": true,
        }),
    );
    assert_eq!(job_row(&db, pairs).await["job_type"], "game_pairs");

    // opening_rack: `total_racks` is derived from the pinned distribution at
    // creation, so it is checked for being a real count rather than a value.
    let (status, created) = admin
        .create_job(json!({
            "job_type": "opening_rack", "variant": "classic",
            "letterdist_id": ld, "layout_id": layout,
            "player_config_id": p1, "racks_per_batch": 50, "rack_size": 5,
        }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let racks: Uuid = created["job"]["id"].as_str().unwrap().parse().unwrap();
    let row = config_row(&db, "job_opening_rack_config", racks).await;
    let total = row["total_racks"].as_i64().expect("total_racks is a count");
    assert!(total > 0, "the rack space was counted: {total}");
    assert_reads_back(
        "job_opening_rack_config",
        &row,
        &json!({
            "job_id": racks, "player_config_id": p1, "racks_per_batch": 50, "rack_size": 5,
            "total_racks": total,
        }),
    );
    assert_eq!(job_row(&db, racks).await["job_type"], "opening_rack");
}

/// A directory removed when dropped, so a test leaves nothing behind however
/// it ends.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("birdtest-jobs-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A stand-in for the one MAGPIE command leave-job creation runs,
/// `createdata klv <name> <letterdist>`, which writes a zeroed KLV into the
/// scratch directory's lexica. What is under test is the config row, not the
/// KLV build, and a real MAGPIE is a tier-6 precondition.
fn stub_magpie(dir: &TempDir) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.0.join("magpie");
    std::fs::write(
        &path,
        "#!/bin/sh\n[ \"$1\" = createdata ] && [ \"$2\" = klv ] || exit 1\n\
         printf 'zeroed-klv' > \"data/lexica/$3.klv2\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// I-JOB-1: a `leave_generation` job created through the admin API writes its
/// config row with every column populated, reading back as requested, and --
/// the part of its creation that happens after the commit -- its
/// generation-0 KLV row and object.
#[tokio::test]
async fn a_leave_generation_job_stores_every_setting_it_was_created_with() {
    let db = TestDb::new().await;
    let dir = TempDir::new();
    let (with_store, bucket) = db.state_with_object_store().await;
    let mut cfg = (*with_store.cfg).clone();
    cfg.magpie_bin = stub_magpie(&dir).to_string_lossy().into_owned();
    let admin = Admin::new(&db, db.state_with(cfg).await).await;
    let (ld, layout) = board(&db).await;
    let kwg = db.input_data("kwg", "NWL23").await;

    let (status, created) = admin
        .create_job(json!({
            "job_type": "leave_generation", "variant": "classic",
            "letterdist_id": ld, "layout_id": layout, "kwg_id": kwg,
            "num_iterations": 200, "generation_count": 3, "target_rack_count": 50,
            "racks_per_task": 20, "use_wordmap": false,
        }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let job: Uuid = created["job"]["id"].as_str().unwrap().parse().unwrap();
    let row = config_row(&db, "job_leave_config", job).await;
    assert_reads_back(
        "job_leave_config",
        &row,
        &json!({
            "job_id": job, "kwg_id": kwg, "num_iterations": 200, "generation_count": 3,
            "target_rack_count": 50, "racks_per_task": 20, "use_wordmap": false,
        }),
    );
    assert_eq!(job_row(&db, job).await["job_type"], "leave_generation");

    let key: String = sqlx::query_scalar(
        "SELECT artifact_key FROM leave_generation_artifacts WHERE job_id = $1 AND generation = 0",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(bucket.keys().await, vec![key], "the zeroed KLV is in the object store");
}

/// I-JOB-2: `validate_shared_player_options` reads the two stored configs --
/// the regression guard for the renamed `winpct_id` column. Two simmers on the
/// same win% model row are accepted; on different rows they are refused; two
/// configs with different `movegen_margin` are refused. For both job types
/// that have two players.
#[tokio::test]
async fn two_players_must_share_the_run_wide_settings_magpie_cannot_vary() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let (ld, layout) = board(&db).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let winpct = db.input_data("winpct", "winpct").await;
    let other_winpct = db.input_data("winpct", "winpct").await;

    let sim_a = admin.simmer("sim-a", kwg, klv, winpct).await;
    let sim_b = admin.simmer("sim-b", kwg, klv, winpct).await;
    let sim_other = admin.simmer("sim-other", kwg, klv, other_winpct).await;
    let margin_5 = admin.static_player("margin-5", kwg, klv, json!({ "movegen_margin": 5.0 })).await;
    let margin_7 = admin.static_player("margin-7", kwg, klv, json!({ "movegen_margin": 7.0 })).await;

    for body in [games_body, pairs_body] {
        let (status, created) = admin.create_job(body(ld, layout, sim_a, sim_b)).await;
        assert_eq!(status, StatusCode::CREATED, "the same win% model row: {created}");

        // Same name, different row: the id pins the bytes, not the name.
        let (status, refused) = admin.create_job(body(ld, layout, sim_a, sim_other)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert!(
            refused["message"].as_str().unwrap().contains("disagree on the win% model"),
            "{refused}"
        );

        let (status, refused) = admin.create_job(body(ld, layout, margin_5, margin_7)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert!(
            refused["message"].as_str().unwrap().contains("disagree on movegen_margin"),
            "{refused}"
        );
    }
    assert_eq!(job_count(&db).await, 2, "only the two accepted jobs exist");
}

/// I-JOB-3: `validate_player_compatibility` reads the players' real
/// `input_data` names. A lexicon from another distribution than the job's is
/// refused for each job type that has players; two English lexicons on an
/// English distribution are accepted.
#[tokio::test]
async fn a_job_whose_lexicons_do_not_fit_its_distribution_is_refused() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let (english, layout) = board(&db).await;
    let german = db.input_data("letterdist", "german").await;
    let nwl_kwg = db.input_data("kwg", "NWL23").await;
    let nwl_klv = db.input_data("klv", "NWL23").await;
    let csw_kwg = db.input_data("kwg", "CSW21").await;
    let csw_klv = db.input_data("klv", "CSW21").await;
    let rd_kwg = db.input_data("kwg", "RD28").await;
    let rd_klv = db.input_data("klv", "RD28").await;
    let nwl = admin.static_player("nwl", nwl_kwg, nwl_klv, json!({})).await;
    let csw = admin.static_player("csw", csw_kwg, csw_klv, json!({})).await;
    let rd = admin.static_player("rd", rd_kwg, rd_klv, json!({})).await;

    // Compatible: two English lexicons on the English distribution.
    let (status, created) = admin.create_job(games_body(english, layout, nwl, csw)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let refusals = [
        (games_body(german, layout, nwl, csw), "player1: lexicon \"NWL23\" is not compatible with letter distribution \"german\""),
        (pairs_body(english, layout, nwl, rd), "player2: lexicon \"RD28\" is not compatible with letter distribution \"english\""),
        (
            json!({
                "job_type": "opening_rack", "variant": "classic", "letterdist_id": german,
                "layout_id": layout, "player_config_id": csw,
            }),
            "player: lexicon \"CSW21\" is not compatible with letter distribution \"german\"",
        ),
    ];
    for (body, message) in refusals {
        let (status, refused) = admin.create_job(body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert_eq!(refused["message"], message, "{refused}");
    }
    assert_eq!(job_count(&db).await, 1, "a refused job leaves no row behind");
}

/// I-JOB-4: a job naming an `input_data` row -- or a player config -- that
/// does not exist is a 400 that says which, never the foreign-key violation
/// the insert would otherwise raise, and the half-written job is rolled back.
#[tokio::test]
async fn a_job_naming_something_that_does_not_exist_is_a_clean_400() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let (ld, layout) = board(&db).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let player = admin.static_player("p", kwg, klv, json!({})).await;
    let missing = Uuid::new_v4();

    let leave = |kwg_id: Uuid| {
        json!({
            "job_type": "leave_generation", "variant": "classic", "letterdist_id": ld,
            "layout_id": layout, "kwg_id": kwg_id, "num_iterations": 10,
            "target_rack_count": 10, "racks_per_task": 10,
        })
    };
    let cases = [
        (games_body(missing, layout, player, player), format!("no input data row {missing}")),
        (games_body(ld, missing, player, player), format!("no input data row {missing}")),
        (games_body(ld, layout, player, missing), "player config not found".to_string()),
        (pairs_body(ld, layout, missing, player), "player config not found".to_string()),
        (
            json!({
                "job_type": "opening_rack", "variant": "classic", "letterdist_id": ld,
                "layout_id": layout, "player_config_id": missing,
            }),
            "no such player config".to_string(),
        ),
        (leave(missing), "no such input data row".to_string()),
    ];
    for (body, message) in cases {
        let (status, refused) = admin.create_job(body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {refused}");
        assert_eq!(refused["code"], "bad_request", "{refused}");
        assert_eq!(refused["message"], message.as_str(), "{body}: {refused}");
    }
    assert_eq!(job_count(&db).await, 0, "nothing was half-created");

    // The same for a player config naming a lexicon that is not there.
    let (status, refused) = admin
        .post(
            "/api/admin/player-configs",
            json!({
                "name": "ghost", "recorder_type": "all", "kwg_id": missing, "klv_id": klv,
                "num_plays_recorded": 1,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["message"], format!("no input data row {missing}"), "{refused}");
}

/// A games job between two static players, created through the API.
async fn api_games_job(db: &TestDb, admin: &Admin) -> Uuid {
    let (ld, layout) = board(db).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let p1 = admin.static_player(&format!("p1-{}", Uuid::new_v4()), kwg, klv, json!({})).await;
    let p2 = admin.static_player(&format!("p2-{}", Uuid::new_v4()), kwg, klv, json!({})).await;
    let (status, created) = admin.create_job(games_body(ld, layout, p1, p2)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["job"]["id"].as_str().unwrap().parse().unwrap()
}

/// `(status, allocation, activated_at IS NOT NULL, deactivated_at IS NOT NULL)`.
async fn lifecycle(db: &TestDb, job: Uuid) -> (String, Option<i32>, bool, bool) {
    sqlx::query_as(
        "SELECT status::text, allocation, activated_at IS NOT NULL, deactivated_at IS NOT NULL
         FROM jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

async fn task_count(db: &TestDb, job: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE job_id = $1")
        .bind(job)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn claim(app: &Router) -> StatusCode {
    send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await.0
}

/// I-JOB-6: a job is created inactive with no allocation; activation sets its
/// allocation and `activated_at` and puts it on offer; deactivation takes it
/// off offer without destroying its tasks, and it can be activated again;
/// completion is terminal -- neither activation nor deactivation moves it
/// again, and nothing is dispatched from it.
#[tokio::test]
async fn a_job_moves_through_its_lifecycle_and_completion_is_final() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let job = api_games_job(&db, &admin).await;
    assert_eq!(lifecycle(&db, job).await, ("inactive".into(), None, false, false));
    assert_eq!(claim(&admin.app).await, StatusCode::NO_CONTENT, "an inactive job is not offered");

    let (status, body) =
        admin.post(&format!("/api/admin/jobs/{job}/activate"), json!({ "allocation": 40 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["allocation"], 40, "{body}");
    assert_eq!(lifecycle(&db, job).await, ("active".into(), Some(40), true, false));
    assert_eq!(claim(&admin.app).await, StatusCode::OK, "an active job is offered");
    assert_eq!(task_count(&db, job).await, 1);

    let (status, body) = admin.call("POST", &format!("/api/admin/jobs/{job}/deactivate"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (state, _, _, deactivated) = lifecycle(&db, job).await;
    assert_eq!((state.as_str(), deactivated), ("inactive", true));
    assert_eq!(task_count(&db, job).await, 1, "deactivation keeps the job's tasks");
    let open_claims: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE t.job_id = $1 AND c.state = 'claimed'",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(open_claims, 1, "and the claim already out on them");
    assert_eq!(claim(&admin.app).await, StatusCode::NO_CONTENT, "but offers nothing more");

    let (status, body) =
        admin.post(&format!("/api/admin/jobs/{job}/activate"), json!({ "allocation": 60 })).await;
    assert_eq!(status, StatusCode::OK, "an inactive job can be activated again: {body}");
    assert_eq!(lifecycle(&db, job).await.1, Some(60));

    let (status, body) = admin.call("POST", &format!("/api/admin/jobs/{job}/complete"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "completed", "{body}");

    let (status, body) =
        admin.post(&format!("/api/admin/jobs/{job}/activate"), json!({ "allocation": 10 })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["message"], "a completed job cannot be reactivated");
    let (status, body) = admin.call("POST", &format!("/api/admin/jobs/{job}/deactivate"), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["message"], "a completed job cannot be deactivated");
    assert_eq!(lifecycle(&db, job).await.0, "completed", "completion is terminal");
    assert_eq!(claim(&admin.app).await, StatusCode::NO_CONTENT, "a completed job is not offered");
    assert_eq!(task_count(&db, job).await, 1);
}

/// I-JOB-7: an allocation outside 0-100 is refused by the endpoint, and by the
/// schema's CHECK for a write that bypasses it; the ends of the range are
/// accepted.
#[tokio::test]
async fn an_allocation_outside_0_to_100_is_refused() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let job = api_games_job(&db, &admin).await;

    for allocation in [101, -1, 1000] {
        let (status, body) = admin
            .post(&format!("/api/admin/jobs/{job}/activate"), json!({ "allocation": allocation }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{allocation}: {body}");
        assert_eq!(body["message"], "allocation must be between 0 and 100", "{body}");
    }
    assert_eq!(lifecycle(&db, job).await, ("inactive".into(), None, false, false), "unchanged");

    for allocation in [0, 100] {
        let (status, body) = admin
            .post(&format!("/api/admin/jobs/{job}/activate"), json!({ "allocation": allocation }))
            .await;
        assert_eq!(status, StatusCode::OK, "{allocation}: {body}");
        assert_eq!(lifecycle(&db, job).await.1, Some(allocation));
    }

    // The schema holds the same line for anything that writes the row
    // directly.
    let err = sqlx::query("UPDATE jobs SET allocation = 101 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .expect_err("the CHECK refuses 101");
    assert_eq!(err.as_database_error().unwrap().constraint(), Some("jobs_allocation_check"));
}

/// Claims and completes one games task, with a captured position whose moves
/// carry per-ply statistics, so the job has a claim, a result, a record, its
/// moves and their plies. Returns the claim's worker UUID.
async fn games_history(app: &Router) -> String {
    let (status, assignment) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let uuid = assignment["worker_uuid"].as_str().unwrap().to_string();
    let mut result = games_result(2, 1);
    result["positions"] = json!([{
        "game_index": 0, "turn_number": 0, "rack": "AEINRST", "position": "cgp",
        "num_moves": 40, "moves": [
            { "move": "8D RETAINS", "score": 74, "equity": 81.2,
              "plies": [{ "ply": 1, "bingo_percentage": 10.0, "average_score": 30.0 }] },
            { "move": "8D STAINER", "score": 70, "equity": 79.0 },
        ],
    }]);
    let (status, body) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", &uuid)],
            json!({ "claim_token": assignment["claim_token"], "result": result }),
        ),
    )
    .await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));
    uuid
}

/// A leave-generation job over the test distribution with its generation-1
/// universe seeded and a generation-0 artifact row, as `leave_gen.rs` builds
/// it.
async fn leave_job(db: &TestDb) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let job = db.bare_job("leave_generation", 1, admin).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, 100, 2, 1000, 2)",
    )
    .bind(job)
    .bind(kwg)
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
    db.derived_ready(job).await;
    let mut conn = db.pool.acquire().await.unwrap();
    let data = birdtest::jobs::load_job_data(&mut conn, job).await.unwrap();
    birdtest::jobs::leave_gen::seed_generation(&mut conn, job, 1, &data.letterdist).await.unwrap();
    job
}

/// Claims one leave task and submits five occurrences of each forced rack.
async fn leave_history(app: &Router) {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let racks: Vec<Value> = body["task_request"]["forced_racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rack| json!({ "rack": rack, "count": 5, "mean": 1.0 }))
        .collect();
    let (status, accepted) = send(
        app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", body["worker_uuid"].as_str().unwrap())],
            json!({ "claim_token": body["claim_token"], "result": { "racks": racks } }),
        ),
    )
    .await;
    assert_eq!((status, &accepted["accepted"]), (StatusCode::OK, &json!(true)), "{accepted}");
}

/// Every column in the schema that can point at a job's rows: each foreign key
/// into `jobs`, `tasks`, `task_claims` and the position tables below them, and
/// every uuid column named like a job, task or claim reference whether or not
/// it carries a key. `(table, column, parent)`; `parent` is `None` for a
/// column found by name, which is checked against every id at once.
///
/// `audit_log` is left out on purpose: it has no foreign keys so that it can
/// outlive what it describes, and the census rows exist to be read after the
/// job is gone (I-JOB-10).
async fn referencing_columns(db: &TestDb) -> Vec<(String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT c.conrelid::regclass::text, a.attname::text, c.confrelid::regclass::text
         FROM pg_constraint c
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
         WHERE c.contype = 'f'
           AND c.confrelid IN ('jobs'::regclass, 'tasks'::regclass, 'task_claims'::regclass,
                               'position_analysis_records'::regclass,
                               'position_analysis_moves'::regclass)
         UNION
         SELECT table_name::text, column_name::text, NULL
         FROM information_schema.columns
         WHERE table_schema = 'public' AND data_type = 'uuid'
           AND column_name IN ('job_id', 'task_id', 'task_claim_id', 'claim_id')
           AND table_name <> 'audit_log'
         ORDER BY 1, 2",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

/// How many rows each referencing column holds that point at `job` or at any
/// of its tasks, claims, records or moves.
async fn rows_pointing_at(
    db: &TestDb,
    columns: &[(String, String, Option<String>)],
    ids: &std::collections::HashMap<&'static str, Vec<String>>,
) -> Vec<(String, i64)> {
    let everything: Vec<String> = ids.values().flatten().cloned().collect();
    let mut counts = Vec::new();
    for (table, column, parent) in columns {
        let against = match parent.as_deref() {
            Some(parent) => ids.get(parent).cloned().unwrap_or_default(),
            None => everything.clone(),
        };
        let n: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE {column}::text = ANY ($1)"
        ))
        .bind(&against)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        counts.push((format!("{table}.{column}"), n));
    }
    counts
}

/// Every id a job owns, by the table it is the primary key of.
async fn owned_ids(db: &TestDb, job: Uuid) -> std::collections::HashMap<&'static str, Vec<String>> {
    let mut ids = std::collections::HashMap::new();
    ids.insert("jobs", vec![job.to_string()]);
    for (table, query) in [
        ("tasks", "SELECT id::text FROM tasks WHERE job_id = $1"),
        (
            "task_claims",
            "SELECT c.id::text FROM task_claims c JOIN tasks t ON t.id = c.task_id WHERE t.job_id = $1",
        ),
        ("position_analysis_records", "SELECT id::text FROM position_analysis_records WHERE job_id = $1"),
        (
            "position_analysis_moves",
            "SELECT m.id::text FROM position_analysis_moves m
             JOIN position_analysis_records r ON r.id = m.record_id WHERE r.job_id = $1",
        ),
    ] {
        let found: Vec<String> =
            sqlx::query_scalar(query).bind(job).fetch_all(&db.pool).await.unwrap();
        ids.insert(table, found);
    }
    ids
}

/// I-JOB-9: deleting a job removes everything that points at it, in every
/// table -- enumerated from the catalog rather than listed by hand, so a table
/// added later is covered without editing this test. A games job with a
/// captured position (records, moves, plies, requests, results) and a leave
/// job with a staged result (universe, staging, generation progress, requests,
/// artifacts) are both deleted and both leave nothing.
#[tokio::test]
async fn deleting_a_job_leaves_nothing_anywhere_that_points_at_it() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let admin = Admin::new(&db, state).await;
    let columns = referencing_columns(&db).await;
    assert!(columns.len() > 20, "the catalog query found the schema: {columns:?}");

    let games = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(games)
        .execute(&db.pool)
        .await
        .unwrap();
    games_history(&admin.app).await;
    // A second claim, left open.
    assert_eq!(claim(&admin.app).await, StatusCode::OK);
    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
        .bind(games)
        .execute(&db.pool)
        .await
        .unwrap();

    let leave = leave_job(&db).await;
    leave_history(&admin.app).await;

    for (job, must_have) in [
        (
            games,
            &["tasks.job_id", "task_claims.task_id", "game_results.job_id", "game_requests.task_id",
              "position_analysis_records.job_id", "position_analysis_moves.record_id",
              "position_analysis_plies.move_id", "job_game_config.job_id"][..],
        ),
        (
            leave,
            &["leave_rack_progress.job_id", "leave_rack_staging.job_id", "leave_requests.task_id",
              "leave_generation_artifacts.job_id", "leave_generation_progress.job_id",
              "job_leave_config.job_id", "leave_records.task_id"][..],
        ),
    ] {
        let ids = owned_ids(&db, job).await;
        let before = rows_pointing_at(&db, &columns, &ids).await;
        for name in must_have {
            let n = before.iter().find(|(c, _)| c == name).map(|(_, n)| *n);
            assert!(n.unwrap_or(0) > 0, "the job has {name} rows to delete: {before:?}");
        }

        let (status, body) = admin.call("DELETE", &format!("/api/admin/jobs/{job}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

        let left: Vec<(String, i64)> = rows_pointing_at(&db, &columns, &ids)
            .await
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .collect();
        assert!(left.is_empty(), "orphans after deleting {job}: {left:?}");
    }
}

/// I-JOB-10: a purge writes its census to `audit_log` before it deletes
/// anything, in the same transaction: the census counts exactly what existed,
/// precedes the `job.purged` row, and survives the rows it describes.
#[tokio::test]
async fn a_purge_writes_its_census_before_it_destroys_anything() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let admin = Admin::new(&db, db.state().await).await;
    games_history(&admin.app).await;

    let (status, body) = admin.call("POST", &format!("/api/admin/jobs/{job}/purge"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: Vec<(i64, String, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, action, reason, job_id FROM audit_log
         WHERE target_id = $1 AND action LIKE 'job.purged%' ORDER BY id",
    )
    .bind(job.to_string())
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(actions, ["job.purged.census", "job.purged"], "census first: {rows:?}");
    let census = rows[0].2.as_deref().unwrap();
    assert_eq!(
        census,
        "tasks=1 claims=1 game_results=1 leave_records=0 positions=1 rack_progress=0 \
         staged_results=0 artifacts=0",
        "the census counts what the purge was about to destroy"
    );
    assert_eq!(rows[0].3, Some(job));

    let remaining: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM tasks WHERE job_id = $1),
                (SELECT COUNT(*) FROM game_results WHERE job_id = $1),
                (SELECT COUNT(*) FROM position_analysis_records WHERE job_id = $1)",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(remaining, (0, 0, 0), "and those rows are gone");
}

/// I-JOB-11: a player config referenced by a job of any type -- either seat of
/// a games or game-pairs job, or an opening-rack job's player -- cannot be
/// deleted, and stays; one nothing references can, and one freed by deleting
/// its job can too.
#[tokio::test]
async fn a_player_config_in_use_by_any_job_cannot_be_deleted() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let (ld, layout) = board(&db).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let mut configs = Vec::new();
    for name in ["games-p1", "games-p2", "pairs-p1", "pairs-p2", "racks", "unused"] {
        configs.push(admin.static_player(name, kwg, klv, json!({})).await);
    }
    let mut jobs = Vec::new();
    for body in [
        games_body(ld, layout, configs[0], configs[1]),
        pairs_body(ld, layout, configs[2], configs[3]),
        json!({
            "job_type": "opening_rack", "variant": "classic", "letterdist_id": ld,
            "layout_id": layout, "player_config_id": configs[4],
        }),
    ] {
        let (status, created) = admin.create_job(body).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        jobs.push(created["job"]["id"].as_str().unwrap().to_string());
    }

    for config in &configs[..5] {
        let (status, body) =
            admin.call("DELETE", &format!("/api/admin/player-configs/{config}"), None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(
            body["message"],
            "a job, a rating pool, a rating history or a clone references this player config"
        );
        let (status, _) = admin.call("GET", &format!("/api/admin/player-configs/{config}"), None).await;
        assert_eq!(status, StatusCode::OK, "the refused config is still there");
    }

    let (status, body) =
        admin.call("DELETE", &format!("/api/admin/player-configs/{}", configs[5]), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _) =
        admin.call("GET", &format!("/api/admin/player-configs/{}", configs[5]), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Once its job is gone, a config is free again.
    let (status, _) = admin.call("DELETE", &format!("/api/admin/jobs/{}", jobs[0]), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) =
        admin.call("DELETE", &format!("/api/admin/player-configs/{}", configs[0]), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// The seed an anonymous claim was handed, or `None` for a 204.
async fn claimed_seed(app: &Router) -> Option<String> {
    let (status, body) = send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    match status {
        StatusCode::OK => Some(body["task_request"]["seed"].as_str().unwrap().to_string()),
        StatusCode::NO_CONTENT => None,
        other => panic!("claim answered {other}: {body}"),
    }
}

/// I-JOB-8: a purge deletes the job's tasks and claims, keeps the job row and
/// its configuration, and task generation starts its space over -- the next
/// claims get seeds 1, 1 + batch, ... again, not a continuation past the
/// deleted tasks (which would leave the start of the space never played) and
/// not a collision with them.
#[tokio::test]
async fn a_purged_job_hands_out_its_seed_space_again_from_the_start() {
    let db = TestDb::new().await;
    let job = db.games_job(1, 2).await;
    let admin = Admin::new(&db, db.state().await).await;

    let mut before = Vec::new();
    for _ in 0..3 {
        before.push(claimed_seed(&admin.app).await.expect("a task"));
    }
    assert_eq!(before, ["1", "3", "5"]);
    assert_eq!(task_count(&db, job).await, 3);

    let (status, body) = admin.post(&format!("/api/admin/jobs/{job}/purge"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tasks_reset"], 3, "{body}");
    assert_eq!(task_count(&db, job).await, 0);
    let claims: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_claims c JOIN tasks t ON t.id = c.task_id WHERE t.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(claims, 0);
    let (status, row) = admin.call("GET", &format!("/api/jobs/{job}"), None).await;
    assert_eq!(status, StatusCode::OK, "the job row survives: {row}");

    let mut after = Vec::new();
    for _ in 0..3 {
        after.push(claimed_seed(&admin.app).await.expect("a task"));
    }
    assert_eq!(after, before, "the space is handed out again from its start");
}
