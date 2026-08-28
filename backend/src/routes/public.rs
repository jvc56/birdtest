use crate::error::{AppError, AppResult};
use crate::jobstats::{self, JobStats};
use crate::models::job::{Job, JobType};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::convert::Infallible;
use std::time::Duration;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/jobs", get(list_jobs))
        .route("/jobs/:id", get(job_detail))
        .route("/jobs/:id/results", get(job_results))
        .route("/jobs/:id/stream", get(job_stream))
        .route("/jobs/:id/results/stream", get(job_results_stream))
        .route("/users", get(list_users))
        .route("/workers", get(list_workers))
}

#[derive(Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub page: i64,
    pub per_page: Option<i64>,
}

#[derive(Serialize)]
struct JobListItem {
    id: Uuid,
    job_type: JobType,
    status: String,
    priority: i32,
    allocation: Option<i32>,
    redundancy: i32,
    created_at: chrono::DateTime<chrono::Utc>,
    tasks_total: i64,
    tasks_completed: i64,
    /// For on-demand SPRT jobs the meaningful denominator is `max_games` /
    /// `max_pairs`, not a task count that grows as work is handed out.
    units_completed: Option<i64>,
    max_units: Option<i32>,
}

async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<super::Page<JobListItem>>> {
    let (limit, offset) = super::paginate(query.page, query.per_page);

    let rows = sqlx::query(
        "SELECT j.id, j.job_type, j.status::text AS status, j.priority, j.allocation,
                j.redundancy, j.created_at,
                (SELECT COUNT(*) FROM tasks t WHERE t.job_id = j.id) AS tasks_total,
                (SELECT COUNT(*) FROM tasks t WHERE t.job_id = j.id AND t.state = 'completed')
                    AS tasks_completed,
                (SELECT COALESCE(SUM(r.games), 0) FROM game_results r
                 JOIN tasks t ON t.id = r.task_id WHERE t.job_id = j.id) AS game_rows,
                gc.max_games, pc.max_pairs
         FROM jobs j
         LEFT JOIN job_game_config gc ON gc.job_id = j.id
         LEFT JOIN job_game_pair_config pc ON pc.job_id = j.id
         ORDER BY j.priority ASC, j.created_at DESC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs")
        .fetch_one(&state.pool)
        .await?;

    let items = rows
        .into_iter()
        .map(|row| {
            let job_type: JobType = row.get("job_type");
            // Games played; two per pair.
            let game_rows: i64 = row.get("game_rows");
            let max_games: Option<i32> = row.get("max_games");
            let max_pairs: Option<i32> = row.get("max_pairs");
            let (units_completed, max_units) = match job_type {
                JobType::Games => (Some(game_rows), max_games),
                JobType::GamePairs => (Some(game_rows / 2), max_pairs),
                _ => (None, None),
            };
            JobListItem {
                id: row.get("id"),
                job_type,
                status: row.get("status"),
                priority: row.get("priority"),
                allocation: row.get("allocation"),
                redundancy: row.get("redundancy"),
                created_at: row.get("created_at"),
                tasks_total: row.get("tasks_total"),
                tasks_completed: row.get("tasks_completed"),
                units_completed,
                max_units,
            }
        })
        .collect();

    Ok(Json(super::Page { items, total, page: query.page.max(0), per_page: limit }))
}

async fn load_job(state: &AppState, id: Uuid) -> AppResult<Job> {
    sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("no such job"))
}

