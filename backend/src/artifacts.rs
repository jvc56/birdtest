use crate::config::Config;
use crate::error::{AppError, AppResult};
use std::sync::Arc;

/// An error with its causes, for errors only an admin reads (stored for an
/// import, an export or a derived build, or answered on an admin route):
/// "dispatch failure" alone told the admin nothing (the audit's pass 7).
fn chain(error: &dyn std::error::Error) -> String {
    aws_sdk_s3::error::DisplayErrorContext(error).to_string()
}

/// [`chain`] for an SDK call's error: a service error as its code and message
/// (`AccessDenied: Access Denied`) rather than with the raw response it came
/// in, whose headers and body made a stored error of two kilobytes (pass 10);
/// a transport failure with everything, since that is where its cause is.
fn sdk<E>(error: &aws_sdk_s3::error::SdkError<E, aws_sdk_s3::config::http::HttpResponse>) -> String
where
    E: std::error::Error + aws_sdk_s3::error::ProvideErrorMetadata + 'static,
{
    match error {
        aws_sdk_s3::error::SdkError::ServiceError(service) => {
            let err = service.err();
            match (err.code(), err.message()) {
                (Some(code), Some(message)) => format!("{code}: {message}"),
                (Some(code), None) => code.to_string(),
                // No code: not S3 answering -- a proxy's HTML, an empty 403 --
                // whose status and the start of whose body are the cause.
                _ => {
                    let raw = service.raw();
                    let body = raw
                        .body()
                        .bytes()
                        .map(|b| String::from_utf8_lossy(&b[..b.len().min(200)]).into_owned())
                        .unwrap_or_default();
                    let status = raw.status().as_u16();
                    match body.trim() {
                        "" => format!("HTTP {status}"),
                        body => format!("HTTP {status}: {body}"),
                    }
                }
            }
        }
        other => chain(other),
    }
}

/// An SDK error as a response to a worker may carry it, with its causes --
/// request ids, codes, a host name holding the bucket's -- logged beside it:
/// the causes in the body told a worker more than it needs (pass 8). Only the
/// object fetch a worker's request makes uses it.
fn cause(error: &dyn std::error::Error) -> String {
    tracing::warn!(error = %aws_sdk_s3::error::DisplayErrorContext(error), "an object store call failed");
    error.to_string()
}

/// S3 (or MinIO in dev — the SDK is identical, only the endpoint differs).
#[derive(Clone)]
pub struct ArtifactStore {
    cfg: Arc<Config>,
    client: aws_sdk_s3::Client,
    /// Signs download links, and nothing else: `client` again unless
    /// `S3_PUBLIC_ENDPOINT` names the store as a browser reaches it. A
    /// presigned URL's signature covers its host, so one signed for
    /// `minio:9000` cannot be rewritten to `localhost` afterwards.
    presigner: aws_sdk_s3::Client,
    /// Where the clients' credentials come from, asked again for each link
    /// ([`Self::presigned_get`]). The S3 config's own accessor for it always
    /// answers `None`.
    credentials: Option<aws_sdk_s3::config::SharedCredentialsProvider>,
}

impl ArtifactStore {
    pub async fn new(cfg: Arc<Config>) -> Self {
        let aws = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client_for = |endpoint: Option<&String>| {
            let mut builder = aws_sdk_s3::config::Builder::from(&aws);
            if let Some(endpoint) = endpoint {
                // MinIO does not do virtual-host-style addressing out of the box.
                builder = builder.endpoint_url(endpoint).force_path_style(true);
            }
            aws_sdk_s3::Client::from_conf(builder.build())
        };
        let client = client_for(cfg.s3_endpoint.as_ref());
        let presigner = match &cfg.s3_public_endpoint {
            Some(public) => client_for(Some(public)),
            None => client.clone(),
        };
        Self { cfg, client, presigner, credentials: aws.credentials_provider() }
    }

    pub async fn put(&self, key: &str, body: Vec<u8>) -> AppResult<String> {
        self.client
            .put_object()
            .bucket(&self.cfg.s3_bucket)
            .key(key)
            .body(body.into())
            .send()
            .await
            .map_err(|e| AppError::internal(format!("S3 put {key} failed: {}", sdk(&e))))?;
        Ok(key.to_string())
    }

