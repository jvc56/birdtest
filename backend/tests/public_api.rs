//! The public API against a real database: the job list and detail, the
//! results feed and rack lookup, the live stats stream, and the contributor
//! lists -- what they page, what they return, and what they must never.

mod common;

use axum::http::StatusCode;
use common::*;
use futures::StreamExt;
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Claims the next task as a new anonymous worker, returning the assignment
/// and the minted UUID.
async fn first_claim(app: &axum::Router) -> (serde_json::Value, String) {
    let (status, body) =
        send(app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uuid = body["worker_uuid"].as_str().expect("a minted worker_uuid").to_string();
    (body, uuid)
}

async fn submit(
    app: &axum::Router,
    assignment: &serde_json::Value,
    uuid: &str,
    result: serde_json::Value,
) {
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

/// The raw bytes of a GET, for comparisons that must be byte-for-byte.
async fn get_bytes(app: &axum::Router, path: &str) -> (StatusCode, Vec<u8>) {
    let response = app.clone().oneshot(get_request(path, &[])).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024).await.unwrap();
    (status, bytes.to_vec())
}

/// Reads an SSE body until its next `stats` event, returning the event's data
/// exactly as sent. Anything else on the wire (a keep-alive comment) is
/// skipped.
async fn next_stats_event(body: &mut axum::body::BodyDataStream, buffer: &mut String) -> String {
    loop {
        if let Some(end) = buffer.find("\n\n") {
            let event: String = buffer.drain(..end + 2).collect();
            let mut name = None;
            let mut data = Vec::new();
            for line in event.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    name = Some(value.strip_prefix(' ').unwrap_or(value).to_string());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.strip_prefix(' ').unwrap_or(value));
                }
            }
            if name.as_deref() == Some("stats") {
                return data.join("\n");
            }
            continue;
        }
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(10), body.next())
            .await
            .expect("no stats event within ten seconds")
            .expect("the stream ended")
            .expect("the stream failed");
        buffer.push_str(std::str::from_utf8(&chunk).unwrap());
    }
}

/// A task and one claim of it for `job`, owned by an account or an anonymous
/// worker, and that claim's `game_results` row submitted at `submitted_at`.
async fn game_result_at(
    db: &TestDb,
    job: Uuid,
    user: Option<Uuid>,
    anon: Option<Uuid>,
    submitted_at: &str,
) -> Uuid {
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
        "INSERT INTO task_claims
             (task_id, job_id, claim_token, state, claimed_by_user_id, claimed_by_anon_uuid, completed_at)
         VALUES ($1, (SELECT job_id FROM tasks WHERE id = $1), gen_random_uuid(), 'completed', $2, $3,
                 $4::timestamptz)
         RETURNING id",
    )
    .bind(task)
    .bind(user)
    .bind(anon)
    .bind(submitted_at)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO game_results
             (task_claim_id, task_id, job_id, games, wins, losses, ties,
              p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd, submitted_at)
         VALUES ($1, $2, $3, 2, 1, 1, 0, 420, 60, 410, 58, $4::timestamptz)",
    )
    .bind(claim)
    .bind(task)
    .bind(job)
    .bind(submitted_at)
    .execute(&db.pool)
    .await
    .unwrap();
    task
}

/// An anonymous worker with `tasks_completed` tasks finished, each held a
/// second.
async fn anon_worker(db: &TestDb, tasks_completed: i64) -> Uuid {
    let uuid = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO anonymous_workers (uuid, tasks_completed, compute_ms, last_completed_at)
         VALUES ($1, $2, $2 * 1000, CASE WHEN $2 > 0 THEN now() END)",
    )
    .bind(uuid)
    .bind(tasks_completed)
    .execute(&db.pool)
    .await
    .unwrap();
    uuid
}

async fn opening_rack_job(db: &TestDb, racks_per_batch: i32) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let player = db.static_player("solver", admin).await;
    let job = db.bare_job("opening_rack", admin).await;
    sqlx::query(
        "INSERT INTO job_opening_rack_config
             (job_id, player_config_id, racks_per_batch, rack_size, total_racks)
         VALUES ($1, $2, $3, 7, 100)",
    )
    .bind(job)
    .bind(player)
    .bind(racks_per_batch)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/// A-PUBLIC-1e: a job's full configuration is public -- the job's settings,