async fn job_detail(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<JobStats>> {
    let job = load_job(&state, id).await?;
    Ok(Json(jobstats::compute(&state.pool, &job).await?))
}

#[derive(Deserialize)]
struct ResultsQuery {
    #[serde(default)]
    page: i64,
    per_page: Option<i64>,
    /// Filter to one contributor: a username, or an anonymous worker UUID.
    worker: Option<String>,
    /// Opening-rack jobs only: look up one rack's full ranked move list.
    rack: Option<String>,
}

async fn job_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ResultsQuery>,
) -> AppResult<Json<super::Page<serde_json::Value>>> {
    let job = load_job(&state, id).await?;
    let (limit, offset) = super::paginate(query.page, query.per_page);

    if let (JobType::OpeningRackAnalysis, Some(rack)) = (job.job_type, query.rack.as_ref()) {
        return Ok(Json(rack_lookup(&state, id, rack).await?));
    }

    let items = match job.job_type {
        JobType::OpeningRackAnalysis => {
            let rows = sqlx::query(
                "SELECT r.task_id, r.rack, r.best_move, r.best_score, r.best_equity,
                        r.num_moves, r.submitted_at, u.username, c.claimed_by_anon_uuid
                 FROM position_analysis_records r
                 JOIN tasks t ON t.id = r.task_id
                 JOIN task_claims c ON c.id = r.task_claim_id
                 LEFT JOIN users u ON u.id = c.claimed_by_user_id
                 WHERE t.job_id = $1
                   AND ($2::text IS NULL
                        OR u.username = $2 OR c.claimed_by_anon_uuid::text = $2)
                 ORDER BY r.submitted_at DESC, r.rack ASC
                 LIMIT $3 OFFSET $4",
            )
            .bind(id)
            .bind(&query.worker)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.pool)
            .await?;

            rows.into_iter()
                .map(|r| {
                    serde_json::json!({
                        "task_id": r.get::<Uuid, _>("task_id"),
                        "rack": r.get::<String, _>("rack"),
                        "best_move": r.get::<String, _>("best_move"),
                        "best_score": r.get::<i32, _>("best_score"),
                        "best_equity": r.get::<f64, _>("best_equity"),
                        "num_moves": r.get::<i32, _>("num_moves"),
                        "submitted_at": r.get::<chrono::DateTime<chrono::Utc>, _>("submitted_at"),
                        "username": r.get::<Option<String>, _>("username"),
                        "anon_uuid": r.get::<Option<Uuid>, _>("claimed_by_anon_uuid"),
                    })
                })
                .collect()
        }
        JobType::Games | JobType::GamePairs => {
            let rows = sqlx::query(
                "SELECT r.task_id, r.games, r.wins, r.losses, r.ties,
                        r.p1_score_mean, r.p1_score_sd, r.p2_score_mean, r.p2_score_sd,
                        r.divergent_games, r.divergent_wins, r.divergent_losses,
                        r.divergent_ties, r.submitted_at,
                        t.seed, u.username, c.claimed_by_anon_uuid
                 FROM game_results r
                 JOIN tasks t ON t.id = r.task_id
                 JOIN task_claims c ON c.id = r.task_claim_id
                 LEFT JOIN users u ON u.id = c.claimed_by_user_id
                 WHERE t.job_id = $1
                   AND ($2::text IS NULL
                        OR u.username = $2 OR c.claimed_by_anon_uuid::text = $2)
                 ORDER BY r.submitted_at DESC
                 LIMIT $3 OFFSET $4",
            )
            .bind(id)
            .bind(&query.worker)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.pool)
            .await?;

            rows.into_iter()
                .map(|r| {
                    serde_json::json!({
                        "task_id": r.get::<Uuid, _>("task_id"),
                        "seed": r.get::<Option<i64>, _>("seed"),
                        "games": r.get::<i32, _>("games"),
                        "wins": r.get::<i32, _>("wins"),
                        "losses": r.get::<i32, _>("losses"),
                        "ties": r.get::<i32, _>("ties"),
                        "p1_score_mean": r.get::<f64, _>("p1_score_mean"),
                        "p1_score_sd": r.get::<f64, _>("p1_score_sd"),
                        "p2_score_mean": r.get::<f64, _>("p2_score_mean"),
                        "p2_score_sd": r.get::<f64, _>("p2_score_sd"),
                        "divergent_games": r.get::<Option<i32>, _>("divergent_games"),
                        "divergent_wins": r.get::<Option<i32>, _>("divergent_wins"),
                        "divergent_losses": r.get::<Option<i32>, _>("divergent_losses"),
                        "divergent_ties": r.get::<Option<i32>, _>("divergent_ties"),
                        "submitted_at": r.get::<chrono::DateTime<chrono::Utc>, _>("submitted_at"),
                        "username": r.get::<Option<String>, _>("username"),
                        "anon_uuid": r.get::<Option<Uuid>, _>("claimed_by_anon_uuid"),
                    })
                })
                .collect()
        }
        JobType::LeaveGeneration => {
            let rows = sqlx::query(
                "SELECT rack, generation, occurrence_count,
                        equity_sum / NULLIF(occurrence_count, 0) AS mean_equity, updated_at
                 FROM leave_rack_progress
                 WHERE job_id = $1
                 ORDER BY generation DESC, occurrence_count ASC, rack ASC
                 LIMIT $2 OFFSET $3",
            )
            .bind(id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.pool)
            .await?;

            rows.into_iter()
                .map(|r| {
                    serde_json::json!({
                        "rack": r.get::<String, _>("rack"),
                        "generation": r.get::<i32, _>("generation"),
                        "occurrence_count": r.get::<i64, _>("occurrence_count"),
                        "mean_equity": r.get::<Option<f64>, _>("mean_equity"),
                        "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
                    })
                })
                .collect()
        }
    };

    Ok(Json(super::Page { items, total: -1, page: query.page.max(0), per_page: limit }))
}

/// The full ranked move list for one rack, from `position_analysis_moves`.
async fn rack_lookup(
    state: &AppState,
    job_id: Uuid,
    rack: &str,
) -> AppResult<super::Page<serde_json::Value>> {
    let canonical: String = {
        let mut chars: Vec<char> = rack.trim().to_uppercase().chars().collect();
        chars.sort_unstable();
        chars.into_iter().collect()
    };

    let rows = sqlx::query(
        "SELECT m.rank, m.move, m.score, m.equity
         FROM position_analysis_moves m
         JOIN tasks t ON t.id = m.task_id
         WHERE t.job_id = $1 AND m.rack = $2
         ORDER BY m.rank ASC",
    )
    .bind(job_id)
    .bind(&canonical)
    .fetch_all(&state.pool)
    .await?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "rank": r.get::<i16, _>("rank"),
                "move": r.get::<String, _>("move"),
                "score": r.get::<i32, _>("score"),
                "equity": r.get::<f64, _>("equity"),
            })
        })
        .collect();

    let total = items.len() as i64;
    Ok(super::Page { items, total, page: 0, per_page: total.max(1) })
}

