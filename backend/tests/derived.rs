//! Derived files (wordmaps and rack info tables) and the pinned-row invariant,
//! driven through `birdtest::derived` and `birdtest::scheduler` directly: what
//! a job queues, how the build queue hands out rows, what a build that cannot
//! start leaves behind, and what a claim states. Nothing here runs MAGPIE: the
//! builds below fail on their inputs before a subprocess would start, which is
//! what the queue tests need and what I-DERIVED-7 is about.

mod common;

use axum::http::StatusCode;
use birdtest::auth::WorkerIdentity;
use birdtest::scheduler::{self, ClaimOutcome, TaskClaim, WorkerCapabilities};
use birdtest::state::AppState;
use birdtest::version::Version;
use common::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A player config on the given lexicon and leaves, asking for a wordmap
/// and/or a rack info table. Plain SQL, like every builder in the harness.
async fn player(db: &TestDb, kwg: Uuid, klv: Uuid, use_wordmap: bool, use_rit: bool) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    sqlx::query_scalar(
        "INSERT INTO player_configs
             (name, recorder_type, sort_strategy, kwg_id, klv_id, num_plies, num_plays,
              num_plies_recorded, num_plays_recorded, use_wordmap, use_rit,
              movegen_margin, created_by)
         VALUES ($1, 'best', 'equity', $2, $3, 0, 100, 2, 10, $4, $5, 5, $6)
         RETURNING id",
    )
    .bind(format!("p{}", Uuid::new_v4().simple()))
    .bind(kwg)
    .bind(klv)
    .bind(use_wordmap)
    .bind(use_rit)
    .bind(admin)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// An active `games` job between two players, pinned to the given
/// distribution and board -- `bare_job` makes fresh ones, and these tests are
/// about rows two jobs share.
async fn games_job_on(db: &TestDb, ld: Uuid, layout: Uuid, p1: Uuid, p2: Uuid) -> Uuid {
    let admin = db.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
    let job: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (job_type, allocation, redundancy, status, created_by,
                           variant, letterdist_id, layout_id, bingo_bonus, sim_cutoff)
         VALUES ('games', 50, 1, 'active', $1, 'classic', $2, $3, 50, 0.005)
         RETURNING id",
    )
    .bind(admin)
    .bind(ld)
    .bind(layout)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO job_game_config
             (job_id, player1_config_id, player2_config_id, games_per_batch, min_games, max_games)
         VALUES ($1, $2, $3, 10, 1000000, 1000000)",
    )
    .bind(job)
    .bind(p1)
    .bind(p2)
    .execute(&db.pool)
    .await
    .unwrap();
    job
}

async fn request_for(db: &TestDb, job: Uuid) -> usize {
    let mut conn = db.pool.acquire().await.unwrap();
    birdtest::derived::request_for_job(&mut conn, job, &test_builders()).await.unwrap()
}

type DerivedRow = (String, String, String, Uuid, Option<Uuid>, Uuid, String);

