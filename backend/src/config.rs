use anyhow::{Context, Result};
use std::path::PathBuf;
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
    pub data_path: PathBuf,
    /// Root of a MAGPIE checkout: the executable is `<dir>/bin/magpie` and its
    /// lexical data is `<dir>/data`. Only leave-generation aggregation uses it.
    pub magpie_dir: PathBuf,
    pub heartbeat_timeout: Duration,
    pub s3_bucket: String,
    pub s3_endpoint: Option<String>,
    /// The oldest MAGPIE that may contribute at all, reported by
    /// `GET /api/worker/client-version`. Jobs may require newer.
    pub min_magpie_version: String,
    pub magpie_download_url: String,
}

impl Config {
    pub fn magpie_bin(&self) -> PathBuf {
        self.magpie_dir.join("bin").join("magpie")
    }

    pub fn magpie_data_dir(&self) -> PathBuf {
        self.magpie_dir.join("data")
    }
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
            data_path: PathBuf::from(var_or("DATA_PATH", "../data")),
            magpie_dir: PathBuf::from(var_or("MAGPIE_DIR", "../MAGPIE")),
            heartbeat_timeout: Duration::from_secs(
                var_or("HEARTBEAT_TIMEOUT_SECONDS", "300").parse().unwrap_or(300),
            ),
            s3_bucket: var_or("S3_BUCKET", "birdtest-artifacts"),
            s3_endpoint: var("S3_ENDPOINT"),
            min_magpie_version: var_or("MIN_MAGPIE_VERSION", "0.0.0"),
            magpie_download_url: var_or(
                "MAGPIE_DOWNLOAD_URL",
                "https://github.com/jvc56/MAGPIE",
            ),
        })
    }
}
