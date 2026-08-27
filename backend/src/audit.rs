//! Append-only record of every significant action, for debugging and
//! accountability. Writes are best-effort in the sense that they share the
//! caller's transaction — an audit failure rolls back the action it describes.

use crate::error::AppResult;
use sqlx::PgConnection;
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn log(
    conn: &mut PgConnection,
    action: &str,
    actor_user_id: Option<Uuid>,
    actor_anon_uuid: Option<Uuid>,
    target_type: Option<&str>,
    target_id: Option<String>,
    job_id: Option<Uuid>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO audit_log
             (action, actor_user_id, actor_anon_uuid, target_type, target_id, job_id)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(action)
    .bind(actor_user_id)
    .bind(actor_anon_uuid)
    .bind(target_type)
    .bind(target_id)
    .bind(job_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Status transitions carry the old and new value so the log reads as a history.
pub async fn log_status_change(
    conn: &mut PgConnection,
    action: &str,
    actor_user_id: Uuid,
    job_id: Uuid,
    old_status: &str,
    new_status: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO audit_log
             (action, actor_user_id, target_type, target_id, job_id, old_status, new_status)
         VALUES ($1, $2, 'job', $3, $4, $5, $6)",
    )
    .bind(action)
    .bind(actor_user_id)
    .bind(job_id.to_string())
    .bind(job_id)
    .bind(old_status)
    .bind(new_status)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn log_ban(
    conn: &mut PgConnection,
    actor_user_id: Uuid,
    target_id: String,
    reason: Option<String>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO audit_log (action, actor_user_id, target_type, target_id, reason)
         VALUES ('worker.banned', $1, 'worker', $2, $3)",
    )
    .bind(actor_user_id)
    .bind(target_id)
    .bind(reason)
    .execute(conn)
    .await?;
    Ok(())
}
