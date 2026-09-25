//! Importing a MAGPIE-DATA versioned tarball into `input_data`.
//!
//! Two phases. Phase 1 fetches the tarball, hashes every entry, and stages a
//! diff against what is already known; phase 2 inserts the new rows on the
//! admin's confirmation. Neither is on the dispatch path -- dispatch reads
//! local tables only, so GitHub can be down for a week without a worker
//! noticing.
//!
//! Phase 1 runs as a spawned task rather than inside the request: the archive
//! is ~190 MB, and a request that waits that long needs a timeout far above the
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
    /// The real tarball is ~190 MB (`data-20251004.tgz`, five chunks).
    pub const COMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
    /// `download_data.sh` stops at 26 (`aa`..`az`); this is headroom without
    /// being unbounded.
    pub const CHUNKS: u32 = 64;
    pub const UNCOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
    /// Real gzip on this content runs about 1.3x (190 MB to 250 MB); a bomb is
    /// thousands. Checked
    /// continuously rather than at the end, which is the case a total cap alone
    /// lets through.
    pub const EXPANSION_RATIO: u64 = 20;
    /// The largest real entry is a ~15 MB .kwg.
    pub const ENTRY_BYTES: u64 = 128 * 1024 * 1024;
    /// MAGPIE-DATA has low hundreds of files.
    pub const ENTRIES: usize = 5_000;
    /// A PAX extension header or GNU long name is a few hundred bytes.
    pub const EXTENSION_BYTES: u64 = 64 * 1024;
}

/// The decompressed stream, counted: every byte the tar reader takes, entry
/// data, headers and skipped data alike, against the total and the ratio. The
/// walk reads the archive raw (see [`walk_archive`]), so nothing is held that
/// the walk did not ask for; this bounds the work of what it skips.
struct Capped<R> {
    inner: R,
    read: u64,
    limit: u64,
}

impl<R: std::io::Read> std::io::Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if self.read > self.limit {
            return Err(std::io::Error::other(format!(
                "archive expands past the {} GiB cap, or more than {}x what was downloaded",
                limits::UNCOMPRESSED_BYTES / 1024 / 1024 / 1024,
                limits::EXPANSION_RATIO
            )));
        }
        Ok(n)
    }
}

fn decompressed(compressed: &[u8]) -> Capped<flate2::read::GzDecoder<&[u8]>> {
    Capped {
        inner: flate2::read::GzDecoder::new(compressed),
        read: 0,
        // What the stream bounds is only work: an entry's data is judged
        // before it is read and an extension header is refused past 64 KiB, so
        // what is decompressed here is held nowhere. The floor is for archives
        // of many small entries, whose headers alone compress eighty to one.
        limit: limits::UNCOMPRESSED_BYTES
            .min(limits::EXPANSION_RATIO.saturating_mul(compressed.len() as u64).max(64 * 1024 * 1024)),
    }
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
    /// Where these bytes belong in the object store, for the roles the server
    /// builds derived files from; see `input_data.object_key`.
    pub object_key: Option<String>,
    /// The bytes destined for the object store, taken by [`upload_inputs`].
    ///
    /// `None` once uploaded, and for every role that stores nothing -- so an
    /// archive's worth of lexica is held while it is being uploaded and not
    /// for the rest of the import. Separate from `content`, which stays on the
    /// row for the whole staging pass because confirmation writes it.
    pub stored: Option<Vec<u8>>,
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
    // A name MAGPIE would refuse is not importable. Every name a task carries
    // becomes a path on the worker, which checks it against one rule
    // (`[A-Za-z0-9_-]+`; a rack info table joins a lexicon and leaves with one
    // `.`) and fails the task otherwise -- after claiming it, as a failure, so
    // a job pinned to `CSW24.v2` would have stopped every worker it reached.
    // Skipped like any file birdtest does not pin, so no job can be made on it.
    if name.is_empty()
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some((format!("{dir}/{basename}"), role.to_string(), name))
}

/// Roles whose bytes are stored on the row, because the server reads them
/// itself and must read exactly what the job pins.
fn keeps_content(role: &str) -> bool {
    matches!(role, "letterdist" | "layout")
}

/// Roles whose bytes go to the object store, because the server builds derived
/// files from them.
///
/// Not the row, unlike [`keeps_content`]: a 6 MB lexicon and a 3.7 MB KLV per
/// row is a different proposition from a 489-byte distribution, nothing
/// queries their contents, and a full tarball is fifty of each. The key is the
/// digest, so two tarballs carrying the same lexicon upload it once.
///
/// Win% models stay out. The server neither reads one nor builds anything from
/// one -- only a simulating player on a worker opens it -- so storing it would
/// be 800 KB a row for nothing.
fn stores_object(role: &str) -> bool {
    matches!(role, "kwg" | "klv")
}

/// The object-store key for a file's bytes. Keyed by digest, not by path:
/// what a derived build needs is these exact bytes, and the same bytes under
/// two paths are one object.
pub fn input_object_key(sha256: &str) -> String {
    format!("inputs/{sha256}")
}

