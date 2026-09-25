use anyhow::{Context, Result};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use std::time::Duration;

/// Runtime configuration.
///
/// In development every value comes from the environment (`.env` is loaded on
/// startup). In ECS the same variables are populated by the task definition,
/// which pulls the secret-valued ones from SSM Parameter Store -- so the
/// process only ever reads environment variables and there is no separate
/// secrets code path.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub session_signing_key: [u8; 32],
    pub session_ttl: Duration,
    pub secure_cookies: bool,
    pub mail_backend: MailBackend,
    /// Where `MAIL_BACKEND=file` writes one file per message. Required by that
    /// backend and read by nothing else.
    pub mail_outbox_dir: Option<std::path::PathBuf>,
    pub mail_from: String,
    pub public_url: String,
    pub heartbeat_timeout: Duration,
    /// How old a job's stats payload may be when served: the job page, the
    /// stream's first event, and the spacing of live pushes. The payload
    /// reads the job's whole history (its contributors, its game results),
    /// which grows without bound; rebuilt on every view and every second a
    /// busy job was watched, it cost a second of database time per second.
    pub stats_cache: Duration,
    pub s3_bucket: String,
    pub s3_endpoint: Option<String>,
    /// The oldest MAGPIE that may contribute at all. Enforced, not advisory:
    /// a client below it is offered nothing and told to update, without any
    /// job being consulted (`scheduler::claim`), and it is the default floor
    /// stamped onto a new job when an admin does not raise it. Also reported
    /// by `GET /api/worker/client-version` so a client can check itself.
    ///
    /// This is a floor on the *contributor's* build, and it is now also a
    /// floor the backend's own pinned MAGPIE has to clear: the server builds
    /// the reference copy of every wordmap, rack info table and
    /// leave-generation KLV, so a backend older than the fleet would be
    /// handing out hashes its workers could not reproduce.
    pub min_magpie_version: String,
    pub magpie_download_url: String,
    /// The pinned MAGPIE binary this process runs for every derived file and
    /// every leave-generation KLV. Baked into the backend image at a fixed
    /// path; overridden in development to point at a local checkout's
    /// `bin/magpie`.
    pub magpie_bin: String,
    /// Threads to give a conversion. A rack info table build scales close to
    /// linearly with them, and this is the builder task's vCPU count -- in the
    /// web task, where only the small conversions run, it stays at 1 so a
    /// build cannot take the whole process's CPU.
    pub magpie_threads: usize,
    /// Where import fetches versioned tarballs from. Configuration, never user
    /// input: the residual exposure of parsing an archive from the network is
    /// a compromised upstream, not an arbitrary URL.
    pub magpie_data_repo: String,
    /// Optional in development, set in production: unauthenticated GitHub ref
    /// resolution is 60 calls an hour per IP.
    pub github_token: Option<String>,
    /// The GitHub API and raw-content hosts import resolves refs and fetches
    /// tarballs from. Configuration for the same reason `magpie_data_repo`
    /// is; overridden only by the end-to-end suite, which serves a fixture
    /// tarball from a static-file container so import runs offline.
    pub github_api_url: String,
    pub github_raw_url: String,
    /// How many reverse proxies sit in front of this process and append to
    /// `X-Forwarded-For`. 0 trusts nothing and keys per-IP limits on the TCP
    /// peer; 1 is right behind the ALB and behind the local Nginx. See
    /// `clientip`.
    pub trusted_proxy_hops: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailBackend {
    /// Log the message body to stdout. The local default: there is no local SES.
    Console,
    Ses,
    /// One file per message in `MAIL_OUTBOX_DIR`, named for its recipient.
    /// For the end-to-end suite: a journey reads the confirmation code sent to
    /// its own address, which the single stream of `console` cannot tell apart
    /// when journeys run in parallel. Never production.
    File,
}

fn var(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Where settings come from: the environment in the process, a table in the
/// tests. An empty value counts as unset, as it does for `var`.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn get(lookup: Lookup, key: &str) -> Option<String> {
    lookup(key).filter(|v| !v.is_empty())
}

fn get_or(lookup: Lookup, key: &str, default: &str) -> String {
    get(lookup, key).unwrap_or_else(|| default.to_string())
}

/// A numeric setting that is wrong fails startup rather than silently becoming
/// the default: a heartbeat timeout typed as `5m` would otherwise run the whole
/// deployment on 300 seconds with nothing to say so.
fn parsed<T: std::str::FromStr>(lookup: Lookup, key: &str, default: T) -> Result<T> {
    match get(lookup, key) {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("{key} must be a whole number, got {raw:?}")),
    }
}

