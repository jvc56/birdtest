//! The pinned MAGPIE binary, run as a subprocess.
//!
//! The server used to contain no MAGPIE at all: it dispatched work, checked
//! digests, and built leave-generation KLVs with its own Rust translation of
//! MAGPIE's code. That translation had to be kept in step by hand, and there
//! was nothing to compare a worker's locally built wordmap or rack info table
//! against. Both problems have the same answer -- run the same builders the
//! workers run -- which is what this module is.
//!
//! **A subprocess, not a library.** MAGPIE can be built as `libmagpie.so`, but
//! its API is the same string-command interface as the CLI, so linking it buys
//! nothing over spawning it, and it would put MAGPIE's memory use (2.4 GB for
//! a rack info table) and any crash inside the web server. A subprocess of a
//! pinned binary keeps failures contained and needs no FFI.
//!
//! **A scratch data directory per run.** MAGPIE resolves data files through a
//! search list that starts at `./data`, relative to its working directory, and
//! it loads a default board layout before it parses any argument -- so the
//! directory has to be complete and the process has to run in it. Every run
//! therefore gets a fresh directory containing exactly the bytes the job pins,
//! written from the object store and from `input_data.content`. Nothing
//! server-side reads a data file off its own disk, which is the rule
//! PLAN.md sets and the reason there is no `data/` in the backend image.

use crate::error::{AppError, AppResult};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a conversion may run before it is killed.
///
/// A rack info table is the long pole: 59 seconds on 8 threads and about 170
/// on one, for CSW24. This is generous against the slowest of those with room
/// for a cold page cache, and exists so a builder that wedges is a failed row
/// an admin can see rather than a task that never exits.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// How long `magpie builders` may run. It prints a compile-time constant and
/// exits, so anything approaching this is a binary that does not work.
const BUILDERS_TIMEOUT: Duration = Duration::from_secs(30);

/// Where MAGPIE lives in the backend image. Overridden by `MAGPIE_BIN` for
/// development against a local checkout.
pub const DEFAULT_MAGPIE_BIN: &str = "/usr/local/bin/magpie";

/// What the pinned binary says about itself.
///
/// Read from the binary rather than from configuration, so the builder
/// recorded beside a hash is always the one that actually produced it. A
/// configured value would drift the first time someone deployed a new image
/// without editing a variable, and the whole point of recording the builder is
/// that a hash without one means nothing.
#[derive(Debug, Clone, Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Builders {
    pub magpie_version: String,
    pub build_target: String,
    pub wmp_builder_version: i32,
    pub rit_builder_version: i32,
    pub klv_builder_version: i32,
}

impl Builders {
    /// The identifier recorded with a wordmap's hash and sent to workers.
    pub fn wmp(&self) -> String {
        format!("wmp-{}", self.wmp_builder_version)
    }

    /// The identifier recorded with a rack info table's hash.
    pub fn rit(&self) -> String {
        format!("rit-{}", self.rit_builder_version)
    }

    /// The identifier recorded on a leave-generation KLV artifact.
    pub fn klv(&self) -> String {
        format!("klv-{}", self.klv_builder_version)
    }

    /// The builder identifier for a derived role, or an error for a role this
    /// server does not build.
    pub fn for_role(&self, role: &str) -> AppResult<String> {
        match role {
            "wmp" => Ok(self.wmp()),
            "rit" => Ok(self.rit()),
            other => Err(AppError::internal(format!("no builder for role {other:?}"))),
        }
    }
}

/// A handle on the pinned binary.
#[derive(Clone, Debug)]
pub struct Magpie {
    binary: PathBuf,
    /// Threads to pass to a conversion. A rack info table build scales close
    /// to linearly, and the builder task is sized for it.
    threads: usize,
}

