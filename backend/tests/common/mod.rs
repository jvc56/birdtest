//! The tier-2/3 harness TESTING.md specifies: a real Postgres, one database per
//! test cloned from a pre-migrated template, and builders that write SQL
//! directly and validate nothing.
//!
//! `TEST_DATABASE_URL` must name a database on a server where the test role
//! may create databases, e.g.
//! `postgres://birdtest:birdtest@localhost:5433/birdtest` against the compose
//! stack. A test that cannot find it fails; it does not skip.

#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use birdtest::config::{Config, MailBackend};
use birdtest::state::AppState;
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, PgPool};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

/// The migrations, used only to *name* the template database. The template
/// itself is built by `db::migrate`, which applies them properly; this is what
/// makes an edit to any of them produce a fresh template rather than a stale
/// one.
const MIGRATIONS: [&str; 1] = [include_str!("../../migrations/0001_initial.sql")];
const TESTDIST: &[u8] = include_bytes!("../../src/jobs/testdata/testdist.csv");

fn server_url() -> String {
    std::env::var("TEST_DATABASE_URL").expect(
        "TEST_DATABASE_URL is required for integration tests (see TESTING.md, tier 2), e.g. \
         postgres://birdtest:birdtest@localhost:5433/birdtest",
    )
}

/// `url` with its database path replaced by `database`.
fn with_database(url: &str, database: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, format!("?{query}")),
        None => (url, String::new()),
    };
    let authority_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let path_start = base[authority_end..]
        .find('/')
        .map(|i| i + authority_end)
        .expect("TEST_DATABASE_URL must name a database, e.g. .../birdtest");
    format!("{}/{database}{query}", &base[..path_start])
}