/// its type's (for a games job the test and its stopping rules) and every
/// setting of each player config, with files by name and its own id to link
/// to -- and names no one: no creator, no user id. An unknown job is a 404.
#[tokio::test]
async fn a_jobs_full_configuration_is_public() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = db.games_job(10).await;

    let (status, config) = send(&app, get_request(&format!("/api/jobs/{job}/config"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{config}");
    assert_eq!(config["job"]["job_type"], "games");
    assert_eq!(config["job"]["letter_distribution"], "english");
    assert_eq!(config["job"]["layout"], "standard15");
    assert_eq!(config["games"]["unit"], "game");
    assert_eq!(config["games"]["max_units"], 1_000_000);
    assert_eq!(config["games"]["per_batch"], 10);
    assert_eq!(config["games"]["test_enabled"], true, "{config}");
    assert_eq!(config["games"]["confidence_pct"], 95.0, "{config}");
    let players = config["players"].as_array().unwrap();
    assert_eq!(players.len(), 2, "{config}");
    assert_eq!(players[0]["role"], "player 1");
    assert_eq!(players[0]["num_plies"], 0);
    assert_eq!(players[0]["recorder_type"], "best");
    assert!(players[0]["lexicon"].as_str().unwrap().starts_with("NWL"), "{config}");
    let (p1, p2): (Uuid, Uuid) = sqlx::query_as(
        "SELECT player1_config_id, player2_config_id FROM job_game_config WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(players[0]["id"], p1.to_string());
    assert_eq!(players[1]["id"], p2.to_string());
    for player in players {
        assert!(player.get("created_by").is_none(), "{player}");
    }

    let (status, _) = send(&app, get_request(&format!("/api/jobs/{}/config", Uuid::new_v4()), &[])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A-PUBLIC-1d: player configs are public, listed newest first and read one
/// at a time, with every setting, files by name and its lineage -- but not
/// which admin made it. An unknown config is a 404.
#[tokio::test]
async fn player_configs_are_public() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = db.games_job(10).await;
    let (p1, p2): (Uuid, Uuid) = sqlx::query_as(
        "SELECT player1_config_id, player2_config_id FROM job_game_config WHERE job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // The second config a clone of the first, so its lineage has something to say.
    sqlx::query("UPDATE player_configs SET cloned_from_id = $1 WHERE id = $2")
        .bind(p1)
        .bind(p2)
        .execute(&db.pool)
        .await
        .unwrap();

    let (status, list) = send(&app, get_request("/api/player-configs", &[])).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [p2.to_string(), p1.to_string()], "newest first: {list}");

    let (status, config) = send(&app, get_request(&format!("/api/player-configs/{p2}"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{config}");
    assert_eq!(config["id"], p2.to_string());
    assert_eq!(config["cloned_from_id"], p1.to_string());
    assert_eq!(config["recorder_type"], "best");
    assert_eq!(config["num_plies"], 0);
    assert!(config["lexicon"].as_str().unwrap().starts_with("NWL"), "{config}");
    assert!(config["created_at"].is_string(), "{config}");
    // Every setting a job's config shows is here, and nothing about who made it.
    for key in ["use_wordmap", "use_rit", "use_wit", "movegen_margin", "num_plays", "sort_strategy", "leaves"] {
        assert!(config.get(key).is_some(), "{key} missing: {config}");
    }
    assert!(config.get("created_by").is_none() && config.get("role").is_none(), "{config}");

    let (status, _) =
        send(&app, get_request(&format!("/api/player-configs/{}", Uuid::new_v4()), &[])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A-PUBLIC-1b: `?status=` filters the job list and its total by status --
/// the home page's active jobs, which it once filtered from the newest page
/// in the browser and so lost every active job older than it.
#[tokio::test]
async fn the_job_list_filters_by_status() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let active = db.bare_job("games", admin).await;
    let inactive = db.bare_job("games", admin).await;
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id = $1")
        .bind(inactive)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = send(&app, get_request("/api/jobs?status=active", &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1, "{body}");
    assert_eq!(body["items"][0]["id"], active.to_string(), "{body}");
    let (status, body) = send(&app, get_request("/api/jobs?status=bogus", &[])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// A-PUBLIC-1c: the job list's `stalled` flag -- an active job with a
/// decline in the last day, no result accepted in the last day, and nothing
/// claimed -- and it clears once a result is accepted. Read from the job's
/// `last_completed_at`, set by the submission that stores a result.
#[tokio::test]
async fn the_job_list_flags_a_stalled_job() {
    let db = TestDb::new().await;
    db.games_job(2).await;
    let app = birdtest::app(db.state().await);
    let stalled = |body: &serde_json::Value| body["items"][0]["stalled"].clone();

    let (status, claim) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let uuid = claim["worker_uuid"].as_str().unwrap().to_string();
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/decline",
            &[("x-worker-uuid", uuid.as_str())],
            json!({ "claim_token": claim["claim_token"], "reason": "missing_data",
                    "missing": [{ "role": "kwg", "name": "NWL23", "expected": "ab", "actual": null }] }),
        ),
    )
    .await;
    assert!(status.is_success(), "{body}");
    let (_, list) = send(&app, get_request("/api/jobs", &[])).await;
    assert_eq!(stalled(&list), json!(true), "{list}");

    // A result accepted since, from another worker, clears it.
    let (status, claim) = send(&app, post_json("/api/worker/task", &[], claim_body("1.0.0", &[]))).await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let other = claim["worker_uuid"].as_str().unwrap().to_string();
    let (status, body) = send(
        &app,
        post_json(
            "/api/worker/result",
            &[("x-worker-uuid", other.as_str())],
            json!({ "claim_token": claim["claim_token"], "movegens": 1000, "result": games_result(2, 1) }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, list) = send(&app, get_request("/api/jobs", &[])).await;
    assert_eq!(stalled(&list), json!(false), "{list}");
}

/// A-PUBLIC-1: the job list pages newest first with a total, and `per_page`
/// is clamped to 1..=500 and a negative page read as the first -- so no
/// request can ask for the whole table in one page.
#[tokio::test]
async fn the_job_list_paginates_and_clamps_its_page_size() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let mut jobs = Vec::new();
    for day in 1..=5 {
        let job = db.bare_job("games", admin).await;
        sqlx::query("UPDATE jobs SET created_at = $2::timestamptz WHERE id = $1")
            .bind(job)
            .bind(format!("2026-01-0{day}T00:00:00Z"))
            .execute(&db.pool)
            .await
            .unwrap();
        jobs.push(job.to_string());
    }
    jobs.reverse();

    let mut seen = Vec::new();
    for page in 0..4 {
        let (status, body) =
            send(&app, get_request(&format!("/api/jobs?page={page}&per_page=2"), &[])).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!((body["total"].as_i64(), body["page"].as_i64()), (Some(5), Some(page)));
        assert_eq!(body["per_page"], 2);
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), [2, 2, 1, 0][page as usize], "page {page}: {body}");
        seen.extend(items.iter().map(|j| j["id"].as_str().unwrap().to_string()));
    }
    assert_eq!(seen, jobs, "every job once, newest first");

    let (_, body) = send(&app, get_request("/api/jobs?per_page=100000", &[])).await;
    assert_eq!(body["per_page"], 500, "clamped to the maximum");
    assert_eq!(body["items"].as_array().unwrap().len(), 5);
    let (_, body) = send(&app, get_request("/api/jobs?per_page=0", &[])).await;
    assert_eq!(body["per_page"], 1, "and to at least one");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let (_, body) = send(&app, get_request("/api/jobs?page=-3&per_page=2", &[])).await;
    assert_eq!(body["page"], 0);
    assert_eq!(body["items"][0]["id"].as_str(), Some(jobs[0].as_str()));
}

/// A-PUBLIC-2: each job type's detail carries its own stats block and no
/// other -- games and pairs the match-test block in their own unit, an opening-rack
/// job its rack progress, a leave job its generation -- and an unknown job is
/// a 404, for the detail and the stream alike.
#[tokio::test]
async fn job_detail_carries_the_stats_block_of_its_type() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;

    let games = db.games_job(2).await;
    let pairs = db.bare_job("game_pairs", admin).await;
    let p1 = db.static_player("p1", admin).await;
    let p2 = db.static_player("p2", admin).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs, max_pairs)
         VALUES ($1, $2, $3, 1, 10, 20)",
    )
    .bind(pairs)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    let racks = opening_rack_job(&db, 6).await;
    let leave = db.bare_job("leave_generation", admin).await;
    let kwg = db.input_data("kwg", "CSW24").await;
    let leave_player = db.leave_player(kwg, true, admin).await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, player_config_id, num_iterations, target_rack_counts, racks_per_task)
         VALUES ($1, $2, 100, ARRAY[100, 500, 1000], 50)",
    )
    .bind(leave)
    .bind(leave_player)
    .execute(&db.pool)
    .await
    .unwrap();

    let blocks = ["games", "opening_racks", "leave_generation"];
    for (job, job_type, block) in [
        (games, "games", "games"),
        (pairs, "game_pairs", "games"),
        (racks, "opening_rack", "opening_racks"),
        (leave, "leave_generation", "leave_generation"),
    ] {
        let (status, body) = send(&app, get_request(&format!("/api/jobs/{job}"), &[])).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["job"]["id"], json!(job));
        assert_eq!(body["job"]["job_type"], job_type);
        for other in blocks {
            assert_eq!(body.get(other).is_some(), other == block, "{job_type} and {other}: {body}");
        }
    }

    let (_, games) = send(&app, get_request(&format!("/api/jobs/{games}"), &[])).await;
    assert_eq!(games["games"]["unit"], "game");
    assert!(games["games"].get("pentanomial").is_none(), "{games}");
    let (_, pairs) = send(&app, get_request(&format!("/api/jobs/{pairs}"), &[])).await;
    assert_eq!(pairs["games"]["unit"], "pair");
    assert_eq!(pairs["games"]["pentanomial"], json!([0, 0, 0, 0, 0]));
    assert_eq!(pairs["games"]["min_units"], 10);
    assert_eq!(pairs["games"]["max_units"], 20);
    // A config row that says nothing of the test runs none, and reports none:
    // `games_job` asks for one, this pairs job does not.
    assert_eq!(games["games"]["test"]["status"], "running", "{games}");
    assert_eq!(pairs["games"]["test"], json!(null), "{pairs}");
    let (_, racks) = send(&app, get_request(&format!("/api/jobs/{racks}"), &[])).await;
    assert_eq!(
        racks["opening_racks"],
        json!({ "racks_analyzed": 0, "racks_settled": 0, "racks_without_consensus": 0, "racks_total": 100 })
    );
    let (_, settings) = send(&app, get_request(&format!("/api/jobs/{leave}/config"), &[])).await;
    assert_eq!(settings["leave_generation"]["target_rack_counts"], json!([100, 500, 1000]));
    // The lexicon and wordmap setting are the player's, shown with it.
    let players = settings["players"].as_array().unwrap();
    assert_eq!(players.len(), 1, "{settings}");
    assert_eq!(players[0]["role"], "player");
    assert_eq!(players[0]["id"], json!(leave_player));
    assert_eq!(players[0]["lexicon"], "CSW24");
    assert_eq!(players[0]["use_wordmap"], true);
    assert!(settings["leave_generation"].get("lexicon").is_none(), "{settings}");
    assert!(settings["leave_generation"].get("use_wordmap").is_none(), "{settings}");
    let (_, leave) = send(&app, get_request(&format!("/api/jobs/{leave}"), &[])).await;
    assert_eq!(leave["leave_generation"]["current_generation"], 1);
    assert_eq!(leave["leave_generation"]["generation_count"], 3);
    assert_eq!(leave["leave_generation"]["target_rack_count"], 100, "generation 1's target");
    assert_eq!(leave["leave_generation"]["target_rack_counts"], json!([100, 500, 1000]));
    assert_eq!(leave["job"]["lexicon"], "CSW24");

    let unknown = Uuid::new_v4();
    for path in [format!("/api/jobs/{unknown}"), format!("/api/jobs/{unknown}/stream")] {
        let (status, body) = send(&app, get_request(&path, &[])).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(body["message"], "no such job", "{path}");
    }
}

/// A-PUBLIC-3: the results feed pages by cursor through every result, newest
/// first, and filters to one contributor -- an account by username, an
/// anonymous worker by pseudonym, a name that is nobody's to an empty page --
/// with `total` always -1: an exact count of a job's results is deliberately
/// never computed.
#[tokio::test]
async fn the_results_feed_paginates_and_filters_without_counting() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = db.games_job(2).await;
    let alice = db.user("alice", false).await;
    let worker = anon_worker(&db, 2).await;
    let mut newest_first = Vec::new();
    for (second, user, anon) in [
        (1, Some(alice), None),
        (2, None, Some(worker)),
        (3, Some(alice), None),
        (4, None, Some(worker)),
        (5, Some(alice), None),
    ] {
        let at = format!("2026-02-01T00:00:0{second}Z");
        newest_first.push((game_result_at(&db, job, user, anon, &at).await, user.is_some()));
    }
    newest_first.reverse();

    // Walks the feed at `per_page` from `query`, returning each page's task ids.
    let walk = |query: String| {
        let app = app.clone();
        async move {
            let mut pages: Vec<Vec<Uuid>> = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let path = match &cursor {
                    Some(c) => format!("/api/jobs/{job}/results?per_page=2{query}&cursor={c}"),
                    None => format!("/api/jobs/{job}/results?per_page=2{query}"),
                };
                let (status, body) = send(&app, get_request(&path, &[])).await;
                assert_eq!(status, StatusCode::OK, "{path}: {body}");
                assert_eq!(body["total"], -1, "{path}: {body}");
                assert_eq!(body["per_page"], 2);
                pages.push(
                    body["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| item["task_id"].as_str().unwrap().parse().unwrap())
                        .collect(),
                );
                match body["next_cursor"].as_str() {
                    Some(next) => cursor = Some(next.to_string()),
                    None => return pages,
                }
                assert!(pages.len() < 10, "the cursor does not advance");
            }
        }
    };

    let all: Vec<Uuid> = newest_first.iter().map(|(task, _)| *task).collect();
    let pages = walk(String::new()).await;
    assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [2, 2, 1]);
    assert_eq!(pages.concat(), all, "every result once, newest first");

    let alices: Vec<Uuid> =
        newest_first.iter().filter(|(_, by_user)| *by_user).map(|(task, _)| *task).collect();
    assert_eq!(walk("&worker=alice".into()).await.concat(), alices);

    let pseudonym = birdtest::auth::public_anon_id(worker);
    let anons: Vec<Uuid> =
        newest_first.iter().filter(|(_, by_user)| !*by_user).map(|(task, _)| *task).collect();
    assert_eq!(walk(format!("&worker={pseudonym}")).await.concat(), anons);

    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{job}/results?worker=nobody"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"], json!([]));
    assert_eq!(body["total"], -1);
    assert!(body.get("next_cursor").is_none(), "{body}");
}