/// Resolves a git ref to a commit sha, so `main` is pinned at import time and
/// the record names a commit rather than a branch.
pub async fn resolve_ref(state: &AppState, git_ref: &str) -> AppResult<String> {
    // A ref is a git ref name. Put into the URL as given, `..` segments
    // resolved -- `../../../user` asked GitHub's API for another endpoint,
    // with the server's token -- and `?` or `#` rewrote the query.
    let well_formed = !git_ref.is_empty()
        && git_ref.len() <= 255
        && !git_ref.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
        && git_ref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    if !well_formed {
        return Err(AppError::bad_request("that is not a git ref name")
            .with_field("git_ref", "letters, digits, '-', '_', '.' and '/' only"));
    }
    let url = format!(
        "{}/repos/{}/commits/{}",
        state.cfg.github_api_url, state.cfg.magpie_data_repo, git_ref
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
            "GitHub returned something that is not a commit sha: {:?}",
            sha.chars().take(80).collect::<String>()
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
        "{}/{}/{}/versioned-tarballs/data-{}.tgz",
        state.cfg.github_raw_url, state.cfg.magpie_data_repo, commit_sha, tarball_date
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
///
/// **Raw, and extensions read here.** The tar reader, left to interpret
/// extensions itself, reads and holds data inside its own `next()`, where no
/// cap of the walk's can reach: a PAX header's or a GNU long name's data whole
/// (a 400 KB gzip held 465 MB), a PAX `size` that makes the reader and a first
/// pass disagree on where every later header starts, and GNU sparse extension
/// blocks expanded to 2.6 times their size (1.1 GB from a 44 MB gzip). Three
/// patches in the thirty-first audit each closed one; this replaced them. The
/// reader runs raw -- every header is an entry, and every entry's data is the
/// size its own header gives -- and the walk reads an extension header's data
/// itself, bounded, taking from it only a path or a link target for the entry
/// after it. A PAX `size` or sparse record, and a sparse entry, are refused:
/// nothing a data release contains needs one.
pub fn walk_archive(compressed: &[u8], progress: Option<&Progress>) -> AppResult<Vec<ImportedFile>> {
    let mut archive = tar::Archive::new(decompressed(compressed));
    let mut files = Vec::new();
    // Symlinks at pinned paths, resolved once every regular file is read:
    // (link's mapped path, role, name, target as written, target's archive
    // path).
    let mut links: Vec<(String, String, String, String, String)> = Vec::new();
    let mut total_uncompressed: u64 = 0;
    let mut entry_count: usize = 0;
    // Every pinned path, file or alias: a tarball naming one twice (an
    // appended `tar -r`, or a hostile one) staged and confirmed a row for
    // each, of which a worker extracting it holds only the last.
    let mut seen = std::collections::HashSet::new();

    let entries = archive
        .entries()
        .map_err(|e| AppError::bad_request(format!("not a readable tar archive: {e}")))?
        .raw(true);
    // A long name, or a PAX `path` or `linkpath`, for the entry after it.
    let mut next_path: Option<String> = None;
    let mut next_link: Option<String> = None;

    for entry in entries {
        let mut entry =
            entry.map_err(|e| AppError::bad_request(format!("malformed tar entry: {e}")))?;

        let entry_type = entry.header().entry_type();
        let pax = entry_type.is_pax_local_extensions() || entry_type.is_pax_global_extensions();
        let extension = pax || entry_type.is_gnu_longname() || entry_type.is_gnu_longlink();
        // Every entry, not every pinned file: directories, symlinks and
        // unrecognised paths are what a hostile archive would multiply. Not an
        // extension header, which a POSIX-format tarball writes one of per
        // entry, and whose bytes are counted below.
        entry_count += usize::from(!extension);
        if entry_count > limits::ENTRIES {
            return Err(AppError::bad_request(format!(
                "archive has more than {} entries",
                limits::ENTRIES
            )));
        }

        // The size of the data that follows this header, which is what the
        // raw reader reads or skips: counted before a byte of it is read,
        // whatever the entry is.
        let size = entry
            .header()
            .size()
            .map_err(|e| AppError::bad_request(format!("unreadable entry size: {e}")))?;
        // Each kind's own bound first, for the clearer refusal, then the
        // running total and the ratio.
        if extension && size > limits::EXTENSION_BYTES {
            return Err(AppError::bad_request(format!(
                "archive has an extension header larger than {} KiB",
                limits::EXTENSION_BYTES / 1024
            )));
        }
        if entry_type.is_file() && size > limits::ENTRY_BYTES {
            let path = next_path.clone().or_else(|| entry.path().ok().map(|p| p.to_string_lossy().to_string()));
            return Err(AppError::bad_request(format!(
                "{} is larger than the {} MiB per-entry cap",
                path.unwrap_or_default(),
                limits::ENTRY_BYTES / 1024 / 1024
            )));
        }
        total_uncompressed += size;
        check_expansion(total_uncompressed, compressed.len() as u64)?;

        if extension {
            let mut data = Vec::with_capacity(size as usize);
            (&mut entry)
                .take(limits::EXTENSION_BYTES + 1)
                .read_to_end(&mut data)
                .map_err(|e| AppError::bad_request(format!("unreadable extension header: {e}")))?;
            // One name for an entry: GNU tar, given two (a second PAX header, a
            // long name beside a PAX path), extracts under one the walk would
            // not have chosen, and so pins bytes under a name no worker has.
            let two_names = || {
                AppError::bad_request("archive gives one entry two names, which extractors resolve differently")
            };
            if !pax {
                // A GNU long name is NUL-terminated.
                let text = String::from_utf8_lossy(data.split(|b| *b == 0).next().unwrap_or(&[])).to_string();
                let slot = if entry_type.is_gnu_longname() { &mut next_path } else { &mut next_link };
                if slot.replace(text).is_some() {
                    return Err(two_names());
                }
                continue;
            }
            for record in tar::PaxExtensions::new(&data) {
                let record =
                    record.map_err(|e| AppError::bad_request(format!("malformed PAX record: {e}")))?;
                let key = record.key().unwrap_or("");
                if key == "size" || key.starts_with("GNU.sparse") {
                    return Err(AppError::bad_request(format!(
                        "archive has a PAX `{key}` record, which no data release needs"
                    )));
                }
                if matches!(key, "path" | "linkpath") {
                    // A global header's name would apply to every entry after
                    // it, for an extractor that honours it.
                    if entry_type.is_pax_global_extensions() {
                        return Err(AppError::bad_request(format!(
                            "archive has a global PAX `{key}` record"
                        )));
                    }
                    let value = String::from_utf8_lossy(record.value_bytes()).to_string();
                    let slot = if key == "path" { &mut next_path } else { &mut next_link };
                    if slot.replace(value).is_some() {
                        return Err(two_names());
                    }
                }
            }
            continue;
        }
        let long_path = next_path.take();
        let long_link = next_link.take();

        if entry_type.is_dir() {
            continue;
        }
        // A device or hard link has no business in a data tarball, and
        // skipping it quietly would make a hostile archive look ordinary. A
        // symlink is resolved below, within the archive, or refused the same
        // way.
        if !entry_type.is_file() && !entry_type.is_symlink() {
            return Err(AppError::bad_request(format!(
                "archive contains a non-regular entry ({entry_type:?})"
            )));
        }

        let path = match long_path {
            Some(path) => path,
            None => entry
                .path()
                .map_err(|e| AppError::bad_request(format!("unreadable entry path: {e}")))?
                .to_string_lossy()
                .to_string(),
        };
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(AppError::bad_request(format!(
                "archive contains an unsafe path: {path:?}"
            )));
        }

        // MAGPIE-DATA ships aliases as symlinks -- `CSW24_super21.klv2 ->
        // CSW21_super21.klv2` -- and `download_data.sh` extracts them as such,
        // so a worker hashing the alias reads the target's bytes. The alias
        // is pinned with those bytes too. One outside a pinned directory is
        // skipped like any other unpinned entry; one that points anywhere but
        // at a pinned file of the same kind in this archive is refused.
        if entry_type.is_symlink() {
            let Some((mapped_path, role, name)) = classify(&path) else {
                continue;
            };
            let target = long_link.unwrap_or_else(|| {
                entry
                    .link_name()
                    .ok()
                    .flatten()
                    .map(|target| target.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
            let Some(resolved) = resolve_link(&path, &target) else {
                return Err(unresolvable_link(&path, &target));
            };
            if !seen.insert(mapped_path.clone()) {
                return Err(twice(&path));
            }
            links.push((mapped_path, role, name, target, resolved));
            continue;
        }

        // `size` is the data's own: a PAX `size`, which would override it for
        // an interpreting reader, is refused above.
        if size > limits::ENTRY_BYTES {
            return Err(AppError::bad_request(format!(
                "{path} is larger than the {} MiB per-entry cap",
                limits::ENTRY_BYTES / 1024 / 1024
            )));
        }

        let Some((mapped_path, role, name)) = classify(&path) else {
            // An unrecognised directory: counted against the caps above, but
            // not something birdtest pins.
            continue;
        };
        if !seen.insert(mapped_path.clone()) {
            return Err(twice(&path));
        }

        let mut bytes = Vec::with_capacity(size as usize);
        (&mut entry)
            .take(limits::ENTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::bad_request(format!("could not read {path}: {e}")))?;
        if bytes.len() as u64 != size {
            return Err(AppError::bad_request(format!("{path} is not the size its header gives")));
        }

        let sha256 = hex::encode(Sha256::digest(&bytes));
        let content = keeps_content(&role).then(|| bytes.clone());
        let stores_object_bytes = stores_object(&role);
        let object_key = stores_object_bytes.then(|| input_object_key(&sha256));
        files.push(ImportedFile {
            path: mapped_path,
            role,
            name,
            sha256,
            bytes: bytes.len() as i64,
            content,
            object_key,
            stored: stores_object_bytes.then_some(bytes),
        });
        if let Some(progress) = progress {
            progress.entries(files.len() as i32);
        }
    }

    for (mapped_path, role, name, written, target) in &links {
        // A chain of aliases is followed a few hops, never round a cycle.
        let mut target = target.clone();
        let mut resolved = None;
        for _ in 0..8 {
            let Some((target_mapped, target_role, _)) = classify(&target) else {
                break;
            };
            if &target_role != role {
                break;
            }
            if let Some(file) = files.iter().find(|f| f.path == target_mapped) {
                resolved = Some(file);
                break;
            }
            match links.iter().find(|(link, ..)| *link == target_mapped) {
                Some((.., next)) => target = next.clone(),
                None => break,
            }
        }
        let Some(file) = resolved else {
            return Err(unresolvable_link(&format!("data/{mapped_path}"), written));
        };
        let alias = ImportedFile {
            path: mapped_path.clone(),
            role: role.clone(),
            name: name.clone(),
            sha256: file.sha256.clone(),
            bytes: file.bytes,
            content: file.content.clone(),
            object_key: file.object_key.clone(),
            // The target's own row uploads the bytes; the key is the digest.
            stored: None,
        };
        files.push(alias);
    }

    if files.is_empty() {
        return Err(AppError::bad_request(
            "archive contained no recognisable data files",
        ));
    }
    Ok(files)
}

/// A symlink's target as an archive path, when it stays inside the archive's
/// `data/` tree. Relative to the link's own directory, as the filesystem
/// `download_data.sh` extracts into would read it.
fn resolve_link(link_path: &str, target: &str) -> Option<String> {
    if target.is_empty() || target.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = link_path.split('/').collect();
    parts.pop();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    let resolved = parts.join("/");
    resolved.starts_with("data/").then_some(resolved)
}

fn twice(path: &str) -> AppError {
    AppError::bad_request(format!("archive names {path} more than once"))
}

fn unresolvable_link(path: &str, target: &str) -> AppError {
    AppError::bad_request(format!(
        "archive contains a non-regular entry (Symlink) at {path} whose target {target:?} \
         is not a data file of the same kind in the archive"
    ))
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
    run_import_within(state, import_id, tarball_date, commit_sha, IMPORT_LIMIT).await
}

/// How long an import may take, download to staged (PLAN.md, "Whole task").
/// The HTTP client's timeouts are per connection and per read, so a download
/// that kept trickling kept its import `running`, holding up to the download
/// cap in memory, for as long as the trickle lasted (thirty-first audit).
pub const IMPORT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// [`run_import`] with its time limit given.
pub async fn run_import_within(
    state: AppState,
    import_id: Uuid,
    tarball_date: String,
    commit_sha: String,
    limit: std::time::Duration,
) {
    let progress = std::sync::Arc::new(Progress::new(state.pool.clone(), import_id));
    let staged = tokio::time::timeout(limit, stage(&state, import_id, &tarball_date, &commit_sha, &progress))
        .await
        .unwrap_or_else(|_| {
            Err(AppError::bad_request(format!(
                "the import did not finish within its {} s; start it again",
                limit.as_secs()
            )))
        });
    match staged {
        Ok(staged) => {
            progress.flush_entries().await;
            // Guarded on `running`: a process that starts reaps rows left
            // `running` as failed, and a rollback or a manual restart could
            // overlap two for a moment. Without the guard this would
            // flip a reaped row back to `staged` with the reaper's error still
            // on it, and the admin would confirm a diff nobody was sure of.
            let _ = sqlx::query(
                "UPDATE input_data_imports SET state = 'staged', tarball_sha256 = $2
                 WHERE id = $1 AND state = 'running'",
            )
            .bind(import_id)
            .bind(staged)
            .execute(&state.pool)
            .await;
        }
        Err(err) => {
            tracing::warn!(%import_id, error = %err.message, "input data import failed");
            let _ = sqlx::query(
                "UPDATE input_data_imports SET state = 'failed', error = $2
                 WHERE id = $1 AND state = 'running'",
            )
            .bind(import_id)
            .bind(&err.message)
            .execute(&state.pool)
            .await;
        }
    }
}

/// Puts every lexicon and leaves file into the object store, and takes its
/// bytes back out of memory.
///
/// Uploaded at staging time because that is when the bytes exist: the archive
/// is in memory, and re-downloading it at confirmation would mean fetching
/// 190 MB again for files the server has already hashed.
///
/// An object already present is skipped. Keys are digests, so a file unchanged
/// between two tarballs -- which is most of them -- is uploaded once, and an
/// import the admin then cancels leaves behind exactly what the next import
/// would have uploaded anyway.
async fn upload_inputs(state: &AppState, files: &mut [ImportedFile]) -> AppResult<()> {
    let mut uploaded = 0;
    for file in files.iter_mut() {
        let (Some(key), Some(bytes)) = (file.object_key.clone(), file.stored.take()) else {
            continue;
        };
        if state.artifacts.exists(&key).await? {
            continue;
        }
        state.artifacts.put(&key, bytes).await?;
        uploaded += 1;
    }
    tracing::info!(uploaded, "stored lexicon and leaves bytes for derived builds");
    Ok(())
}

async fn stage(
    state: &AppState,
    import_id: Uuid,
    tarball_date: &str,
    commit_sha: &str,
    progress: &std::sync::Arc<Progress>,
) -> AppResult<String> {
    let (body, tarball_sha256) = download(state, commit_sha, tarball_date, progress).await?;
    // On the blocking pool: gunzipping, untarring and hashing a whole tarball
    // is seconds of computation with no `await` in it, and an async worker
    // thread that does not yield can stall every other request the server has
    // (see `exports::upload_rows`). The archive goes in and comes back out,
    // since the uploads below still read from it.
    let (mut files, body) = {
        let progress = progress.clone();
        tokio::task::spawn_blocking(move || {
            let files = walk_archive(&body, Some(&progress))?;
            Ok::<_, AppError>((files, body))
        })
        .await
        .map_err(|e| AppError::internal(format!("reading the archive failed: {e}")))??
    };
    // Before anything is staged: a row that names an object has to be a row
    // whose object is there, or the first derived build from it fails with a
    // missing key rather than a reason. Uploading before the transaction also
    // keeps a multi-minute upload out of it.
    upload_inputs(state, &mut files).await?;
    // The whole archive is no longer needed once its entries are in `files`,
    // and it is about 300 MB uncompressed.
    drop(body);

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
                 (import_id, path, role, name, sha256, bytes, disposition, content,
                  object_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
        .bind(file.object_key.as_deref())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(tarball_sha256)
}

/// How long a staged import waits for the admin's confirmation before it is
/// expired. The diff it shows is against `input_data` as it was when the
/// import ran, so an old one is a proposal about a vocabulary that may have
/// moved on; and each staged row keeps the bytes of its letter distributions
/// and layouts, which is storage held for a decision nobody is going to make.
pub const UNCONFIRMED_IMPORT_TTL: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);

/// Expires staged imports older than [`UNCONFIRMED_IMPORT_TTL`]: the staged
/// rows -- and the bytes they carry -- are deleted, and the import is marked
/// `cancelled` with a reason, so the admin page says what happened to it
/// rather than showing a gap. The objects an import uploaded stay: they are
/// keyed by digest and are exactly what the next import of the same files
/// would upload anyway.
///
/// Returns how many imports were expired.
pub async fn expire_unconfirmed_imports(pool: &sqlx::PgPool) -> AppResult<u64> {
    let mut tx = pool.begin().await?;
    let expired: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE input_data_imports
         SET state = 'cancelled',
             error = 'not confirmed within 24 hours; start the import again to review it'
         WHERE state = 'staged'
           AND requested_at < now() - make_interval(secs => $1)
         RETURNING id",
    )
    .bind(UNCONFIRMED_IMPORT_TTL.as_secs_f64())
    .fetch_all(&mut *tx)
    .await?;
    if !expired.is_empty() {
        sqlx::query("DELETE FROM input_data_import_rows WHERE import_id = ANY($1)")
            .bind(&expired)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(expired.len() as u64)
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
        // A name the worker would refuse as a path is not importable.
        assert_eq!(classify("data/lexica/CSW24.v2.kwg"), None);
        assert_eq!(classify("data/lexica/CSW 24.kwg"), None);
        assert_eq!(classify("data/lexica/.kwg"), None);
        assert!(classify("data/lexica/CSW21_ab-2.kwg").is_some());
        assert_eq!(classify("data/lexica/NWL23.wmp"), None);
        assert_eq!(classify("testdata/lexica/NWL23.kwg"), None);
        assert_eq!(classify("data/lexica/nested/NWL23.kwg"), None);
    }

    /// The server builds a reference wordmap from a lexicon and a reference
    /// rack info table from a lexicon and its leaves, so those two roles have
    /// to be fetchable afterwards. A win% model is neither read by the server
    /// nor built from, so storing one would be 800 KB a row for nothing.
    #[test]
    fn the_roles_a_derived_build_needs_go_to_the_object_store() {
        assert!(stores_object("kwg"));
        assert!(stores_object("klv"));
        assert!(!stores_object("winpct"));
        assert!(!stores_object("letterdist"));
        assert!(!stores_object("layout"));
        // Keyed by digest, so the same bytes under two paths are one object
        // and a file unchanged between tarballs is uploaded once.
        assert_eq!(input_object_key("abc123"), "inputs/abc123");
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
        // They go to the object store instead, so the server can build a
        // reference wordmap from them. Keyed by digest: the same bytes in two
        // tarballs are one object.
        assert_eq!(kwg.object_key.as_deref(), Some(input_object_key(&kwg.sha256).as_str()));
        assert_eq!(kwg.stored.as_deref(), Some(b"kwg-bytes" as &[u8]));
        assert!(ld.object_key.is_none(), "the server reads a distribution off its row");
        assert!(ld.stored.is_none());
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

    /// A tar of regular files and then symlinks, as (path, target) pairs --
    /// the order MAGPIE-DATA's own tarball happens to put the targets in last
    /// is covered by listing a link before its target.
    fn tarball_with_links(files: &[(&str, &[u8])], links: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, target) in links {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_mode(0o777);
            builder.append_link(&mut header, path, target).unwrap();
        }
        for (path, bytes) in files {
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

    /// data-20251004.tgz, as re-cut upstream, carries aliases as symlinks
    /// (`data/lexica/CSW24_super21.klv2 -> CSW21_super21.klv2`, and a chain
    /// of them) and refusing them refused the whole release. An alias is
    /// pinned with the bytes `download_data.sh` leaves a worker reading
    /// through it: its target's.
    #[test]
    fn a_symlink_alias_is_pinned_with_its_targets_bytes() {
        let archive = tarball_with_links(
            &[("data/lexica/CSW21_super21.klv2", b"klv-bytes" as &[u8])],
            &[
                ("data/lexica/CSW24_super21.klv2", "CSW21_super21.klv2"),
                ("data/lexica/CSW15_super21.klv2", "./CSW24_super21.klv2"),
                ("data/lexica/CSW12_super21.klv2", "../lexica/CSW21_super21.klv2"),
                // Outside anything birdtest pins: skipped, as a file there is.
                ("data/strategy/NWL23_super21.pat", "CSW24_super21.pat"),
            ],
        );
        let files = walk_archive(&archive, None).unwrap();
        assert_eq!(files.len(), 4, "{files:?}");
        let digest = hex::encode(Sha256::digest(b"klv-bytes"));
        for name in ["CSW21_super21", "CSW24_super21", "CSW15_super21", "CSW12_super21"] {
            let file = files.iter().find(|f| f.name == name).unwrap();
            assert_eq!((file.role.as_str(), file.sha256.as_str()), ("klv", digest.as_str()));
            assert_eq!(file.path, format!("lexica/{name}.klv2"));
            assert_eq!(file.object_key.as_deref(), Some(input_object_key(&digest).as_str()));
        }
        // One upload for the one set of bytes.
        assert_eq!(files.iter().filter(|f| f.stored.is_some()).count(), 1);
    }

    /// A symlink at a pinned path that leaves the archive, dangles, loops, or
    /// names a file of another kind is refused rather than skipped: it is
    /// exactly what a hostile archive would use to look ordinary.
    #[test]
    fn a_symlink_that_is_not_an_alias_inside_the_archive_is_refused() {
        let files: &[(&str, &[u8])] =
            &[("data/lexica/NWL23.kwg", b"kwg"), ("data/lexica/NWL23.klv2", b"klv")];
        for (link, target) in [
            ("data/lexica/evil.klv2", "../../../etc/passwd"),
            ("data/lexica/evil.klv2", "/etc/passwd"),
            ("data/lexica/evil.klv2", "missing.klv2"),
            ("data/lexica/evil.klv2", "NWL23.kwg"),
            ("data/lexica/evil.klv2", "evil.klv2"),
            ("data/lexica/evil.klv2", "../strategy/winpct.csv"),
        ] {
            let archive = tarball_with_links(files, &[(link, target)]);
            let err = walk_archive(&archive, None).unwrap_err();
            assert!(err.message.contains("non-regular"), "{target}: {}", err.message);
            assert!(err.message.contains(target), "{target}: {}", err.message);
        }
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

    /// Gzips a raw tar stream built by the caller, header by header.
    fn gzip(tar: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, tar).unwrap();
        encoder.finish().unwrap()
    }

    /// A header of the given type, path and claimed size, checksummed.
    fn header(path: &str, entry_type: tar::EntryType, size: u64) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_size(size);
        header.set_mode(0o644);
        header.set_entry_type(entry_type);
        header.set_cksum();
        header
    }

    /// PLAN.md, "Limits, all enforced during the walk": single entry 128 MiB.
    /// Judged from the header, before a byte of the entry is read: the archive
    /// here is a header claiming one byte over the cap and *no data at all*,
    /// so a walk that read first would fail on the truncation instead.
    #[test]
    fn an_entry_whose_header_claims_more_than_the_per_entry_cap_is_refused_unread() {
        let claimed = limits::ENTRY_BYTES + 1;
        let archive = gzip(header("data/lexica/HUGE.kwg", tar::EntryType::Regular, claimed).as_bytes());
        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("per-entry cap"), "{}", err.message);
        assert!(err.message.contains("data/lexica/HUGE.kwg"), "{}", err.message);

        // Exactly at the cap is not refused by that rule: the same truncated
        // archive then fails on the missing bytes, which is what proves the
        // refusal above came before the read.
        let archive = gzip(
            header("data/lexica/HUGE.kwg", tar::EntryType::Regular, limits::ENTRY_BYTES).as_bytes(),
        );
        let err = walk_archive(&archive, None).unwrap_err();
        assert!(!err.message.contains("per-entry cap"), "{}", err.message);
    }

    /// PLAN.md's allowlist: regular files only (plus aliases resolved inside
    /// the archive). A hard link, a character or block device and a FIFO are
    /// each refused rather than skipped, even at a path birdtest ignores.
    #[test]
    fn hard_link_and_device_entries_are_refused() {
        for (entry_type, path) in [
            (tar::EntryType::Link, "data/lexica/NWL23.kwg"),
            (tar::EntryType::Link, "data/quackle/linked.dat"),
            (tar::EntryType::Char, "data/lexica/tty.kwg"),
            (tar::EntryType::Block, "data/quackle/sda"),
            (tar::EntryType::Fifo, "data/layouts/pipe.txt"),
        ] {
            let mut builder = tar::Builder::new(Vec::new());
            let mut regular = header("data/lexica/REAL.kwg", tar::EntryType::Regular, 3);
            builder.append_data(&mut regular, "data/lexica/REAL.kwg", b"kwg" as &[u8]).unwrap();
            let mut odd = header(path, entry_type, 0);
            if entry_type == tar::EntryType::Link {
                odd.set_link_name("data/lexica/REAL.kwg").unwrap();
                odd.set_cksum();
            }
            builder.append(&odd, std::io::empty()).unwrap();
            let archive = gzip(&builder.into_inner().unwrap());

            let err = walk_archive(&archive, None).unwrap_err();
            assert!(
                err.message.contains(&format!("non-regular entry ({entry_type:?})")),
                "{entry_type:?} at {path}: {}",
                err.message
            );
        }
    }

    /// PLAN.md: total uncompressed bytes 1 GiB. Every entry's size counts,
    /// pinned or not, and the cap holds where the ratio does not catch it --
    /// here nine unpinned entries of zeros total 1 GiB and a byte, and the
    /// compressed input is padded past 1/20th of that (trailing bytes after
    /// the gzip member, which the decoder never reads), so only the total can
    /// refuse it.
    #[test]
    fn an_archive_expanding_past_the_total_cap_is_refused() {
        let entry = limits::ENTRY_BYTES;
        let entries = limits::UNCOMPRESSED_BYTES / entry;
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let zeros = vec![0u8; 1024 * 1024];
        for i in 0..entries {
            let head = header(&format!("data/quackle/part{i}.dat"), tar::EntryType::Regular, entry);
            std::io::Write::write_all(&mut encoder, head.as_bytes()).unwrap();
            for _ in 0..entry / zeros.len() as u64 {
                std::io::Write::write_all(&mut encoder, &zeros).unwrap();
            }
        }
        // One byte more, at a path birdtest pins.
        let last = header("data/lexica/LAST.kwg", tar::EntryType::Regular, 1);
        std::io::Write::write_all(&mut encoder, last.as_bytes()).unwrap();
        std::io::Write::write_all(&mut encoder, &[b'x'; 512]).unwrap();
        std::io::Write::write_all(&mut encoder, &[0u8; 1024]).unwrap();
        let mut archive = encoder.finish().unwrap();
        let ratio_floor = (limits::UNCOMPRESSED_BYTES + 1) / limits::EXPANSION_RATIO + 1;
        archive.resize(ratio_floor as usize, 0);

        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("GiB cap"), "{}", err.message);
    }

    /// PLAN.md: entry count 5,000 -- entries, not pinned files. Directories,
    /// symlinks and files at paths birdtest ignores are all entries a hostile
    /// archive could multiply, and they once went uncounted: the check read
    /// the number of pinned files, so any number of the rest walked through.
    #[test]
    fn an_archive_of_more_than_the_entry_cap_is_refused_whatever_the_entries_are() {
        fn archive_of(unpinned: usize) -> Vec<u8> {
            let mut builder = tar::Builder::new(Vec::new());
            for i in 0..unpinned {
                let (path, entry_type) = match i % 3 {
                    0 => (format!("data/quackle/d{i}/"), tar::EntryType::Directory),
                    1 => (format!("data/quackle/f{i}.dat"), tar::EntryType::Regular),
                    _ => (format!("data/quackle/l{i}.dat"), tar::EntryType::Symlink),
                };
                let mut head = header(&path, entry_type, 0);
                if entry_type == tar::EntryType::Symlink {
                    head.set_link_name("f1.dat").unwrap();
                    head.set_cksum();
                }
                builder.append(&head, std::io::empty()).unwrap();
            }
            let mut pinned = header("data/lexica/NWL23.kwg", tar::EntryType::Regular, 3);
            builder.append_data(&mut pinned, "data/lexica/NWL23.kwg", b"kwg" as &[u8]).unwrap();
            gzip(&builder.into_inner().unwrap())
        }

        // 5,000 entries, one of them pinned: at the cap, accepted.
        let files = walk_archive(&archive_of(limits::ENTRIES - 1), None).unwrap();
        assert_eq!(files.len(), 1);

        // 5,001: refused, although only one of them is a pinned file.
        let err = walk_archive(&archive_of(limits::ENTRIES), None).unwrap_err();
        assert!(err.message.contains("more than 5000 entries"), "{}", err.message);
    }

    /// The limits are PLAN.md's table; a change to one is a design change.
    #[test]
    fn the_walk_limits_are_the_ones_the_design_states() {
        assert_eq!(limits::COMPRESSED_BYTES, 512 * 1024 * 1024);
        assert_eq!(limits::CHUNKS, 64);
        assert_eq!(limits::UNCOMPRESSED_BYTES, 1024 * 1024 * 1024);
        assert_eq!(limits::EXPANSION_RATIO, 20);
        assert_eq!(limits::ENTRY_BYTES, 128 * 1024 * 1024);
        assert_eq!(limits::ENTRIES, 5_000);
    }

    /// A gzipped tar of one pinned entry whose ustar header says 0 bytes while
    /// a PAX `size` record says `pax_size`, and no data at all: a walk that
    /// judges the entry before reading it refuses it on the PAX size; one that
    /// read the header's size read it (a truncation here, 900 MiB of zeros from
    /// a 4 MiB gzip in the thirty-first audit's reproduction).
    fn pax_sized(path: &str, pax_size: u64) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        builder.append_pax_extensions([("size", pax_size.to_string().as_bytes())]).unwrap();
        let mut head = tar::Header::new_ustar();
        head.set_path(path).unwrap();
        head.set_size(0);
        head.set_mode(0o644);
        head.set_entry_type(tar::EntryType::Regular);
        head.set_cksum();
        builder.append(&head, std::io::empty()).unwrap();
        let tar = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        encoder.finish().unwrap()
    }

    /// U-ARCHIVE-8: a PAX `size` record is refused, pinned entry or not: an
    /// interpreting reader follows it over the header's own field, which is how
    /// a header saying 0 and a record saying 900 MiB passed every cap, and how a
    /// record could make two passes disagree on where every later header
    /// starts. No data release needs one.
    #[test]
    fn a_pax_size_record_is_refused() {
        for (path, size) in [
            ("data/lexica/BIG.kwg", limits::ENTRY_BYTES + 1),
            ("data/lexica/BOMB.kwg", 100 * 1024 * 1024),
            ("data/quackle/skipped.dat", 100 * 1024 * 1024),
        ] {
            let err = walk_archive(&pax_sized(path, size), None).unwrap_err();
            assert!(err.message.contains("PAX `size` record"), "{path}: {}", err.message);
        }
    }

    /// U-ARCHIVE: a tarball naming one pinned path twice -- two files, or a file
    /// and an alias -- is refused; it staged and confirmed a row for each.
    #[test]
    fn an_archive_naming_a_path_twice_is_refused() {
        let archive = tarball(&[("data/lexica/NWL23.kwg", b"one" as &[u8]), ("data/lexica/NWL23.kwg", b"two")]);
        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("more than once"), "{}", err.message);

        let mut builder = tar::Builder::new(Vec::new());
        let mut head = tar::Header::new_gnu();
        head.set_size(3);
        head.set_mode(0o644);
        head.set_entry_type(tar::EntryType::Regular);
        head.set_cksum();
        builder.append_data(&mut head, "data/lexica/CSW21.klv2", &b"klv"[..]).unwrap();
        builder.append_data(&mut head, "data/lexica/CSW24.klv2", &b"klv"[..]).unwrap();
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_mode(0o777);
        builder.append_link(&mut link, "data/lexica/CSW24.klv2", "CSW21.klv2").unwrap();
        let tar = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        let err = walk_archive(&encoder.finish().unwrap(), None).unwrap_err();
        assert!(err.message.contains("more than once"), "a file and an alias: {}", err.message);
    }

    /// U-ARCHIVE-10: an extension header -- a PAX record or a GNU long name,
    /// which the tar reader reads whole before the walk sees an entry -- larger
    /// than a few KiB is refused before it is read, and every decompressed byte
    /// counts against the caps, headers and skipped data included. A 400 KB
    /// gzip with a 400 MB PAX header held 465 MB, and was accepted.
    #[test]
    fn a_large_extension_header_is_refused_before_it_is_read() {
        let mut builder = tar::Builder::new(Vec::new());
        let big = "x".repeat(1024 * 1024);
        builder.append_pax_extensions([("comment", big.as_bytes())]).unwrap();
        let mut head = tar::Header::new_ustar();
        head.set_path("data/lexica/A.kwg").unwrap();
        head.set_size(5);
        head.set_mode(0o644);
        head.set_entry_type(tar::EntryType::Regular);
        head.set_cksum();
        builder.append(&head, &b"bytes"[..]).unwrap();
        let tar = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        let err = walk_archive(&encoder.finish().unwrap(), None).unwrap_err();
        assert!(err.message.contains("extension header"), "{}", err.message);

        let mut builder = tar::Builder::new(Vec::new());
        let mut long = tar::Header::new_gnu();
        long.set_path("././@LongLink").unwrap();
        long.set_size(100 * 1024);
        long.set_mode(0o644);
        long.set_entry_type(tar::EntryType::GNULongName);
        long.set_cksum();
        builder.append(&long, &vec![b'a'; 100 * 1024][..]).unwrap();
        let tar = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &tar).unwrap();
        let err = walk_archive(&encoder.finish().unwrap(), None).unwrap_err();
        assert!(err.message.contains("extension header"), "a GNU long name: {}", err.message);
    }

    /// U-ARCHIVE-10: data the walk skips -- a directory entry claiming a size --
    /// is counted as it is decompressed, so a small gzip cannot make the server
    /// inflate gigabytes it never keeps.
    #[test]
    fn skipped_data_counts_against_the_caps() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let size = 96 * 1024 * 1024;
        let head = header("data/emptydir/", tar::EntryType::Directory, size);
        std::io::Write::write_all(&mut encoder, head.as_bytes()).unwrap();
        let zeros = vec![0u8; 1024 * 1024];
        for _ in 0..size / zeros.len() as u64 {
            std::io::Write::write_all(&mut encoder, &zeros).unwrap();
        }
        let archive = encoder.finish().unwrap();
        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("20x"), "{}", err.message);
    }

    fn peak_resident_mib() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
        line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() / 1024
    }

    /// U-ARCHIVE-11: a GNU sparse entry is refused as it is met, its extension
    /// blocks never expanded: an interpreting reader built 64 bytes of bookkeeping
    /// for every 24 of them, 1.1 GB from a 44 MB gzip, before the walk saw the
    /// entry to refuse it.
    #[test]
    fn a_sparse_entry_is_refused_unexpanded() {
        let blocks = 32 * 1024 * 1024 / 512;
        let mut head = tar::Header::new_gnu();
        head.set_path("data/other/sparse").unwrap();
        head.set_size(0);
        head.set_mode(0o644);
        head.set_entry_type(tar::EntryType::GNUSparse);
        {
            let gnu = head.as_gnu_mut().unwrap();
            gnu.isextended[0] = 1;
            gnu.set_real_size(blocks as u64 * 21);
        }
        head.set_cksum();
        let mut tar = head.as_bytes().to_vec();
        let mut offset = 0u64;
        for i in 0..blocks {
            let mut ext = [0u8; 512];
            for j in 0..21 {
                offset += 1;
                ext[j * 24..j * 24 + 12].copy_from_slice(format!("{offset:011o}\0").as_bytes());
                ext[j * 24 + 12..j * 24 + 24].copy_from_slice(b"00000000000\0");
            }
            ext[504] = u8::from(i + 1 < blocks);
            tar.extend_from_slice(&ext);
        }
        tar.extend_from_slice(&[0u8; 1024]);
        let archive = gzip(&tar);
        drop(tar);
        std::fs::write("/proc/self/clear_refs", "5").unwrap();
        let before = peak_resident_mib();
        let err = walk_archive(&archive, None).unwrap_err();
        assert!(err.message.contains("non-regular"), "{}", err.message);
        assert!(peak_resident_mib() < before + 16, "{} MiB held", peak_resident_mib() - before);
    }

    /// U-ARCHIVE-8: a PAX `size` record ahead of an ordinary entry cannot make
    /// the walk and the reader disagree on where the next header starts, and so
    /// slip a large extension header past its bound: the record itself is
    /// refused. (Against a first pass that read raw and a walk that did not, an
    /// 8 MiB PAX header after it was read whole and the archive accepted.)
    #[test]
    fn a_pax_size_record_cannot_move_the_next_header() {
        let mut builder = tar::Builder::new(Vec::new());
        builder.append_pax_extensions([("size", &b"1024"[..])]).unwrap();
        let mut pad = tar::Header::new_ustar();
        pad.set_path("data/other/pad").unwrap();
        pad.set_size(0);
        pad.set_mode(0o644);
        pad.set_entry_type(tar::EntryType::Regular);
        pad.set_cksum();
        builder.append(&pad, std::io::empty()).unwrap();
        let mut tar = builder.into_inner().unwrap();
        tar.extend_from_slice(&[0u8; 1024]);
        let mut rest = tar::Builder::new(Vec::new());
        let big = "x".repeat(8 * 1024 * 1024);
        rest.append_pax_extensions([("comment", big.as_bytes())]).unwrap();
        let mut kwg = tar::Header::new_ustar();
        kwg.set_path("data/lexica/A.kwg").unwrap();
        kwg.set_size(5);
        kwg.set_mode(0o644);
        kwg.set_entry_type(tar::EntryType::Regular);
        kwg.set_cksum();
        rest.append(&kwg, &b"bytes"[..]).unwrap();
        tar.extend_from_slice(&rest.into_inner().unwrap());
        let err = walk_archive(&gzip(&tar), None).unwrap_err();
        assert!(err.message.contains("PAX `size` record"), "{}", err.message);
    }

    /// U-ARCHIVE-12: what a release built by another tool carries still walks:
    /// a PAX `path` (and an `mtime`, which Python's tarfile writes for a
    /// fractional time) names the entry after it, and so does a GNU long name.
    #[test]
    fn pax_paths_and_long_names_name_the_entry_after_them() {
        let long = format!("data/lexica/{}.kwg", "L".repeat(120));
        let mut builder = tar::Builder::new(Vec::new());
        builder
            .append_pax_extensions([("mtime", &b"1759000000.25"[..]), ("path", &b"data/lexica/PAX.kwg"[..])])
            .unwrap();
        let mut head = tar::Header::new_ustar();
        head.set_path("data/lexica/truncated.kwg").unwrap();
        head.set_size(3);
        head.set_mode(0o644);
        head.set_entry_type(tar::EntryType::Regular);
        head.set_cksum();
        builder.append(&head, &b"pax"[..]).unwrap();
        let mut gnu = tar::Header::new_gnu();
        gnu.set_size(4);
        gnu.set_mode(0o644);
        gnu.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut gnu, &long, &b"long"[..]).unwrap();
        let files = walk_archive(&gzip(&builder.into_inner().unwrap()), None).unwrap();
        let mut named: Vec<(String, i64)> = files.iter().map(|f| (f.path.clone(), f.bytes)).collect();
        named.sort();
        assert_eq!(named, vec![(long.trim_start_matches("data/").to_string(), 4), ("lexica/PAX.kwg".to_string(), 3)]);
    }

    /// U-ARCHIVE-12: one name for one entry. A second PAX header, or a long
    /// name beside a PAX path, before an entry, and a global PAX `path` --
    /// each of which GNU tar resolves to a name the walk would not choose -- is
    /// refused; a global header of comments (as `git archive` writes) is not.
    #[test]
    fn an_entry_given_two_names_is_refused() {
        fn archive(extensions: &[(tar::EntryType, &[u8])]) -> Vec<u8> {
            let mut tar = Vec::new();
            for (kind, data) in extensions {
                let mut head = tar::Header::new_ustar();
                head.set_path("././@Ext").unwrap();
                head.set_size(data.len() as u64);
                head.set_mode(0o644);
                head.set_entry_type(*kind);
                head.set_cksum();
                tar.extend_from_slice(head.as_bytes());
                tar.extend_from_slice(data);
                tar.resize(tar.len().div_ceil(512) * 512, 0);
            }
            let mut head = tar::Header::new_ustar();
            head.set_path("data/lexica/HDR.kwg").unwrap();
            head.set_size(3);
            head.set_mode(0o644);
            head.set_entry_type(tar::EntryType::Regular);
            head.set_cksum();
            tar.extend_from_slice(head.as_bytes());
            tar.extend_from_slice(b"abc");
            tar.resize(tar.len().div_ceil(512) * 512 + 1024, 0);
            gzip(&tar)
        }
        // A record counts its own length digits: two here.
        let path = |name: &str| format!("{} path=data/lexica/{name}.kwg\n", 25 + name.len());
        let x = tar::EntryType::XHeader;
        let first = path("FIRST");
        let again = path("AGAIN");
        for (what, extensions) in [
            ("two PAX paths", vec![(x, first.as_bytes()), (x, again.as_bytes())]),
            ("a PAX path and a long name", vec![(x, first.as_bytes()), (tar::EntryType::GNULongName, &b"data/lexica/L.kwg\0"[..])]),
            ("a global path", vec![(tar::EntryType::XGlobalHeader, first.as_bytes())]),
        ] {
            let err = walk_archive(&archive(&extensions), None).unwrap_err();
            assert!(err.message.contains("two names") || err.message.contains("global PAX"), "{what}: {}", err.message);
        }
        let files = walk_archive(&archive(&[(tar::EntryType::XGlobalHeader, b"18 comment=abc123\n")]), None).unwrap();
        assert_eq!(files[0].path, "lexica/HDR.kwg", "a comment-only global header is fine");
    }
}
