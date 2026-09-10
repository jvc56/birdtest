use crate::config::Config;
use crate::error::{AppError, AppResult};
use std::sync::Arc;

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
            .map_err(|e| AppError::internal(format!("S3 put {key} failed: {e}")))?;
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
            Err(e) => match e.into_service_error() {
                aws_sdk_s3::operation::head_object::HeadObjectError::NotFound(_) => Ok(false),
                other => Err(AppError::internal(format!("S3 head {key} failed: {other}"))),
            },
        }
    }

    pub async fn get(&self, key: &str) -> AppResult<Vec<u8>> {
        let object = self
            .client
            .get_object()
            .bucket(&self.cfg.s3_bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| AppError::not_found(format!("no artifact at {key}: {e}")))?;

        let bytes = object
            .body
            .collect()
            .await
            .map_err(|e| AppError::internal(format!("S3 read {key} failed: {e}")))?;
        Ok(bytes.into_bytes().to_vec())
    }
}