/// A-PUBLIC-3b: a contributor's page is read through their own claims, newest
/// completion first, and pages exactly -- inside one opening-rack batch as
/// well as across batches -- in the unfiltered feed's order. A name that is
/// both an account and an anonymous pseudonym is both contributors' work,
/// merged in that order.
#[tokio::test]
async fn the_filtered_feed_pages_through_a_contributors_claims() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = opening_rack_job(&db, 3).await;

    // Claims and completes the next batch, answering every rack the same way.
    let complete = |headers: Vec<(&'static str, String)>| {
        let app = app.clone();
        async move {
            let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let (status, assignment) =
                send(&app, post_json("/api/worker/task", &headers, claim_body("1.0.0", &[]))).await;
            assert_eq!(status, StatusCode::OK, "{assignment}");
            let racks = assignment["task_request"]["racks"].as_array().unwrap().clone();
            // A new anonymous worker submits as the identity it was just given.
            let minted = assignment["worker_uuid"].as_str().map(str::to_string);
            let headers = match &minted {
                Some(uuid) if headers.is_empty() => vec![("x-worker-uuid", uuid.as_str())],
                _ => headers,
            };
            let result = json!({
                "racks": racks.iter().map(|rack| json!({
                    "rack": rack,
                    "num_moves": 1,
                    "moves": [{ "move": "8G WUZ", "score": 30, "equity": 32.5 }],
                })).collect::<Vec<_>>()
            });
            let (status, body) = send(
                &app,
                post_json(
                    "/api/worker/result",
                    &headers,
                    json!({ "claim_token": assignment["claim_token"], "movegens": 1000, "result": result }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            minted
        }
    };

    let a = complete(Vec::new()).await.unwrap();
    let b = complete(Vec::new()).await.unwrap();
    // An account whose name is `b`'s pseudonym.
    let b_pseudonym = birdtest::auth::public_anon_id(b.parse().unwrap());
    let user = db.user(&b_pseudonym, false).await;
    let raw_key = "bt_".to_string() + &"c".repeat(64);
    sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, $2)")
        .bind(user)
        .bind(birdtest::auth::api_key::hash_key(&raw_key))
        .execute(&db.pool)
        .await
        .unwrap();
    let bearer = vec![("authorization", format!("Bearer {raw_key}"))];
    complete(vec![("x-worker-uuid", a.clone())]).await;
    complete(bearer.clone()).await;
    complete(vec![("x-worker-uuid", b.clone())]).await;
    complete(vec![("x-worker-uuid", a.clone())]).await;

    // Walks the feed at two a page, returning (task, rack, contributor) per
    // item and the page sizes.
    let walk = |query: String| {
        let app = app.clone();
        async move {
            let mut items = Vec::new();
            let mut sizes = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let path = match &cursor {
                    Some(c) => format!("/api/jobs/{job}/results?per_page=2{query}&cursor={c}"),
                    None => format!("/api/jobs/{job}/results?per_page=2{query}"),
                };
                let (status, body) = send(&app, get_request(&path, &[])).await;
                assert_eq!(status, StatusCode::OK, "{path}: {body}");
                let page = body["items"].as_array().unwrap();
                sizes.push(page.len());
                items.extend(page.iter().map(|item| {
                    let who = item["username"].as_str().or(item["anon_id"].as_str());
                    (
                        item["task_id"].as_str().unwrap().to_string(),
                        item["rack"].as_str().unwrap().to_string(),
                        who.unwrap().to_string(),
                    )
                }));
                match body["next_cursor"].as_str() {
                    Some(next) => cursor = Some(next.to_string()),
                    None => return (items, sizes),
                }
                assert!(sizes.len() < 20, "the cursor does not advance");
            }
        }
    };

    let (all, _) = walk(String::new()).await;
    assert_eq!(all.len(), 18, "six batches of three");
    let a_pseudonym = birdtest::auth::public_anon_id(a.parse().unwrap());
    let by = |who: &str| all.iter().filter(|item| item.2 == who).cloned().collect::<Vec<_>>();

    let (mine, sizes) = walk(format!("&worker={a_pseudonym}")).await;
    assert_eq!(mine, by(&a_pseudonym), "the unfiltered feed's order, every record once");
    assert_eq!(sizes, [2, 2, 2, 2, 1], "pages break inside a batch");

    // `b`'s pseudonym is also the account's name: both contributors' work.
    let (both, _) = walk(format!("&worker={b_pseudonym}")).await;
    let expected: Vec<_> = all.iter().filter(|item| item.2 == b_pseudonym).cloned().collect();
    assert_eq!(expected.len(), 9, "two anonymous batches and one of the account's");
    assert_eq!(both, expected);
}

/// A-PUBLIC-4: `?rack=` on an opening-rack job returns an analysed rack's
/// whole ranked list, however the rack is typed. A rack with no analysis is
/// an empty list rather than a 404 -- see the report on TESTING.md's wording
/// -- and an unknown job is a 404.
#[tokio::test]
async fn rack_lookup_finds_an_analysed_rack() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = opening_rack_job(&db, 3).await;

    let (assignment, uuid) = first_claim(&app).await;
    let racks: Vec<String> = assignment["task_request"]["racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    let result = json!({
        "racks": racks.iter().map(|rack| json!({
            "rack": rack,
            "num_moves": 2,
            "moves": [
                { "move": "8G WUZ", "score": 30, "equity": 32.5 },
                { "move": "8H ZA", "score": 22, "equity": 21.0 },
            ],
        })).collect::<Vec<_>>()
    });
    submit(&app, &assignment, &uuid, result).await;

    // Lower case and reversed: the lookup canonicalises before it searches.
    let typed: String = racks[1].to_lowercase().chars().rev().collect();
    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{job}/results?rack={typed}"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["items"],
        json!([
            { "analysis": 1, "rank": 1, "move": "8G WUZ", "score": 30, "equity": 32.5,
              "iterations": null, "win_percentage": null, "plies": [] },
            { "analysis": 1, "rank": 2, "move": "8H ZA", "score": 22, "equity": 21.0,
              "iterations": null, "win_percentage": null, "plies": [] },
        ]),
        "{typed} -> {}",
        racks[1]
    );
    assert_eq!(body["total"], 2, "a lookup is whole, so its count is exact");

    let unanalysed = "QQQQQQQ";
    assert!(!racks.contains(&unanalysed.to_string()));
    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{job}/results?rack={unanalysed}"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"], json!([]));
    assert_eq!(body["total"], 0);

    let (status, body) = send(
        &app,
        get_request(&format!("/api/jobs/{}/results?rack={typed}", Uuid::new_v4()), &[]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["message"], "no such job");
}

