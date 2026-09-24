//! Input data import and the `input_data` vocabulary against a real database
//! and object store (`I-INPUT-*`, `A-ADMIN-5..7`).
//!
//! GitHub is played by a small HTTP server in the test, on the hosts
//! `Config.github_api_url` / `github_raw_url` point at: it resolves a ref to a
//! commit and serves a tarball built here, whole or in `split`-style chunks,
//! and can hold a chunk back so an import can be watched while it runs.

mod common;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use birdtest::state::AppState;
use common::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const DATE: &str = "20250101";
const TESTDIST: &[u8] = include_bytes!("../src/jobs/testdata/testdist.csv");
const STANDARD15: &[u8] = include_bytes!("../src/magpie_standard15.txt");

/// Bytes that do not compress, so the fixture archive is nowhere near the
/// walk's expansion-ratio cap.
fn noise(label: &str, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut block = Sha256::digest(label.as_bytes()).to_vec();
    while out.len() < len {
        out.extend_from_slice(&block);
        block = Sha256::digest(&block).to_vec();
    }
    out.truncate(len);
    out
}

/// The files the fixture tarball carries, by their `data/` path.
fn fixture_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("data/letterdistributions/english.csv", TESTDIST.to_vec()),
        ("data/layouts/standard15.txt", STANDARD15.to_vec()),
        ("data/lexica/NWL23.kwg", noise("kwg", 2000)),
        ("data/lexica/NWL23.klv2", noise("klv", 1500)),
        ("data/strategy/winpct.csv", noise("winpct", 300)),
        // Not something birdtest pins: skipped by the walk.
        ("data/quackle/ignored.dat", noise("ignored", 100)),
    ]
}

/// The file at `path` in the fixture, as `input_data` would name it
/// (`lexica/NWL23.kwg`).
fn fixture_file(path: &str) -> Vec<u8> {
    fixture_files().into_iter().find(|(p, _)| p.strip_prefix("data/") == Some(path)).unwrap().1
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A gzipped tar of `entries`.
fn tarball(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append_data(&mut header, path, bytes.as_slice()).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, &tar).unwrap();
    encoder.finish().unwrap()
}

/// What the fake GitHub serves: bodies by request path, and at most one path
/// held until `release` is called.
struct Fixture {
    files: Mutex<HashMap<String, Vec<u8>>>,
    held: Mutex<Option<String>>,
    gate: tokio::sync::Semaphore,
    requests: Mutex<Vec<String>>,
}

impl Fixture {
    fn new() -> Arc<Self> {
        let fixture = Arc::new(Fixture {
            files: Mutex::new(HashMap::new()),
            held: Mutex::new(None),
            gate: tokio::sync::Semaphore::new(0),
            requests: Mutex::new(Vec::new()),
        });
        fixture.put("/repos/example/data/commits/main", COMMIT.as_bytes().to_vec());
        fixture
    }

    fn put(&self, path: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(path.to_string(), bytes);
    }

    /// The tarball's URL path, plus `.<suffix>` for a chunk.
    fn tarball_path(suffix: Option<&str>) -> String {
        let base = format!("/example/data/{COMMIT}/versioned-tarballs/data-{DATE}.tgz");
        match suffix {
            Some(suffix) => format!("{base}.{suffix}"),
            None => base,
        }
    }

    /// Serves `archive` unchunked, at the name the chunk probe falls back to.
    fn serve_whole(&self, archive: Vec<u8>) {
        self.put(&Self::tarball_path(None), archive);
    }

    fn hold(&self, path: String) {
        *self.held.lock().unwrap() = Some(path);
    }