/// Every `derived_data` row: role, name, builder, kwg, klv, distribution, state.
async fn derived_rows(db: &TestDb) -> Vec<DerivedRow> {
    sqlx::query_as(
        "SELECT role, name, builder, kwg_id, klv_id, letterdist_id, state FROM derived_data
         ORDER BY role, name, letterdist_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

/// Queues a build directly, requested `minutes_ago`.
#[allow(clippy::too_many_arguments)]
async fn queue(
    db: &TestDb,
    role: &str,
    name: &str,
    builder: &str,
    kwg: Uuid,
    klv: Option<Uuid>,
    ld: Uuid,
    minutes_ago: i32,
) {
    sqlx::query(
        "INSERT INTO derived_data (role, name, builder, kwg_id, klv_id, letterdist_id, requested_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() - make_interval(mins => $7))",
    )
    .bind(role)
    .bind(name)
    .bind(builder)
    .bind(kwg)
    .bind(klv)
    .bind(ld)
    .bind(minutes_ago)
    .execute(&db.pool)
    .await
    .unwrap();
}

/// A row as the queue sees it: state, attempts, error, whether it holds a lease.
async fn queued(db: &TestDb, name: &str, builder: &str) -> (String, i32, Option<String>, bool) {
    sqlx::query_as(
        "SELECT state, attempts, error, leased_until IS NOT NULL FROM derived_data
         WHERE name = $1 AND builder = $2",
    )
    .bind(name)
    .bind(builder)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// One pass of the builder task, as `build-derived` runs it. Bounded, so a
/// builder that waited on a lock fails the test rather than hanging it.
async fn build_next(state: &AppState) -> bool {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        birdtest::derived::build_next(&state.pool, &state.artifacts, &state.magpie, &state.builders),
    )
    .await
    .expect("the builder waited instead of skipping")
    .unwrap()
}

// ---------------------------------------------------------------------------
// I-DERIVED
// ---------------------------------------------------------------------------

/// I-DERIVED-1: a wordmap is queued once per (lexicon, distribution), however
/// many players -- in one job or in two -- share it; the same lexicon under a
/// second distribution is a second wordmap; and a player asking for a rack
/// info table queues the table *and* the wordmap it is built from, even
/// without asking for a wordmap to play with.
#[tokio::test]
async fn a_shared_lexicon_queues_one_wordmap_and_a_table_queues_its_wordmap_too() {
    let db = TestDb::new().await;
    let ld = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let nwl = db.input_data("kwg", "NWL23").await;
    let nwl_leaves = db.input_data("klv", "NWL23").await;

    let p1 = player(&db, nwl, nwl_leaves, true, false).await;
    let p2 = player(&db, nwl, nwl_leaves, true, false).await;
    let first = games_job_on(&db, ld, layout, p1, p2).await;
    assert_eq!(request_for(&db, first).await, 1, "two players, one lexicon: one wordmap");
    assert_eq!(request_for(&db, first).await, 0, "and asking again queues nothing");

    let p3 = player(&db, nwl, nwl_leaves, true, false).await;
    let second = games_job_on(&db, ld, layout, p3, p1).await;
    assert_eq!(request_for(&db, second).await, 0, "a second job on the same pair shares the row");
    assert_eq!(
        derived_rows(&db).await,
        vec![("wmp".into(), "NWL23".into(), "wmp-1".into(), nwl, None, ld, "pending".into())]
    );

    // Another distribution: another wordmap.
    let other_ld = db.input_data("letterdist", "english").await;
    let third = games_job_on(&db, other_ld, layout, p1, p2).await;
    assert_eq!(request_for(&db, third).await, 1);

    // A table without a wordmap asked for: both.
    let csw = db.input_data("kwg", "CSW24").await;
    let csw_leaves = db.input_data("klv", "CSW21").await;
    let table_only = player(&db, csw, csw_leaves, false, true).await;
    let neither = player(&db, csw, csw_leaves, false, false).await;
    let fourth = games_job_on(&db, ld, layout, table_only, neither).await;
    assert_eq!(request_for(&db, fourth).await, 2);
    let rows: Vec<DerivedRow> =
        derived_rows(&db).await.into_iter().filter(|row| row.3 == csw).collect();
    assert_eq!(
        rows,
        vec![
            ("rit".into(), "CSW24.CSW21".into(), "rit-1".into(), csw, Some(csw_leaves), ld, "pending".into()),
            ("wmp".into(), "CSW24".into(), "wmp-1".into(), csw, None, ld, "pending".into()),
        ]
    );
    assert_eq!(derived_rows(&db).await.len(), 4);
}

/// I-DERIVED-5, the lease: a row another builder holds a live lease on is not
/// taken, and the same row is taken over once that lease has lapsed -- a
/// builder killed mid-build does not strand it.
#[tokio::test]
async fn a_leased_row_is_left_to_its_builder_until_the_lease_lapses() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let ld = db.input_data("letterdist", "english").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    queue(&db, "wmp", "NWL23", "wmp-1", kwg, None, ld, 10).await;
    sqlx::query(
        "UPDATE derived_data SET state = 'building', attempts = 1,
                                 leased_until = now() + interval '45 minutes'",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    assert!(!build_next(&state).await, "a leased row is someone else's");
    assert_eq!(queued(&db, "NWL23", "wmp-1").await, ("building".into(), 1, None, true));

    sqlx::query("UPDATE derived_data SET leased_until = now() - interval '1 minute'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(build_next(&state).await, "a lapsed lease is taken over");
    let (row_state, attempts, error, _) = queued(&db, "NWL23", "wmp-1").await;
    assert_eq!((row_state.as_str(), attempts), ("pending", 2), "{error:?}");
}

/// I-DERIVED-5, `SKIP LOCKED`: a builder that finds the oldest row locked by
/// another builder in the middle of taking it takes the next row instead of
/// waiting; with every row either locked or leased it takes nothing; and the
/// locked row is taken once the other builder lets go.
#[tokio::test]
async fn a_row_another_builder_is_taking_is_skipped_not_shared() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let ld = db.input_data("letterdist", "english").await;
    let older = db.input_data("kwg", "NWL23").await;
    let newer = db.input_data("kwg", "CSW24").await;
    queue(&db, "wmp", "NWL23", "wmp-1", older, None, ld, 10).await;
    queue(&db, "wmp", "CSW24", "wmp-1", newer, None, ld, 5).await;

    // The other builder, between its SELECT ... FOR UPDATE and its commit.
    let mut other = db.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM derived_data WHERE name = 'NWL23' FOR UPDATE")
        .execute(&mut *other)
        .await
        .unwrap();

    assert!(build_next(&state).await);
    assert_eq!(queued(&db, "NWL23", "wmp-1").await.1, 0, "the locked row was not taken");
    assert_eq!(queued(&db, "CSW24", "wmp-1").await.1, 1, "the next row was");

    // Now the other builder is building CSW24 as well as taking NWL23.
    sqlx::query(
        "UPDATE derived_data SET state = 'building', leased_until = now() + interval '45 minutes'
         WHERE name = 'CSW24'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(!build_next(&state).await, "nothing is free");
    assert_eq!(queued(&db, "NWL23", "wmp-1").await.1, 0);
    assert_eq!(queued(&db, "CSW24", "wmp-1").await.1, 1);

    other.rollback().await.unwrap();
    assert!(build_next(&state).await);
    assert_eq!(queued(&db, "NWL23", "wmp-1").await.1, 1, "taken once released");
    let (row_state, attempts, _, leased) = queued(&db, "CSW24", "wmp-1").await;
    assert_eq!((row_state.as_str(), attempts, leased), ("building", 1, true), "still its builder's");
}

/// I-DERIVED-6: a row queued under a builder version this binary does not have
/// -- queued before a MAGPIE upgrade, say -- is left alone: not taken, not
/// failed, no attempt counted. And it does not hold up the rows behind it:
/// taken as the head of the queue it answered "nothing to build", and every
/// row requested after it waited for a build this binary will never do.
#[tokio::test]
async fn a_row_for_a_builder_this_binary_lacks_is_left_alone_and_blocks_nothing() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let ld = db.input_data("letterdist", "english").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "CSW21").await;
    // Older than ours, so they sit at the head of the queue.
    queue(&db, "wmp", "NWL23", "wmp-2", kwg, None, ld, 60).await;
    queue(&db, "rit", "NWL23.CSW21", "rit-2", kwg, Some(klv), ld, 60).await;
    queue(&db, "wmp", "NWL23", "wmp-1", kwg, None, ld, 1).await;

    let mut builds = 0;
    while build_next(&state).await {
        builds += 1;
        assert!(builds <= 10, "the queue never drained");
    }
    // Ours was taken, failed on its inputs and was given up on; the others
    // were never touched.
    assert_eq!(builds, 3);
    assert_eq!(queued(&db, "NWL23", "wmp-1").await.0, "failed");
    assert_eq!(queued(&db, "NWL23", "wmp-2").await, ("pending".into(), 0, None, false));
    assert_eq!(queued(&db, "NWL23.CSW21", "rit-2").await, ("pending".into(), 0, None, false));
}

/// I-DERIVED-7: a lexicon imported before the server kept lexicon bytes has
/// neither `content` nor `object_key`, and a build from it fails with a message
/// that names the file and the remedy -- re-import the tarball -- before any
/// MAGPIE runs. The row is retried the bounded number of times and then left
/// `failed` for an admin, not retried forever.
#[tokio::test]
async fn a_build_from_a_lexicon_stored_before_object_keys_fails_naming_the_remedy() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let ld = db.input_data("letterdist", "english").await;
    let kwg = db.input_data("kwg", "NWL20").await;
    let (content, key): (Option<Vec<u8>>, Option<String>) =
        sqlx::query_as("SELECT content, object_key FROM input_data WHERE id = $1")
            .bind(kwg)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((content, key), (None, None), "a row from before object_key");
    queue(&db, "wmp", "NWL20", "wmp-1", kwg, None, ld, 1).await;

    assert!(build_next(&state).await);
    let (row_state, attempts, error, leased) = queued(&db, "NWL20", "wmp-1").await;
    let error = error.expect("the failure is recorded on the row");
    assert!(error.contains("lexica/NWL20.kwg"), "names the file: {error}");
    assert!(error.contains("Re-import that tarball"), "names the remedy: {error}");
    assert_eq!((row_state.as_str(), attempts, leased), ("pending", 1, false));

    let mut builds = 1;
    while build_next(&state).await {
        builds += 1;
        assert!(builds <= 10, "retried without end");
    }
    assert_eq!(builds, 3, "a bounded number of attempts");
    let (row_state, attempts, error, leased) = queued(&db, "NWL20", "wmp-1").await;
    assert_eq!((row_state.as_str(), attempts, leased), ("failed", 3, false));
    assert!(error.unwrap().contains("Re-import that tarball"));
    assert!(!build_next(&state).await, "a failed row stays failed");
}