/// One SSE event per accepted result, carrying the same payload `GET
/// /api/jobs/:id` would return.
async fn job_stream(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let job = load_job(&state, id).await?;
    let initial = jobstats::compute(&state.pool, &job).await?;
    let initial = serde_json::to_string(&initial).unwrap_or_else(|_| "{}".into());

    let receiver = state.sse.subscribe(id);
    let updates = tokio_stream::wrappers::BroadcastStream::new(receiver)
        .filter_map(|msg| async move { msg.ok() });

    let stream = futures::stream::once(async move { initial })
        .chain(updates)
        .map(|payload| Ok(Event::default().event("stats").data(payload)));

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

/// Newline-delimited JSON of every record for the job, streamed straight from a
/// cursor so an offline analysis download never buffers the whole job in memory.
async fn job_results_stream(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    let job = load_job(&state, id).await?;
    let pool = state.pool.clone();

    let stream = async_stream::stream! {
        let query = match job.job_type {
            JobType::OpeningRackAnalysis =>
                "SELECT to_jsonb(r) AS row FROM position_analysis_records r
                 JOIN tasks t ON t.id = r.task_id WHERE t.job_id = $1",
            JobType::Games | JobType::GamePairs =>
                "SELECT to_jsonb(r) AS row FROM game_results r
                 JOIN tasks t ON t.id = r.task_id WHERE t.job_id = $1",
            JobType::LeaveGeneration =>
                "SELECT to_jsonb(r) AS row FROM leave_rack_progress r WHERE r.job_id = $1",
        };

        let mut rows = sqlx::query(query).bind(id).fetch(&pool);
        while let Some(row) = rows.next().await {
            match row {
                Ok(row) => {
                    let value: serde_json::Value = row.get("row");
                    yield Ok::<_, std::io::Error>(format!("{value}\n"));
                }
                Err(err) => {
                    tracing::error!(error = %err, "result stream failed mid-flight");
                    break;
                }
            }
        }
    };

    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        axum::body::Body::from_stream(stream),
    ))
}

#[derive(Serialize)]
struct UserListItem {
    id: Uuid,
    username: String,
    is_admin: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    tasks_completed: i64,
}

async fn list_users(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<super::Page<UserListItem>>> {
    let (limit, offset) = super::paginate(query.page, query.per_page);

    // Email addresses are deliberately absent — this endpoint is public.
    let rows = sqlx::query(
        "SELECT u.id, u.username, u.is_admin, u.created_at,
                (SELECT COUNT(*) FROM task_claims c
                 WHERE c.claimed_by_user_id = u.id AND c.state = 'completed') AS tasks_completed
         FROM users u
         ORDER BY tasks_completed DESC, u.created_at ASC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(super::Page {
        items: rows
            .into_iter()
            .map(|r| UserListItem {
                id: r.get("id"),
                username: r.get("username"),
                is_admin: r.get("is_admin"),
                created_at: r.get("created_at"),
                tasks_completed: r.get("tasks_completed"),
            })
            .collect(),
        total,
        page: query.page.max(0),
        per_page: limit,
    }))
}

#[derive(Serialize)]
struct WorkerListItem {
    user_id: Option<Uuid>,
    anon_uuid: Option<Uuid>,
    username: Option<String>,
    tasks_completed: i64,
    last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn list_workers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<super::Page<WorkerListItem>>> {
    let (limit, offset) = super::paginate(query.page, query.per_page);

    let rows = sqlx::query(
        "SELECT c.claimed_by_user_id AS user_id,
                c.claimed_by_anon_uuid AS anon_uuid,
                u.username,
                COUNT(*)::bigint AS tasks_completed,
                MAX(c.completed_at) AS last_seen_at
         FROM task_claims c
         LEFT JOIN users u ON u.id = c.claimed_by_user_id
         WHERE c.state = 'completed'
         GROUP BY 1, 2, 3
         ORDER BY 4 DESC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (
             SELECT 1 FROM task_claims WHERE state = 'completed'
             GROUP BY claimed_by_user_id, claimed_by_anon_uuid
         ) w",
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(super::Page {
        items: rows
            .into_iter()
            .map(|r| WorkerListItem {
                user_id: r.get("user_id"),
                anon_uuid: r.get("anon_uuid"),
                username: r.get("username"),
                tasks_completed: r.get("tasks_completed"),
                last_seen_at: r.get("last_seen_at"),
            })
            .collect(),
        total,
        page: query.page.max(0),
        per_page: limit,
    }))
}
