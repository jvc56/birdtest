//! Importing a MAGPIE-DATA versioned tarball into `input_data`.
//!
//! Two phases. Phase 1 fetches the tarball, hashes every entry, and stages a
//! diff against what is already known; phase 2 inserts the new rows on the
//! admin's confirmation. Neither is on the dispatch path -- dispatch reads
//! local tables only, so GitHub can be down for a week without a worker
//! noticing.
//!
//! Phase 1 runs as a spawned task rather than inside the request: the archive
//! is ~94 MB, and a request that waits that long needs a timeout far above the
//! framework default and must not hold a transaction open across the network
//! I/O. birdtest runs as a single instance, so the task needs no lease -- and,
//! for the same reason, startup may fail any row still marked `running`.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Read;
use uuid::Uuid;

/// Limits on the archive walk. Every one of them aborts the import rather than
/// skipping the entry: a malformed archive is not a partially trustworthy one.
mod limits {
    /// The real tarball is ~94 MB.
    pub const COMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
    /// `download_data.sh` stops at 26 (`aa`..`az`); this is headroom without
    /// being unbounded.
    pub const CHUNKS: u32 = 64;
    pub const UNCOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
    /// Real gzip on this content runs about 3-4x; a bomb is thousands. Checked
    /// continuously rather than at the end, which is the case a total cap alone
    /// lets through.
    pub const EXPANSION_RATIO: u64 = 20;
    /// The largest real entry is a ~15 MB .kwg.
    pub const ENTRY_BYTES: u64 = 128 * 1024 * 1024;
    /// MAGPIE-DATA has low hundreds of files.
    pub const ENTRIES: usize = 5_000;
}

/// A file the archive contained, hashed.
#[derive(Debug)]
pub struct ImportedFile {
    pub path: String,
    pub role: String,
    pub name: String,
    pub sha256: String,
    pub bytes: i64,
    /// Kept only for the roles the server itself reads; see `input_data.content`.
    pub content: Option<Vec<u8>>,
}

/// `data/<dir>/<basename>` to (path, role, name). The inverse of the table in
/// the design's "which files a task needs" section; unrecognised directories
/// are ignored rather than rejected, since MAGPIE-DATA carries more than
/// birdtest pins.
fn classify(entry_path: &str) -> Option<(String, String, String)> {
    let rest = entry_path.strip_prefix("data/")?;
    let (dir, basename) = rest.split_once('/')?;
    if basename.is_empty() || basename.contains('/') {
        return None;
    }
    let (role, suffix) = match dir {
        "lexica" if basename.ends_with(".kwg") => ("kwg", ".kwg"),
        "lexica" if basename.ends_with(".klv2") => ("klv", ".klv2"),
        "letterdistributions" if basename.ends_with(".csv") => ("letterdist", ".csv"),
        "layouts" if basename.ends_with(".txt") => ("layout", ".txt"),
        "strategy" if basename.ends_with(".csv") => ("winpct", ".csv"),
        _ => return None,
    };
    let name = basename.strip_suffix(suffix)?.to_string();
    Some((format!("{dir}/{basename}"), role.to_string(), name))
}

/// Roles whose bytes are stored on the row, because the server reads them
/// itself and must read exactly what the job pins.
fn keeps_content(role: &str) -> bool {
    matches!(role, "letterdist" | "layout")
}