async fn built(db: &TestDb, role: &str, name: &str, kwg: Uuid, klv: Option<Uuid>, ld: Uuid) -> String {
    let sha = hex::encode(Sha256::digest(format!("{role}/{name}/{klv:?}").as_bytes()));
    sqlx::query(
        "INSERT INTO derived_data
             (role, name, builder, kwg_id, klv_id, letterdist_id,
              state, sha256, bytes, build_target, built_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'built', $7, 1, 'nehalem', now())",
    )
    .bind(role)
    .bind(name)
    .bind(format!("{role}-1"))
    .bind(kwg)
    .bind(klv)
    .bind(ld)
    .bind(&sha)
    .execute(&db.pool)
    .await
    .unwrap();
    sha
}

async fn anon(db: &TestDb) -> WorkerIdentity {
    let uuid = Uuid::new_v4();
    sqlx::query("INSERT INTO anonymous_workers (uuid) VALUES ($1)")
        .bind(uuid)
        .execute(&db.pool)
        .await
        .unwrap();
    WorkerIdentity::Anonymous { uuid }
}

async fn claim_from(db: &TestDb, state: &AppState, not: Uuid) -> TaskClaim {
    let caps = WorkerCapabilities {
        magpie_version: Version::new(1, 0, 0),
        unsupported_jobs: vec![not],
    };
    match scheduler::claim(state, &anon(db).await, &caps).await.unwrap() {
        ClaimOutcome::Task(task) => *task,
        _ => panic!("expected a task"),
    }
}