/// A-PUBLIC-4h: `/rack-samples` draws distinct racks an opening-rack job has
/// analysed: exactly, from a small job read whole; by probes of its rack
/// space, from a large one. Only an opening-rack job's; an unknown job is a
/// 404.
#[tokio::test]
async fn rack_samples_are_distinct_analysed_racks() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = opening_rack_job(&db, 12).await;
    let path = format!("/api/jobs/{job}/rack-samples");
    let racks_of = |body: &serde_json::Value| -> Vec<String> {
        body["racks"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect()
    };

    let (status, body) = send(&app, get_request(&path, &[])).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "racks": [] })), "nothing analysed yet");

    let (assignment, uuid) = first_claim(&app).await;
    let mut analysed: Vec<String> = assignment["task_request"]["racks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    assert_eq!(analysed.len(), 12);
    let result = json!({
        "racks": analysed.iter().map(|rack| json!({
            "rack": rack, "num_moves": 1, "moves": [{ "move": "8G AB", "score": 8, "equity": 9.5 }],
        })).collect::<Vec<_>>()
    });
    submit(&app, &assignment, &uuid, result).await;

    // A small job, read whole: ten of its racks by default, all of them when
    // asked for more, never one twice.
    let (status, body) = send(&app, get_request(&path, &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ten = racks_of(&body);
    assert_eq!(ten.len(), 10, "{body}");
    assert_eq!(ten.iter().collect::<std::collections::HashSet<_>>().len(), 10, "{body}");
    assert!(ten.iter().all(|rack| analysed.contains(rack)), "{body}");
    let (_, body) = send(&app, get_request(&format!("{path}?n=500"), &[])).await;
    let mut all = racks_of(&body);
    all.sort();
    analysed.sort();
    assert_eq!(all, analysed, "every rack once, the request held to the cap");
    let (_, body) = send(&app, get_request(&format!("{path}?n=0"), &[])).await;
    assert_eq!(racks_of(&body).len(), 1, "at least one is asked for");

    // A large one, probed: past a thousand analyses, spread over the space of
    // strings the probes are drawn from.
    let (claim, task): (Uuid, Uuid) =
        sqlx::query_as("SELECT id, task_id FROM task_claims WHERE claim_token = $1")
            .bind(Uuid::parse_str(assignment["claim_token"].as_str().unwrap()).unwrap())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let mut more = std::collections::BTreeSet::new();
    let mut seed: u64 = 7;
    while more.len() < 1200 {
        let rack: String = (0..7)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                b"?ABCDE"[(seed >> 33) as usize % 6] as char
            })
            .collect();
        if !analysed.contains(&rack) {
            more.insert(rack);
        }
    }
    let more: Vec<String> = more.into_iter().collect();
    sqlx::query(
        "INSERT INTO position_analysis_records (task_claim_id, task_id, job_id, rack, analysis, num_moves)
         SELECT $1, $2, $3, rack, 'static', 1 FROM unnest($4::text[]) AS rack",
    )
    .bind(claim)
    .bind(task)
    .bind(job)
    .bind(&more)
    .execute(&db.pool)
    .await
    .unwrap();
    let every: std::collections::HashSet<&String> = analysed.iter().chain(&more).collect();
    let (status, body) = send(&app, get_request(&path, &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let probed = racks_of(&body);
    assert_eq!(probed.len(), 10, "{body}");
    assert_eq!(probed.iter().collect::<std::collections::HashSet<_>>().len(), 10, "{body}");
    assert!(probed.iter().all(|rack| every.contains(rack)), "{body}");

    let games = db.games_job(2).await;
    let (status, body) = send(&app, get_request(&format!("/api/jobs/{games}/rack-samples"), &[])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _) =
        send(&app, get_request(&format!("/api/jobs/{}/rack-samples", Uuid::new_v4()), &[])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A games job with `capture_positions` on.
async fn capturing_games_job(db: &TestDb) -> Uuid {
    let job = db.games_job(2).await;
    sqlx::query("UPDATE job_game_config SET capture_positions = true WHERE job_id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    job
}

/// A-PUBLIC-4g: a simulated position's moves show their win percentage and
/// their first two plies' statistics, however many more the job recorded, and
/// the position what its player inferred of the opponent's leave first. A
/// position that claims an inference it cannot have -- on a static analysis,
/// on a game's first turn, with eleven leaves, or leaves out of order -- is
/// refused.
#[tokio::test]
async fn a_simulated_position_shows_its_plies_and_its_inference() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let cfg = state.cfg.clone();
    let app = birdtest::app(state);
    let job = capturing_games_job(&db).await;
    db.simulate_player1(job, true).await;
    // Recording four plies, so the read is what keeps it to two.
    sqlx::query(
        "UPDATE player_configs SET num_plies_recorded = 4
         WHERE id = (SELECT player1_config_id FROM job_game_config WHERE job_id = $1)",
    )
    .bind(job)
    .execute(&db.pool)
    .await
    .unwrap();

    let plies = json!([
        { "ply": 0, "bingo_percentage": 12.5, "average_score": 31.25 },
        { "ply": 1, "bingo_percentage": 7.5, "average_score": 28.5 },
        { "ply": 2, "bingo_percentage": 5.0, "average_score": 30.5 },
    ]);
    let leaves = |n: usize| -> Vec<serde_json::Value> {
        (0..n).map(|i| json!({ "leave": "EIR", "draws": 100 - i as i64, "equity": 18.25 })).collect()
    };
    let inference = json!({
        "num_leaves": 40, "total_draws": 900, "average_equity": 12.5,
        "leaves": [
            { "leave": "EIR", "draws": 120, "equity": 18.25 },
            { "leave": "AET", "draws": 80, "equity": 15.0 },
        ],
    });
    let position = |turn: i64, analysis: &str, inference: Option<serde_json::Value>| {
        let simmed = analysis == "sim";
        let mut position = json!({
            "game_index": 0, "turn_number": turn, "rack": "AABCDE?",
            "position": format!("cgp-{turn}"), "played_move": "8D BACCAE",
            "played_move_score": 74, "num_moves": 40, "analysis": analysis,
            "moves": [{
                "move": "8D BACCAE", "score": 74, "equity": 81.2,
                "iterations": if simmed { 340 } else { 0 },
                "win_percentage": if simmed { json!(61.5) } else { json!(null) },
                "plies": if simmed { plies.clone() } else { json!([]) },
            }],
        });
        if turn > 0 {
            position["previous_move"] = json!("8G DAB");
            position["previous_move_score"] = json!(12);
        }
        if let Some(inference) = inference {
            position["inference"] = inference;
        }
        position
    };
    // Every submission also carries game 1's first turn, which a capturing
    // job requires, so a refusal is the inference's.
    let other_game = {
        let mut other = position(0, "sim", None);
        other["game_index"] = json!(1);
        other["rack"] = json!("ABBCDEE");
        other
    };
    let submit_positions = |positions: serde_json::Value| {
        let app = app.clone();
        let other_game = other_game.clone();
        async move {
            let (assignment, uuid) = first_claim(&app).await;
            let mut result = games_result(2, 1);
            let mut positions = positions.as_array().unwrap().clone();
            positions.push(other_game);
            result["positions"] = json!(positions);
            send(
                &app,
                post_json(
                    "/api/worker/result",
                    &[("x-worker-uuid", uuid.as_str())],
                    json!({ "claim_token": assignment["claim_token"], "movegens": 1000, "result": result }),
                ),
            )
            .await
        }
    };

    let mut too_many = inference.clone();
    too_many["leaves"] = json!(leaves(11));
    too_many["num_leaves"] = json!(400);
    let mut out_of_order = inference.clone();
    out_of_order["leaves"] = json!([
        { "leave": "AET", "draws": 80, "equity": 15.0 },
        { "leave": "EIR", "draws": 120, "equity": 18.25 },
    ]);
    for (bad, why) in [
        (position(3, "static", Some(inference.clone())), "a static analysis"),
        (position(0, "sim", Some(inference.clone())), "a first turn"),
        (position(3, "sim", Some(too_many)), "eleven leaves"),
        (position(3, "sim", Some(out_of_order)), "leaves out of order"),
    ] {
        let (status, body) = submit_positions(json!([bad])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
    }

    let (status, body) = submit_positions(json!([
        position(0, "sim", None),
        position(3, "sim", Some(inference.clone())),
    ]))
    .await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({ "accepted": true })));

    let user = db.user(&format!("reader{}", Uuid::new_v4().simple()), false).await;
    let headers = admin_headers(&cfg, user);
    let (status, page) = send(
        &app,
        get_request(&format!("/api/jobs/{job}/positions?rack=AABCDE%3F"), &headers),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{page}");
    let at = |turn: i64| items.iter().find(|p| p["turn_number"] == turn).unwrap();
    // Stored to the four plies recorded, shown to two.
    let shown = json!([
        { "ply": 0, "bingo_percentage": 12.5, "average_score": 31.25 },
        { "ply": 1, "bingo_percentage": 7.5, "average_score": 28.5 },
    ]);
    for turn in [0, 3] {
        assert_eq!(at(turn)["moves"][0]["plies"], shown, "{page}");
        assert_eq!(at(turn)["moves"][0]["win_percentage"], json!(61.5), "{page}");
        assert_eq!(at(turn)["moves"][0]["iterations"], json!(340), "{page}");
    }
    assert_eq!(at(0)["inference"], json!(null), "{page}");
    assert_eq!(at(3)["inference"], inference, "{page}");
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM position_analysis_plies p
         JOIN position_analysis_moves m ON m.id = p.move_id
         JOIN position_analysis_records r ON r.id = m.record_id WHERE r.job_id = $1",
    )
    .bind(job)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stored, 9, "three plies a move, three positions (game 1's too)");
}

/// A-PUBLIC-4b: a games job's captured positions are searchable by rack by a
/// signed-in user -- newest first, a page at a time, each with its ranked
/// moves -- the rack however it is typed, spelt as MAGPIE spells one (blank
/// last), and by nobody signed out. A search names a rack; a job type that
/// captures nothing is refused.
#[tokio::test]
async fn captured_positions_are_searchable_when_signed_in() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let cfg = state.cfg.clone();
    let app = birdtest::app(state);
    let job = capturing_games_job(&db).await;
    // The racks as MAGPIE writes them (`rack_get_string`): machine-letter
    // order, the blank last. The job's distribution is `testdist.csv`.
    for batch in 0..2 {
        let mut result = games_result(2, 1);
        result["positions"] = json!([
            { "game_index": 0, "turn_number": 0, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "AABCDE?", "position": format!("cgp-{batch}-0"),
              "num_moves": 40, "moves": [
                  { "move": "8D BACCAE", "score": 74, "equity": 81.2 },
                  { "move": "8D ABACE", "score": 72, "equity": 79.0 } ] },
            { "game_index": 1, "turn_number": 3, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "ABBCDEE", "position": format!("cgp-{batch}-1"),
              "previous_move": "8D DAB", "previous_move_score": 10,
              "num_moves": 30, "moves": [{ "move": "8D BEDE", "score": 70, "equity": 77.0 }] },
        ]);
        let (assignment, uuid) = first_claim(&app).await;
        submit(&app, &assignment, &uuid, result).await;
    }

    let path = format!("/api/jobs/{job}/positions");
    let (status, _) = send(&app, get_request(&format!("{path}?rack=AABCDE?"), &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "signed out");

    let user = db.user(&format!("reader{}", Uuid::new_v4().simple()), false).await;
    let headers = admin_headers(&cfg, user);

    let (status, body) = send(&app, get_request(&path, &headers)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "no rack: {body}");

    // One rack, typed in lower case, out of order and with the blank first
    // (URL-encoded), one to a page, newest first.
    let (status, first) =
        send(&app, get_request(&format!("{path}?rack=%3Fedcbaa&per_page=1"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["items"].as_array().unwrap().len(), 1, "{first}");
    assert_eq!(first["items"][0]["position"], "cgp-1-0", "{first}");
    assert_eq!(first["items"][0]["rack"], "AABCDE?");
    assert_eq!(first["items"][0]["analysis"], "static");
    assert_eq!(
        first["items"][0]["moves"],
        json!([
            { "rank": 1, "move": "8D BACCAE", "score": 74, "equity": 81.2, "iterations": null,
              "win_percentage": null, "mean_spread": null, "fidelity_plies": null, "plies": [] },
            { "rank": 2, "move": "8D ABACE", "score": 72, "equity": 79.0, "iterations": null,
              "win_percentage": null, "mean_spread": null, "fidelity_plies": null, "plies": [] },
        ])
    );
    // A static position infers nothing.
    assert_eq!(first["items"][0]["inference"], json!(null));
    let cursor = first["next_cursor"].as_str().expect("a full page has a next");
    let (_, second) = send(
        &app,
        get_request(&format!("{path}?rack=%3Fedcbaa&per_page=1&cursor={cursor}"), &headers),
    )
    .await;
    assert_eq!(second["items"][0]["position"], "cgp-0-0", "{second}");
    // The rack's last position: no "Next" to a page with nothing on it.
    assert!(second["next_cursor"].is_null(), "{second}");

    let (_, later) = send(&app, get_request(&format!("{path}?rack=eedcbba"), &headers)).await;
    let items = later["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{later}");
    assert!(later["next_cursor"].is_null(), "{later}");
    assert_eq!(items[0]["game_index"], 1);
    assert_eq!(items[0]["turn_number"], 3);
    assert_eq!(items[0]["previous_move"], "8D DAB");
    assert_eq!(items[0]["previous_move_score"], 10);
    // And the move played from it, which the board draws where it goes.
    assert_eq!(items[0]["played_move"], "8D PLAYED");
    assert_eq!(items[0]["played_move_score"], 10);
    assert_eq!(items[0]["num_moves"], 30);

    // A rack no tile of the distribution spells finds nothing, rather than
    // failing.
    let (status, none) = send(&app, get_request(&format!("{path}?rack=QQQQQQQ"), &headers)).await;
    assert_eq!((status, &none["items"]), (StatusCode::OK, &json!([])));

    let racks = opening_rack_job(&db, 3).await;
    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{racks}/positions?rack=A"), &headers)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _) = send(
        &app,
        get_request(&format!("/api/jobs/{}/positions?rack=A", Uuid::new_v4()), &headers),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A-PUBLIC-4c: a random position is drawn from the tasks that have one,
/// passing over those still being played, and a job that has captured
/// nothing -- no task yet, or none returned -- answers `null`. Signed in only;
/// games and pairs jobs only.
#[tokio::test]
async fn a_random_position_is_drawn_from_the_tasks_that_have_one() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let cfg = state.cfg.clone();
    let app = birdtest::app(state);
    let job = capturing_games_job(&db).await;
    let path = format!("/api/jobs/{job}/positions/random");
    let (status, _) = send(&app, get_request(&path, &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "signed out");
    let user = db.user(&format!("reader{}", Uuid::new_v4().simple()), false).await;
    let headers = admin_headers(&cfg, user);

    let (status, body) = send(&app, get_request(&path, &headers)).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!(null)), "no task yet");

    // Five tasks claimed, none returned: every draw lands on a task with no
    // positions, and so does the fallback.
    let mut claims = Vec::new();
    for _ in 0..5 {
        claims.push(first_claim(&app).await);
    }
    let seeds: Vec<i64> = sqlx::query_scalar("SELECT seed FROM tasks WHERE job_id = $1 ORDER BY seed")
        .bind(job)
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(seeds.len(), 5, "one task a claim");
    let (status, body) = send(&app, get_request(&path, &headers)).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!(null)), "none returned yet");

    // The middle task returns two positions. Every draw is one of them; the
    // first captured is never the fallback (the newest), so its appearing is
    // a draw that found its task past the four that have none.
    let (assignment, uuid) = &claims[2];
    let mut result = games_result(2, 1);
    result["positions"] = json!([
        { "game_index": 0, "turn_number": 0, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "AABCDE?", "position": "first",
          "num_moves": 3, "moves": [{ "move": "8D BACCAE", "score": 74, "equity": 81.2 }] },
        { "game_index": 1, "turn_number": 4, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "ABBCDEE", "position": "second",
          "previous_move": "8D DAB", "previous_move_score": 10,
          "num_moves": 3, "moves": [{ "move": "8D BEDE", "score": 70, "equity": 77.0 }] },
    ]);
    submit(&app, assignment, uuid, result).await;
    let task: Uuid = sqlx::query_scalar("SELECT task_id FROM task_claims WHERE claim_token = $1")
        .bind(Uuid::parse_str(assignment["claim_token"].as_str().unwrap()).unwrap())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..40 {
        let (status, body) = send(&app, get_request(&path, &headers)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["task_id"], json!(task), "{body}");
        assert!(!body["moves"].as_array().unwrap().is_empty(), "with its ranked moves: {body}");
        seen.insert(body["position"].as_str().unwrap().to_string());
    }
    assert_eq!(seen.len(), 2, "both positions drawn: {seen:?}");

    let racks = opening_rack_job(&db, 3).await;
    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{racks}/positions/random"), &headers)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _) = send(
        &app,
        get_request(&format!("/api/jobs/{}/positions/random", Uuid::new_v4()), &headers),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A-PUBLIC-4d: a job's board is public: its layout square by square, the
/// start square, and every letter of its distribution with its blank's
/// spelling and its score.
#[tokio::test]
async fn a_jobs_board_is_its_layout_and_letter_scores() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let job = db.games_job(2).await;
    let (status, board) = send(&app, get_request(&format!("/api/jobs/{job}/board"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{board}");
    assert_eq!(board["start"], json!([7, 7]));
    let squares = board["squares"].as_array().unwrap();
    assert_eq!(squares.len(), 15);
    assert!(squares.iter().all(|row| row.as_array().unwrap().len() == 15));
    assert_eq!(squares[0][0], "triple_word");
    assert_eq!(squares[0][3], "double_letter");
    assert_eq!(squares[1][1], "double_word");
    assert_eq!(squares[1][5], "triple_letter");
    assert_eq!(squares[0][1], "normal");
    assert_eq!(
        board["letters"],
        json!([
            { "letter": "?", "blank": "?", "score": 0 },
            { "letter": "A", "blank": "a", "score": 1 },
            { "letter": "B", "blank": "b", "score": 3 },
            { "letter": "C", "blank": "c", "score": 3 },
            { "letter": "D", "blank": "d", "score": 2 },
            { "letter": "E", "blank": "e", "score": 1 },
        ])
    );
    let (status, _) =
        send(&app, get_request(&format!("/api/jobs/{}/board", Uuid::new_v4()), &[])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// The live stream
// ---------------------------------------------------------------------------

/// A-PUBLIC-5: the stream opens with the job's stats, and after a result is
/// accepted it sends the stats again -- each event's data byte-for-byte what
/// `GET /api/jobs/:id` returns at that moment, so the dashboard can replace
/// its state with an event rather than merge it.
#[tokio::test]
async fn the_stream_sends_what_a_reload_would_fetch_after_each_result() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let app = birdtest::app(db.state().await);
    let detail = format!("/api/jobs/{job}");

    let response = app
        .clone()
        .oneshot(get_request(&format!("{detail}/stream"), &[]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body().into_data_stream();
    let mut buffer = String::new();

    let initial = next_stats_event(&mut body, &mut buffer).await;
    let (status, reload) = get_bytes(&app, &detail).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial.as_bytes(), reload.as_slice(), "the opening event is the reload");

    let (assignment, uuid) = first_claim(&app).await;
    submit(&app, &assignment, &uuid, games_result(2, 1)).await;

    let pushed = next_stats_event(&mut body, &mut buffer).await;
    let parsed: serde_json::Value = serde_json::from_str(&pushed).unwrap();
    assert_eq!(parsed["games"]["units_completed"], 2, "the event carries the result: {parsed}");
    assert_eq!(parsed["games"]["wins"], 1);
    let (_, reload) = get_bytes(&app, &detail).await;
    assert_eq!(
        pushed,
        String::from_utf8(reload).unwrap(),
        "the pushed event is byte-for-byte the reload"
    );
}

/// A-PUBLIC-6: a dashboard that goes away takes its subscription with it --
/// the job has no subscribers once the stream's body is dropped, which is what
/// the submission path asks before building a payload -- and the next
/// submission is accepted as usual.
#[tokio::test]
async fn the_stream_unsubscribes_when_the_client_disconnects() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    assert!(!state.sse.has_subscribers(job));

    let open = |app: axum::Router| async move {
        let response =
            app.oneshot(get_request(&format!("/api/jobs/{job}/stream"), &[])).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        next_stats_event(&mut body, &mut String::new()).await;
        body
    };
    let first = open(app.clone()).await;
    let second = open(app.clone()).await;
    assert!(state.sse.has_subscribers(job));

    drop(first);
    assert!(state.sse.has_subscribers(job), "one dashboard is still open");
    drop(second);
    assert!(!state.sse.has_subscribers(job), "nobody is watching once both have gone");

    let (assignment, uuid) = first_claim(&app).await;
    submit(&app, &assignment, &uuid, games_result(2, 1)).await;
    assert!(!state.sse.has_subscribers(job));
}

// ---------------------------------------------------------------------------
// Contributor lists
// ---------------------------------------------------------------------------

/// A-PUBLIC-7: the user and worker lists page by contribution with a total,
/// leave deleted accounts out of the user list, and never carry an email
/// address, a password hash, an API key hash or an anonymous worker's UUID --
/// checked against the whole response body, not just the fields expected.
#[tokio::test]
async fn contributor_lists_paginate_and_leak_no_credentials() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    db.user("root", true).await;
    let mut secrets = Vec::new();
    let mut users = Vec::new();
    for (name, tasks) in [("alice", 5), ("bob", 3), ("carol", 1), ("dave", 9)] {
        let id = db.user(name, false).await;
        users.push(id);
        sqlx::query(
            "UPDATE users SET tasks_completed = $2, compute_ms = $2 * 1000, last_completed_at = now(),
                              password_hash = '$argon2id$v=19$secret-' || username
             WHERE id = $1",
        )
        .bind(id)
        .bind(tasks)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO api_keys (user_id, key_hash) VALUES ($1, 'keyhash-' || $2)")
            .bind(id)
            .bind(name)
            .execute(&db.pool)
            .await
            .unwrap();
        secrets.push(format!("{name}@example.invalid"));
        secrets.push(format!("secret-{name}"));
        secrets.push(format!("keyhash-{name}"));
    }
    // Deleted as `delete_user` does it: the name becomes a tombstone, and the
    // account's contributions stay in the worker ranking under it.
    sqlx::query(
        "UPDATE users SET deleted_at = now(), username = 'deleted-dave' WHERE username = 'dave'",
    )
        .execute(&db.pool)
        .await
        .unwrap();
    secrets.push("root@example.invalid".into());
    let busy = anon_worker(&db, 4).await;
    let light = anon_worker(&db, 2).await;
    let idle = anon_worker(&db, 0).await;
    for uuid in [busy, light, idle] {
        secrets.push(uuid.to_string());
    }

    let pages = |path: &'static str, count: usize| {
        let app = app.clone();
        let secrets = secrets.clone();
        async move {
            let mut names = Vec::new();
            for page in 0..count {
                let sep = if path.contains('?') { '&' } else { '?' };
                let (status, body) =
                    send(&app, get_request(&format!("{path}{sep}page={page}&per_page=2"), &[])).await;
                assert_eq!(status, StatusCode::OK, "{body}");
                let text = body.to_string();
                for secret in &secrets {
                    assert!(!text.contains(secret.as_str()), "{path} leaks {secret}: {text}");
                }
                assert!(!text.contains("\"email\""), "{path} has an email field: {text}");
                assert!(!text.contains("hash"), "{path} names a hash: {text}");
                assert!(!text.contains("anon_uuid"), "{path} names a credential: {text}");
                names.push(
                    body["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| match item["username"].as_str() {
                            Some(name) => name.to_string(),
                            None => item["anon_id"].as_str().unwrap().to_string(),
                        })
                        .collect::<Vec<_>>(),
                );
                assert_eq!(body["per_page"], 2);
                names.push(vec![body["total"].to_string()]);
            }
            names
        }
    };

    assert_eq!(
        pages("/api/users", 2).await,
        [vec!["alice", "bob"], vec!["4"], vec!["carol", "root"], vec!["4"]],
        "by contribution, deleted accounts left out"
    );
    let (busy_id, light_id) =
        (birdtest::auth::public_anon_id(busy), birdtest::auth::public_anon_id(light));
    assert_eq!(
        pages("/api/workers?sort=tasks", 3).await,
        [
            vec!["deleted-dave".to_string(), "alice".into()],
            vec!["6".into()],
            vec![busy_id, "bob".into()],
            vec!["6".into()],
            vec![light_id, "carol".into()],
            vec!["6".into()],
        ],
        "both kinds of contributor in one ranking, the idle worker left out"
    );

    // Ranked by movegens unless asked otherwise: each order its own, and the
    // same contributors in each. Movegens run against tasks here, so the
    // default is visibly its own order.
    for (name, movegens) in [("alice", 600), ("bob", 500), ("carol", 400), ("deleted-dave", 300)] {
        sqlx::query("UPDATE users SET movegens = $2 WHERE username = $1")
            .bind(name)
            .bind(movegens as i64)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    for (uuid, movegens) in [(busy, 200), (light, 100), (idle, 0)] {
        sqlx::query("UPDATE anonymous_workers SET movegens = $2 WHERE uuid = $1")
            .bind(uuid)
            .bind(movegens as i64)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    let ranked = |sort: &'static str| {
        let app = app.clone();
        async move {
            let (status, body) = send(&app, get_request(&format!("/api/workers{sort}"), &[])).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["total"], 6, "{sort}");
            body["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["username"].as_str().or(item["anon_id"].as_str()).unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    let (busy_id, light_id) =
        (birdtest::auth::public_anon_id(busy), birdtest::auth::public_anon_id(light));
    let by_tasks = ["deleted-dave", "alice", &busy_id, "bob", &light_id, "carol"];
    let by_movegens = ["alice", "bob", "carol", "deleted-dave", &busy_id, &light_id];
    assert_eq!(ranked("").await, by_movegens, "movegens, by default");
    assert_eq!(ranked("?sort=movegens").await, by_movegens);
    assert_eq!(ranked("?sort=compute").await, by_tasks);
    assert_eq!(ranked("?sort=tasks").await, by_tasks);
    let (_, body) = send(&app, get_request("/api/workers?per_page=1&sort=tasks", &[])).await;
    assert_eq!(
        body["items"][0],
        json!({
            "user_id": users[3], "anon_id": null, "username": "deleted-dave",
            "compute_seconds": 9.0, "movegens": 300,
            "tasks_completed": 9, "last_seen_at": body["items"][0]["last_seen_at"],
        })
    );
    // Games and racks were counters once; neither ranks now.
    for sort in ["username", "games", "racks"] {
        let (status, body) = send(&app, get_request(&format!("/api/workers?sort={sort}"), &[])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "only a counter ranks ({sort}): {body}");
    }

    for path in ["/api/users", "/api/workers"] {
        let (_, body) = send(&app, get_request(&format!("{path}?per_page=100000"), &[])).await;
        assert_eq!(body["per_page"], 500, "{path}");
    }
}

/// A-PUBLIC-7 (ties): paging through the worker list shows every contributor
/// exactly once when their contributions tie -- the common case, every worker
/// with one task finished. Bug: the list was ordered by the count alone, and
/// Postgres ordered the tie differently for each page's LIMIT, so some
/// contributors appeared on two pages and others on none.
#[tokio::test]
async fn tied_contributors_are_each_listed_exactly_once() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let mut expected = Vec::new();
    for i in 0..9 {
        let id = db.user(&format!("user{i}"), false).await;
        sqlx::query("UPDATE users SET tasks_completed = 1, compute_ms = 1000 WHERE id = $1")
            .bind(id)
            .execute(&db.pool)
            .await
            .unwrap();
        expected.push(format!("user{i}"));
        expected.push(birdtest::auth::public_anon_id(anon_worker(&db, 1).await));
    }
    expected.sort();

    // Every order ties here -- one task each, a second each, no movegens.
    for (per_page, sort) in [(2, ""), (3, "compute"), (5, "movegens"), (7, "tasks")] {
        let mut seen = Vec::new();
        for page in 0..=(18 / per_page) {
            let sort = if sort.is_empty() { String::new() } else { format!("&sort={sort}") };
            let path = format!("/api/workers?page={page}&per_page={per_page}{sort}");
            let (status, body) = send(&app, get_request(&path, &[])).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["total"], 18);
            for item in body["items"].as_array().unwrap() {
                let name = item["username"].as_str().or(item["anon_id"].as_str()).unwrap();
                seen.push(name.to_string());
            }
        }
        seen.sort();
        assert_eq!(seen, expected, "per_page={per_page} sort={sort}");
    }

    // Far past the end: an empty page with the true total, not a scan.
    let (status, body) = send(&app, get_request("/api/workers?page=1000000&per_page=500", &[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["total"].as_i64(), body["items"].as_array().map(Vec::len)), (Some(18), Some(0)));
}

/// A-PUBLIC-1, A-PUBLIC-7: the job and user lists page every row exactly once
/// even when their sort keys tie. Rows written in one transaction share
/// `created_at` (it is `now()`, the transaction's start), and without the
/// primary key as a last sort key Postgres may order a tie differently for
/// each page's LIMIT -- the bug `/api/workers` had.
#[tokio::test]
async fn tied_jobs_and_users_are_each_listed_exactly_once() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let mut jobs = Vec::new();
    for _ in 0..9 {
        jobs.push(db.bare_job("games", admin).await.to_string());
    }
    for i in 0..9 {
        db.user(&format!("tied{i}"), false).await;
    }
    // One instant for everything: the tie a batch insert produces.
    sqlx::query("UPDATE jobs SET created_at = '2026-01-01T00:00:00Z'").execute(&db.pool).await.unwrap();
    sqlx::query("UPDATE users SET created_at = '2026-01-01T00:00:00Z', tasks_completed = 0")
        .execute(&db.pool)
        .await
        .unwrap();
    let user_names: Vec<String> = {
        let mut names = vec!["root".to_string()];
        names.extend((0..9).map(|i| format!("tied{i}")));
        names.sort();
        names
    };
    jobs.sort();

    for per_page in [2, 3, 4] {
        let (mut seen_jobs, mut seen_users) = (Vec::new(), Vec::new());
        for page in 0..=(10 / per_page) {
            let (status, body) =
                send(&app, get_request(&format!("/api/jobs?page={page}&per_page={per_page}"), &[]))
                    .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            seen_jobs.extend(body["items"].as_array().unwrap().iter().map(|j| j["id"].as_str().unwrap().to_string()));
            let (status, body) =
                send(&app, get_request(&format!("/api/users?page={page}&per_page={per_page}"), &[]))
                    .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            seen_users.extend(
                body["items"].as_array().unwrap().iter().map(|u| u["username"].as_str().unwrap().to_string()),
            );
        }
        seen_jobs.sort();
        seen_users.sort();
        assert_eq!(seen_jobs, jobs, "jobs, per_page={per_page}");
        assert_eq!(seen_users, user_names, "users, per_page={per_page}");
    }
}

/// A-PUBLIC-6d: live pushes for one job are spaced by the stats interval,
/// however submissions arrive. The loop paused only when a submission came in
/// during a build, so a job whose submissions came slower than one build was
/// rebuilt and pushed for each: six full builds in two seconds under a
/// ten-second interval. An admin's change still reaches the page at once.
#[tokio::test]
async fn live_pushes_are_spaced_by_the_stats_interval_but_admin_changes_are_not() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let admin = db.user("root", true).await;
    let mut cfg = db.config();
    cfg.stats_cache = std::time::Duration::from_secs(10);
    let headers = admin_headers(&cfg, admin);
    let app = birdtest::app(db.state_with(cfg).await);
    let response = app
        .clone()
        .oneshot(get_request(&format!("/api/jobs/{job}/stream"), &[]))
        .await
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut buffer = String::new();
    next_stats_event(&mut body, &mut buffer).await;

    let (mut assignment, uuid) = first_claim(&app).await;
    let started = std::time::Instant::now();
    let mut pushes = 0;
    for i in 0..6 {
        submit(&app, &assignment, &uuid, games_result(2, 1)).await;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(400);
        while tokio::time::timeout_at(deadline, next_stats_event(&mut body, &mut buffer)).await.is_ok() {
            pushes += 1;
        }
        if i < 5 {
            let (status, next) = send(
                &app,
                post_json("/api/worker/task", &[("x-worker-uuid", &uuid)], claim_body("1.0.0", &[])),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{next}");
            assignment = next;
        }
    }
    assert!(pushes <= 1, "{pushes} pushes in {} ms under a 10 s interval", started.elapsed().as_millis());

    // Mid-interval, a deactivation is pushed within a second or two.
    let refs: Vec<(&str, &str)> = headers.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (status, deactivated) =
        send(&app, allocate(job, 0, &refs)).await;
    assert_eq!(status, StatusCode::OK, "{deactivated}");
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = next_stats_event(&mut body, &mut buffer).await;
            let parsed: serde_json::Value = serde_json::from_str(&event).unwrap();
            if parsed["job"]["status"] == "inactive" {
                return parsed;
            }
        }
    })
    .await
    .expect("the deactivation reaches the page without waiting out the interval");
    assert_eq!(event["job"]["status"], "inactive");
}