/// Named after the migrations' content, so an edited migration gets a fresh
/// template rather than a stale one, and two concurrent `cargo test` processes
/// on the same schema share one.
fn template_name() -> String {
    let mut hasher = Sha256::new();
    for migration in MIGRATIONS {
        hasher.update(migration.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    format!("birdtest_tpl_{}", &digest[..16])
}

async fn ensure_template() {
    static READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    READY
        .get_or_init(|| async {
            let url = server_url();
            let name = template_name();
            let mut admin = PgConnection::connect(&url).await.expect("connect TEST_DATABASE_URL");
            // Serializes template creation across processes as well as threads.
            sqlx::query("SELECT pg_advisory_lock(815_274_001)")
                .execute(&mut admin)
                .await
                .unwrap();
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
                    .bind(&name)
                    .fetch_one(&mut admin)
                    .await
                    .unwrap();
            if !exists {
                let building = format!("{name}_building");
                sqlx::query(&format!("DROP DATABASE IF EXISTS {building} WITH (FORCE)"))
                    .execute(&mut admin)
                    .await
                    .unwrap();
                sqlx::query(&format!("CREATE DATABASE {building}"))
                    .execute(&mut admin)
                    .await
                    .unwrap();
                {
                    // Nothing may hold a connection to a template while it is
                    // cloned, so this pool is closed before the rename.
                    let pool = PgPool::connect(&with_database(&url, &building)).await.unwrap();
                    birdtest::db::migrate(&pool).await.expect("migrate template");
                    pool.close().await;
                }
                sqlx::query(&format!("ALTER DATABASE {building} RENAME TO {name}"))
                    .execute(&mut admin)
                    .await
                    .unwrap();
            }
            sqlx::query("SELECT pg_advisory_unlock(815_274_001)")
                .execute(&mut admin)
                .await
                .unwrap();
            admin.close().await.ok();
        })
        .await;
}

/// The builder versions a test's `AppState` reports, without running MAGPIE.
///
/// The real server reads these out of the binary at startup, which is right
/// there and wrong here: a tier-2 or tier-3 test asserts what the server does
/// with a builder identity, not that a subprocess can be spawned. A test that
/// actually needs a build belongs in tier 6, where a real MAGPIE is a
/// precondition rather than an accident of the machine.
///
/// The versions are the ones `src/def/builder_defs.h` ships, so a fixture that
/// names `wmp-1` stays honest until someone bumps them -- at which point these
/// and the contract fixtures move together.
pub fn test_builders() -> birdtest::magpie::Builders {
    birdtest::magpie::Builders {
        magpie_version: "0.1.0".into(),
        build_target: "nehalem".into(),
        wmp_builder_version: 1,
        rit_builder_version: 1,
        klv_builder_version: 1,
    }
}

pub struct TestDb {
    pub pool: PgPool,
    pub url: String,
    name: String,
}

impl TestDb {
    pub async fn new() -> Self {
        ensure_template().await;
        let name = format!("birdtest_test_{}", Uuid::new_v4().simple());
        let server = server_url();
        let mut admin = PgConnection::connect(&server).await.unwrap();
        // Retried: Postgres refuses a clone while another session is briefly
        // attached to the template, which a concurrent clone can be.
        let mut attempts = 0;
        loop {
            match sqlx::query(&format!("CREATE DATABASE {name} TEMPLATE {}", template_name()))
                .execute(&mut admin)
                .await
            {
                Ok(_) => break,
                Err(err) if attempts < 20 => {
                    attempts += 1;
                    let _ = err;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(err) => panic!("could not clone the test template: {err}"),
            }
        }
        admin.close().await.ok();
        let url = with_database(&server, &name);
        let pool = PgPool::connect(&url).await.unwrap();
        TestDb { pool, url, name }
    }

    pub fn config(&self) -> Config {
        Config {
            database_url: self.url.clone(),
            bind_addr: "127.0.0.1:0".into(),
            session_signing_key: [7u8; 32],
            session_ttl: Duration::from_secs(3600),
            secure_cookies: false,
            mail_backend: MailBackend::Console,
            mail_from: "test@birdtest.local".into(),
            public_url: "http://localhost".into(),
            heartbeat_timeout: Duration::from_secs(300),
            s3_bucket: "birdtest-test".into(),
            // Nothing in these tests touches the object store; an unroutable
            // endpoint makes an accidental call fail fast rather than reach AWS.
            s3_endpoint: Some("http://127.0.0.1:9".into()),
            min_magpie_version: "0.1.0".into(),
            magpie_download_url: "https://example.invalid/magpie".into(),
            // A path that is not a binary. Nothing below tier 6 runs a
            // conversion, and a test that reached one should fail loudly
            // rather than pick up whatever MAGPIE happens to be installed on
            // the machine -- which is how a test starts depending on a build
            // nobody chose.
            magpie_bin: "/nonexistent/magpie".into(),
            magpie_threads: 1,
            magpie_data_repo: "example/data".into(),
            github_token: None,
            trusted_proxy_hops: 0,
        }
    }

    pub async fn state(&self) -> AppState {
        // Credentials are resolved lazily, on the first S3 call. Without these
        // the SDK walks the whole default chain -- including the EC2 instance
        // metadata endpoint, which is unroutable here and takes about a minute
        // to give up on -- so a test that touches the object store at all hung
        // for that long before failing. The endpoint above is closed, so these
        // are never used for anything.
        for (key, value) in [
            ("AWS_ACCESS_KEY_ID", "test"),
            ("AWS_SECRET_ACCESS_KEY", "test"),
            ("AWS_REGION", "us-east-1"),
            ("AWS_EC2_METADATA_DISABLED", "true"),
        ] {
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, value);
            }
        }
        let cfg = Arc::new(self.config());
        AppState {
            result_streams: std::sync::Arc::new(tokio::sync::Semaphore::new(
                birdtest::state::MAX_CONCURRENT_RESULT_STREAMS,
            )),
            pool: self.pool.clone(),
            // One pool in tests: a test reads what it just wrote, and the
            // separation is about connection budgets, not visibility.
            read_pool: self.pool.clone(),
            magpie: birdtest::magpie::Magpie::new(&cfg.magpie_bin, 1),
            builders: Arc::new(test_builders()),
            cfg: cfg.clone(),
            sse: birdtest::sse::SseBroadcaster::new(),
            finish_checks: Default::default(),
            derived_ready: Default::default(),
            templates: Default::default(),
            limits: birdtest::ratelimit::RateLimiters::new(),
            mailer: birdtest::email::Mailer::new(cfg.clone()).await,
            artifacts: birdtest::artifacts::ArtifactStore::new(cfg.clone()).await,
            http: reqwest::Client::new(),
        }
    }

    /// Marks every wordmap and rack info table this job needs as built, with a
    /// made-up hash.
    ///
    /// A job whose derived files are not built is not dispatched, which is the
    /// whole point of `derived_data` — so a test that builds a job by hand has
    /// to satisfy that gate by hand too, exactly as it inserts the
    /// generation-0 artifact row by hand. The hash is arbitrary because
    /// nothing below tier 6 reproduces one: what these tests exercise is the
    /// gate and what the claim carries, not the build.
    ///
    /// Returns how many rows it wrote, so a test can assert a job needed what
    /// it expected to need.
    pub async fn derived_ready(&self, job: Uuid) -> usize {
        let builders = test_builders();
        let mut conn = self.pool.acquire().await.unwrap();
        let needs = birdtest::derived::needs_for_job(&mut conn, job).await.unwrap();
        for (i, need) in needs.iter().enumerate() {
            sqlx::query(
                "INSERT INTO derived_data
                     (role, name, builder, kwg_id, klv_id, letterdist_id,
                      state, sha256, bytes, build_target, built_at)
                 VALUES ($1,$2,$3,$4,$5,$6,'built',$7,1,$8,now())
                 ON CONFLICT DO NOTHING",
            )
            .bind(&need.role)
            .bind(&need.name)
            .bind(builders.for_role(&need.role).unwrap())
            .bind(need.kwg_id)
            .bind(need.klv_id)
            .bind(need.letterdist_id)
            .bind(format!("{i:064x}"))
            .bind(&builders.build_target)
            .execute(&mut *conn)
            .await
            .unwrap();
        }
        needs.len()
    }

    // --- builders: plain SQL, no validation --------------------------------

    pub async fn user(&self, username: &str, is_admin: bool) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO users (username, email, password_hash, email_confirmed_at, is_admin)
             VALUES ($1, $1 || '@example.invalid', 'x', now(), $2) RETURNING id",
        )
        .bind(username)
        .bind(is_admin)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    pub async fn input_data(&self, role: &str, name: &str) -> Uuid {
        let (path, content): (String, Option<&[u8]>) = match role {
            "letterdist" => (format!("letterdistributions/{name}.csv"), Some(TESTDIST)),
            "layout" => (format!("layouts/{name}.txt"), Some(b"layout".as_slice())),
            "kwg" => (format!("lexica/{name}.kwg"), None),
            "klv" => (format!("lexica/{name}.klv2"), None),
            _ => (format!("strategy/{name}.csv"), None),
        };
        sqlx::query_scalar(
            "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
             VALUES ($1, $2, $3, $4, 1, '20251004', $5) RETURNING id",
        )
        .bind(path)
        .bind(role)
        .bind(name)
        .bind(hex::encode(Sha256::digest(Uuid::new_v4().as_bytes())))
        .bind(content)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    pub async fn static_player(&self, name: &str, created_by: Uuid) -> Uuid {
        let kwg = self.input_data("kwg", &format!("NWL{name}")).await;
        let klv = self.input_data("klv", &format!("NWL{name}")).await;
        sqlx::query_scalar(
            "INSERT INTO player_configs
                 (name, recorder_type, sort_strategy, kwg_id, klv_id, num_plies, num_plays,
                  num_plies_recorded, num_plays_recorded, use_wordmap, use_rit,
                  movegen_margin, created_by)
             VALUES ($1, 'best', 'equity', $2, $3, 0, 100, 2, 10, false, false, 5, $4)
             RETURNING id",
        )
        .bind(name)
        .bind(kwg)
        .bind(klv)
        .bind(created_by)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    /// An active `games` job at 50% allocation, with its config row.
    pub async fn games_job(&self, redundancy: i32, games_per_batch: i32) -> Uuid {
        let admin = self.user(&format!("admin{}", Uuid::new_v4().simple()), true).await;
        let p1 = self.static_player(&format!("p1{}", Uuid::new_v4().simple()), admin).await;
        let p2 = self.static_player(&format!("p2{}", Uuid::new_v4().simple()), admin).await;
        let job = self.bare_job("games", redundancy, admin).await;
        sqlx::query(
            "INSERT INTO job_game_config
                 (job_id, player1_config_id, player2_config_id, games_per_batch, min_games, max_games)
             VALUES ($1, $2, $3, $4, 1000000, 1000000)",
        )
        .bind(job)
        .bind(p1)
        .bind(p2)
        .bind(games_per_batch)
        .execute(&self.pool)
        .await
        .unwrap();
        job
    }

    /// A `jobs` row and nothing else: active, allocation 50, floor 0.1.0.
    pub async fn bare_job(&self, job_type: &str, redundancy: i32, created_by: Uuid) -> Uuid {
        let ld = self.input_data("letterdist", "english").await;
        let layout = self.input_data("layout", "standard15").await;
        sqlx::query_scalar(
            "INSERT INTO jobs (job_type, allocation, redundancy, status, created_by,
                               variant, letterdist_id, layout_id, bingo_bonus, sim_cutoff)
             VALUES ($1::job_type, 50, $2, 'active', $3, 'classic', $4, $5, 50, 0.005)
             RETURNING id",
        )
        .bind(job_type)
        .bind(redundancy)
        .bind(created_by)
        .bind(ld)
        .bind(layout)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let name = self.name.clone();
        let server = server_url();
        // Drop cannot await, and the test's runtime may already be gone, so
        // the database is dropped from a thread with its own runtime. FORCE
        // ends the pool's connections rather than waiting on them.
        let _ = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            if let Ok(runtime) = runtime {
                runtime.block_on(async {
                    if let Ok(mut admin) = PgConnection::connect(&server).await {
                        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
                            .execute(&mut admin)
                            .await;
                    }
                });
            }
        })
        .join();
    }
}