impl Magpie {
    pub fn new(binary: impl Into<PathBuf>, threads: usize) -> Self {
        Self { binary: binary.into(), threads: threads.max(1) }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// The builder versions and build target this binary was compiled with.
    ///
    /// Called once at startup and cached: it is a constant of the image, and a
    /// failure here means the image has no working MAGPIE, which is worth
    /// finding out before anything depends on it rather than on the first
    /// build request.
    pub async fn builders(&self) -> AppResult<Builders> {
        // `builders` reads no data file, but MAGPIE still loads its default
        // board layout at startup, so it needs a directory to find one in.
        let scratch = ScratchData::empty().await?;
        let output = self.run(&scratch, &["builders"], BUILDERS_TIMEOUT).await?;
        serde_json::from_str(output.trim()).map_err(|e| {
            AppError::internal(format!(
                "{} builders printed something this server cannot read ({e}): {}",
                self.binary.display(),
                output.trim()
            ))
        })
    }

    /// `magpie convert <conversion> <name> <letter_distribution>` in `scratch`.
    ///
    /// The letter distribution is always stated. MAGPIE would otherwise infer
    /// one from the lexicon's name, and a wordmap built against an inferred
    /// distribution is not necessarily the one built against the distribution
    /// the job pins -- which is the whole thing this is trying to make
    /// comparable.
    pub async fn convert(
        &self,
        scratch: &ScratchData,
        conversion: &str,
        name: &str,
        letterdist: &str,
    ) -> AppResult<()> {
        self.run(
            scratch,
            &["convert", conversion, name, letterdist, "-threads", &self.threads.to_string()],
            CONVERT_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// `magpie convert klvwmp2rit <name> <ld> <klv_name> <wmp_name>`.
    ///
    /// The three names are separate because a rack info table is named for the
    /// (lexicon, leaves) pair it belongs to -- it stores precomputed leave
    /// values -- while its inputs are named for themselves. `klvwmp2rit` used
    /// to load both under the output's name, which forced the table to borrow
    /// one of their names and left no way to say which pair a file was for.
    pub async fn convert_rack_info_table(
        &self,
        scratch: &ScratchData,
        name: &str,
        letterdist: &str,
        klv_name: &str,
        wmp_name: &str,
    ) -> AppResult<()> {
        self.run(
            scratch,
            &[
                "convert",
                "klvwmp2rit",
                name,
                letterdist,
                klv_name,
                wmp_name,
                "-threads",
                &self.threads.to_string(),
            ],
            CONVERT_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// `magpie createdata klv <name> <letter_distribution>`: a KLV whose every
    /// leave is worth zero, which is the generation-0 artifact a
    /// leave-generation job starts from.
    ///
    /// MAGPIE_DEPENDENCY.md proposed a `convert zero2klv` for this. It is not
    /// needed: `createdata klv` already builds exactly that file from the
    /// distribution alone, through the same `klv_create_empty` the proposal
    /// named, and adding a second spelling of it would be one more thing to
    /// keep in step.
    pub async fn create_zero_klv(
        &self,
        scratch: &ScratchData,
        name: &str,
        letterdist: &str,
    ) -> AppResult<()> {
        self.run(scratch, &["createdata", "klv", name, letterdist], CONVERT_TIMEOUT)
            .await
            .map(|_| ())
    }

    /// Runs MAGPIE in `scratch` and returns its stdout.
    ///
    /// The working directory is the scratch root, not the data directory:
    /// MAGPIE's default search path is the literal `./data`, and it is
    /// consulted before any argument is parsed.
    async fn run(
        &self,
        scratch: &ScratchData,
        args: &[&str],
        timeout: Duration,
    ) -> AppResult<String> {
        let mut command = tokio::process::Command::new(&self.binary);
        command
            .args(args)
            .current_dir(scratch.root())
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = command.spawn().map_err(|e| {
            AppError::internal(format!("could not run {}: {e}", self.binary.display()))
        })?;
        let finished = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                AppError::internal(format!(
                    "magpie {} did not finish within {} seconds",
                    args.join(" "),
                    timeout.as_secs()
                ))
            })?
            .map_err(|e| AppError::internal(format!("magpie {} failed: {e}", args.join(" "))))?;

        let stdout = String::from_utf8_lossy(&finished.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&finished.stderr).into_owned();
        if !finished.status.success() {
            return Err(AppError::internal(format!(
                "magpie {} exited {}: {}",
                args.join(" "),
                finished.status,
                first_lines(&format!("{stdout}\n{stderr}"))
            )));
        }
        // **A failed conversion exits 0.** MAGPIE prints what went wrong as
        // "(error NN) ..." on stderr and returns success, so the exit status is
        // not the check -- both streams are. Missing this once meant a refused
        // conversion was reported as a successful one, and the only thing
        // standing behind it was the caller noticing that no file had appeared.
        //
        // That caller check stays, because it catches the other direction too:
        // MAGPIE can report nothing and still write nothing. This exists so the
        // reason reaches an admin rather than "MAGPIE wrote no file".
        let reported = [&stdout, &stderr]
            .into_iter()
            .find(|stream| stream.contains("(error "));
        if let Some(reported) = reported {
            return Err(AppError::internal(format!(
                "magpie {} reported: {}",
                args.join(" "),
                first_lines(reported)
            )));
        }
        Ok(stdout)
    }
}

/// The first few lines of a subprocess's output, for an error message that has
/// to fit in a database column and an admin's screen.
fn first_lines(text: &str) -> String {
    let joined: String = text.lines().filter(|l| !l.trim().is_empty()).take(5).collect::<Vec<_>>().join("; ");
    joined.chars().take(1000).collect()
}

/// A throwaway data directory laid out the way MAGPIE expects, holding only
/// the files one run needs.
///
/// Removed when dropped, including whatever MAGPIE wrote into it -- which for
/// a rack info table is 1.9 GB, so leaking one of these fills a disk in a
/// handful of builds. The hash is taken before the drop; MAGPIE_DEPENDENCY.md
/// asked whether the server should keep its copies, and this is the answer:
/// hash and discard. Keeping them would cost 1.9 GB per table for the sake of
/// an inspection nobody has needed yet, and the bytes are reproducible from
/// the inputs, which are what the object store holds.
pub struct ScratchData {
    root: PathBuf,
}

impl Drop for ScratchData {
    fn drop(&mut self) {
        // Blocking, in a Drop, on purpose: an async cleanup would have to be
        // called explicitly and so would be skipped on every early return,
        // which for a directory holding a 1.9 GB table is the one case that
        // matters. Removing a handful of files is microseconds.
        if let Err(err) = std::fs::remove_dir_all(&self.root) {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(dir = %self.root.display(), %err, "could not remove a MAGPIE scratch directory");
            }
        }
    }
}

impl ScratchData {
    /// A directory with MAGPIE's layout and nothing in it but board layouts.
    ///
    /// The layouts are not optional: `config_create` loads a default board
    /// layout before it parses any argument, so a directory without one fails
    /// every command including `builders`.
    ///
    /// `MAGPIE_SCRATCH_DIR` names where these go, because the builder task
    /// mounts several gigabytes of ephemeral storage for them and the
    /// container's default temp directory is not it.
    pub async fn empty() -> AppResult<Self> {
        let base = match std::env::var("MAGPIE_SCRATCH_DIR") {
            Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => std::env::temp_dir(),
        };
        let root = base.join(format!("birdtest-magpie-{}", uuid::Uuid::new_v4()));
        for dir in ["lexica", "letterdistributions", "layouts", "strategy"] {
            tokio::fs::create_dir_all(root.join("data").join(dir)).await.map_err(|e| {
                AppError::internal(format!("could not make a MAGPIE scratch directory: {e}"))
            })?;
        }
        let scratch = Self { root };
        tokio::fs::write(scratch.data().join("layouts/standard15.txt"), STANDARD15_LAYOUT)
            .await
            .map_err(|e| {
                AppError::internal(format!("could not write the scratch board layout: {e}"))
            })?;
        Ok(scratch)
    }

    /// The directory MAGPIE runs in; `data/` sits directly under it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    /// Writes `bytes` as `data/<dir>/<name><extension>`.
    pub async fn write(
        &self,
        dir: &str,
        name: &str,
        extension: &str,
        bytes: &[u8],
    ) -> AppResult<()> {
        let path = self.data().join(dir).join(format!("{name}{extension}"));
        tokio::fs::write(&path, bytes).await.map_err(|e| {
            AppError::internal(format!("could not write {}: {e}", path.display()))
        })
    }

    /// The path a derived or built file lands at, for hashing.
    pub fn lexicon_path(&self, name: &str, extension: &str) -> PathBuf {
        self.data().join("lexica").join(format!("{name}{extension}"))
    }
}

/// The board layout every scratch directory carries, so MAGPIE can start.
///
/// Inlined rather than read from `input_data`: this is not the job's layout
/// and is never played on. It exists only because MAGPIE loads *a* default
/// layout at startup, before it knows what it has been asked to do, and a
/// conversion never consults it. The job's own layout is pinned, verified and
/// loaded on the worker, where the games are actually played.
const STANDARD15_LAYOUT: &str = include_str!("magpie_standard15.txt");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_ids_are_role_and_version() {
        let builders = Builders {
            magpie_version: "0.5.1".into(),
            build_target: "nehalem".into(),
            wmp_builder_version: 1,
            rit_builder_version: 2,
            klv_builder_version: 3,
        };
        assert_eq!(builders.wmp(), "wmp-1");
        assert_eq!(builders.rit(), "rit-2");
        assert_eq!(builders.klv(), "klv-3");
        assert_eq!(builders.for_role("wmp").unwrap(), "wmp-1");
        assert_eq!(builders.for_role("rit").unwrap(), "rit-2");
        assert!(builders.for_role("wit").is_err());
    }

    /// The exact shape `magpie builders` prints. If MAGPIE's output changes,
    /// this is what says so, rather than every build request failing at once.
    #[test]
    fn the_builders_json_magpie_prints_parses() {
        let printed = r#"{"magpie_version":"0.5.1","build_target":"nehalem","wmp_builder_version":1,"rit_builder_version":1,"klv_builder_version":1}"#;
        let builders: Builders = serde_json::from_str(printed).unwrap();
        assert_eq!(builders.magpie_version, "0.5.1");
        assert_eq!(builders.build_target, "nehalem");
        assert_eq!(builders.wmp(), "wmp-1");
    }

    #[test]
    fn subprocess_output_is_bounded_for_an_error_column() {
        let noisy = "line\n".repeat(1000);
        assert!(first_lines(&noisy).len() <= 1000);
        assert_eq!(first_lines("a\n\nb\n"), "a; b");
    }
}