/// A-PUBLIC-6d, the other half: submissions that arrive during a cool-down
/// are pushed when it ends, with nothing urgent to wake it -- together, in
/// one push that counts them all.
#[tokio::test]
async fn submissions_during_a_cool_down_are_pushed_when_it_ends() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let mut cfg = db.config();
    cfg.stats_cache = std::time::Duration::from_secs(2);
    let app = birdtest::app(db.state_with(cfg).await);
    let response = app
        .clone()
        .oneshot(get_request(&format!("/api/jobs/{job}/stream"), &[]))
        .await
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut buffer = String::new();
    next_stats_event(&mut body, &mut buffer).await;

    let (mut assignment, uuid) = first_claim(&app).await;
    for i in 0..3 {
        submit(&app, &assignment, &uuid, games_result(2, 1)).await;
        if i < 2 {
            let (status, next) = send(
                &app,
                post_json("/api/worker/task", &[("x-worker-uuid", &uuid)], claim_body("1.0.0", &[])),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{next}");
            assignment = next;
        }
    }
    // The first submission's push, then the cool-down's: within one
    // interval and a build, the page has all six games.
    let everything = tokio::time::timeout(std::time::Duration::from_secs(6), async {
        loop {
            let event = next_stats_event(&mut body, &mut buffer).await;
            let parsed: serde_json::Value = serde_json::from_str(&event).unwrap();
            if parsed["games"]["units_completed"] == 6 {
                return parsed;
            }
        }
    })
    .await
    .expect("the submissions made during the cool-down are pushed when it ends");
    assert_eq!(everything["games"]["units_completed"], 6);
}

