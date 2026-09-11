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
            MailBackend::Console => None,
        };
        Self { cfg, ses }
    }

    /// In `console` mode the message is written to the log instead of being sent.
    /// That is what makes the local registration and password-reset flows usable
    /// without any AWS access: the confirmation code is in the server's stdout.
    pub async fn send(&self, to: &str, subject: &str, body: &str) -> AppResult<()> {
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
