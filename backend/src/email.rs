use crate::config::{Config, MailBackend};
use crate::error::{AppError, AppResult};
use std::sync::Arc;

#[derive(Clone)]
pub struct Mailer {
    cfg: Arc<Config>,
    ses: Option<aws_sdk_sesv2::Client>,
}

impl Mailer {
    pub async fn new(cfg: Arc<Config>) -> Self {
        let ses = match cfg.mail_backend {
            MailBackend::Ses => {
                let aws = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
                Some(aws_sdk_sesv2::Client::new(&aws))
            }
            MailBackend::Console | MailBackend::File => None,
        };
        Self { cfg, ses }
    }

    /// In `console` mode the message is written to the log instead of being sent.
    /// That is what makes the local registration and password-reset flows usable
    /// without any AWS access: the confirmation code is in the server's stdout.
    pub async fn send(&self, to: &str, subject: &str, body: &str) -> AppResult<()> {
        if self.cfg.mail_backend == MailBackend::File {
            return self.write_to_outbox(to, subject, body).await;
        }
        match &self.ses {
            None => {
                tracing::info!(
                    to,
                    subject,
                    "\n---- email (MAIL_BACKEND=console) ----\n{body}\n--------------------------------------"
                );
                Ok(())
            }
            Some(client) => {
                let destination = aws_sdk_sesv2::types::Destination::builder()
                    .to_addresses(to)
                    .build();
                let content = aws_sdk_sesv2::types::EmailContent::builder()
                    .simple(
                        aws_sdk_sesv2::types::Message::builder()
                            .subject(
                                // SES reads a part with no charset as 7-bit
                                // ASCII; the mails name accounts, and a name
                                // may be `李小龍` or `émile`.
                                aws_sdk_sesv2::types::Content::builder()
                                    .data(subject)
                                    .charset("UTF-8")
                                    .build()
                                    .map_err(|e| AppError::internal(e.to_string()))?,
                            )
                            .body(
                                aws_sdk_sesv2::types::Body::builder()
                                    .text(
                                        aws_sdk_sesv2::types::Content::builder()
                                            .data(body)
                                            .charset("UTF-8")
                                            .build()
                                            .map_err(|e| AppError::internal(e.to_string()))?,
                                    )
                                    .build(),
                            )
                            .build(),
                    )
                    .build();

                client
                    .send_email()
                    .from_email_address(&self.cfg.mail_from)
                    .destination(destination)
                    .content(content)
                    .send()
                    .await
                    .map_err(|e| AppError::internal(format!("SES send failed: {}", ses_error(&e))))?;
                Ok(())
            }
        }
    }
}