    /// Whether the object is still there. Used by the artifact rebuild path,
    /// which has to distinguish "the bytes changed" from "the object is gone"
    /// — a database restored to before an object was written is the second
    /// case, and only that one is a reason to write.
    pub async fn exists(&self, key: &str) -> AppResult<bool> {
        match self
            .client
            .head_object()
            .bucket(&self.cfg.s3_bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => match e.as_service_error() {
                Some(aws_sdk_s3::operation::head_object::HeadObjectError::NotFound(_)) => Ok(false),
                _ => Err(AppError::internal(format!("S3 head {key} failed: {}", sdk(&e)))),
            },
        }
    }

    /// Start a multipart upload, for an object too large to hold in memory.
    ///
    /// [`put`](Self::put) takes the whole body at once, which is right for a
    /// KLV (a few megabytes, built in memory anyway) and impossible for a job
    /// export (a completed opening-rack job runs to gigabytes of NDJSON). Parts
    /// are uploaded as they are produced and nothing larger than one part is
    /// ever resident.
    pub async fn start_multipart(&self, key: &str) -> AppResult<MultipartUpload> {
        let started = self
            .client
            .create_multipart_upload()
            .bucket(&self.cfg.s3_bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("S3 multipart start {key} failed: {}", sdk(&e))))?;
        let upload_id = started
            .upload_id()
            .ok_or_else(|| AppError::internal("S3 returned no upload id"))?
            .to_string();
        Ok(MultipartUpload {
            client: self.client.clone(),
            bucket: self.cfg.s3_bucket.clone(),
            key: key.to_string(),
            upload_id,
            parts: Vec::new(),
        })
    }

    /// A URL that fetches `key` directly, valid for `expires_in`.
    ///
    /// The point is that the bytes never pass through this process: a job
    /// export is far too large to read into memory and stream out again, and
    /// doing so would put it back on the connection pool that limiting the
    /// stream exists to protect. The bucket blocks public access, so a
    /// presigned URL is the only way out of it, and it is minted for an admin
    /// who has just asked for it. Signed for `S3_PUBLIC_ENDPOINT` when that is
    /// set, since the browser, not this process, follows it.
    ///
    /// A presigned URL stops working when the credentials that signed it
    /// expire, whatever its own expiry says. On ECS those are the task role's
    /// temporary ones, which the SDK's cache keeps until moments before they
    /// expire, so a link minted late in their life died minutes, or seconds,
    /// after it was handed out (S3 answers `ExpiredToken`). It is signed with
    /// credentials asked for now instead -- the newest the task has -- and
    /// for no longer than they last ([`presign_ttl`]). Static keys (MinIO, an
    /// environment's) carry no expiry and are signed for all of `expires_in`.
    pub async fn presigned_get(
        &self,
        key: &str,
        expires_in: std::time::Duration,
    ) -> AppResult<String> {
        use aws_sdk_s3::config::ProvideCredentials;
        let mut request =
            self.presigner.get_object().bucket(&self.cfg.s3_bucket).key(key).customize();
        let mut expires_in = expires_in;
        if let Some(provider) = &self.credentials {
            let credentials = provider.provide_credentials().await.map_err(|e| {
                AppError::internal(format!("S3 presign {key}: no credentials: {}", chain(&e)))
            })?;
            let now = std::time::SystemTime::now();
            expires_in = presign_ttl(expires_in, credentials.expiry(), now);
            // Not through the client's cache: these are used once, and a
            // provider of their own would be a cache partition each call.
            request = request.config_override(
                aws_sdk_s3::config::Builder::default()
                    .credentials_provider(credentials)
                    .identity_cache(aws_sdk_s3::config::IdentityCache::no_cache()),
            );
        }
        let config = aws_sdk_s3::presigning::PresigningConfig::expires_in(expires_in)
            .map_err(|e| AppError::internal(format!("invalid presigning config: {e}")))?;
        let request = request
            .presigned(config)
            .await
            .map_err(|e| AppError::internal(format!("S3 presign {key} failed: {}", sdk(&e))))?;
        Ok(request.uri().to_string())
    }

    /// Remove an object: exports, which are derived data with a finite life,
    /// and (by the derived builder) a damaged input. The leave-generation KLVs
    /// are never deleted.
    pub async fn delete(&self, key: &str) -> AppResult<()> {
        self.client
            .delete_object()
            .bucket(&self.cfg.s3_bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::internal(format!("S3 delete {key} failed: {}", sdk(&e))))?;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> AppResult<Vec<u8>> {
        Ok(self.get_bytes(key).await?.to_vec())
    }

    /// [`Self::get`] for the derived-file builder, whose errors are stored for
    /// an admin to read at `/admin/derived-data` and never sent to a client:
    /// with the SDK's whole cause, where a response carries only the first
    /// line ("dispatch failure" told the admin nothing -- the audit's pass 7).
    pub async fn get_for_build(&self, key: &str) -> AppResult<Vec<u8>> {
        Ok(self.get_bytes_as(key, true).await?.to_vec())
    }

    /// The object's bytes as S3 handed them over, without a copy.
    ///
    /// Only a missing key is a 404. Every other failure -- throttling, a
    /// timeout, credentials -- was one too, and a worker told a leave
    /// generation's KLV does not exist does not retry: every leave worker's
    /// run ended on a transient S3 error. They are 503 with a `Retry-After`.
    pub async fn get_bytes(&self, key: &str) -> AppResult<axum::body::Bytes> {
        self.get_bytes_as(key, false).await
    }

    /// `for_admin`: the error as [`sdk`] gives it, for a stored or admin-read
    /// error; otherwise plain, for a worker's response.
    async fn get_bytes_as(&self, key: &str, for_admin: bool) -> AppResult<axum::body::Bytes> {
        use aws_sdk_s3::operation::get_object::GetObjectError;
        let object = self
            .client
            .get_object()
            .bucket(&self.cfg.s3_bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                if let Some(GetObjectError::NoSuchKey(_)) = e.as_service_error() {
                    return AppError::not_found(format!("no artifact at {key}"));
                }
                let why = if for_admin { sdk(&e) } else { cause(&e.into_service_error()) };
                unavailable(format!("S3 get {key} failed: {why}"))
            })?;

        let bytes = object
            .body
            .collect()
            .await
            .map_err(|e| {
                let why = if for_admin { chain(&e) } else { cause(&e) };
                unavailable(format!("S3 read {key} failed: {why}"))
            })?;
        Ok(bytes.into_bytes())
    }
}