/// Resolves a git ref to a commit sha, so `main` is pinned at import time and
/// the record names a commit rather than a branch.
pub async fn resolve_ref(state: &AppState, git_ref: &str) -> AppResult<String> {
    let url = format!(
        "https://api.github.com/repos/{}/commits/{}",
        state.cfg.magpie_data_repo, git_ref
    );
    let mut request = state
        .http
        .get(&url)
        .header("User-Agent", "birdtest")
        .header("Accept", "application/vnd.github.sha");
    if let Some(token) = &state.cfg.github_token {
        request = request.bearer_auth(token);
    }

    let response = request.send().await.map_err(github_error)?;
    let status = response.status();
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        // Unauthenticated resolution is 60 calls an hour per IP, and a bare
        // 403 does not say so. GitHub returns both of these headers; rendering
        // them is the difference between a two-minute fix and an hour of
        // confusion.
        let remaining = header(&response, "x-ratelimit-remaining");
        let reset = header(&response, "x-ratelimit-reset")
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
            .map(|t| t.format("%H:%M UTC").to_string());
        if remaining.as_deref() == Some("0") {
            return Err(AppError::bad_request(format!(
                "GitHub rate limit reached{}; set GITHUB_TOKEN to raise it from 60 to 5000 \
                 requests per hour",
                reset.map(|r| format!(", resets at {r}")).unwrap_or_default()
            )));
        }
        return Err(AppError::bad_request(format!(
            "GitHub refused the request ({status})"
        )));
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::not_found(format!(
            "no such ref {git_ref:?} in {}",
            state.cfg.magpie_data_repo
        )));
    }
    if !status.is_success() {
        return Err(AppError::bad_request(format!(
            "GitHub returned {status} resolving {git_ref:?}"
        )));
    }

    let sha = response.text().await.map_err(github_error)?.trim().to_string();
    if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::bad_request(format!(
            "GitHub returned something that is not a commit sha: {sha:?}"
        )));
    }
    Ok(sha)
}

fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
}

fn github_error(err: reqwest::Error) -> AppError {
    AppError::bad_request(format!("could not reach GitHub: {err}"))
}

/// Downloads the tarball, mirroring `download_data.sh`'s chunking exactly: try
/// `.aa` first and walk `aa -> ab -> ... -> az -> ba ...` while chunks exist,
/// falling back to the unchunked name. Returns the concatenated bytes and their
/// SHA-256, which is a single value identifying a whole install.
async fn download(
    state: &AppState,
    commit_sha: &str,
    tarball_date: &str,
    progress: &Progress,
) -> AppResult<(Vec<u8>, String)> {
    let base = format!(
        "https://raw.githubusercontent.com/{}/{}/versioned-tarballs/data-{}.tgz",
        state.cfg.magpie_data_repo, commit_sha, tarball_date
    );

    let mut body = Vec::new();
    let mut hasher = Sha256::new();
    let mut chunked = false;

    for index in 0..limits::CHUNKS {
        let suffix = chunk_suffix(index);
        let url = format!("{base}.{suffix}");
        let response = state.http.get(&url).send().await.map_err(github_error)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            if index == 0 {
                break;
            }
            chunked = true;
            break;
        }
        if !response.status().is_success() {
            return Err(AppError::bad_request(format!(
                "fetching {url} returned {}",
                response.status()
            )));
        }
        read_into(response, &mut body, &mut hasher, progress).await?;
        chunked = true;
    }

    if !chunked {
        let response = state.http.get(&base).send().await.map_err(github_error)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            // The ordinary "no such version" answer, not a transport error.
            return Err(AppError::not_found(format!(
                "no data-{tarball_date}.tgz in {} at {}",
                state.cfg.magpie_data_repo,
                &commit_sha[..7.min(commit_sha.len())]
            )));
        }
        if !response.status().is_success() {
            return Err(AppError::bad_request(format!(
                "fetching {base} returned {}",
                response.status()
            )));
        }
        read_into(response, &mut body, &mut hasher, progress).await?;
    } else if body.is_empty() {
        return Err(AppError::not_found(format!(
            "no data-{tarball_date}.tgz chunks in {}",
            state.cfg.magpie_data_repo
        )));
    }

    Ok((body, hex::encode(hasher.finalize())))
}

async fn read_into(
    response: reqwest::Response,
    body: &mut Vec<u8>,
    hasher: &mut Sha256,
    progress: &Progress,
) -> AppResult<()> {
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(github_error)?;
        if body.len() as u64 + chunk.len() as u64 > limits::COMPRESSED_BYTES {
            return Err(AppError::bad_request(format!(
                "tarball exceeds the {} MiB download cap",
                limits::COMPRESSED_BYTES / 1024 / 1024
            )));
        }
        hasher.update(&chunk);
        body.extend_from_slice(&chunk);
        progress.bytes(body.len() as i64).await;
    }
    Ok(())
}

/// `aa`, `ab`, ... `az`, `ba`, ... -- the suffixes `split` produces and
/// `download_data.sh` walks.
fn chunk_suffix(index: u32) -> String {
    let first = (b'a' + (index / 26) as u8) as char;
    let second = (b'a' + (index % 26) as u8) as char;
    format!("{first}{second}")
}