/// I-DERIVED-8: two jobs on one lexicon and one distribution, with different
/// leaves, each get a claim naming its own table -- `<lexicon>.<leaves>` -- with
/// its own hash, and the one wordmap they share.
#[tokio::test]
async fn two_jobs_on_one_lexicon_with_different_leaves_get_their_own_table() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let admin = db.user("admin", true).await;
    let ld = db.input_data("letterdist", "english").await;
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let csw_leaves = db.input_data("klv", "CSW21").await;
    let nwl_leaves = db.input_data("klv", "NWL23").await;

    let with_csw = player(&db, kwg, csw_leaves, true, true).await;
    let with_nwl = player(&db, kwg, nwl_leaves, true, true).await;
    let csw_job = games_job_on(&db, ld, layout, with_csw, db.static_player("a", admin).await).await;
    let nwl_job = games_job_on(&db, ld, layout, with_nwl, db.static_player("b", admin).await).await;

    let wordmap = built(&db, "wmp", "NWL23", kwg, None, ld).await;
    let csw_table = built(&db, "rit", "NWL23.CSW21", kwg, Some(csw_leaves), ld).await;
    let nwl_table = built(&db, "rit", "NWL23.NWL23", kwg, Some(nwl_leaves), ld).await;
    assert_ne!(csw_table, nwl_table);

    let stated = |task: &TaskClaim| -> Vec<(String, String, String)> {
        task.derived_data
            .iter()
            .map(|d| (d.role.clone(), d.name.clone(), d.sha256.clone()))
            .collect()
    };
    let csw_claim = claim_from(&db, &state, nwl_job).await;
    assert_eq!(csw_claim.job_id, csw_job);
    assert_eq!(
        stated(&csw_claim),
        vec![
            ("rit".into(), "NWL23.CSW21".into(), csw_table),
            ("wmp".into(), "NWL23".into(), wordmap.clone()),
        ]
    );
    let nwl_claim = claim_from(&db, &state, csw_job).await;
    assert_eq!(nwl_claim.job_id, nwl_job);
    assert_eq!(
        stated(&nwl_claim),
        vec![
            ("rit".into(), "NWL23.NWL23".into(), nwl_table),
            ("wmp".into(), "NWL23".into(), wordmap),
        ]
    );
}

