use crate::config::{Config, MailBackend};
use crate::error::{AppError, AppResult};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Pace = governor::RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

/// One account mail waiting its turn.
struct Outgoing {
    to: String,
    subject: String,
    body: String,
    /// When its link stops working: a mail that waits past this is dropped
    /// (and alarmed) rather than sent dead.
    send_by: Option<Instant>,
}

#[derive(Clone)]
pub struct Mailer {
    cfg: Arc<Config>,
    /// Under `ses`, the queue the one sender drains at the account's rate.
    queue: Option<tokio::sync::mpsc::Sender<Outgoing>>,
}

/// How long one attempt at a send, and the send with its retries, may take.
/// The SDK sets only a connect timeout: an endpoint that took the connection
/// and never answered held the send, silently, for good -- no log line, no
/// alarm (the audit's pass 25).
const SEND_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// The most mails waiting: an hour at the account's rate, and never more than
/// this. A mail past it is not queued, and alarms.
const MAX_QUEUED: u32 = 20_000;

fn with_timeouts(
    conf: aws_sdk_sesv2::config::Builder,
    attempt: Duration,
    total: Duration,
) -> aws_sdk_sesv2::config::Builder {
    conf.timeout_config(
        aws_sdk_sesv2::config::timeout::TimeoutConfig::builder()
            .operation_attempt_timeout(attempt)
            .operation_timeout(total)
            .build(),
    )
}

/// One send every `1 / per_second` seconds, no burst: SES counts its rate
/// per second, and a burst of the whole second's worth at once is what it
/// refuses.
fn pace(per_second: u32) -> Pace {
    let every = Duration::from_secs(1) / per_second.max(1);
    governor::RateLimiter::direct(governor::Quota::with_period(every).expect("a positive period"))
}

impl Mailer {
    pub async fn new(cfg: Arc<Config>) -> Self {
        match cfg.mail_backend {
            MailBackend::Ses => {
                let aws = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
                // A throttled attempt is retried no sooner than a second on,
                // not at once, past the rate it was refused for.
                let conf = aws_sdk_sesv2::config::Builder::from(&aws).retry_config(
                    aws_sdk_sesv2::config::retry::RetryConfig::standard()
                        .with_initial_backoff(Duration::from_secs(1)),
                );
                let client = aws_sdk_sesv2::Client::from_conf(
                    with_timeouts(conf, SEND_ATTEMPT_TIMEOUT, SEND_TIMEOUT).build(),
                );
                let capacity = cfg.mail_max_per_second.saturating_mul(3600).min(MAX_QUEUED);
                Self::with_ses(cfg, client, capacity as usize)
            }
            MailBackend::Console | MailBackend::File => Self { cfg, queue: None },
        }
    }

    /// Under `ses`: a queue of `capacity`, and the one task that sends from
    /// it. Each waiting send used to wait on the rate limiter by itself, and
    /// a limiter is not a queue: under a sustained flood the newest waiter
    /// won each turn, so the mail queued first -- a reset, a real registrant's
    /// confirmation -- was never sent, nothing bounded the waiters (10 KB
    /// each), and nothing said so (the audit's pass 25). In order now, and
    /// bounded.
    fn with_ses(cfg: Arc<Config>, client: aws_sdk_sesv2::Client, capacity: usize) -> Self {
        let (queue, mut waiting) = tokio::sync::mpsc::channel::<Outgoing>(capacity.max(1));
        let pace = pace(cfg.mail_max_per_second);
        let from = cfg.mail_from.clone();
        tokio::spawn(async move {
            while let Some(mail) = waiting.recv().await {
                pace.until_ready().await;
                if mail.send_by.is_some_and(|by| Instant::now() > by) {
                    tracing::error!(
                        alarm = "mail_failed",
                        subject = %mail.subject,
                        "email failed to send: it waited in the mail queue past its link's life"
                    );
                    continue;
                }
                // Sent on its own, so that a slow answer holds up nothing
                // behind it; the pace above keeps to the rate.
                let (client, from) = (client.clone(), from.clone());
                tokio::spawn(async move {
                    if let Err(err) = deliver(&client, &from, &mail.to, &mail.subject, &mail.body).await {
                        tracing::error!(
                            alarm = "mail_failed",
                            error = %err.message,
                            subject = %mail.subject,
                            "email failed to send"
                        );
                    }
                });
            }
        });
        Self { cfg, queue: Some(queue) }
    }