/// What SES said, as `code: message`: `{e}` alone reads "service error" for a
/// paused account, an unverified address and a missing permission alike (the
/// audit's pass 24). Not `DisplayErrorContext`, which appends the whole raw
/// response -- a line of 1,300 characters, the recipient in it four times. An
/// error with no answer from SES (a timeout, no route) has no code, and its
/// chain of causes is the reason.
fn ses_error(
    e: &aws_sdk_sesv2::error::SdkError<
        aws_sdk_sesv2::operation::send_email::SendEmailError,
        aws_sdk_sesv2::config::http::HttpResponse,
    >,
) -> String {
    use aws_sdk_sesv2::error::ProvideErrorMetadata;
    match e {
        aws_sdk_sesv2::error::SdkError::ServiceError(service) => {
            let err = service.err();
            match (err.code(), err.message()) {
                (Some(code), Some(message)) => format!("{code}: {message}"),
                (Some(code), None) => code.to_string(),
                _ => format!("HTTP {}", service.raw().status().as_u16()),
            }
        }
        other => {
            let mut chain = other.to_string();
            let mut source = std::error::Error::source(other);
            while let Some(cause) = source {
                chain.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            chain
        }
    }
}

/// The outbox file name for a message to `to`: when it was written, then the
/// recipient with everything but letters, digits and dashes spelled out or
/// replaced, e.g. `20260910-191500-000123-e2e-1f2e-at-example-invalid.txt`.
/// Sorting by name sorts by time, and a reader finds its own mail by suffix.
pub fn outbox_file_name(to: &str, at: chrono::DateTime<chrono::Utc>) -> String {
    let recipient: String = to
        .to_ascii_lowercase()
        .replace('@', "-at-")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    format!("{}-{recipient}.txt", at.format("%Y%m%d-%H%M%S-%6f"))
}

impl Mailer {
    /// `MAIL_BACKEND=file`: one message per file. Written under a temporary
    /// name and renamed, so a reader polling the directory never sees half a
    /// message.
    async fn write_to_outbox(&self, to: &str, subject: &str, body: &str) -> AppResult<()> {
        let dir = self
            .cfg
            .mail_outbox_dir
            .as_ref()
            .ok_or_else(|| AppError::internal("MAIL_BACKEND=file without MAIL_OUTBOX_DIR"))?;
        let name = outbox_file_name(to, chrono::Utc::now());
        let contents = format!("To: {to}\nSubject: {subject}\n\n{body}\n");
        let partial = dir.join(format!(".{name}.partial"));
        let write = async {
            tokio::fs::create_dir_all(dir).await?;
            tokio::fs::write(&partial, contents).await?;
            tokio::fs::rename(&partial, dir.join(&name)).await
        };
        write
            .await
            .map_err(|e| AppError::internal(format!("writing mail to {}: {e}", dir.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn an_outbox_file_is_named_for_its_time_and_recipient() {
        let at = chrono::Utc.with_ymd_and_hms(2026, 9, 10, 19, 15, 0).unwrap();
        assert_eq!(
            outbox_file_name("E2E-1f2e@example.invalid", at),
            "20260910-191500-000000-e2e-1f2e-at-example-invalid.txt"
        );
        // Nothing in an address can climb out of the directory.
        assert!(!outbox_file_name("../../etc/passwd@x", at).contains('/'));
    }

    #[tokio::test]
    async fn the_file_backend_writes_one_readable_message_per_send() {
        let path = std::env::temp_dir().join(format!("birdtest-outbox-{}", uuid::Uuid::new_v4()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = Cleanup(path);
        let cfg = crate::config::Config::from_lookup(&|k: &str| match k {
            "DATABASE_URL" => Some("postgres://a:b@c/d".into()),
            "SESSION_SIGNING_KEY" => Some("00".repeat(32)),
            "MAIL_BACKEND" => Some("file".into()),
            "MAIL_OUTBOX_DIR" => Some(dir.0.as_path().display().to_string()),
            _ => None,
        })
        .unwrap();
        let mailer = Mailer::new(Arc::new(cfg)).await;
        mailer.send("a@example.invalid", "Confirm", "code: 123").await.unwrap();
        mailer.send("b@example.invalid", "Reset", "link").await.unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.0.as_path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), 2, "{names:?}");
        let a = names.iter().find(|n| n.ends_with("-a-at-example-invalid.txt")).unwrap();
        let text = std::fs::read_to_string(dir.0.as_path().join(a)).unwrap();
        assert!(text.contains("Subject: Confirm") && text.contains("code: 123"), "{text}");
    }

    /// The alarm on failed mail (infra/ses.tf) matches a field in the log
    /// line: every send's failure must carry it, and the filter must still
    /// look for it.
    #[test]
    fn every_failed_send_is_logged_as_the_alarm_expects() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let infra = std::fs::read_to_string(root.join("../infra/ses.tf")).unwrap();
        assert!(infra.contains(r#"pattern        = "{ $.fields.alarm = \"mail_failed\" }""#));
        // Without whitespace, so that a call split across lines counts.
        let auth: String = std::fs::read_to_string(root.join("src/routes/auth.rs"))
            .unwrap()
            .split_whitespace()
            .collect();
        let sends = auth.matches("mailer.send(").count();
        assert!(sends >= 3, "{sends}");
        assert_eq!(auth.matches(r#"tracing::error!(alarm="mail_failed","#).count(), sends);
    }

    /// A send SES refuses is logged with SES's own code and message: what an
    /// operator reads to tell a paused account from an unverified address or
    /// a missing permission. It used to read `SES send failed: service error`,
    /// the same for every one of them.
    #[tokio::test]
    async fn a_refused_send_keeps_what_ses_said() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    // The whole request, headers and body, before answering.
                    let mut request = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else { return };
                        if n == 0 {
                            return;
                        }
                        request.extend_from_slice(&buf[..n]);
                        let text = String::from_utf8_lossy(&request);
                        if let Some(end) = text.find("\r\n\r\n") {
                            let length = text[..end]
                                .lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.eq_ignore_ascii_case("content-length").then(|| v.trim().parse().ok())?
                                })
                                .unwrap_or(0usize);
                            if request.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    let body = r#"{"message":"Sending paused for this account."}"#;
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\n\
                         x-amzn-ErrorType: SendingPausedException\r\ncontent-length: {}\r\n\
                         connection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        let cfg = crate::config::Config::from_lookup(&|k: &str| match k {
            "DATABASE_URL" => Some("postgres://a:b@c/d".into()),
            "SESSION_SIGNING_KEY" => Some("00".repeat(32)),
            "MAIL_BACKEND" => Some("ses".into()),
            "MAIL_FROM" => Some("no-reply@example.com".into()),
            "PUBLIC_URL" => Some("https://example.com".into()),
            _ => None,
        })
        .unwrap();
        let ses = |endpoint: String| {
            aws_sdk_sesv2::Client::from_conf(
                aws_sdk_sesv2::Config::builder()
                    .behavior_version(aws_sdk_sesv2::config::BehaviorVersion::latest())
                    .region(aws_sdk_sesv2::config::Region::new("us-east-1"))
                    .credentials_provider(aws_sdk_sesv2::config::Credentials::new("a", "b", None, None, "test"))
                    .endpoint_url(endpoint)
                    .retry_config(aws_sdk_sesv2::config::retry::RetryConfig::disabled())
                    .build(),
            )
        };
        let cfg = Arc::new(cfg);
        let mailer = Mailer { cfg: cfg.clone(), ses: Some(ses(endpoint)) };
        let err = mailer.send("a@example.com", "Confirm", "code").await.unwrap_err();
        assert_eq!(err.message, "SES send failed: SendingPausedException: Sending paused for this account.");

        // No answer at all: the reason is in the causes.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let nowhere = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let mailer = Mailer { cfg, ses: Some(ses(nowhere)) };
        let err = mailer.send("a@example.com", "Confirm", "code").await.unwrap_err();
        assert!(err.message.starts_with("SES send failed: dispatch failure: "), "{}", err.message);
        assert!(err.message.to_lowercase().contains("connect"), "{}", err.message);
    }
}
