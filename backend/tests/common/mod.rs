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

const MIGRATION: &str = include_str!("../../migrations/0001_initial.sql");
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

/// Named after the migration's content, so an edited `0001_initial.sql` gets a
/// fresh template rather than a stale one, and two concurrent `cargo test`
/// processes on the same schema share one.
fn template_name() -> String {
    let digest = hex::encode(Sha256::digest(MIGRATION.as_bytes()));
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
            min_magpie_version: "0.0.1".into(),
            magpie_download_url: "https://example.invalid/magpie".into(),
            magpie_data_repo: "example/data".into(),
            github_token: None,
            trusted_proxy_hops: 0,
        }
    }

    pub async fn state(&self) -> AppState {
        let cfg = Arc::new(self.config());
        AppState {
            pool: self.pool.clone(),
            cfg: cfg.clone(),
            sse: birdtest::sse::SseBroadcaster::new(),
            limits: birdtest::ratelimit::RateLimiters::new(),
            mailer: birdtest::email::Mailer::new(cfg.clone()).await,
            artifacts: birdtest::artifacts::ArtifactStore::new(cfg.clone()).await,
            http: reqwest::Client::new(),
        }
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
            "INSERT INTO player_configs (name, recorder_type, sort_strategy, kwg_id, klv_id, created_by)
             VALUES ($1, 'best', 'equity', $2, $3, $4) RETURNING id",
        )
        .bind(name)
        .bind(kwg)
        .bind(klv)
        .bind(created_by)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    /// An active `games` job at priority 0, with its config row.
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

    /// A `jobs` row and nothing else: active, allocation 50, floor 0.0.1.
    pub async fn bare_job(&self, job_type: &str, redundancy: i32, created_by: Uuid) -> Uuid {
        let ld = self.input_data("letterdist", "english").await;
        let layout = self.input_data("layout", "standard15").await;
        sqlx::query_scalar(
            "INSERT INTO jobs (job_type, priority, allocation, redundancy, status, created_by,
                               variant, letterdist_id, layout_id)
             VALUES ($1::job_type, 0, 50, $2, 'active', $3, 'classic', $4, $5)
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
    let token = birdtest::auth::session::issue(cfg, user_id, "admin", true).unwrap();
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
