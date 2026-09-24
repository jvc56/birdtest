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
                                aws_sdk_sesv2::types::Content::builder()
                                    .data(subject)
                                    .build()
                                    .map_err(|e| AppError::internal(e.to_string()))?,
                            )
                            .body(
                                aws_sdk_sesv2::types::Body::builder()
                                    .text(
                                        aws_sdk_sesv2::types::Content::builder()
                                            .data(body)
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
                    .map_err(|e| AppError::internal(format!("SES send failed: {e}")))?;
                Ok(())
            }
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
}