/// Walks the gzipped tar, hashing every entry it recognises.
///
/// This is the one place birdtest parses an untrusted container format, so
/// every entry is checked against an explicit allowlist before it is trusted
/// enough to hash: regular files only, relative paths, no `..` segment, and the
/// expected `data/<dir>/<basename>` shape. No filesystem path is ever
/// constructed from an archive name -- the import hashes bytes and has no
/// reason to form one.
pub fn walk_archive(compressed: &[u8], progress: Option<&Progress>) -> AppResult<Vec<ImportedFile>> {
    let decoder = flate2::read::GzDecoder::new(compressed);
    let mut archive = tar::Archive::new(decoder);
    let mut files = Vec::new();
    let mut total_uncompressed: u64 = 0;

    let entries = archive
        .entries()
        .map_err(|e| AppError::bad_request(format!("not a readable tar archive: {e}")))?;

    for entry in entries {
        let mut entry =
            entry.map_err(|e| AppError::bad_request(format!("malformed tar entry: {e}")))?;

        if files.len() >= limits::ENTRIES {
            return Err(AppError::bad_request(format!(
                "archive has more than {} entries",
                limits::ENTRIES
            )));
        }

        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() {
            if entry_type.is_dir() {
                continue;
            }
            // A symlink, device or hard link has no business in a data
            // tarball, and skipping it quietly would make a hostile archive
            // look ordinary.
            return Err(AppError::bad_request(format!(
                "archive contains a non-regular entry ({entry_type:?})"
            )));
        }

        let path = entry
            .path()
            .map_err(|e| AppError::bad_request(format!("unreadable entry path: {e}")))?
            .to_string_lossy()
            .to_string();
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(AppError::bad_request(format!(
                "archive contains an unsafe path: {path:?}"
            )));
        }

        let size = entry.header().size().unwrap_or(0);
        if size > limits::ENTRY_BYTES {
            return Err(AppError::bad_request(format!(
                "{path} is larger than the {} MiB per-entry cap",
                limits::ENTRY_BYTES / 1024 / 1024
            )));
        }

        let Some((mapped_path, role, name)) = classify(&path) else {
            // An unrecognised directory: still counted against the caps, but
            // not something birdtest pins.
            total_uncompressed += size;
            check_expansion(total_uncompressed, compressed.len() as u64)?;
            continue;
        };

        let mut bytes = Vec::with_capacity(size as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::bad_request(format!("could not read {path}: {e}")))?;

        total_uncompressed += bytes.len() as u64;
        check_expansion(total_uncompressed, compressed.len() as u64)?;

        let sha256 = hex::encode(Sha256::digest(&bytes));
        let content = keeps_content(&role).then(|| bytes.clone());
        files.push(ImportedFile {
            path: mapped_path,
            role,
            name,
            sha256,
            bytes: bytes.len() as i64,
            content,
        });
        if let Some(progress) = progress {
            progress.entries(files.len() as i32);
        }
    }

    if files.is_empty() {
        return Err(AppError::bad_request(
            "archive contained no recognisable data files",
        ));
    }
    Ok(files)
}

fn check_expansion(uncompressed: u64, compressed: u64) -> AppResult<()> {
    if uncompressed > limits::UNCOMPRESSED_BYTES {
        return Err(AppError::bad_request(format!(
            "archive expands past the {} GiB cap",
            limits::UNCOMPRESSED_BYTES / 1024 / 1024 / 1024
        )));
    }
    if compressed > 0 && uncompressed / compressed.max(1) > limits::EXPANSION_RATIO {
        return Err(AppError::bad_request(format!(
            "archive expands more than {}x, which a total cap alone would not catch",
            limits::EXPANSION_RATIO
        )));
    }
    Ok(())
}

/// Writes progress onto the import row so the admin UI's poll has something to
/// render while the download runs.
pub struct Progress {
    pool: sqlx::PgPool,
    import_id: Uuid,
    entries: std::sync::atomic::AtomicI32,
}

impl Progress {
    pub fn new(pool: sqlx::PgPool, import_id: Uuid) -> Self {
        Self { pool, import_id, entries: std::sync::atomic::AtomicI32::new(0) }
    }