/// An upload in progress. Parts must be at least 5 MiB except the last, which
/// is S3's rule, so the caller buffers to [`MultipartUpload::PART_SIZE`] before
/// sending.
pub struct MultipartUpload {
    client: aws_sdk_s3::Client,
    bucket: String,
    key: String,
    upload_id: String,
    parts: Vec<aws_sdk_s3::types::CompletedPart>,
}

impl MultipartUpload {
    /// Comfortably over S3's 5 MiB minimum, and small enough that one part in
    /// memory is not worth thinking about.
    pub const PART_SIZE: usize = 8 * 1024 * 1024;

    pub async fn upload_part(&mut self, body: Vec<u8>) -> AppResult<()> {
        // Parts are numbered from 1.
        let part_number = self.parts.len() as i32 + 1;
        let uploaded = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .part_number(part_number)
            .body(body.into())
            .send()
            .await
            .map_err(|e| {
                AppError::internal(format!("S3 upload part {part_number} of {} failed: {}", self.key, sdk(&e)))
            })?;
        self.parts.push(
            aws_sdk_s3::types::CompletedPart::builder()
                .part_number(part_number)
                .set_e_tag(uploaded.e_tag().map(str::to_owned))
                .build(),
        );
        Ok(())
    }

    /// Complete the upload; one that fails to complete is aborted, so its
    /// parts are not left for the lifecycle rule.
    pub async fn finish(mut self) -> AppResult<String> {
        let completed = aws_sdk_s3::types::CompletedMultipartUpload::builder()
            .set_parts(Some(std::mem::take(&mut self.parts)))
            .build();
        let finished = self
            .client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .multipart_upload(completed)
            .send()
            .await;
        match finished {
            Ok(_) => Ok(self.key),
            Err(e) => {
                let err = AppError::internal(format!("S3 multipart finish {} failed: {}", self.key, sdk(&e)));
                self.abort().await;
                Err(err)
            }
        }
    }