/// A game-pairs job capturing positions, keeping first divergences or not.
async fn capturing_pairs_job(db: &TestDb, first_divergence: bool) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let p1 = db.static_player(&format!("p1{}", Uuid::new_v4().simple()), admin).await;
    let p2 = db.static_player(&format!("p2{}", Uuid::new_v4().simple()), admin).await;
    let job = db.bare_job("game_pairs", admin).await;
    sqlx::query(
        "INSERT INTO job_game_pair_config
             (job_id, player1_config_id, player2_config_id, pairs_per_batch, min_pairs,
              max_pairs, capture_positions, capture_first_divergence)
         VALUES ($1, $2, $3, 2, 1000000, 1000000, TRUE, $4)",
    )
    .bind(job)
    .bind(p1)
    .bind(p2)
    .bind(first_divergence)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

/// Two pairs, the second diverging: as a pairs result reports them.
fn pairs_result(positions: serde_json::Value) -> serde_json::Value {
    let mut result = games_result(4, 2);
    result["pentanomial"] = json!([0, 0, 2, 0, 0]);
    result["divergent_games"] = games_result(2, 1)["all_games"].clone();
    result["positions"] = positions;
    result
}

fn divergence(game: i32, turn: i32, scores: &str, best: &str) -> serde_json::Value {
    json!({ "game_index": game, "turn_number": turn, "played_move": "8D PLAYED", "played_move_score": 10, "analysis": "static", "rack": "ABBCDEE",
            "position": format!("15/15/15/15/15/15/15/7DAB5/15/15/15/15/15/15/15 {scores} 0"),
            "previous_move": "8H DAB", "previous_move_score": 10,
            "num_moves": 30, "moves": [{ "move": best, "score": 70, "equity": 77.0 }] })
}