    /// In `console` mode the message is written to the log instead of being sent.
    /// That is what makes the local registration and password-reset flows usable
    /// without any AWS access: the confirmation code is in the server's stdout.
    ///
    /// Under `ses` the mail is queued, and sent in its turn: an error here is
    /// one that kept it out of the queue; one SES returns is logged when it
    /// comes, with the same alarm field.
    pub async fn send(&self, to: &str, subject: &str, body: &str) -> AppResult<()> {
        self.send_within(to, subject, body, None).await
    }

    /// [`Mailer::send`] for a mail whose link works for `valid_for`: one that
    /// would go out after that is not sent.
    pub async fn send_within(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        valid_for: Option<Duration>,
    ) -> AppResult<()> {
        if self.cfg.mail_backend == MailBackend::File {
            return self.write_to_outbox(to, subject, body).await;
        }
        match &self.queue {
            None => {
                tracing::info!(
                    to,
                    subject,
                    "\n---- email (MAIL_BACKEND=console) ----\n{body}\n--------------------------------------"
                );
                Ok(())
            }
            Some(queue) => {
                let mail = Outgoing {
                    to: to.to_string(),
                    subject: subject.to_string(),
                    body: body.to_string(),
                    send_by: valid_for.map(|d| Instant::now() + d),
                };
                queue.try_send(mail).map_err(|e| match e {
                    tokio::sync::mpsc::error::TrySendError::Full(_) => {
                        AppError::internal("the mail queue is full: not sent")
                    }
                    tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                        AppError::internal("the mail sender has stopped: not sent")
                    }
                })
            }
        }
    }
}

