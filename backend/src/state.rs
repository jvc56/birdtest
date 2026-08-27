use crate::artifacts::ArtifactStore;
use crate::config::Config;
use crate::email::Mailer;
use crate::ratelimit::RateLimiters;
use crate::sse::SseBroadcaster;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: Arc<Config>,
    pub sse: SseBroadcaster,
    pub limits: RateLimiters,
    pub mailer: Mailer,
    pub artifacts: ArtifactStore,
}