/// Sends `request` through `app` and returns the status and JSON body (Null
/// for an empty body).
pub async fn send(app: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024).await.unwrap();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::String(
            String::from_utf8_lossy(&bytes).into_owned(),
        ))
    };
    (status, body)
}

pub fn post_json(path: &str, headers: &[(&str, &str)], body: serde_json::Value) -> Request<Body> {
    let mut builder = Request::post(path).header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

/// Headers for an admin session: the session cookie plus the CSRF double-submit
/// pair.
pub fn admin_headers(cfg: &Config, user_id: Uuid) -> Vec<(String, String)> {
    let token = birdtest::auth::session::issue(cfg, user_id, "admin", true, 0).unwrap();
    vec![
        ("cookie".into(), format!("birdtest_session={token}; birdtest_csrf=testcsrf")),
        ("x-csrf-token".into(), "testcsrf".into()),
    ]
}

pub fn claim_body(version: &str, unsupported: &[Uuid]) -> serde_json::Value {
    serde_json::json!({ "magpie_version": version, "unsupported_jobs": unsupported })
}

pub fn games_result(games: i32, wins: i32) -> serde_json::Value {
    serde_json::json!({
        "all_games": {
            "games": games, "wins": wins, "losses": games - wins, "ties": 0,
            "p1_score_mean": 420.0, "p1_score_sd": 60.0,
            "p2_score_mean": 410.0, "p2_score_sd": 58.0
        }
    })
}

pub fn get_request(path: &str, headers: &[(String, String)]) -> Request<Body> {
    let mut builder = Request::get(path);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder.body(Body::empty()).unwrap()
}