/// `DATABASE_URL` if it is set; otherwise one assembled from `DB_HOST`,
/// `DB_PORT`, `DB_NAME`, `DB_USER` and `DB_PASSWORD`.
///
/// The parts are for a deployment that injects the password on its own --
/// from a secret RDS manages and rotates, say. This one does not: the master
/// password is set by hand and lives only inside the `DATABASE_URL` SSM
/// parameter (`infra/rds.tf`, README.md "Deploying"), because a password RDS
/// rotated would leave that URL stale. The password is percent-encoded: a
/// generated one can contain `@`, `/` or `:`.
fn resolve_database_url(get: impl Fn(&str) -> Option<String>) -> Result<String> {
    if let Some(url) = get("DATABASE_URL") {
        return Ok(url);
    }
    let required = |key: &str| {
        get(key).with_context(|| format!("DATABASE_URL is not set, so {key} is required"))
    };
    let host = required("DB_HOST")?;
    let name = required("DB_NAME")?;
    let user = required("DB_USER")?;
    let password = required("DB_PASSWORD")?;
    let port = get("DB_PORT").unwrap_or_else(|| "5432".to_string());
    let mut url = format!(
        "postgres://{}:{}@{host}:{port}/{name}",
        utf8_percent_encode(&user, NON_ALPHANUMERIC),
        utf8_percent_encode(&password, NON_ALPHANUMERIC),
    );
    if let Some(sslmode) = get("DB_SSLMODE") {
        url.push_str(&format!("?sslmode={sslmode}"));
    }
    Ok(url)
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(&var)
    }

    /// The configuration `lookup` describes. `from_env` is this over the
    /// process environment; taking the source as a function is what lets every
    /// default and every refusal be tested without touching global state.
    pub fn from_lookup(lookup: Lookup) -> Result<Self> {
        let var = |key: &str| get(lookup, key);
        let var_or = |key: &str, default: &str| get_or(lookup, key, default);
        let parsed_u64 = |key: &str, default: u64| parsed(lookup, key, default);
        let raw_key = var_or("SESSION_SIGNING_KEY", "");
        let key_bytes = if raw_key.is_empty() {
            anyhow::bail!("SESSION_SIGNING_KEY is required (32 bytes hex-encoded)");
        } else {
            hex::decode(raw_key.trim()).context("SESSION_SIGNING_KEY must be hex-encoded")?
        };
        let session_signing_key: [u8; 32] = key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("SESSION_SIGNING_KEY must decode to exactly 32 bytes"))?;

        let mail_backend = match var_or("MAIL_BACKEND", "console").as_str() {
            "ses" => MailBackend::Ses,
            "console" => MailBackend::Console,
            "file" => MailBackend::File,
            other => anyhow::bail!(
                "unknown MAIL_BACKEND {other:?} (expected 'console', 'ses' or 'file')"
            ),
        };
        let mail_outbox_dir = var("MAIL_OUTBOX_DIR").map(std::path::PathBuf::from);
        if mail_backend == MailBackend::File && mail_outbox_dir.is_none() {
            anyhow::bail!("MAIL_BACKEND=file needs MAIL_OUTBOX_DIR, the directory to write to");
        }

        let secure_cookies = match var_or("SECURE_COOKIES", "false").as_str() {
            "true" => true,
            "false" => false,
            other => anyhow::bail!("SECURE_COOKIES must be 'true' or 'false', got {other:?}"),
        };

        let min_magpie_version = var_or("MIN_MAGPIE_VERSION", "0.1.1");
        // Strictly, as a job's floor is: the new-job form offers this value,
        // and one read loosely here ("0.2.0-rc1") was then refused there on
        // every job an admin created without touching the field.
        if crate::version::Version::parse_strict(&min_magpie_version).is_none() {
            anyhow::bail!(
                "MIN_MAGPIE_VERSION {min_magpie_version:?} is not a version (major.minor[.patch])"
            );
        }

        Ok(Self {
            database_url: resolve_database_url(var)?,
            bind_addr: var_or("BIND_ADDR", "0.0.0.0:8080"),
            session_signing_key,
            session_ttl: Duration::from_secs(parsed_u64("SESSION_TTL_SECONDS", 604_800)?),
            secure_cookies,
            mail_backend,
            mail_outbox_dir,
            mail_from: var_or("MAIL_FROM", "no-reply@birdtest.local"),
            public_url: var_or("PUBLIC_URL", "http://localhost:5173"),
            heartbeat_timeout: Duration::from_secs(parsed_u64("HEARTBEAT_TIMEOUT_SECONDS", 300)?),
            stats_cache: Duration::from_secs(parsed_u64("JOB_STATS_CACHE_SECONDS", 10)?),
            s3_bucket: var_or("S3_BUCKET", "birdtest-artifacts"),
            s3_endpoint: var("S3_ENDPOINT"),
            // 0.1.1 is the `birdtest-contribute` version the backend image
            // pins. The branch's version moves whenever a change can alter
            // what a task computes, and this floor moves with it: it is the
            // only way to keep a build known to compute something wrong off
            // the fleet. 0.1.0 is below it because builds reporting it
            // include ones where a capturing static player played its worst
            // move, and every one of them played a leave task after its
            // first with the previous task's KLV.
            min_magpie_version,
            magpie_download_url: var_or(
                "MAGPIE_DOWNLOAD_URL",
                "https://github.com/jvc56/MAGPIE",
            ),
            magpie_bin: var_or("MAGPIE_BIN", crate::magpie::DEFAULT_MAGPIE_BIN),
            magpie_threads: parsed(lookup, "MAGPIE_THREADS", 1usize)?,
            magpie_data_repo: var_or("MAGPIE_DATA_REPO", "jvc56/MAGPIE-DATA"),
            github_token: var("GITHUB_TOKEN"),
            github_api_url: var_or("GITHUB_API_URL", "https://api.github.com")
                .trim_end_matches('/')
                .to_string(),
            github_raw_url: var_or("GITHUB_RAW_URL", "https://raw.githubusercontent.com")
                .trim_end_matches('/')
                .to_string(),
            trusted_proxy_hops: parsed(lookup, "TRUSTED_PROXY_HOPS", 0usize)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |key| map.get(key).cloned()
    }

    const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    /// The two settings nothing can default: where the database is, and the
    /// key sessions are signed with.
    const REQUIRED: [(&str, &str); 2] =
        [("DATABASE_URL", "postgres://a:b@c/d"), ("SESSION_SIGNING_KEY", KEY)];

    fn config(extra: &[(&str, &str)]) -> Result<Config> {
        let mut pairs = REQUIRED.to_vec();
        pairs.extend_from_slice(extra);
        Config::from_lookup(&env(&pairs))
    }

    /// U-CFG-1: every setting with a default takes it when unset and the
    /// given value when set. Each row names the variable, so a renamed one
    /// that silently fell back to its default fails here by name.
    #[test]
    fn every_default_applies_when_unset_and_yields_to_a_value() {
        type Read = fn(&Config) -> String;
        let rows: &[(&str, &str, &str, Read)] = &[
            ("BIND_ADDR", "0.0.0.0:8080", "127.0.0.1:9", |c| c.bind_addr.clone()),
            ("SESSION_TTL_SECONDS", "604800", "60", |c| c.session_ttl.as_secs().to_string()),
            ("SECURE_COOKIES", "false", "true", |c| c.secure_cookies.to_string()),
            ("MAIL_BACKEND", "Console", "ses", |c| format!("{:?}", c.mail_backend)),
            ("MAIL_FROM", "no-reply@birdtest.local", "a@b.c", |c| c.mail_from.clone()),
            ("PUBLIC_URL", "http://localhost:5173", "https://x.y", |c| c.public_url.clone()),
            ("HEARTBEAT_TIMEOUT_SECONDS", "300", "70", |c| {
                c.heartbeat_timeout.as_secs().to_string()
            }),
            ("JOB_STATS_CACHE_SECONDS", "10", "0", |c| c.stats_cache.as_secs().to_string()),
            ("S3_BUCKET", "birdtest-artifacts", "other", |c| c.s3_bucket.clone()),
            ("S3_ENDPOINT", "None", "http://minio:9000", |c| {
                c.s3_endpoint.clone().unwrap_or_else(|| "None".into())
            }),
            ("MIN_MAGPIE_VERSION", "0.1.1", "1.10.0", |c| c.min_magpie_version.clone()),
            ("MAGPIE_DOWNLOAD_URL", "https://github.com/jvc56/MAGPIE", "https://d", |c| {
                c.magpie_download_url.clone()
            }),
            ("MAGPIE_BIN", crate::magpie::DEFAULT_MAGPIE_BIN, "/opt/magpie", |c| {
                c.magpie_bin.clone()
            }),
            ("MAGPIE_THREADS", "1", "8", |c| c.magpie_threads.to_string()),
            ("MAGPIE_DATA_REPO", "jvc56/MAGPIE-DATA", "me/data", |c| c.magpie_data_repo.clone()),
            ("GITHUB_TOKEN", "None", "ghp_x", |c| {
                c.github_token.clone().unwrap_or_else(|| "None".into())
            }),
            ("TRUSTED_PROXY_HOPS", "0", "2", |c| c.trusted_proxy_hops.to_string()),
            ("GITHUB_API_URL", "https://api.github.com", "http://fixtures:80", |c| {
                c.github_api_url.clone()
            }),
            ("GITHUB_RAW_URL", "https://raw.githubusercontent.com", "http://fixtures:80", |c| {
                c.github_raw_url.clone()
            }),
            ("MAIL_OUTBOX_DIR", "None", "/outbox", |c| {
                c.mail_outbox_dir.as_ref().map_or("None".into(), |p| p.display().to_string())
            }),
        ];
        for (key, default, set, read) in rows {
            let unset = config(&[]).unwrap();
            assert_eq!(read(&unset), *default, "{key} unset");
            // An empty value is unset, not an empty setting.
            let empty = config(&[(key, "")]).unwrap();
            assert_eq!(read(&empty), *default, "{key} empty");
            let given = config(&[(key, set)]).unwrap();
            let expected = if *key == "MAIL_BACKEND" { "Ses" } else { set };
            assert_eq!(read(&given), expected, "{key} set");
        }
    }

    /// U-CFG-2: a required setting that is missing is a startup error that
    /// names it, not a panic and not an empty value.
    #[test]
    fn a_missing_required_setting_is_an_error_naming_it() {
        let err = Config::from_lookup(&env(&[("DATABASE_URL", "postgres://a:b@c/d")])).unwrap_err();
        assert!(err.to_string().contains("SESSION_SIGNING_KEY"), "{err}");
        let err = Config::from_lookup(&env(&[("SESSION_SIGNING_KEY", KEY)])).unwrap_err();
        assert!(err.to_string().contains("DB_HOST"), "{err}");
        let err = config(&[("SESSION_SIGNING_KEY", "abcd")]).unwrap_err();
        assert!(err.to_string().contains("SESSION_SIGNING_KEY"), "{err}");
    }

    /// A value that is present but wrong fails startup too, naming the
    /// setting, rather than becoming the default.
    #[test]
    fn a_malformed_value_is_refused_rather_than_defaulted() {
        for (key, value) in [
            ("HEARTBEAT_TIMEOUT_SECONDS", "5m"),
            ("SESSION_TTL_SECONDS", "-1"),
            ("MAGPIE_THREADS", "two"),
            ("TRUSTED_PROXY_HOPS", "one"),
            ("SECURE_COOKIES", "yes"),
            ("MAIL_BACKEND", "smtp"),
            // The file backend with nowhere to write.
            ("MAIL_BACKEND", "file"),
        ] {
            let err = config(&[(key, value)]).unwrap_err();
            assert!(err.to_string().contains(key), "{key}={value}: {err}");
        }
        let file = config(&[("MAIL_BACKEND", "file"), ("MAIL_OUTBOX_DIR", "/outbox")]).unwrap();
        assert_eq!(file.mail_backend, MailBackend::File);
    }

    /// U-CFG-3: `MIN_MAGPIE_VERSION` goes through `Version`, so a malformed
    /// floor fails at startup instead of becoming 0.0.0 and admitting every
    /// client; an explicit 0.0.0 is still a floor someone chose.
    #[test]
    fn a_malformed_version_floor_fails_startup() {
        // And strictly, as a job's floor is: "0.2.0-rc1" and "1.6.x" were
        // accepted here and refused on every job created from the form.
        for bad in ["latest", "v1", "1.x", "one.two.three", "0.2.0-rc1", "1.6.x", "1.6.0.1"] {
            let err = config(&[("MIN_MAGPIE_VERSION", bad)]).unwrap_err();
            assert!(err.to_string().contains("MIN_MAGPIE_VERSION"), "{bad}: {err}");
        }
        assert_eq!(config(&[("MIN_MAGPIE_VERSION", "0.0.0")]).unwrap().min_magpie_version, "0.0.0");
        assert_eq!(config(&[("MIN_MAGPIE_VERSION", "1.10.0")]).unwrap().min_magpie_version, "1.10.0");
    }

    #[test]
    fn database_url_wins_when_set() {
        let url = resolve_database_url(env(&[
            ("DATABASE_URL", "postgres://a:b@c/d"),
            ("DB_HOST", "ignored"),
        ]))
        .unwrap();
        assert_eq!(url, "postgres://a:b@c/d");
    }

    /// RDS-generated passwords contain URL metacharacters; unencoded, `@`
    /// would be read as the end of the userinfo and the host would be garbage.
    #[test]
    fn database_parts_are_assembled_with_the_password_encoded() {
        let url = resolve_database_url(env(&[
            ("DB_HOST", "db.internal"),
            ("DB_NAME", "birdtest"),
            ("DB_USER", "birdtest"),
            ("DB_PASSWORD", "p@ss/w:rd"),
            ("DB_SSLMODE", "require"),
        ]))
        .unwrap();
        assert_eq!(
            url,
            "postgres://birdtest:p%40ss%2Fw%3Ard@db.internal:5432/birdtest?sslmode=require"
        );
    }

    #[test]
    fn a_missing_part_names_itself() {
        let err = resolve_database_url(env(&[("DB_HOST", "h"), ("DB_NAME", "n")])).unwrap_err();
        assert!(err.to_string().contains("DB_USER"), "{err}");
    }
}