/// One mail through SES, now.
async fn deliver(
    client: &aws_sdk_sesv2::Client,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> AppResult<()> {
    let destination = aws_sdk_sesv2::types::Destination::builder().to_addresses(to).build();
    let content = aws_sdk_sesv2::types::EmailContent::builder()
        .simple(
            aws_sdk_sesv2::types::Message::builder()
                .subject(
                    // SES reads a part with no charset as 7-bit ASCII; the
                    // mails name accounts, and a name may be `李小龍` or `émile`.
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
        .from_email_address(from)
        .destination(destination)
        .content(content)
        .send()
        .await
        .map_err(|e| AppError::internal(format!("SES send failed: {}", ses_error(&e))))?;
    Ok(())
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
                (None, Some(message)) => format!("HTTP {}: {message}", service.raw().status().as_u16()),
                (None, None) => format!("HTTP {}", service.raw().status().as_u16()),
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
    /// look for it. The sources are compiled in, as a unit test's fixtures are.
    #[test]
    fn every_failed_send_is_logged_as_the_alarm_expects() {
        let infra = include_str!("../../infra/ses.tf");
        assert!(infra.contains(r#"pattern        = "{ $.fields.alarm = \"mail_failed\" }""#));
        // Without whitespace, so that a call split across lines counts.
        let auth: String = include_str!("routes/auth.rs").split_whitespace().collect();
        let sends = auth.matches("mailer.send(").count() + auth.matches("mailer.send_within(").count();
        assert!(sends >= 3, "{sends}");
        assert_eq!(auth.matches(r#"tracing::error!(alarm="mail_failed","#).count(), sends);
        // And the queue's own failures: SES's answer, a mail that waited too long.
        // The code, not these tests, which spell the pattern out.
        let code = include_str!("email.rs").split("#[cfg(test)]").next().unwrap();
        let email: String = code.split_whitespace().collect();
        assert_eq!(email.matches(r#"tracing::error!(alarm="mail_failed","#).count(), 2);
    }

    /// A stand-in for SES's send endpoint: answers each request whole with
    /// `status`, the error type `kind` (none for a success) and `body`, or
    /// never, with `status` 0. Returns its URL and each request's arrival and
    /// body.
    async fn fake_ses(
        status: u16,
        kind: &'static str,
        body: &'static str,
    ) -> (String, Arc<std::sync::Mutex<Vec<(Instant, String)>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let arrivals = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = arrivals.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let seen = seen.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else { return };
                        if n == 0 {
                            return;
                        }
                        request.extend_from_slice(&buf[..n]);
                        let text = String::from_utf8_lossy(&request).into_owned();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let length = text[..end]
                                .lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.eq_ignore_ascii_case("content-length").then(|| v.trim().parse().ok())?
                                })
                                .unwrap_or(0usize);
                            if request.len() >= end + 4 + length {
                                seen.lock().unwrap().push((Instant::now(), text[end + 4..].to_string()));
                                if status == 0 {
                                    // Holds the connection and says nothing.
                                    tokio::time::sleep(Duration::from_secs(3600)).await;
                                    return;
                                }
                                let kind = match kind {
                                    "" => String::new(),
                                    kind => format!("x-amzn-ErrorType: {kind}\r\n"),
                                };
                                let response = format!(
                                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{kind}\
                                     content-length: {}\r\n\r\n{body}",
                                    body.len()
                                );
                                let _ = socket.write_all(response.as_bytes()).await;
                                request.clear();
                            }
                        }
                    }
                });
            }
        });
        (endpoint, arrivals)
    }

    fn ses_client(endpoint: String, attempt: Duration, total: Duration) -> aws_sdk_sesv2::Client {
        let conf = aws_sdk_sesv2::Config::builder()
            .behavior_version(aws_sdk_sesv2::config::BehaviorVersion::latest())
            .region(aws_sdk_sesv2::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_sesv2::config::Credentials::new("a", "b", None, None, "test"))
            .endpoint_url(endpoint)
            .retry_config(aws_sdk_sesv2::config::retry::RetryConfig::disabled());
        aws_sdk_sesv2::Client::from_conf(with_timeouts(conf, attempt, total).build())
    }

    fn ses_mailer(endpoint: String, per_second: u32, capacity: usize) -> Mailer {
        let per_second = per_second.to_string();
        let cfg = crate::config::Config::from_lookup(&|k: &str| match k {
            "DATABASE_URL" => Some("postgres://a:b@c/d".into()),
            "SESSION_SIGNING_KEY" => Some("00".repeat(32)),
            "MAIL_BACKEND" => Some("ses".into()),
            "MAIL_FROM" => Some("no-reply@example.com".into()),
            "PUBLIC_URL" => Some("https://example.com".into()),
            "MAIL_MAX_PER_SECOND" => Some(per_second.clone()),
            _ => None,
        })
        .unwrap();
        let client = ses_client(endpoint, SEND_ATTEMPT_TIMEOUT, SEND_TIMEOUT);
        Mailer::with_ses(Arc::new(cfg), client, capacity)
    }

    /// Waits for `count` requests to reach the stand-in, or five seconds.
    async fn arrived(
        arrivals: &Arc<std::sync::Mutex<Vec<(Instant, String)>>>,
        count: usize,
    ) -> Vec<(Instant, String)> {
        for _ in 0..250 {
            if arrivals.lock().unwrap().len() >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        arrivals.lock().unwrap().clone()
    }

    /// A send SES refuses is logged with SES's own code and message: what an
    /// operator reads to tell a paused account from an unverified address or
    /// a missing permission. It used to read `SES send failed: service error`,
    /// the same for every one of them.
    #[tokio::test]
    async fn a_refused_send_keeps_what_ses_said() {
        let (endpoint, _) =
            fake_ses(400, "SendingPausedException", r#"{"message":"Sending paused for this account."}"#).await;
        let client = ses_client(endpoint, SEND_ATTEMPT_TIMEOUT, SEND_TIMEOUT);
        let err = deliver(&client, "no-reply@example.com", "a@example.com", "Confirm", "code").await.unwrap_err();
        assert_eq!(err.message, "SES send failed: SendingPausedException: Sending paused for this account.");

        // No answer at all: the reason is in the causes.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let nowhere = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let client = ses_client(nowhere, SEND_ATTEMPT_TIMEOUT, SEND_TIMEOUT);
        let err = deliver(&client, "no-reply@example.com", "a@example.com", "Confirm", "code").await.unwrap_err();
        assert!(err.message.starts_with("SES send failed: dispatch failure: "), "{}", err.message);
        assert!(err.message.to_lowercase().contains("connect"), "{}", err.message);
    }

    /// An endpoint that takes the request and never answers is a failed send
    /// once the attempt's time is up -- logged, and so alarmed -- where it
    /// held the send for good.
    #[tokio::test]
    async fn a_send_nobody_answers_times_out() {
        let (endpoint, _) = fake_ses(0, "", "").await;
        let client = ses_client(endpoint, Duration::from_millis(500), Duration::from_secs(1));
        let started = Instant::now();
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            deliver(&client, "no-reply@example.com", "a@example.com", "Confirm", "code"),
        )
        .await
        .expect("the send ended by itself")
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert!(err.message.to_lowercase().contains("timeout"), "{}", err.message);
    }

    /// Sends reach SES at the account's rate and in the order they were
    /// queued: a burst neither goes out at once, which SES refused past its
    /// rate, nor lets the newest mail jump the oldest, which starved a reset
    /// behind a flood.
    #[tokio::test]
    async fn a_burst_of_sends_is_paced_in_order() {
        let (endpoint, arrivals) = fake_ses(200, "", r#"{"MessageId":"x"}"#).await;
        let mailer = ses_mailer(endpoint, 4, 100);
        for i in 0..8 {
            mailer.send(&format!("u{i}@example.com"), "Confirm", "code").await.unwrap();
        }
        let mut at = arrived(&arrivals, 8).await;
        assert_eq!(at.len(), 8);
        at.sort_by_key(|(when, _)| *when);
        // Eight at four a second: seven gaps of a quarter second at least.
        let spread = at[7].0 - at[0].0;
        assert!(spread >= Duration::from_millis(1600), "{spread:?}");
        for (i, (_, body)) in at.iter().enumerate() {
            assert!(body.contains(&format!("u{i}@example.com")), "#{i}: {body}");
        }
    }

    /// The queue is bounded: past it a mail is refused -- which its caller
    /// logs with the alarm field -- rather than held in memory without limit.
    #[tokio::test]
    async fn a_full_mail_queue_refuses_rather_than_growing() {
        let (endpoint, _) = fake_ses(200, "", r#"{"MessageId":"x"}"#).await;
        let mailer = ses_mailer(endpoint, 1, 2);
        let mut refused = 0;
        for i in 0..10 {
            if let Err(err) = mailer.send(&format!("u{i}@example.com"), "Confirm", "code").await {
                assert_eq!(err.message, "the mail queue is full: not sent");
                refused += 1;
            }
        }
        // Two waiting, and at most two taken by the sender.
        assert!(refused >= 6, "{refused} refused");
    }

    /// A mail whose link would be dead by the time its turn comes is not sent.
    #[tokio::test]
    async fn a_mail_that_waited_past_its_link_is_not_sent() {
        let (endpoint, arrivals) = fake_ses(200, "", r#"{"MessageId":"x"}"#).await;
        let mailer = ses_mailer(endpoint, 1, 10);
        mailer.send("first@example.com", "Confirm", "code").await.unwrap();
        mailer
            .send_within("reset@example.com", "Reset", "link", Some(Duration::from_millis(100)))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(2000)).await;
        let got = arrivals.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].1.contains("first@example.com"));
    }
}