    /// Abandon the upload, so its parts do not sit in the bucket being billed.
    /// The bucket also has a lifecycle rule for this, which covers the case
    /// where the process dies before it can abort.
    pub async fn abort(self) {
        if let Err(err) = self
            .client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .send()
            .await
        {
            tracing::warn!(key = %self.key, %err, "could not abort a multipart upload");
        }
    }
}

/// An object-store failure worth retrying: 503, asked back in thirty seconds.
fn unavailable(message: String) -> AppError {
    AppError {
        retry_after: Some(30),
        ..AppError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
    }
}

/// How long a link signed with credentials expiring at `expiry` may say it
/// lasts: `wanted`, or less by what they have left, short of a minute for the
/// clocks of this process and S3 to disagree by. At least a second, which is
/// the least a presigning config takes; credentials that close to expiring
/// are not what ECS hands out.
fn presign_ttl(
    wanted: std::time::Duration,
    expiry: Option<std::time::SystemTime>,
    now: std::time::SystemTime,
) -> std::time::Duration {
    const CLOCK_SLACK: std::time::Duration = std::time::Duration::from_secs(60);
    match expiry {
        None => wanted,
        Some(expiry) => {
            let left = expiry.duration_since(now).unwrap_or_default().saturating_sub(CLOCK_SLACK);
            wanted.min(left).max(std::time::Duration::from_secs(1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    const HOUR: Duration = Duration::from_secs(3600);

    /// A store whose presigner signs with `credentials`, offline: presigning
    /// makes no request.
    fn store(credentials: aws_sdk_s3::config::Credentials) -> ArtifactStore {
        let provider = aws_sdk_s3::config::SharedCredentialsProvider::new(credentials);
        let lookup = |key: &str| match key {
            "DATABASE_URL" => Some("postgres://a:b@c/d".to_string()),
            "SESSION_SIGNING_KEY" => Some("00".repeat(32)),
            _ => None,
        };
        let cfg = Arc::new(Config::from_lookup(&lookup).unwrap());
        let conf = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .credentials_provider(provider.clone())
            .build();
        let client = aws_sdk_s3::Client::from_conf(conf);
        ArtifactStore { cfg, presigner: client.clone(), client, credentials: Some(provider) }
    }

    fn expires(url: &str) -> u64 {
        let (_, rest) = url.split_once("X-Amz-Expires=").unwrap_or_else(|| panic!("{url}"));
        rest.split('&').next().unwrap().parse().unwrap()
    }

    /// U-ART-1: a download link says no longer than the credentials that sign
    /// it last. Signed with the task role's temporary credentials, a link
    /// minted late in their life said an hour and stopped working with them,
    /// minutes or seconds later (`ExpiredToken`).
    #[tokio::test]
    async fn a_link_lasts_no_longer_than_its_credentials() {
        let soon = SystemTime::now() + Duration::from_secs(20 * 60);
        let temporary = aws_sdk_s3::config::Credentials::new(
            "ASIAEXAMPLE",
            "secret",
            Some("token".to_string()),
            Some(soon),
            "test",
        );
        let url = store(temporary).presigned_get("exports/x.ndjson.gz", HOUR).await.unwrap();
        let said = expires(&url);
        assert!((19 * 60 - 5..=19 * 60).contains(&said), "{said}: {url}");
        assert!(url.contains("X-Amz-Security-Token=token"), "{url}");

        // Static keys have no expiry: the whole hour.
        let fixed =
            aws_sdk_s3::config::Credentials::new("AKIAEXAMPLE", "secret", None, None, "test");
        let url = store(fixed).presigned_get("exports/x.ndjson.gz", HOUR).await.unwrap();
        assert_eq!(expires(&url), 3600, "{url}");
    }

    /// U-ART-1: the arithmetic at its edges.
    #[test]
    fn presign_ttl_is_the_shorter_less_a_minute_and_never_zero() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert_eq!(presign_ttl(HOUR, None, now), HOUR);
        assert_eq!(presign_ttl(HOUR, Some(now + 6 * HOUR), now), HOUR);
        assert_eq!(presign_ttl(HOUR, Some(now + HOUR), now), HOUR - Duration::from_secs(60));
        let second = Duration::from_secs(1);
        assert_eq!(presign_ttl(HOUR, Some(now + Duration::from_secs(30)), now), second);
        assert_eq!(presign_ttl(HOUR, Some(now - HOUR), now), second);
    }
}