    async fn bytes(&self, bytes: i64) {
        // Best effort: a lost progress update is not worth failing an import
        // over, and the terminal state is written on a separate path.
        let _ = sqlx::query("UPDATE input_data_imports SET progress_bytes = $2 WHERE id = $1")
            .bind(self.import_id)
            .bind(bytes)
            .execute(&self.pool)
            .await;
    }

    fn entries(&self, count: i32) {
        self.entries.store(count, std::sync::atomic::Ordering::Relaxed);
    }

    async fn flush_entries(&self) {
        let count = self.entries.load(std::sync::atomic::Ordering::Relaxed);
        let _ = sqlx::query("UPDATE input_data_imports SET progress_entries = $2 WHERE id = $1")
            .bind(self.import_id)
            .bind(count)
            .execute(&self.pool)
            .await;
    }
}

/// Phase 1, off the request thread: fetch, hash, diff, stage.
pub async fn run_import(state: AppState, import_id: Uuid, tarball_date: String, commit_sha: String) {
    let progress = Progress::new(state.pool.clone(), import_id);
    match stage(&state, import_id, &tarball_date, &commit_sha, &progress).await {
        Ok(staged) => {
            progress.flush_entries().await;
            let _ = sqlx::query(
                "UPDATE input_data_imports SET state = 'staged', tarball_sha256 = $2 WHERE id = $1",
            )
            .bind(import_id)
            .bind(staged)
            .execute(&state.pool)
            .await;
        }
        Err(err) => {
            tracing::warn!(%import_id, error = %err.message, "input data import failed");
            let _ = sqlx::query(
                "UPDATE input_data_imports SET state = 'failed', error = $2 WHERE id = $1",
            )
            .bind(import_id)
            .bind(&err.message)
            .execute(&state.pool)
            .await;
        }
    }
}

async fn stage(
    state: &AppState,
    import_id: Uuid,
    tarball_date: &str,
    commit_sha: &str,
    progress: &Progress,
) -> AppResult<String> {
    let (body, tarball_sha256) = download(state, commit_sha, tarball_date, progress).await?;
    let files = walk_archive(&body, Some(progress))?;

    let mut tx = state.pool.begin().await?;
    for file in &files {
        // Three dispositions, and the third is the one that deserves a second
        // look: a path already known under a different digest is either a
        // legitimate data update or a tarball re-cut under a name that was
        // already used.
        let known_path: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM input_data WHERE path = $1)",
        )
        .bind(&file.path)
        .fetch_one(&mut *tx)
        .await?;
        let known_exact: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM input_data WHERE path = $1 AND sha256 = $2)",
        )
        .bind(&file.path)
        .bind(&file.sha256)
        .fetch_one(&mut *tx)
        .await?;

        let disposition = if known_exact {
            "known"
        } else if known_path {
            "collision"
        } else {
            "new"
        };

        sqlx::query(
            "INSERT INTO input_data_import_rows
                 (import_id, path, role, name, sha256, bytes, disposition, content)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (import_id, path, sha256) DO NOTHING",
        )
        .bind(import_id)
        .bind(&file.path)
        .bind(&file.role)
        .bind(&file.name)
        .bind(&file.sha256)
        .bind(file.bytes)
        .bind(disposition)
        .bind(file.content.as_deref())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(tarball_sha256)
}

