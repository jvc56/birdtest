use anyhow::{Context, Result};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use std::time::Duration;

/// Runtime configuration.
///
/// In development every value comes from the environment (`.env` is loaded on
/// startup). In ECS the same variables are populated by the task definition,
/// which pulls the secret-valued ones from SSM Parameter Store and Secrets
/// Manager — so the process only ever reads environment variables and there is
/// no separate secrets code path.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub session_signing_key: [u8; 32],
    pub session_ttl: Duration,
    pub secure_cookies: bool,
    pub mail_backend: MailBackend,
    pub mail_from: String,
    pub public_url: String,
    pub heartbeat_timeout: Duration,
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
/// The parts exist for the deployment. RDS manages the master password in
/// Secrets Manager and rotates it (every seven days by default), so a
/// hand-written `DATABASE_URL` in SSM goes stale by itself. ECS can inject the
/// password straight from the managed secret, and the host is a Terraform
/// output, so neither needs to be copied anywhere by hand. The password is
/// percent-encoded: a generated one can contain `@`, `/` or `:`.
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
            other => anyhow::bail!("unknown MAIL_BACKEND {other:?} (expected 'console' or 'ses')"),
        };

        let secure_cookies = match var_or("SECURE_COOKIES", "false").as_str() {
            "true" => true,
            "false" => false,
            other => anyhow::bail!("SECURE_COOKIES must be 'true' or 'false', got {other:?}"),
        };

        let min_magpie_version = var_or("MIN_MAGPIE_VERSION", "0.1.0");
        if crate::version::Version::parse_or_zero(&min_magpie_version)
            == crate::version::Version::ZERO
            && min_magpie_version.trim() != "0.0.0"
        {
            anyhow::bail!("MIN_MAGPIE_VERSION {min_magpie_version:?} is not a version");
        }

        Ok(Self {
            database_url: resolve_database_url(var)?,
            bind_addr: var_or("BIND_ADDR", "0.0.0.0:8080"),
            session_signing_key,
            session_ttl: Duration::from_secs(parsed_u64("SESSION_TTL_SECONDS", 604_800)?),
            secure_cookies,
            mail_backend,
            mail_from: var_or("MAIL_FROM", "no-reply@birdtest.local"),
            public_url: var_or("PUBLIC_URL", "http://localhost:5173"),
            heartbeat_timeout: Duration::from_secs(parsed_u64("HEARTBEAT_TIMEOUT_SECONDS", 300)?),
            s3_bucket: var_or("S3_BUCKET", "birdtest-artifacts"),
            s3_endpoint: var("S3_ENDPOINT"),
            // 0.1.0 is `birdtest-contribute`'s pre-release version. Neither
            // birdtest nor the branch is in production yet, so everything the
            // protocol relies on -- every result-changing setting stated on
            // the request rather than taken from the worker's build, input
            // data and derived files checked against the hashes the job pins,
            // the word info table switched off before every load, a seed on
            // every task -- is in 0.1.0. The version moves only when a
            // release changes what a task computes, and this floor moves with
            // it; until then there is nothing below it to refuse, and the
            // floor exists so that the first such release can raise it.
            min_magpie_version,
            magpie_download_url: var_or(
                "MAGPIE_DOWNLOAD_URL",
                "https://github.com/jvc56/MAGPIE",
            ),
            magpie_bin: var_or("MAGPIE_BIN", crate::magpie::DEFAULT_MAGPIE_BIN),
            magpie_threads: parsed(lookup, "MAGPIE_THREADS", 1usize)?,
            magpie_data_repo: var_or("MAGPIE_DATA_REPO", "jvc56/MAGPIE-DATA"),
            github_token: var("GITHUB_TOKEN"),
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
            ("S3_BUCKET", "birdtest-artifacts", "other", |c| c.s3_bucket.clone()),
            ("S3_ENDPOINT", "None", "http://minio:9000", |c| {
                c.s3_endpoint.clone().unwrap_or_else(|| "None".into())
            }),
            ("MIN_MAGPIE_VERSION", "0.1.0", "1.10.0", |c| c.min_magpie_version.clone()),
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
        ] {
            let err = config(&[(key, value)]).unwrap_err();
            assert!(err.to_string().contains(key), "{key}={value}: {err}");
        }
    }

    /// U-CFG-3: `MIN_MAGPIE_VERSION` goes through `Version`, so a malformed
    /// floor fails at startup instead of becoming 0.0.0 and admitting every
    /// client; an explicit 0.0.0 is still a floor someone chose.
    #[test]
    fn a_malformed_version_floor_fails_startup() {
        for bad in ["latest", "v1", "1.x", "one.two.three"] {
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
