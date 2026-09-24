//! Tier 3: the admin routes through the real router, against a real database.
//! Each test names the TESTING.md `A-ADMIN` guarantee it proves; the
//! destructive-path regressions live in `admin_api.rs`, imports and account
//! deletion elsewhere.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::{json, Value};
use uuid::Uuid;

// --- helpers ----------------------------------------------------------------

/// An admin session against a fresh app.
struct Admin {
    app: axum::Router,
    id: Uuid,
    headers: Vec<(String, String)>,
}

impl Admin {
    async fn new(db: &TestDb) -> Self {
        let state = db.state().await;
        let id = db.user("root", true).await;
        let headers = admin_headers(&state.cfg, id);
        Admin { app: birdtest::app(state), id, headers }
    }

    fn refs(&self) -> Vec<(&str, &str)> {
        self.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        send(&self.app, post_json(path, &self.refs(), body)).await
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        send(&self.app, get_request(path, &self.headers)).await
    }

    async fn delete(&self, path: &str) -> (StatusCode, Value) {
        let mut builder = Request::delete(path);
        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        send(&self.app, builder.body(Body::empty()).unwrap()).await
    }
}

fn message(body: &Value) -> &str {
    body["message"].as_str().unwrap_or_else(|| panic!("no message in {body}"))
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

/// The files a job over English on NWL23 pins.
struct Files {
    letterdist: Uuid,
    layout: Uuid,
    kwg: Uuid,
    klv: Uuid,
}

async fn files(db: &TestDb) -> Files {
    Files {
        letterdist: db.input_data("letterdist", "english").await,
        layout: db.input_data("layout", "standard15").await,
        kwg: db.input_data("kwg", "NWL23").await,
        klv: db.input_data("klv", "NWL23").await,
    }
}

fn static_config(name: &str, files: &Files) -> Value {
    json!({
        "name": name, "recorder_type": "all", "sort_strategy": "equity",
        "kwg_id": files.kwg, "klv_id": files.klv, "num_plays_recorded": 3,
    })
}

fn simming_config(name: &str, files: &Files, winpct: Option<Uuid>) -> Value {
    json!({
        "name": name, "recorder_type": "best", "kwg_id": files.kwg, "klv_id": files.klv,
        "winpct_id": winpct, "num_plies": 2, "num_plays": 10, "max_iterations": 100,
        "time_limit_secs": 0, "num_plays_recorded": 1,
    })
}

async fn created_config(admin: &Admin, body: Value) -> Value {
    let (status, created) = admin.post("/api/admin/player-configs", body).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created
}

fn games_job_body(files: &Files, p1: &Value, p2: &Value) -> Value {
    json!({
        "job_type": "games", "variant": "classic",
        "letterdist_id": files.letterdist, "layout_id": files.layout,
        "player1_config_id": p1, "player2_config_id": p2,
        "games_per_batch": 2, "min_games": 1, "max_games": 1000,
    })
}

/// A games job created through the API, between two copies of one static
/// config, returning its id.
async fn api_games_job(admin: &Admin, files: &Files) -> String {
    let player = created_config(admin, static_config(&format!("p{}", Uuid::new_v4().simple()), files)).await;
    let (status, body) = admin.post("/api/admin/jobs", games_job_body(files, &player["id"], &player["id"])).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["job"]["id"].as_str().unwrap().to_string()
}

async fn claim(app: &axum::Router, headers: &[(&str, &str)], version: &str, unsupported: &[Uuid]) -> (StatusCode, Value) {
    send(app, post_json("/api/worker/task", headers, claim_body(version, unsupported))).await
}

async fn decline(app: &axum::Router, uuid: &str, assignment: &Value, missing: Value) {
    let (status, body) = send(
        app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", uuid)],
            json!({ "claim_token": assignment["claim_token"], "reason": "missing_data", "missing": missing }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

// --- A-ADMIN-1 ---------------------------------------------------------------

/// A-ADMIN-1: a player config comes back from `get` and `list` exactly as
/// created, and deleting it works only while nothing references it -- a config
/// a job plays is refused with `409`, and stays readable.
#[tokio::test]
async fn a_player_config_round_trips_and_one_in_use_cannot_be_deleted() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    let files = files(&db).await;

    let alpha = created_config(&admin, static_config("alpha", &files)).await;
    let beta = created_config(&admin, simming_config("beta", &files, Some(db.input_data("winpct", "winpct").await))).await;
    assert_eq!(alpha["name"], "alpha");
    assert!(beta["winpct_id"].is_string(), "{beta}");

    for config in [&alpha, &beta] {
        let (status, got) = admin.get(&format!("/api/admin/player-configs/{}", config["id"].as_str().unwrap())).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!(&got, config, "get returns what create returned");
    }
    let (status, list) = admin.get("/api/admin/player-configs").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let listed = list.as_array().expect("a list");
    assert_eq!(listed.len(), 2, "{list}");
    assert!(listed.contains(&alpha) && listed.contains(&beta), "{list}");

    // alpha is played by a job; beta by nothing.
    let (status, job) = admin.post("/api/admin/jobs", games_job_body(&files, &alpha["id"], &alpha["id"])).await;
    assert_eq!(status, StatusCode::CREATED, "{job}");

    let alpha_path = format!("/api/admin/player-configs/{}", alpha["id"].as_str().unwrap());
    let (status, body) = admin.delete(&alpha_path).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(message(&body).contains("references this player config"), "{body}");
    let (status, _) = admin.get(&alpha_path).await;
    assert_eq!(status, StatusCode::OK, "a refused delete deleted it anyway");

    let beta_path = format!("/api/admin/player-configs/{}", beta["id"].as_str().unwrap());
    let (status, body) = admin.delete(&beta_path).await;
    assert_eq!((status, &body), (StatusCode::NO_CONTENT, &Value::Null));
    let (status, body) = admin.get(&beta_path).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(message(&body), "no such player config");
    let (status, body) = admin.delete(&beta_path).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (_, list) = admin.get("/api/admin/player-configs").await;
    assert_eq!(list, json!([alpha]));

    // None of it is anyone else's business.
    let (status, _) = send(&admin.app, get_request("/api/admin/player-configs", &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// --- A-ADMIN-2, 3, 4: jobs ----------------------------------------------------

/// A-ADMIN-2: creating a job of each type answers `{job}`, and the job is
/// inactive, has no allocation and has nothing issued: no task row exists and
/// no worker is offered it until an admin activates it. (Leave generation,
/// whose creation runs MAGPIE, is pinned in `magpie_routes.rs`.)
#[tokio::test]
async fn creating_each_job_type_answers_it_inactive_and_unallocated() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    let files = files(&db).await;
    let player = created_config(&admin, static_config("solver", &files)).await;

    let bodies = [
        ("games", "job_game_config", games_job_body(&files, &player["id"], &player["id"])),
        (
            "game_pairs",
            "job_game_pair_config",
            json!({
                "job_type": "game_pairs", "variant": "classic",
                "letterdist_id": files.letterdist, "layout_id": files.layout,
                "player1_config_id": player["id"], "player2_config_id": player["id"],
                "min_pairs": 1, "max_pairs": 100,
            }),
        ),
        (
            "opening_rack",
            "job_opening_rack_config",
            json!({
                "job_type": "opening_rack", "variant": "classic",
                "letterdist_id": files.letterdist, "layout_id": files.layout,
                "player_config_id": player["id"], "racks_per_batch": 10,
            }),
        ),
    ];
    for (job_type, config_table, body) in bodies {
        let (status, created) = admin.post("/api/admin/jobs", body).await;
        assert_eq!(status, StatusCode::CREATED, "{job_type}: {created}");
        let keys: Vec<&String> = created.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["job"], "{job_type}: {created}");
        let job = &created["job"];
        assert_eq!(job["job_type"], job_type, "{created}");
        assert_eq!(job["status"], "inactive", "{created}");
        assert_eq!(job["allocation"], Value::Null, "{created}");
        assert_eq!(job["claims_issued"], json!(0), "{created}");
        assert_eq!(job["created_by"], json!(admin.id), "{created}");
        assert_eq!(
            (&job["min_magpie_major"], &job["min_magpie_minor"], &job["min_magpie_patch"]),
            (&json!(0), &json!(1), &json!(1)),
            "the server's floor is the default: {created}"
        );

        let id: Uuid = job["id"].as_str().unwrap().parse().unwrap();
        let (tasks, configs, audited): (i64, i64, i64) = sqlx::query_as(&format!(
            "SELECT (SELECT COUNT(*) FROM tasks WHERE job_id = $1),
                    (SELECT COUNT(*) FROM {config_table} WHERE job_id = $1),
                    (SELECT COUNT(*) FROM audit_log WHERE action = 'job.created' AND job_id = $1)"
        ))
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!((tasks, configs, audited), (0, 1, 1), "{job_type}");
    }

    let (status, body) = claim(&admin.app, &[], "1.0.0", &[]).await;
    assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null), "an inactive job is offered to nobody");
}

/// A-ADMIN-3: job creation refuses every combination MAGPIE could not run, and
/// says which: two simmers on different win% models, a lexicon from another
/// letter distribution, an `input_data` id that does not exist (or is the wrong
/// kind of file), a player config that does not exist. The win% rules on the
/// players themselves are refused where the player is made: a simmer must name
/// a model and a static player must not. Nothing is written for any of them.
#[tokio::test]
async fn job_creation_refuses_each_impossible_combination_and_says_which() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    let files = files(&db).await;
    let german = db.input_data("letterdist", "german").await;
    let winpct = db.input_data("winpct", "winpct").await;
    let other_winpct = db.input_data("winpct", "winpct2").await;

    let plain = created_config(&admin, static_config("static", &files)).await;
    let simmer = created_config(&admin, simming_config("simmer", &files, Some(winpct))).await;
    let other_simmer = created_config(&admin, simming_config("simmer2", &files, Some(other_winpct))).await;

    let with = |change: &dyn Fn(&mut Value)| {
        let mut body = games_job_body(&files, &plain["id"], &plain["id"]);
        change(&mut body);
        body
    };
    let cases = [
        (
            "two simmers on different win% models",
            games_job_body(&files, &simmer["id"], &other_simmer["id"]),
            "disagree on the win% model",
        ),
        (
            "a lexicon from another letter distribution",
            with(&|body| body["letterdist_id"] = json!(german)),
            "is not compatible with letter distribution \"german\"",
        ),
        ("a letter distribution that does not exist", with(&|body| body["letterdist_id"] = json!(Uuid::new_v4())), "no input data row"),
        ("a layout that does not exist", with(&|body| body["layout_id"] = json!(Uuid::new_v4())), "no input data row"),
        ("a lexicon given as the letter distribution", with(&|body| body["letterdist_id"] = json!(files.kwg)), "expected a letterdist row"),
        ("a player config that does not exist", with(&|body| body["player2_config_id"] = json!(Uuid::new_v4())), "player config not found"),
    ];
    for (name, body, says) in cases {
        let (status, refusal) = admin.post("/api/admin/jobs", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {refusal}");
        assert!(message(&refusal).contains(says), "{name}: expected {says:?} in {refusal}");
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM jobs").await, 0, "a refused job was written");

    let (status, refusal) = admin.post("/api/admin/player-configs", simming_config("no-model", &files, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refusal}");
    assert!(message(&refusal).contains("must name a win% model"), "{refusal}");
    let mut static_with_model = static_config("static-with-model", &files);
    static_with_model["winpct_id"] = json!(winpct);
    let (status, refusal) = admin.post("/api/admin/player-configs", static_with_model).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refusal}");
    assert!(message(&refusal).contains("must not name a win% model"), "{refusal}");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM player_configs").await, 3);
}

/// A-ADMIN-4: activate, deactivate and complete each answer with the job as it
/// now stands, purge with how many tasks it reset, and delete with `204` -- and
/// the public job page read afterwards agrees with each.
#[tokio::test]
async fn each_lifecycle_action_answers_its_shape_and_a_read_agrees() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    let files = files(&db).await;
    let job = api_games_job(&admin, &files).await;
    let read = || async {
        let (status, body) = send(&admin.app, get_request(&format!("/api/jobs/{job}"), &[])).await;
        (status, body)
    };
    let action = |name: &str| format!("/api/admin/jobs/{job}/{name}");

    let (status, body) = admin.post(&action("activate"), json!({ "allocation": 40 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((&body["id"], &body["status"], &body["allocation"]), (&json!(job), &json!("active"), &json!(40)));
    let (_, page) = read().await;
    assert_eq!((&page["job"]["status"], &page["job"]["allocation"]), (&json!("active"), &json!(40)), "{page}");

    // Some history, for the purge to reset.
    let (status, assignment) = claim(&admin.app, &[], "1.0.0", &[]).await;
    assert_eq!(status, StatusCode::OK, "{assignment}");
    let (_, accepted) = send(
        &admin.app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", assignment["worker_uuid"].as_str().unwrap())],
            json!({ "claim_token": assignment["claim_token"], "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(accepted, json!({ "accepted": true }));

    let (status, body) = admin.post(&action("deactivate"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((&body["id"], &body["status"]), (&json!(job), &json!("inactive")));
    let (_, page) = read().await;
    assert_eq!(page["job"]["status"], "inactive", "{page}");
    assert_eq!((&page["tasks_total"], &page["results_accepted"]), (&json!(1), &json!(1)), "{page}");

    let (status, body) = admin.post(&action("purge"), json!({})).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "tasks_reset": 1 })));
    let (_, page) = read().await;
    assert_eq!((&page["tasks_total"], &page["results_accepted"]), (&json!(0), &json!(0)), "{page}");
    assert_eq!(page["games"]["units_completed"], json!(0), "{page}");

    let (status, body) = admin.post(&action("complete"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((&body["id"], &body["status"]), (&json!(job), &json!("completed")));
    let (_, page) = read().await;
    assert_eq!(page["job"]["status"], "completed", "{page}");

    let (status, body) = admin.delete(&format!("/api/admin/jobs/{job}")).await;
    assert_eq!((status, &body), (StatusCode::NO_CONTENT, &Value::Null));
    let (status, _) = read().await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a deleted job is still readable");
    let (status, body) = admin.delete(&format!("/api/admin/jobs/{job}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

// --- A-ADMIN-8, 9, 10, 11 -------------------------------------------------------

/// A-ADMIN-8: a job's data-gaps view reports, per file a worker declined for,
/// how many distinct workers and how many declines -- the difference between
/// "one worker keeps retrying" and "the fleet is missing a file" -- most
/// widespread first, and only for the job asked about.
#[tokio::test]
async fn data_gaps_report_what_workers_declined_for() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    let job = db.games_job(1, 2).await;
    let other = db.games_job(1, 2).await;
    let (kwg, klv) = ("a".repeat(64), "b".repeat(64));

    // Two workers, steered to `job` by listing the other one as unsupported.
    let (_, first) = claim(&admin.app, &[], "1.0.0", &[other]).await;
    let w1 = first["worker_uuid"].as_str().unwrap().to_string();
    assert_eq!(first["job_id"], json!(job.to_string()), "{first}");
    decline(&admin.app, &w1, &first, json!([
        { "role": "kwg", "name": "NWL23", "expected": kwg, "actual": null },
        { "role": "klv", "name": "NWL23", "expected": klv, "actual": "c".repeat(64) },
    ]))
    .await;
    let (_, again) = claim(&admin.app, &[("x-worker-uuid", &w1)], "1.0.0", &[other]).await;
    decline(&admin.app, &w1, &again, json!([{ "role": "kwg", "name": "NWL23", "expected": kwg }])).await;
    let (_, second) = claim(&admin.app, &[], "1.0.0", &[other]).await;
    let w2 = second["worker_uuid"].as_str().unwrap().to_string();
    decline(&admin.app, &w2, &second, json!([{ "role": "kwg", "name": "NWL23", "expected": kwg }])).await;

    // A gap on another job is that job's.
    let (_, elsewhere) = claim(&admin.app, &[], "1.0.0", &[job]).await;
    assert_eq!(elsewhere["job_id"], json!(other.to_string()), "{elsewhere}");
    let w3 = elsewhere["worker_uuid"].as_str().unwrap().to_string();
    decline(&admin.app, &w3, &elsewhere, json!([{ "role": "layout", "name": "standard15", "expected": kwg }])).await;

    let (status, gaps) = admin.get(&format!("/api/admin/jobs/{job}/data-gaps")).await;
    assert_eq!(status, StatusCode::OK, "{gaps}");
    let summary: Vec<(String, String, String, i64, i64)> = gaps
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            assert!(g["last_reported_at"].is_string(), "{g}");
            (
                g["role"].as_str().unwrap().into(),
                g["name"].as_str().unwrap().into(),
                g["expected"].as_str().unwrap().into(),
                g["workers"].as_i64().unwrap(),
                g["declines"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("kwg".into(), "NWL23".into(), kwg.clone(), 2, 3),
            ("klv".into(), "NWL23".into(), klv.clone(), 1, 1),
        ]
    );

    let (status, gaps) = admin.get(&format!("/api/admin/jobs/{}/data-gaps", Uuid::new_v4())).await;
    assert_eq!((status, gaps), (StatusCode::OK, json!([])));
    let (status, _) = send(&admin.app, get_request(&format!("/api/admin/jobs/{job}/data-gaps"), &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A-ADMIN-9: the fleet view counts the workers -- and the claims -- behind
/// each MAGPIE version the field has claimed with in the last week, most
/// widely run first. A worker last seen longer ago than that is not part of
/// the fleet.
#[tokio::test]
async fn the_fleet_view_counts_workers_by_the_version_they_run() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    db.games_job(1, 2).await;

    let (_, first) = claim(&admin.app, &[], "1.0.0", &[]).await;
    let w1 = first["worker_uuid"].as_str().unwrap().to_string();
    let (status, _) = claim(&admin.app, &[("x-worker-uuid", &w1)], "1.0.0", &[]).await;
    assert_eq!(status, StatusCode::OK);
    for version in ["1.0.0", "1.2.3", "0.9.0"] {
        let (status, body) = claim(&admin.app, &[], version, &[]).await;
        assert_eq!(status, StatusCode::OK, "{version}: {body}");
    }
    sqlx::query("UPDATE task_claims SET claimed_at = now() - interval '8 days' WHERE magpie_version = '0.9.0'")
        .execute(&db.pool)
        .await
        .unwrap();

    let (status, fleet) = admin.get("/api/admin/fleet").await;
    assert_eq!(status, StatusCode::OK, "{fleet}");
    assert_eq!(
        fleet,
        json!([
            { "magpie_version": "1.0.0", "workers": 2, "claims": 3 },
            { "magpie_version": "1.2.3", "workers": 1, "claims": 1 },
        ])
    );
}

/// A backup run as `scripts/backup.sh` records one, finished `hours_ago`.
async fn backup_run(db: &TestDb, hours_ago: i64, ok: bool, rows: Value) {
    sqlx::query(
        "INSERT INTO backups (kind, s3_key, started_at, finished_at, row_counts, ok)
         VALUES ('pg_dump', 'pg/' || gen_random_uuid(),
                 now() - make_interval(hours => $1) - interval '3 minutes',
                 now() - make_interval(hours => $1), $2, $3)",
    )
    .bind(hours_ago as i32)
    .bind(rows)
    .bind(ok)
    .execute(&db.pool)
    .await
    .unwrap();
}

/// A-ADMIN-10: the backups view reports staleness from the `backups` table:
/// never having had a good backup is stale; a good one older than 36 hours is
/// stale however many failed runs came since; a recent good one is not. Runs
/// are listed newest first.
#[tokio::test]
async fn the_backups_view_reports_staleness_from_the_backups_table() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;

    let (status, view) = admin.get("/api/admin/backups").await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!((&view["stale"], &view["last_success_at"], &view["recent"]), (&json!(true), &Value::Null, &json!([])));

    backup_run(&db, 40, true, json!({ "jobs": 3, "users": 4 })).await;
    backup_run(&db, 1, false, json!({})).await;
    let (_, view) = admin.get("/api/admin/backups").await;
    assert_eq!(view["stale"], true, "a failure does not freshen anything: {view}");
    let age = view["last_success_age_seconds"].as_i64().unwrap();
    assert!((40 * 3600..40 * 3600 + 60).contains(&age), "{view}");
    let recent = view["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 2, "{view}");
    assert_eq!((&recent[0]["ok"], &recent[1]["ok"]), (&json!(false), &json!(true)), "newest first: {view}");
    assert_eq!(recent[1]["total_rows"], json!(7), "{view}");
    assert_eq!(recent[1]["duration_seconds"], json!(180), "{view}");

    backup_run(&db, 2, true, json!({})).await;
    let (_, view) = admin.get("/api/admin/backups").await;
    assert_eq!(view["stale"], false, "{view}");
    let age = view["last_success_age_seconds"].as_i64().unwrap();
    assert!((2 * 3600..2 * 3600 + 60).contains(&age), "{view}");
}

/// A-ADMIN-11, the part that needs no MAGPIE: only a leave-generation job has
/// artifacts, and asking for any other job's is refused before anything is
/// built. The rebuild itself is pinned in `magpie_routes.rs`.
#[tokio::test]
async fn only_a_leave_jobs_artifacts_can_be_rebuilt() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    let job = db.games_job(1, 2).await;

    let (status, body) = admin.post(&format!("/api/admin/jobs/{job}/rebuild-artifacts"), json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(message(&body).contains("only leave generation jobs have artifacts"), "{body}");
    let (status, body) =
        admin.post(&format!("/api/admin/jobs/{}/rebuild-artifacts", Uuid::new_v4()), json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

// --- A-ADMIN-12, 13 ---------------------------------------------------------------

/// A-ADMIN-12: an admin bans a worker by anonymous UUID or by user id over the
/// API; that worker's next claim is refused, and lifting the ban restores it.
/// A ban names exactly one identity, and both halves are audited.
#[tokio::test]
async fn a_ban_by_either_identity_refuses_the_next_claim_and_unban_restores_it() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    db.games_job(1, 2).await;

    let (_, first) = claim(&admin.app, &[], "1.0.0", &[]).await;
    let anon = first["worker_uuid"].as_str().unwrap().to_string();
    let user = db.user("contributor", false).await;
    let raw_key = "bt_".to_string() + &"b".repeat(64);
    sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, $2)")
        .bind(user)
        .bind(birdtest::auth::api_key::hash_key(&raw_key))
        .execute(&db.pool)
        .await
        .unwrap();
    let bearer = format!("Bearer {raw_key}");

    let identities: [(&str, Value, (&str, &str)); 2] = [
        ("anon_uuid", json!(anon), ("x-worker-uuid", anon.as_str())),
        ("user_id", json!(user), ("authorization", bearer.as_str())),
    ];
    for (field, id, header) in &identities {
        let mut ban = json!({ "reason": "garbage" });
        ban[*field] = id.clone();
        let (status, body) = admin.post("/api/admin/workers/ban", ban).await;
        assert_eq!(status, StatusCode::CREATED, "{field}: {body}");
        let ban = body["id"].as_str().expect("the ban's id").to_string();

        let (status, body) = claim(&admin.app, &[*header], "1.0.0", &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{field}: a banned worker claimed: {body}");
        assert!(message(&body).contains("banned"), "{body}");

        let (status, body) = admin.delete(&format!("/api/admin/workers/ban/{ban}")).await;
        assert_eq!((status, &body), (StatusCode::NO_CONTENT, &Value::Null), "{field}");
        let (status, body) = claim(&admin.app, &[*header], "1.0.0", &[]).await;
        assert_eq!(status, StatusCode::OK, "{field}: unban did not restore the worker: {body}");
    }

    let audited: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, target_id FROM audit_log
         WHERE action IN ('worker.banned', 'worker.unbanned') ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        audited,
        vec![
            ("worker.banned".into(), anon.clone()),
            ("worker.unbanned".into(), anon.clone()),
            ("worker.banned".into(), user.to_string()),
            ("worker.unbanned".into(), user.to_string()),
        ]
    );

    for body in [json!({ "user_id": user, "anon_uuid": anon }), json!({ "reason": "nobody" })] {
        let (status, refusal) = admin.post("/api/admin/workers/ban", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refusal}");
        assert!(message(&refusal).contains("exactly one of user_id or anon_uuid"), "{refusal}");
    }
    let (status, _) = admin.delete(&format!("/api/admin/workers/ban/{}", Uuid::new_v4())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A-ADMIN-13: the audit log filters by job and pages through the result
/// newest first, every row exactly once, with a total that counts the filter
/// rather than the table.
#[tokio::test]
async fn the_audit_log_pages_and_filters_by_job() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db).await;
    let files = files(&db).await;
    let first = api_games_job(&admin, &files).await;
    let second = api_games_job(&admin, &files).await;
    for (path, body) in [
        (format!("/api/admin/jobs/{first}/activate"), json!({ "allocation": 40 })),
        (format!("/api/admin/jobs/{first}/deactivate"), json!({})),
        (format!("/api/admin/jobs/{second}/activate"), json!({ "allocation": 40 })),
        (format!("/api/admin/jobs/{first}/activate"), json!({ "allocation": 30 })),
        (format!("/api/admin/jobs/{first}/deactivate"), json!({})),
    ] {
        let (status, body) = admin.post(&path, body).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }
    // A row about no job at all.
    let nuisance = db.user("nuisance", false).await;
    let (status, _) = admin.post("/api/admin/workers/ban", json!({ "user_id": nuisance })).await;
    assert_eq!(status, StatusCode::CREATED);

    let mut walked: Vec<Value> = Vec::new();
    for page in 0..4 {
        let (status, body) = admin.get(&format!("/api/admin/audit-log?job_id={first}&per_page=2&page={page}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!((&body["total"], &body["page"], &body["per_page"]), (&json!(5), &json!(page), &json!(2)), "{body}");
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), [2, 2, 1, 0][page as usize], "page {page}: {body}");
        walked.extend(items.iter().cloned());
    }
    assert!(walked.iter().all(|row| row["job_id"] == json!(first)), "{walked:?}");
    let actions: Vec<&str> = walked.iter().map(|row| row["action"].as_str().unwrap()).collect();
    assert_eq!(
        actions,
        ["job.deactivated", "job.activated", "job.deactivated", "job.activated", "job.created"],
        "newest first"
    );
    let ids: Vec<i64> = walked.iter().map(|row| row["id"].as_i64().unwrap()).collect();
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]), "every row once, in order: {ids:?}");
    assert_eq!(
        (&walked[0]["old_status"], &walked[0]["new_status"]),
        (&json!("active"), &json!("inactive")),
        "{:?}",
        walked[0]
    );

    let (_, body) = admin.get(&format!("/api/admin/audit-log?job_id={second}")).await;
    let actions: Vec<&str> = body["items"].as_array().unwrap().iter().map(|r| r["action"].as_str().unwrap()).collect();
    assert_eq!((&body["total"], actions), (&json!(2), vec!["job.activated", "job.created"]));

    let (_, body) = admin.get(&format!("/api/admin/audit-log?job_id={first}&action=job.activated")).await;
    assert_eq!(body["total"], json!(2), "{body}");
    let (_, body) = admin.get("/api/admin/audit-log").await;
    assert_eq!(body["total"], json!(8), "the unfiltered log counts every row: {body}");
}
