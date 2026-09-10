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

/// An action logged with free-text detail in `reason`.
///
/// Its reason for existing is the destructive endpoints: each writes a census
/// of what it is about to destroy, *before* destroying it and in the same
/// transaction, so the row exists whether or not the delete succeeds and
/// describes state that by the time anyone reads it is gone. That census is
/// what makes a selective restore tractable (PLAN.md, "Making backups visible" and "Restore") —
/// the first question after a mistaken purge is what was lost, and the
/// database no longer contains the answer.
pub async fn log_detail(
    conn: &mut PgConnection,
    action: &str,
    actor_user_id: Uuid,
    target_type: &str,
    target_id: String,
    job_id: Option<Uuid>,
    census: String,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO audit_log
             (action, actor_user_id, target_type, target_id, job_id, reason)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(action)
    .bind(actor_user_id)
    .bind(target_type)
    .bind(target_id)
    .bind(job_id)
    .bind(census)
    .execute(conn)
    .await?;
    Ok(())
}
