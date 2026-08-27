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
