//! Read-only view of the backup history, for the admin dashboard.
//!
//! Backups are performed by `scripts/backup.sh` running as a scheduled task,
//! never by this process — see PLAN.md, "Where the dump runs, and where it lands". The deployment most in
//! need of a working backup is the one whose backend is crashlooping, so the
//! backend's whole relationship to backups is reading the rows the backup task
//! writes and saying how old they are.

use crate::error::AppResult;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// How old the newest successful backup may be before the dashboard calls it
/// stale. Matches the CloudWatch alarm in infra/backup.tf, so the admin page
/// and the alert agree about what "overdue" means for a nightly schedule.
pub const STALE_AFTER_HOURS: i64 = 36;

/// How many runs the dashboard card shows.
const RECENT_LIMIT: i64 = 10;

#[derive(Serialize)]
pub struct BackupRun {
    pub id: Uuid,
    pub kind: String,
    pub location: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub duration_seconds: i64,
    pub dump_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub ok: bool,
    /// Summed across every table in the manifest — one number the card can
    /// show, where the per-table detail is only useful during a restore.
    pub total_rows: i64,
}

#[derive(Serialize)]
pub struct BackupStatus {
    /// When the most recent *successful* backup finished. A failed run does not
    /// reset this: what matters is the age of the newest restorable thing.
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_success_age_seconds: Option<i64>,
    /// No successful backup within `STALE_AFTER` — including the case of never
    /// having had one, which is what a freshly deployed stack looks like.
    pub stale: bool,
    pub recent: Vec<BackupRun>,
}

/// Rows summed across the manifest. A manifest that is not an object — which
/// a failed run writes as `{}`, and a hand-inserted row could write as
/// anything — counts as zero rather than failing the page.
fn total_rows(counts: &serde_json::Value) -> i64 {
    counts
        .as_object()
        .map(|tables| tables.values().filter_map(serde_json::Value::as_i64).sum())
        .unwrap_or(0)
}

/// Whether the newest successful backup is old enough to report. `None` — no
/// successful backup at all — is stale by definition: a stack that has never
/// produced one is the case this most needs to be loud about.
fn is_stale(last_success_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    last_success_at.is_none_or(|at| (now - at).num_hours() >= STALE_AFTER_HOURS)
}

fn row_to_run(row: &sqlx::postgres::PgRow) -> BackupRun {
    let started_at: DateTime<Utc> = row.get("started_at");
    let finished_at: DateTime<Utc> = row.get("finished_at");
    let counts: serde_json::Value = row.get("row_counts");
    BackupRun {
        id: row.get("id"),
        kind: row.get("kind"),
        // Exactly one of the two is set, per the backups_has_single_location
        // check, so collapsing them loses nothing the caller needs.
        location: row
            .get::<Option<String>, _>("s3_key")
            .or_else(|| row.get::<Option<String>, _>("snapshot_id")),
        started_at,
        finished_at,
        duration_seconds: (finished_at - started_at).num_seconds(),
        dump_bytes: row.get("dump_bytes"),
        sha256: row.get("sha256"),
        ok: row.get("ok"),
        total_rows: total_rows(&counts),
    }
}

pub async fn status(pool: &PgPool) -> AppResult<BackupStatus> {
    let rows = sqlx::query(
        "SELECT id, kind, s3_key, snapshot_id, started_at, finished_at,
                dump_bytes, sha256, ok, row_counts
         FROM backups
         ORDER BY finished_at DESC
         LIMIT $1",
    )
    .bind(RECENT_LIMIT)
    .fetch_all(pool)
    .await?;

    // Deliberately not `recent.iter().find(|r| r.ok)`: a run of failures longer
    // than RECENT_LIMIT would then report "never", which reads as a stack that
    // has never been backed up rather than one that is breaking.
    let last_success_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        "SELECT finished_at FROM backups WHERE ok ORDER BY finished_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    let now = Utc::now();
    Ok(BackupStatus {
        last_success_at,
        last_success_age_seconds: last_success_at.map(|at| (now - at).num_seconds()),
        stale: is_stale(last_success_at, now),
        recent: rows.iter().map(row_to_run).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A fixed `now`, so the assertions are about the window and not about how
    /// long the test took to reach its second line.
    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-07T03:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn hours_ago(hours: i64) -> Option<DateTime<Utc>> {
        Some(now() - chrono::Duration::hours(hours))
    }

    #[test]
    fn never_backed_up_is_stale() {
        assert!(is_stale(None, now()));
    }

    #[test]
    fn a_backup_within_the_window_is_not_stale() {
        assert!(!is_stale(hours_ago(1), now()));
        // A nightly schedule that slipped a few hours is not yet news.
        assert!(!is_stale(hours_ago(STALE_AFTER_HOURS - 1), now()));
    }

    #[test]
    fn a_missed_night_is_stale() {
        assert!(is_stale(hours_ago(STALE_AFTER_HOURS), now()));
        assert!(is_stale(hours_ago(STALE_AFTER_HOURS + 24), now()));
    }

    #[test]
    fn row_counts_sum_across_tables() {
        assert_eq!(total_rows(&json!({"users": 3, "tasks": 40})), 43);
    }

    #[test]
    fn a_failed_runs_empty_manifest_counts_as_zero() {
        assert_eq!(total_rows(&json!({})), 0);
        assert_eq!(total_rows(&json!(null)), 0);
        // Non-numeric values are skipped rather than poisoning the total.
        assert_eq!(total_rows(&json!({"users": 3, "note": "partial"})), 3);
    }
}