    fn release(&self) {
        self.gate.add_permits(1);
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

async fn serve(State(fixture): State<Arc<Fixture>>, uri: Uri) -> Response {
    let path = uri.path().to_string();
    fixture.requests.lock().unwrap().push(path.clone());
    let held = fixture.held.lock().unwrap().as_deref() == Some(path.as_str());
    if held {
        fixture.gate.acquire().await.unwrap().forget();
    }
    let body = fixture.files.lock().unwrap().get(&path).cloned();
    match body {
        Some(bytes) => (StatusCode::OK, bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Starts the fake GitHub on a free port, for the life of the test's runtime,
/// and returns its base URL.
async fn fake_github(fixture: Arc<Fixture>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(serve).with_state(fixture);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A state whose GitHub is `base` and whose object store is a real, fresh
/// bucket.
async fn import_state(db: &TestDb, base: &str) -> (AppState, TestBucket) {
    let (with_store, bucket) = db.state_with_object_store().await;
    let mut cfg = (*with_store.cfg).clone();
    cfg.github_api_url = base.to_string();
    cfg.github_raw_url = base.to_string();
    (db.state_with(cfg).await, bucket)
}

/// A state whose GitHub is `base`, with the closed object store: for an
/// import that must fail before it would upload anything.
async fn import_state_without_store(db: &TestDb, base: &str) -> AppState {
    let mut cfg = db.config();
    cfg.github_api_url = base.to_string();
    cfg.github_raw_url = base.to_string();
    db.state_with(cfg).await
}

/// Phase 1 of an import, run to completion: the row `start_import` would
/// insert, then the background task it would spawn, awaited.
async fn run_import(db: &TestDb, state: &AppState) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO input_data_imports (tarball_date, commit_sha) VALUES ($1, $2) RETURNING id",
    )
    .bind(DATE)
    .bind(COMMIT)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    birdtest::inputdata::run_import(state.clone(), id, DATE.into(), COMMIT.into()).await;
    id
}

/// `(state, error, tarball_sha256)` of an import.
async fn import_state_row(db: &TestDb, id: Uuid) -> (String, Option<String>, Option<String>) {
    sqlx::query_as("SELECT state, error, tarball_sha256 FROM input_data_imports WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// `(path, role, name, sha256, disposition)` of an import's staged rows.
async fn staged(db: &TestDb, id: Uuid) -> Vec<(String, String, String, String, String)> {
    sqlx::query_as(
        "SELECT path, role, name, sha256, disposition FROM input_data_import_rows
         WHERE import_id = $1 ORDER BY path",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn input_data_count(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM input_data").fetch_one(&db.pool).await.unwrap()
}

/// The pinned files in the fixture, as staged rows with `disposition`.
fn expected_staged(disposition: &str) -> Vec<(String, String, String, String, String)> {
    let mut rows: Vec<_> = [
        ("layouts/standard15.txt", "layout", "standard15"),
        ("letterdistributions/english.csv", "letterdist", "english"),
        ("lexica/NWL23.klv2", "klv", "NWL23"),
        ("lexica/NWL23.kwg", "kwg", "NWL23"),
        ("strategy/winpct.csv", "winpct", "winpct"),
    ]
    .into_iter()
    .map(|(path, role, name)| {
        (path.into(), role.into(), name.into(), sha(&fixture_file(path)), disposition.into())
    })
    .collect();
    rows.sort();
    rows
}

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
    id: Uuid,
    app: Router,
    headers: Vec<(String, String)>,
}

impl Admin {
    async fn new(db: &TestDb, state: AppState) -> Self {
        let id = db.user(&format!("root{}", Uuid::new_v4().simple()), true).await;
        let headers = admin_headers(&state.cfg, id);
        Admin { id, app: birdtest::app(state), headers }
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        send(&self.app, request(method, path, &self.headers, body)).await
    }

    async fn confirm(&self, import: Uuid) -> (StatusCode, Value) {
        self.call("POST", &format!("/api/admin/input-data/imports/{import}/confirm"), None).await
    }
}

/// A fixture serving the standard tarball whole, its fake GitHub, and a state
/// on both with a real object store.
async fn standard_import_setup(db: &TestDb) -> (Arc<Fixture>, AppState, TestBucket) {
    let fixture = Fixture::new();
    fixture.serve_whole(tarball(&fixture_files()));
    let base = fake_github(fixture.clone()).await;
    let (state, bucket) = import_state(db, &base).await;
    (fixture, state, bucket)
}

/// I-INPUT-1: phase 1 stages one `input_data_import_rows` row per pinned file
/// in the tarball -- path, role, name and digest, all `new` against an empty
/// vocabulary, the unpinned directory skipped -- records the tarball's digest,
/// and writes no `input_data` row until the import is confirmed.
#[tokio::test]
async fn a_staged_import_writes_its_diff_and_no_input_data() {
    let db = TestDb::new().await;
    let (fixture, state, _bucket) = standard_import_setup(&db).await;

    let import = run_import(&db, &state).await;
    let archive = tarball(&fixture_files());
    assert_eq!(
        import_state_row(&db, import).await,
        ("staged".into(), None, Some(sha(&archive))),
        "the tarball's digest identifies the whole install"
    );
    assert_eq!(staged(&db, import).await, expected_staged("new"));
    assert_eq!(input_data_count(&db).await, 0, "nothing is data until it is confirmed");
    assert!(
        fixture.requests().contains(&Fixture::tarball_path(Some("aa"))),
        "chunks are probed before the whole file: {:?}",
        fixture.requests()
    );
}

/// I-INPUT-2: confirming inserts `input_data` rows for what is new, and
/// dedupes by `(path, sha256)`: a file already known byte-for-byte is reported
/// `known` and not inserted again; a known path with different bytes is
/// reported `collision` and inserted as the new row it is. The import says
/// which was which, and the confirmation how many it inserted.
#[tokio::test]
async fn confirming_inserts_what_is_new_and_reports_what_was_already_there() {
    let db = TestDb::new().await;
    let (_fixture, state, _bucket) = standard_import_setup(&db).await;
    let admin = Admin::new(&db, state.clone()).await;

    // Already known exactly: the win% model. Known by path only: an older
    // English distribution.
    let known: Uuid = sqlx::query_scalar(
        "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date)
         VALUES ('strategy/winpct.csv', 'winpct', 'winpct', $1, 300, '20240101') RETURNING id",
    )
    .bind(sha(&fixture_file("strategy/winpct.csv")))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let older: Uuid = sqlx::query_scalar(
        "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
         VALUES ('letterdistributions/english.csv', 'letterdist', 'english', $1, 3, '20240101',
                 'old') RETURNING id",
    )
    .bind(sha(b"old"))
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let import = run_import(&db, &state).await;
    let (status, detail) =
        admin.call("GET", &format!("/api/admin/input-data/imports/{import}"), None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let dispositions: HashMap<String, String> = detail["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap().into(), f["disposition"].as_str().unwrap().into()))
        .collect();
    assert_eq!(
        dispositions,
        HashMap::from([
            ("strategy/winpct.csv".into(), "known".into()),
            ("letterdistributions/english.csv".into(), "collision".into()),
            ("layouts/standard15.txt".into(), "new".into()),
            ("lexica/NWL23.kwg".into(), "new".into()),
            ("lexica/NWL23.klv2".into(), "new".into()),
        ])
    );

    let (status, confirmed) = admin.confirm(import).await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    assert_eq!(confirmed, json!({ "inserted": 4 }), "three new and one collision");

    let rows: Vec<(Uuid, String, String, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, path, sha256, tarball_date, imported_by FROM input_data ORDER BY path, tarball_date",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 6, "{rows:?}");
    let by_path = |path: &str| rows.iter().filter(|r| r.1 == path).collect::<Vec<_>>();
    let winpct = by_path("strategy/winpct.csv");
    assert_eq!(winpct.len(), 1, "a known file is not inserted twice");
    assert_eq!(winpct[0].0, known);
    let english = by_path("letterdistributions/english.csv");
    assert_eq!(english.len(), 2, "a collision is a new row beside the old one");
    assert_eq!(english[0].0, older);
    assert_eq!(english[1].2, sha(TESTDIST));
    for row in rows.iter().filter(|r| r.0 != known && r.0 != older) {
        assert_eq!((row.3.as_str(), row.4), (DATE, Some(admin.id)), "{row:?}");
    }
    assert_eq!(import_state_row(&db, import).await.0, "confirmed");
}

/// I-INPUT-3: only the roles the server reads itself (`letterdist`,
/// `layout`) keep their bytes on the row; `kwg`, `klv` and `winpct` rows hold
/// a digest and NULL content. And the schema's CHECK enforces it in both
/// directions for any writer, not only the import.
#[tokio::test]
async fn only_server_read_roles_keep_their_bytes_and_the_schema_insists() {
    let db = TestDb::new().await;
    let (_fixture, state, _bucket) = standard_import_setup(&db).await;
    let admin = Admin::new(&db, state.clone()).await;
    let import = run_import(&db, &state).await;
    let (status, body) = admin.confirm(import).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: Vec<(String, String, i64, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT path, role, bytes, content FROM input_data ORDER BY path")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 5);
    for (path, role, bytes, content) in rows {
        let file = fixture_file(&path);
        assert_eq!(bytes, file.len() as i64, "{path}");
        match role.as_str() {
            "letterdist" | "layout" => assert_eq!(content, Some(file), "{path} keeps its bytes"),
            _ => assert_eq!(content, None, "{path} keeps only its digest"),
        }
    }

    for (role, content) in [("kwg", Some(b"bytes".as_slice())), ("winpct", Some(b"bytes")), ("letterdist", None)] {
        let err = sqlx::query(
            "INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
             VALUES ($1, $2, 'x', repeat('a', 64), 5, '20250101', $3)",
        )
        .bind(format!("{role}/x"))
        .bind(role)
        .bind(content)
        .execute(&db.pool)
        .await
        .expect_err("the CHECK refuses it");
        assert_eq!(
            err.as_database_error().unwrap().constraint(),
            Some("input_data_check"),
            "{role} with content {content:?}"
        );
    }
    // Postgres names a column CHECK that reads a second column after the
    // table; this is that one.
    let definition: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'input_data_check'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(definition.contains("content IS NOT NULL"), "{definition}");
}

/// I-INPUT-8: `kwg` and `klv` rows carry an `object_key` -- the digest -- and
/// their bytes are in the object store under it; `winpct`, `letterdist` and
/// `layout` rows carry none, and nothing else is uploaded. Re-importing a
/// tarball whose lexica have not changed uploads nothing: the objects are
/// found by key and left as they are.
#[tokio::test]
async fn lexica_and_leaves_are_stored_once_by_digest() {
    let db = TestDb::new().await;
    let (_fixture, state, bucket) = standard_import_setup(&db).await;
    let admin = Admin::new(&db, state.clone()).await;
    let import = run_import(&db, &state).await;
    let (status, body) = admin.confirm(import).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT path, role, object_key FROM input_data ORDER BY path")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    let mut stored = Vec::new();
    for (path, role, key) in rows {
        match role.as_str() {
            "kwg" | "klv" => {
                let key = key.unwrap_or_else(|| panic!("{path} names its object"));
                assert_eq!(key, format!("inputs/{}", sha(&fixture_file(&path))), "{path}");
                assert_eq!(state.artifacts.get(&key).await.unwrap(), fixture_file(&path), "{path}");
                stored.push(key);
            }
            _ => assert_eq!(key, None, "{path} stores no object"),
        }
    }
    stored.sort();
    let mut keys = bucket.keys().await;
    keys.sort();
    assert_eq!(keys, stored, "only the lexicon and its leaves were uploaded");

    // Marks the stored objects, then imports the same lexica again: an upload
    // would replace the marks.
    for key in &stored {
        state.artifacts.put(key, b"already here".to_vec()).await.unwrap();
    }
    let again = run_import(&db, &state).await;
    assert_eq!(import_state_row(&db, again).await.0, "staged");
    for key in &stored {
        assert_eq!(state.artifacts.get(key).await.unwrap(), b"already here", "{key} re-uploaded");
    }
    let mut keys = bucket.keys().await;
    keys.sort();
    assert_eq!(keys, stored, "and nothing new was uploaded beside them");
}

/// I-INPUT-4: importing the same tarball a second time changes nothing:
/// every file stages as `known`, confirming inserts nothing, and the
/// vocabulary is exactly what the first import left.
#[tokio::test]
async fn a_second_import_of_the_same_tarball_is_a_no_op() {
    let db = TestDb::new().await;
    let (_fixture, state, _bucket) = standard_import_setup(&db).await;
    let admin = Admin::new(&db, state.clone()).await;
    let snapshot = || async {
        sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
            "SELECT id, path, sha256, object_key FROM input_data ORDER BY path",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap()
    };

    let first = run_import(&db, &state).await;
    assert_eq!(admin.confirm(first).await, (StatusCode::OK, json!({ "inserted": 5 })));
    let before = snapshot().await;

    let second = run_import(&db, &state).await;
    assert_eq!(staged(&db, second).await, expected_staged("known"));
    assert_eq!(admin.confirm(second).await, (StatusCode::OK, json!({ "inserted": 0 })));
    assert_eq!(snapshot().await, before);
}

/// I-INPUT-5: the startup reaper fails every import left `running` -- its
/// process is gone -- with a reason, and leaves imports in every other state
/// exactly as they were.
#[tokio::test]
async fn a_restart_fails_running_imports_and_leaves_the_rest() {
    let db = TestDb::new().await;
    let mut ids = Vec::new();
    for (state, error) in [
        ("running", None),
        ("staged", None),
        ("confirmed", None),
        ("failed", Some("earlier failure")),
        ("cancelled", Some("expired")),
    ] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO input_data_imports (tarball_date, commit_sha, state, error)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(DATE)
        .bind(COMMIT)
        .bind(state)
        .bind(error)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        ids.push(id);
    }

    assert_eq!(birdtest::inputdata::fail_orphaned_imports(&db.pool).await.unwrap(), 1);

    let mut after = Vec::new();
    for id in &ids {
        let (state, error, _) = import_state_row(&db, *id).await;
        after.push((state, error));
    }
    assert_eq!(
        after,
        vec![
            ("failed".into(), Some("the server restarted while this import was running".into())),
            ("staged".into(), None),
            ("confirmed".into(), None),
            ("failed".into(), Some("earlier failure".into())),
            ("cancelled".into(), Some("expired".into())),
        ]
    );
}

/// I-INPUT-7: an import that fails records why, stages nothing and inserts
/// nothing -- whether the tarball is missing or is not an archive at all.
#[tokio::test]
async fn a_failed_import_records_its_error_and_stages_nothing() {
    let db = TestDb::new().await;
    let fixture = Fixture::new();
    let base = fake_github(fixture.clone()).await;
    let state = import_state_without_store(&db, &base).await;

    let missing = run_import(&db, &state).await;
    let (state_name, error, digest) = import_state_row(&db, missing).await;
    assert_eq!(state_name, "failed");
    assert_eq!(error.as_deref(), Some("no data-20250101.tgz in example/data at 0123456"));
    assert_eq!(digest, None);

    fixture.serve_whole(b"this is not a gzipped tar archive".to_vec());
    let garbage = run_import(&db, &state).await;
    let (state_name, error, _) = import_state_row(&db, garbage).await;
    assert_eq!(state_name, "failed");
    let error = error.expect("the failure says why");
    assert!(
        error.contains("tar") || error.contains("archive"),
        "the reason names the archive: {error}"
    );

    for import in [missing, garbage] {
        assert!(staged(&db, import).await.is_empty(), "nothing staged for {import}");
    }
    assert_eq!(input_data_count(&db).await, 0);
}

/// Polls `GET .../imports/<id>` until `done` holds of it, or a generous
/// deadline passes. The import runs on a spawned task, so its progress is
/// only observable by asking.
async fn poll_import(admin: &Admin, id: &str, done: impl Fn(&Value) -> bool) -> Value {
    let mut last = Value::Null;
    for _ in 0..400 {
        let (status, body) =
            admin.call("GET", &format!("/api/admin/input-data/imports/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if done(&body) {
            return body;
        }
        last = body;
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the import never reached the expected state: {last}");
}

/// A-ADMIN-5: an import over HTTP. Starting it validates the date and
/// resolves the ref synchronously, then returns at once; polling shows it
/// running with its progress while the download is held part-way through its
/// chunks; confirming it before it is staged is refused; once staged, polling
/// shows the diff and confirming inserts it. Each step writes its audit row.
#[tokio::test]
async fn an_import_is_started_polled_while_running_and_confirmed_over_http() {
    let db = TestDb::new().await;
    let fixture = Fixture::new();
    let archive = tarball(&fixture_files());
    let (aa, ab) = archive.split_at(archive.len() / 2);
    fixture.put(&Fixture::tarball_path(Some("aa")), aa.to_vec());
    fixture.put(&Fixture::tarball_path(Some("ab")), ab.to_vec());
    fixture.hold(Fixture::tarball_path(Some("ab")));
    let base = fake_github(fixture.clone()).await;
    let (state, _bucket) = import_state(&db, &base).await;
    let admin = Admin::new(&db, state).await;

    let start = |body: Value| admin.call("POST", "/api/admin/input-data/imports", Some(body));
    let (status, body) = start(json!({ "tarball_date": "2025-01-01" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["message"], "tarball_date must be YYYYMMDD");
    let (status, body) = start(json!({ "tarball_date": DATE, "git_ref": "no-such-branch" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["message"], "no such ref \"no-such-branch\" in example/data");

    let (status, started) = start(json!({ "tarball_date": DATE })).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started["state"], "running");
    let id = started["id"].as_str().unwrap().to_string();

    // The first chunk is in and the second is held: the import is running,
    // and says how far it has got.
    let running = poll_import(&admin, &id, |b| b["progress_bytes"] == json!(aa.len())).await;
    assert_eq!(running["state"], "running", "{running}");
    assert_eq!(running["commit_sha"], COMMIT, "the ref was pinned to a commit");
    assert_eq!(running["files"], json!([]), "nothing staged yet");
    let (status, body) = admin.confirm(id.parse().unwrap()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["message"], "this import is running, not staged");

    fixture.release();
    let staged = poll_import(&admin, &id, |b| b["state"] != "running").await;
    assert_eq!(staged["state"], "staged", "{staged}");
    assert_eq!(staged["progress_bytes"], json!(archive.len()), "{staged}");
    assert_eq!(staged["progress_entries"], 5, "{staged}");
    assert_eq!(staged["tarball_sha256"], sha(&archive), "the chunks, concatenated");
    assert_eq!(staged["files"].as_array().unwrap().len(), 5, "{staged}");
    let requests = fixture.requests();
    for suffix in ["aa", "ab", "ac"] {
        assert!(requests.contains(&Fixture::tarball_path(Some(suffix))), "{suffix}: {requests:?}");
    }
    assert!(!requests.contains(&Fixture::tarball_path(None)), "chunked, so no whole file");

    let (status, body) = admin.confirm(id.parse().unwrap()).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "inserted": 5 })));
    let confirmed = poll_import(&admin, &id, |_| true).await;
    assert_eq!(confirmed["state"], "confirmed");
    assert!(!confirmed["confirmed_at"].is_null(), "{confirmed}");
    assert_eq!(input_data_count(&db).await, 5);

    let audit: Vec<(String, Option<Uuid>, Option<String>)> =
        sqlx::query_as("SELECT action, actor_user_id, target_id FROM audit_log ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        audit,
        vec![
            ("input_data.import_staged".into(), Some(admin.id), Some(id.clone())),
            ("input_data.import_confirmed".into(), Some(admin.id), Some(id.clone())),
        ]
    );
}

/// A-ADMIN-6: only a `staged` import can be confirmed. One running, failed,
/// cancelled or already confirmed is refused with its state, and inserts
/// nothing even when it has rows; one that does not exist is a 404.
#[tokio::test]
async fn only_a_staged_import_can_be_confirmed() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    for state in ["running", "failed", "cancelled", "confirmed"] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO input_data_imports (tarball_date, commit_sha, state)
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(DATE)
        .bind(COMMIT)
        .bind(state)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO input_data_import_rows
                 (import_id, path, role, name, sha256, bytes, disposition)
             VALUES ($1, 'strategy/winpct.csv', 'winpct', 'winpct', repeat('b', 64), 1, 'new')",
        )
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();

        let (status, body) = admin.confirm(id).await;
        assert_eq!(status, StatusCode::CONFLICT, "{state}: {body}");
        assert_eq!(body["message"], format!("this import is {state}, not staged"));
        assert_eq!(import_state_row(&db, id).await.0, state, "unchanged");
    }
    assert_eq!(input_data_count(&db).await, 0);

    let (status, body) = admin.confirm(Uuid::new_v4()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["message"], "no such import");
}

/// I-INPUT-6 and A-ADMIN-7: the list reports, per file, how many jobs, player
/// configs and rating pools pin it; a file pinned by any of them -- a job's
/// distribution or board, a player's lexicon, leaves or win% model, a leave
/// job's lexicon, a rating pool's distribution or board -- cannot be deleted
/// and stays; an unpinned one can, once.
#[tokio::test]
async fn an_input_file_in_use_cannot_be_deleted() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;

    // A job's distribution and board.
    let pool = &db.pool;
    let board = |job: Uuid| async move {
        sqlx::query_as::<_, (Uuid, Uuid)>("SELECT letterdist_id, layout_id FROM jobs WHERE id = $1")
            .bind(job)
            .fetch_one(pool)
            .await
            .unwrap()
    };
    let (job_ld, job_layout) = board(db.bare_job("games", 1, admin.id).await).await;
    // A simming player's lexicon, leaves and win% model.
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let winpct = db.input_data("winpct", "winpct").await;
    let (status, body) = admin
        .call(
            "POST",
            "/api/admin/player-configs",
            Some(json!({
                "name": "simmer", "recorder_type": "best", "kwg_id": kwg, "klv_id": klv,
                "winpct_id": winpct, "num_plies": 2, "num_plays": 10, "max_iterations": 100,
                "time_limit_secs": 0, "num_plays_recorded": 1,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let player: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    // A leave job's lexicon.
    let leave = db.bare_job("leave_generation", 1, admin.id).await;
    let (leave_ld, leave_layout) = board(leave).await;
    let leave_kwg = db.input_data("kwg", "CSW21").await;
    sqlx::query(
        "INSERT INTO job_leave_config
             (job_id, kwg_id, num_iterations, target_rack_count, racks_per_task)
         VALUES ($1, $2, 10, 10, 10)",
    )
    .bind(leave)
    .bind(leave_kwg)
    .execute(&db.pool)
    .await
    .unwrap();
    // A rating pool's distribution and board.
    let pool_ld = db.input_data("letterdist", "english_pool").await;
    let pool_layout = db.input_data("layout", "pool15").await;
    sqlx::query(
        "INSERT INTO rating_pools (name, variant, letterdist_id, layout_id, anchor_player_config_id)
         VALUES ('pool', 'classic', $1, $2, $3)",
    )
    .bind(pool_ld)
    .bind(pool_layout)
    .bind(player)
    .execute(&db.pool)
    .await
    .unwrap();
    let unused = db.input_data("kwg", "unused").await;

    let pinned = [
        job_ld, job_layout, kwg, klv, winpct, leave_ld, leave_layout, leave_kwg, pool_ld, pool_layout,
    ];
    let (status, list) = admin.call("GET", "/api/admin/input-data", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let references: HashMap<Uuid, i64> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["id"].as_str().unwrap().parse().unwrap(), row["references"].as_i64().unwrap()))
        .collect();
    assert_eq!(references.len(), pinned.len() + 1, "every file is listed: {list}");
    for id in pinned {
        assert_eq!(references[&id], 1, "{id} is pinned once: {list}");
    }
    assert_eq!(references[&unused], 0);

    for id in pinned {
        let (status, body) = admin.call("DELETE", &format!("/api/admin/input-data/{id}"), None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{id}: {body}");
        assert_eq!(body["message"], "this file is pinned by 1 job, player config or rating pool");
    }
    let (status, body) = admin.call("DELETE", &format!("/api/admin/input-data/{unused}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = admin.call("DELETE", &format!("/api/admin/input-data/{unused}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["message"], "no such input data row");

    let left: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM input_data ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let mut expected = pinned.to_vec();
    expected.sort();
    assert_eq!(left, expected, "every pinned file survived its refused delete");
}

/// I-INPUT-6, A-ADMIN-7: once nothing pins a file, what was built from it is
/// no reason to keep it. A wordmap and a rack info table hold foreign keys to
/// their lexicon, leaves and distribution, and are never collected; before
/// this, every file anything had ever been built from was undeletable, with a
/// generic "still referenced" 409 against a file the list said had no
/// references. The derived rows go with the file, and only those rows.
#[tokio::test]
async fn a_file_only_derived_data_refers_to_can_be_deleted_and_takes_that_data_with_it() {
    let db = TestDb::new().await;
    let admin = Admin::new(&db, db.state().await).await;
    let kwg = db.input_data("kwg", "NWL23").await;
    let klv = db.input_data("klv", "NWL23").await;
    let ld = db.input_data("letterdist", "english").await;
    let other_kwg = db.input_data("kwg", "CSW21").await;
    for (role, name, kwg_id, klv_id) in [
        ("wmp", "NWL23", kwg, None),
        ("rit", "NWL23.NWL23", kwg, Some(klv)),
        ("wmp", "CSW21", other_kwg, None),
    ] {
        sqlx::query(
            "INSERT INTO derived_data (role, name, builder, kwg_id, klv_id, letterdist_id,
                                       state, sha256, bytes, build_target, built_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'built', repeat('a', 64), 1, 'nehalem', now())",
        )
        .bind(role)
        .bind(name)
        .bind(format!("{role}-1"))
        .bind(kwg_id)
        .bind(klv_id)
        .bind(ld)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let (_, list) = admin.call("GET", "/api/admin/input-data", None).await;
    let row = list.as_array().unwrap().iter().find(|r| r["id"] == kwg.to_string()).unwrap();
    assert_eq!(row["references"], 0, "derived data pins nothing: {row}");

    let (status, body) = admin.call("DELETE", &format!("/api/admin/input-data/{kwg}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let left: Vec<String> = sqlx::query_scalar("SELECT name FROM derived_data ORDER BY name")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(left, ["CSW21"], "the file's wordmap and table went with it, and nothing else");

    // The distribution is still shared with CSW21's wordmap; deleting it takes
    // that too, since no job pins it either.
    let (status, body) = admin.call("DELETE", &format!("/api/admin/input-data/{ld}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let left: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM derived_data").fetch_one(&db.pool).await.unwrap();
    assert_eq!(left, 0);
    for id in [kwg, ld] {
        let audited: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'input_data.deleted' AND target_id = $1",
        )
        .bind(id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(audited, 1);
    }
}