/// A-PUBLIC-4f: a game-pairs job that keeps first divergences takes from each
/// diverging pair both games' positions at that one turn and nothing else --
/// a pair's lone position, or two at different turns, is a `400` -- and every
/// saved position of a pairs job comes with its partner, the same turn of the
/// pair's other game, at random and by rack, where the rack finds the pair
/// once.
#[tokio::test]
async fn a_pairs_job_keeps_first_divergences_and_shows_each_with_its_partner() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let cfg = state.cfg.clone();
    let app = birdtest::app(state);
    let job = capturing_pairs_job(&db, true).await;

    let (assignment, uuid) = first_claim(&app).await;
    let request = &assignment["task_request"];
    assert_eq!(request["job_type"], "game_pairs");
    assert_eq!(request["capture_positions"], json!(true));
    assert_eq!(request["capture_first_divergence"], json!(true));

    for (positions, why) in [
        (json!([divergence(2, 5, "ABBCDEE/ 20/10", "8D BEDE")]), "keeps both games' or neither"),
        (
            json!([divergence(2, 5, "a 20/10", "8D BEDE"), divergence(3, 6, "b 10/20", "8D BED")]),
            "turns 5 and 6",
        ),
        (json!([]), "divergent_games says 1 diverged"),
    ] {
        let (status, body) = send(
            &app,
            post_json(
                "/api/worker/result",
                &[("x-worker-uuid", uuid.as_str())],
                json!({ "claim_token": assignment["claim_token"], "movegens": 1000, "result": pairs_result(positions) }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
        assert!(body.to_string().contains(why), "{why}: {body}");
    }
    // Pair 1 diverged at turn 5: player 1 played BEDE in game 2 and player 2
    // BED in game 3, from the same board and tiles.
    submit(
        &app,
        &assignment,
        &uuid,
        pairs_result(json!([
            divergence(3, 5, "ABBCDEE/XYZ 10/20", "8D BED"),
            divergence(2, 5, "XYZ/ABBCDEE 20/10", "8D BEDE"),
        ])),
    )
    .await;

    let user = db.user(&format!("reader{}", Uuid::new_v4().simple()), false).await;
    let headers = admin_headers(&cfg, user);
    let (status, config) = send(&app, get_request(&format!("/api/jobs/{job}/config"), &[])).await;
    assert_eq!(status, StatusCode::OK, "{config}");
    assert_eq!(config["games"]["capture_first_divergence"], json!(true), "{config}");

    let (status, random) =
        send(&app, get_request(&format!("/api/jobs/{job}/positions/random"), &headers)).await;
    assert_eq!(status, StatusCode::OK, "{random}");
    let partner = &random["partner"];
    assert_eq!(random["turn_number"], 5, "{random}");
    assert_eq!(partner["turn_number"], 5, "{random}");
    let mut games = [random["game_index"].as_i64().unwrap(), partner["game_index"].as_i64().unwrap()];
    games.sort();
    assert_eq!(games, [2, 3], "{random}");
    assert!(partner.get("partner").is_none(), "a partner carries no partner of its own: {random}");
    let played = |item: &serde_json::Value| item["moves"][0]["move"].as_str().unwrap().to_string();
    let mut plays = [played(&random), played(partner)];
    plays.sort();
    assert_eq!(plays, ["8D BED", "8D BEDE"], "{random}");

    // Both games hold the rack; the pair is one result, led by its first game.
    let (status, page) = send(
        &app,
        get_request(&format!("/api/jobs/{job}/positions?rack=ABBCDEE"), &headers),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{page}");
    assert_eq!(items[0]["game_index"], 2, "{page}");
    assert_eq!(items[0]["partner"]["game_index"], 3, "{page}");
}

/// A-PUBLIC-4f, continued: a pairs job capturing every position shows each with its
/// partner where the other game has that turn, and `null` where it does not;
/// a games job's positions carry no partner field at all.
#[tokio::test]
async fn a_pairs_position_without_a_partner_turn_says_so() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let cfg = state.cfg.clone();
    let app = birdtest::app(state);
    let job = capturing_pairs_job(&db, false).await;
    let (assignment, uuid) = first_claim(&app).await;
    assert_eq!(assignment["task_request"]["capture_first_divergence"], json!(false));
    // Every game has positions; game 1 has a turn 9 game 0 never reached.
    let mut positions: Vec<serde_json::Value> =
        (0..4).map(|game| divergence(game, 0, "a 0/0", "8D BED")).collect();
    positions.push(divergence(1, 9, "b 0/0", "8D BEDE"));
    submit(&app, &assignment, &uuid, pairs_result(json!(positions))).await;

    let user = db.user(&format!("reader{}", Uuid::new_v4().simple()), false).await;
    let headers = admin_headers(&cfg, user);
    let (_, page) = send(
        &app,
        get_request(&format!("/api/jobs/{job}/positions?rack=ABBCDEE&per_page=20"), &headers),
    )
    .await;
    let items = page["items"].as_array().unwrap();
    // Turn 0 of each pair once, with its partner; game 1's turn 9 alone.
    assert_eq!(items.len(), 3, "{page}");
    let lone = items.iter().find(|i| i["turn_number"] == 9).expect("turn 9");
    assert!(lone["partner"].is_null(), "{lone}");
    for item in items.iter().filter(|i| i["turn_number"] == 0) {
        assert_eq!(item["game_index"].as_i64().unwrap() % 2, 0, "{item}");
        assert_eq!(item["partner"]["game_index"], item["game_index"].as_i64().unwrap() + 1);
    }

    // Out of the way, so the next claim is the games job's.
    sqlx::query("UPDATE jobs SET status = 'inactive', allocation = 0 WHERE id = $1")
        .bind(job)
        .execute(&db.pool)
        .await
        .unwrap();
    let games = capturing_games_job(&db).await;
    let mut result = games_result(2, 1);
    result["positions"] = json!([divergence(0, 0, "a 0/0", "8D BED"), divergence(1, 0, "a 0/0", "8D BED")]);
    let (games_assignment, games_uuid) = first_claim(&app).await;
    assert_eq!(games_assignment["job_id"], games.to_string());
    assert_eq!(games_assignment["task_request"]["capture_first_divergence"], json!(false));
    submit(&app, &games_assignment, &games_uuid, result).await;
    let (_, random) =
        send(&app, get_request(&format!("/api/jobs/{games}/positions/random"), &headers)).await;
    assert!(random.get("partner").is_none(), "{random}");
}

/// A-PUBLIC-8: a NUL in what an unauthenticated caller sends is the caller's
/// `400`, not a `500`. Postgres stores no NUL in text and refuses the
/// statement; that refusal was a 500 and an error line, written at will by
/// anyone with `?worker=%00` on a public, unlimited route.
#[tokio::test]
async fn a_nul_in_a_public_request_is_a_bad_request() {
    let db = TestDb::new().await;
    let job = db.games_job(2).await;
    let app = birdtest::app(db.state().await);

    let (status, body) =
        send(&app, get_request(&format!("/api/jobs/{job}/results?worker=%00"), &[])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "bad_request", "{body}");
    let (status, body) = send(
        &app,
        post_json("/api/auth/login", &[], json!({ "username": "\u{0}", "password": "hunter22" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = send(
        &app,
        post_json("/api/auth/reset-password/request", &[], json!({ "email": "a\u{0}@b.co" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// A-PUBLIC-3d: a results cursor holding a time before 4713 BC, where
/// Postgres's `timestamptz` starts, reads as no cursor -- the first page --
/// like any other cursor this server did not produce. chrono holds such a time
/// and Postgres refused it at the bind (22008): a `500` on a public route. On
/// an opening-rack job and a games job, with and without `?worker=`.
#[tokio::test]
async fn a_cursor_older_than_postgres_can_hold_reads_as_the_first_page() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let alice = db.user("alice", false).await;
    let games = db.games_job(2).await;
    for second in 1..=3 {
        game_result_at(&db, games, Some(alice), None, &format!("2026-02-01T00:00:0{second}Z")).await;
    }
    let racks = opening_rack_job(&db, 3).await;
    let task: Uuid = sqlx::query_scalar(
        "INSERT INTO tasks (job_id, seed, state, accepted_count, completed_at)
         VALUES ($1, 0, 'completed', 1, now()) RETURNING id",
    )
    .bind(racks)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "WITH c AS (
             INSERT INTO task_claims
                 (task_id, job_id, claim_token, state, claimed_by_user_id, completed_at)
             VALUES ($1, $2, gen_random_uuid(), 'completed', $3, now()) RETURNING id
         )
         INSERT INTO position_analysis_records
             (task_claim_id, task_id, job_id, rack, analysis, num_moves)
         SELECT c.id, $1, $2, rack, 'static', 1 FROM c, unnest(ARRAY['AEINRST', 'EIQSTUW']) rack",
    )
    .bind(task)
    .bind(racks)
    .bind(alice)
    .execute(&db.pool)
    .await
    .unwrap();

    // About 29,700 BC, with an id of each feed's kind.
    let ancient = "-1000000000000000000".to_string();
    for (job, id) in [(racks, "1".to_string()), (games, Uuid::new_v4().to_string())] {
        let cursor = birdtest::routes::encode_cursor(&[ancient.clone(), id]);
        for worker in ["", "&worker=alice"] {
            let first = format!("/api/jobs/{job}/results?per_page=2{worker}");
            let (status, expected) = send(&app, get_request(&first, &[])).await;
            assert_eq!(status, StatusCode::OK, "{first}: {expected}");
            assert_eq!(expected["items"].as_array().unwrap().len(), 2, "{first}: {expected}");
            let path = format!("{first}&cursor={cursor}");
            let (status, body) = send(&app, get_request(&path, &[])).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            assert_eq!(body["items"], expected["items"], "{path}: the first page");
        }
    }
}
