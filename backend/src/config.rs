use anyhow::{Context, Result};
use std::time::Duration;

/// Runtime configuration.
///
/// In development every value comes from the environment (`.env` is loaded on
/// startup). In ECS the same variables are populated by the task definition,
/// which pulls the secret-valued ones from SSM Parameter Store — so the process
/// only ever reads environment variables and there is no separate SSM code path.
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
    /// This is a floor on the *contributor's* build. The backend itself has no
    /// MAGPIE dependency -- leave-generation aggregation builds its KLV
    /// artifact directly (see `jobs::klv`).
    pub min_magpie_version: String,
    pub magpie_download_url: String,
    /// Where import fetches versioned tarballs from. Configuration, never user
    /// input: the residual exposure of parsing an archive from the network is
    /// a compromised upstream, not an arbitrary URL.
    pub magpie_data_repo: String,
    /// Optional in development, set in production: unauthenticated GitHub ref
    /// resolution is 60 calls an hour per IP.
    pub github_token: Option<String>,
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

fn var_or(key: &str, default: &str) -> String {
    var(key).unwrap_or_else(|| default.to_string())
}

impl Config {
    pub fn from_env() -> Result<Self> {
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

        Ok(Self {
            database_url: var("DATABASE_URL").context("DATABASE_URL is required")?,
            bind_addr: var_or("BIND_ADDR", "0.0.0.0:8080"),
            session_signing_key,
            session_ttl: Duration::from_secs(
                var_or("SESSION_TTL_SECONDS", "604800").parse().unwrap_or(604_800),
            ),
            secure_cookies: var_or("SECURE_COOKIES", "false") == "true",
            mail_backend,
            mail_from: var_or("MAIL_FROM", "no-reply@birdtest.local"),
            public_url: var_or("PUBLIC_URL", "http://localhost:5173"),
            heartbeat_timeout: Duration::from_secs(
                var_or("HEARTBEAT_TIMEOUT_SECONDS", "300").parse().unwrap_or(300),
            ),
            s3_bucket: var_or("S3_BUCKET", "birdtest-artifacts"),
            s3_endpoint: var("S3_ENDPOINT"),
            // A placeholder for the MAGPIE release that implements the
            // expected_data check, to be raised to that release's real number
            // before launch. Not 0.0.0: every job pins data now, and a floor of
            // zero would admit a client that cannot verify it.
            min_magpie_version: var_or("MIN_MAGPIE_VERSION", "0.0.1"),
            magpie_download_url: var_or(
                "MAGPIE_DOWNLOAD_URL",
                "https://github.com/jvc56/MAGPIE",
            ),
            magpie_data_repo: var_or("MAGPIE_DATA_REPO", "jvc56/MAGPIE-DATA"),
            github_token: var("GITHUB_TOKEN"),
        })
    }
}