// ---------------------------------------------------------------------------
// I-DATA
// ---------------------------------------------------------------------------

/// I-DATA-1: the server reads the distribution a job pins, by row, not by name.
/// Two `english` distributions with different bytes -- one with a fourth A --
/// size two otherwise identical opening-rack jobs' rack spaces differently, and
/// each as its own bytes say. Through the admin route, because that is where
/// `total_racks` is computed and written.
#[tokio::test]
async fn two_distributions_with_one_name_size_two_different_rack_spaces() {
    let db = TestDb::new().await;
    let state = db.state().await;
    let app = birdtest::app(state.clone());
    let admin = db.user("root", true).await;
    let headers = admin_headers(&state.cfg, admin);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let standard = include_bytes!("../src/jobs/testdata/testdist.csv").to_vec();
    let text = String::from_utf8(standard.clone()).unwrap();
    assert!(text.contains("A,a,3,"));
    let more_as = text.replacen("A,a,3,", "A,a,4,", 1).into_bytes();
    let mut distributions = Vec::new();
    for bytes in [standard, more_as] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
             VALUES ('letterdistributions/english.csv', 'letterdist', 'english', $1, $2,
                     '20251004', $3)
             RETURNING id",
        )
        .bind(hex::encode(Sha256::digest(&bytes)))
        .bind(bytes.len() as i64)
        .bind(&bytes)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        distributions.push((id, bytes));
    }
    let layout = db.input_data("layout", "standard15").await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let analyst: Uuid = sqlx::query_scalar(
        "INSERT INTO player_configs
             (name, recorder_type, sort_strategy, kwg_id, klv_id, num_plies, num_plays,
              num_plies_recorded, num_plays_recorded, use_wordmap, use_rit,
              movegen_margin, created_by)
         VALUES ('analyst', 'all', 'equity', $1, $2, 0, 100, 1, 10, false, false, 5, $3)
         RETURNING id",
    )
    .bind(kwg)
    .bind(klv)
    .bind(admin)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let mut totals = Vec::new();
    for (ld, bytes) in &distributions {
        let (status, body) = send(
            &app,
            post_json(
                "/api/admin/jobs",
                &headers,
                json!({
                    "job_type": "opening_rack", "variant": "classic",
                    "letterdist_id": ld, "layout_id": layout,
                    "player_config_id": analyst, "racks_per_batch": 10, "rack_size": 7,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let job: Uuid = body["job"]["id"].as_str().expect("a job id").parse().unwrap();
        let total: i64 =
            sqlx::query_scalar("SELECT total_racks FROM job_opening_rack_config WHERE job_id = $1")
                .bind(job)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let own = birdtest::jobs::racks::LetterDistribution::parse(bytes, "english").unwrap();
        assert_eq!(total, birdtest::jobs::opening_rack::total_racks(&own, 7), "sized by its own row");
        totals.push(total);
    }
    assert_ne!(totals[0], totals[1], "{totals:?}");
}
