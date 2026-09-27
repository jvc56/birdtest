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
}

impl ArtifactStore {
    pub async fn new(cfg: Arc<Config>) -> Self {
        let aws = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let mut builder = aws_sdk_s3::config::Builder::from(&aws);
        if let Some(endpoint) = &cfg.s3_endpoint {
            // MinIO does not do virtual-host-style addressing out of the box.
            builder = builder.endpoint_url(endpoint).force_path_style(true);
        }
        Self { cfg, client: aws_sdk_s3::Client::from_conf(builder.build()) }
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
    /// who has just asked for it.
    pub async fn presigned_get(
        &self,
        key: &str,
        expires_in: std::time::Duration,
    ) -> AppResult<String> {
        let config = aws_sdk_s3::presigning::PresigningConfig::expires_in(expires_in)
            .map_err(|e| AppError::internal(format!("invalid presigning config: {e}")))?;
        let request = self
            .client
            .get_object()
            .bucket(&self.cfg.s3_bucket)
            .key(key)
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
