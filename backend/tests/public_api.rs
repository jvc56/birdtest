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
            json!({ "claim_token": assignment["claim_token"], "result": result }),
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
             (task_id, claim_token, state, claimed_by_user_id, claimed_by_anon_uuid, completed_at)
         VALUES ($1, gen_random_uuid(), 'completed', $2, $3, now()) RETURNING id",
    )
    .bind(task)
    .bind(user)
    .bind(anon)
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

async fn anon_worker(db: &TestDb, tasks_completed: i64) -> Uuid {
    let uuid = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO anonymous_workers (uuid, tasks_completed, last_completed_at)
         VALUES ($1, $2, CASE WHEN $2 > 0 THEN now() END)",
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
    let job = db.bare_job("opening_rack", 1, admin).await;
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

/// A-PUBLIC-1b: `?status=` filters the job list and its total by status --
/// the home page's active jobs, which it once filtered from the newest page
/// in the browser and so lost every active job older than it.
#[tokio::test]
async fn the_job_list_filters_by_status() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;
    let active = db.bare_job("games", 1, admin).await;
    let inactive = db.bare_job("games", 1, admin).await;
    sqlx::query("UPDATE jobs SET status = 'inactive' WHERE id = $1")
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
    db.games_job(1, 2).await;
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
            json!({ "claim_token": claim["claim_token"], "result": games_result(2, 1) }),
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
        let job = db.bare_job("games", 1, admin).await;
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
/// other -- games and pairs the SPRT block in their own unit, an opening-rack
/// job its rack progress, a leave job its generation -- and an unknown job is
/// a 404, for the detail and the stream alike.
#[tokio::test]
async fn job_detail_carries_the_stats_block_of_its_type() {
    let db = TestDb::new().await;
    let app = birdtest::app(db.state().await);
    let admin = db.user("root", true).await;

    let games = db.games_job(1, 2).await;
    let pairs = db.bare_job("game_pairs", 1, admin).await;
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
    let leave = db.bare_job("leave_generation", 1, admin).await;
    let kwg = db.input_data("kwg", "CSW24").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, generation_count, target_rack_count, racks_per_task)
         VALUES ($1, $2, 100, 3, 1000, 50)",
    )
    .bind(leave)
    .bind(kwg)
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
    let (_, racks) = send(&app, get_request(&format!("/api/jobs/{racks}"), &[])).await;
    assert_eq!(racks["opening_racks"], json!({ "racks_analyzed": 0, "racks_total": 100 }));
    let (_, leave) = send(&app, get_request(&format!("/api/jobs/{leave}"), &[])).await;
    assert_eq!(leave["leave_generation"]["current_generation"], 1);
    assert_eq!(leave["leave_generation"]["generation_count"], 3);
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
    let job = db.games_job(1, 2).await;
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
            { "rank": 1, "move": "8G WUZ", "score": 30, "equity": 32.5 },
            { "rank": 2, "move": "8H ZA", "score": 22, "equity": 21.0 },
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
    let job = db.games_job(1, 2).await;
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
    let job = db.games_job(1, 2).await;
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
            "UPDATE users SET tasks_completed = $2, last_completed_at = now(),
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
                let (status, body) =
                    send(&app, get_request(&format!("{path}?page={page}&per_page=2"), &[])).await;
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
        pages("/api/workers", 3).await,
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
        sqlx::query("UPDATE users SET tasks_completed = 1 WHERE id = $1")
            .bind(id)
            .execute(&db.pool)
            .await
            .unwrap();
        expected.push(format!("user{i}"));
        expected.push(birdtest::auth::public_anon_id(anon_worker(&db, 1).await));
    }
    expected.sort();

    for per_page in [2, 3, 4, 5, 7] {
        let mut seen = Vec::new();
        for page in 0..=(18 / per_page) {
            let path = format!("/api/workers?page={page}&per_page={per_page}");
            let (status, body) = send(&app, get_request(&path, &[])).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["total"], 18);
            for item in body["items"].as_array().unwrap() {
                let name = item["username"].as_str().or(item["anon_id"].as_str()).unwrap();
                seen.push(name.to_string());
            }
        }
        seen.sort();
        assert_eq!(seen, expected, "per_page={per_page}");
    }
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
        jobs.push(db.bare_job("games", 1, admin).await.to_string());
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