/// Startup reaper. Single instance, so a row left `running` belongs to a
/// process that is gone.
pub async fn fail_orphaned_imports(pool: &sqlx::PgPool) -> AppResult<u64> {
    let result = sqlx::query(
        "UPDATE input_data_imports
         SET state = 'failed',
             error = 'the server restarted while this import was running'
         WHERE state = 'running'",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_suffixes_match_the_download_script() {
        assert_eq!(chunk_suffix(0), "aa");
        assert_eq!(chunk_suffix(1), "ab");
        assert_eq!(chunk_suffix(25), "az");
        assert_eq!(chunk_suffix(26), "ba");
    }

    #[test]
    fn classifies_the_paths_birdtest_pins() {
        assert_eq!(
            classify("data/lexica/NWL23.kwg"),
            Some(("lexica/NWL23.kwg".into(), "kwg".into(), "NWL23".into()))
        );
        assert_eq!(
            classify("data/lexica/NWL23.klv2"),
            Some(("lexica/NWL23.klv2".into(), "klv".into(), "NWL23".into()))
        );
        assert_eq!(
            classify("data/letterdistributions/english.csv"),
            Some((
                "letterdistributions/english.csv".into(),
                "letterdist".into(),
                "english".into()
            ))
        );
        assert_eq!(
            classify("data/layouts/standard15.txt"),
            Some(("layouts/standard15.txt".into(), "layout".into(), "standard15".into()))
        );
        assert_eq!(
            classify("data/strategy/winpct.csv"),
            Some(("strategy/winpct.csv".into(), "winpct".into(), "winpct".into()))
        );
    }

    #[test]
    fn ignores_what_birdtest_does_not_pin() {
        // Unrecognised directories and suffixes are ignored rather than
        // rejected: MAGPIE-DATA carries more than birdtest pins.
        assert_eq!(classify("data/quackle/whatever.dat"), None);
        assert_eq!(classify("data/lexica/NWL23.wmp"), None);
        assert_eq!(classify("testdata/lexica/NWL23.kwg"), None);
        assert_eq!(classify("data/lexica/nested/NWL23.kwg"), None);
    }

    #[test]
    fn only_server_read_roles_keep_their_bytes() {
        assert!(keeps_content("letterdist"));
        assert!(keeps_content("layout"));
        // A 15 MB .kwg in a table row is a different proposition, and nothing
        // server-side reads one.
        assert!(!keeps_content("kwg"));
        assert!(!keeps_content("klv"));
        assert!(!keeps_content("winpct"));
    }

    /// Builds a gzipped tar in memory from (path, bytes) pairs.
    fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append_data(&mut header, path, *bytes).unwrap();
        }
        let tar = builder.into_inner().unwrap();
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn walks_a_well_formed_archive() {
        let archive = tarball(&[
            ("data/letterdistributions/english.csv", b"A,a,9,1,1\n" as &[u8]),
            ("data/lexica/NWL23.kwg", b"kwg-bytes"),
            ("data/quackle/ignored.dat", b"whatever"),
        ]);
        let files = walk_archive(&archive, None).unwrap();
        assert_eq!(files.len(), 2, "the unrecognised directory is skipped");

        let ld = files.iter().find(|f| f.role == "letterdist").unwrap();
        assert_eq!(ld.name, "english");
        assert_eq!(ld.sha256, hex::encode(Sha256::digest(b"A,a,9,1,1\n")));
        assert_eq!(ld.content.as_deref(), Some(b"A,a,9,1,1\n" as &[u8]));

        let kwg = files.iter().find(|f| f.role == "kwg").unwrap();
        assert!(kwg.content.is_none(), "lexica are not stored in the row");
    }

    #[test]
    fn rejects_a_symlink_entry() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_mode(0o777);
        builder
            .append_link(&mut header, "data/lexica/evil.kwg", "/etc/passwd")
            .unwrap();
        let tar = builder.into_inner().unwrap();
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        let archive = encoder.finish().unwrap();

        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("non-regular"), "{}", err.message);
    }

    #[test]
    fn rejects_a_traversing_path() {
        // The name is written into the header directly: `Builder::append_data`
        // refuses to *create* a traversing path, which is exactly why the walk
        // cannot assume archives it receives were built by a well-behaved
        // writer.
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        let name = b"data/../../etc/passwd";
        header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
        header.set_cksum();
        builder.append(&header, b"nope" as &[u8]).unwrap();
        let tar = builder.into_inner().unwrap();
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        let archive = encoder.finish().unwrap();

        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("unsafe path"), "{}", err.message);
    }

    #[test]
    fn rejects_an_archive_with_nothing_recognisable() {
        let archive = tarball(&[("data/quackle/only.dat", b"x" as &[u8])]);
        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("no recognisable"), "{}", err.message);
    }

    #[test]
    fn rejects_a_bomb_by_ratio() {
        // Highly compressible content: a megabyte of zeros gzips to a few
        // hundred bytes, which trips the ratio long before any total cap.
        let payload = vec![0u8; 4 * 1024 * 1024];
        let archive = tarball(&[("data/lexica/BOMB.kwg", payload.as_slice())]);
        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("expands"), "{}", err.message);
    }
}
