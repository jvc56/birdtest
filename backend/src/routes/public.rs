use crate::error::{AppError, AppResult};
use crate::jobstats;
use crate::models::job::{Job, JobType};
use crate::state::AppState;
use crate::extract::{ApiPath as Path, ApiQuery as Query};
use axum::extract::State;
use axum::http::StatusCode;
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
        .route("/users", get(list_users))
        .route("/workers", get(list_workers))
}

#[derive(Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub page: i64,
    pub per_page: Option<i64>,
}

/// The job list's query: a page, and optionally only jobs of one status --
/// the home page's "active jobs" filtered the newest page in the browser,
/// and lost every active job older than it.
#[derive(Deserialize)]
struct JobListQuery {
    #[serde(default)]
    page: i64,
    per_page: Option<i64>,
    status: Option<crate::models::job::JobStatus>,
}

#[derive(Serialize)]
struct JobListItem {
    id: Uuid,
    job_type: JobType,
    status: String,
    allocation: Option<i32>,
    redundancy: i32,
    created_at: chrono::DateTime<chrono::Utc>,
    tasks_total: i64,
    tasks_completed: i64,
    /// For on-demand SPRT jobs the meaningful denominator is `max_games` /
    /// `max_pairs`, not a task count that grows as work is handed out.
    units_completed: Option<i64>,
    max_units: Option<i64>,
    /// Workers are declining this job and none is completing it.
    ///
    /// A job pinned to data nobody has does not announce itself: the workers
    /// go on contributing elsewhere and this one simply gets nothing done. The
    /// symptom is an absence, so it has to be stated rather than noticed.
    /// There is deliberately no alert -- that needs a notification channel
    /// birdtest does not have -- so this is shown where an admin already looks.
    stalled: bool,
}

async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<JobListQuery>,
) -> AppResult<Json<super::Page<JobListItem>>> {
    let (limit, offset) = super::paginate(query.page, query.per_page);

    let rows = sqlx::query(
        "SELECT j.id, j.job_type, j.status::text AS status, j.allocation,
                j.redundancy, j.created_at,
                -- Running totals, like games_completed below. Counted, these
                -- were two scans of a job's whole task history for every job on
                -- the page -- see PLAN.md on what these reads cost.
                j.tasks_total, j.tasks_completed,
                -- The running total the submit path maintains, one result per
                -- task, rather than the aggregate over every result row this
                -- used to compute: it grew with the job's whole history, for
                -- every job on the page, on every page view -- see PLAN.md on
                -- what these reads cost. The dashboard's own counts still come
                -- from the rows.
                j.games_completed AS game_rows,
                gc.max_games, pc.max_pairs,
                -- Tasks are made on demand, so for opening racks and leave
                -- generation a task count is no denominator: it is only what
                -- has been handed out so far. Their own units instead: racks
                -- analysed of the rack space, generations closed of the count.
                j.racks_analyzed, rc.total_racks, lc.generation_count,
                (SELECT COUNT(*) FROM leave_generation_artifacts a
                  WHERE a.job_id = j.id AND a.generation >= 1) AS generations_closed,
                -- Stalled: at least one decline and no submission in the last
                -- 24 hours, with nothing currently claimed. Long enough not to
                -- flap overnight, short enough that an admin sees it the next
                -- morning. Everything it reads is already recorded.
                (j.status = 'active'
                 AND EXISTS (SELECT 1 FROM worker_data_gaps g
                              WHERE g.job_id = j.id
                                AND g.reported_at > now() - interval '24 hours')
                 AND NOT EXISTS (SELECT 1 FROM task_claims c
                                 JOIN tasks t ON t.id = c.task_id
                                 WHERE t.job_id = j.id
                                   AND c.state = 'completed'
                                   AND c.completed_at > now() - interval '24 hours')
                 AND NOT EXISTS (SELECT 1 FROM task_claims c
                                 JOIN tasks t ON t.id = c.task_id
                                 WHERE t.job_id = j.id AND c.state = 'claimed')
                ) AS stalled
         FROM jobs j
         LEFT JOIN job_game_config gc ON gc.job_id = j.id
         LEFT JOIN job_game_pair_config pc ON pc.job_id = j.id
         LEFT JOIN job_opening_rack_config rc ON rc.job_id = j.id
         LEFT JOIN job_leave_config lc ON lc.job_id = j.id
         WHERE $3::job_status IS NULL OR j.status = $3
         ORDER BY j.created_at DESC, j.id DESC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .bind(query.status)
    .fetch_all(&state.read_pool)
    .await?;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM jobs WHERE $1::job_status IS NULL OR status = $1",
    )
    .bind(query.status)
    .fetch_one(&state.read_pool)
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
                JobType::Games => (Some(game_rows), max_games.map(i64::from)),
                JobType::GamePairs => (Some(game_rows / 2), max_pairs.map(i64::from)),
                JobType::OpeningRack => {
                    (Some(row.get::<i64, _>("racks_analyzed")), row.get::<Option<i64>, _>("total_racks"))
                }
                JobType::LeaveGeneration => (
                    Some(row.get::<i64, _>("generations_closed")),
                    row.get::<Option<i32>, _>("generation_count").map(i64::from),
                ),
            };
            JobListItem {
                id: row.get("id"),
                job_type,
                status: row.get("status"),
                allocation: row.get("allocation"),
                redundancy: row.get("redundancy"),
                created_at: row.get("created_at"),
                tasks_total: row.get("tasks_total"),
                tasks_completed: row.get("tasks_completed"),
                units_completed,
                max_units,
                stalled: row.get("stalled"),
            }
        })
        .collect();

    Ok(Json(super::Page { items, total, page: query.page.max(0), per_page: limit }))
}

async fn load_job(state: &AppState, id: Uuid) -> AppResult<Job> {
    sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.read_pool)
        .await?
        .ok_or_else(|| AppError::not_found("no such job"))
}

/// A `JobStats`, as JSON; see `Config::stats_cache` for how fresh.
async fn job_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let job = load_job(&state, id).await?;
    let payload = jobstats::payload(&state.read_pool, &job, state.cfg.stats_cache).await?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        payload.to_string(),
    )
        .into_response())
}

#[derive(Deserialize)]
struct ResultsQuery {
    per_page: Option<i64>,
    /// Where the previous page left off. Absent for the first page. This route
    /// pages by cursor rather than by offset — see [`super::CursorPage`].
    cursor: Option<String>,
    /// Filter to one contributor: a username, or an anonymous worker UUID.
    worker: Option<String>,
    /// Opening-rack jobs only: look up one rack's full ranked move list.
    rack: Option<String>,
}

async fn job_results(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ResultsQuery>,
) -> AppResult<Json<super::CursorPage<serde_json::Value>>> {
    let job = load_job(&state, id).await?;
    let (limit, _) = super::paginate(0, query.per_page);
    let cursor = query.cursor.as_deref().and_then(super::decode_cursor);

    if let (JobType::OpeningRack, Some(rack)) = (job.job_type, query.rack.as_ref()) {
        return Ok(Json(rack_lookup(&state, id, rack).await?));
    }

    // `?worker=` names a contributor; the rows are filtered on who that *is*.
    // A name that belongs to nobody is an empty page, decided here in two
    // indexed probes rather than by reading the job to find out.
    let worker = match query.worker.as_deref() {
        Some(name) => match resolve_worker(&state, name).await? {
            Some(worker) => Some(worker),
            None => {
                return Ok(Json(super::CursorPage {
                    items: Vec::new(),
                    total: -1,
                    per_page: limit,
                    next_cursor: None,
                }))
            }
        },
        None => None,
    };
    let (worker_user, worker_anon) = match &worker {
        Some(w) => (w.user_id, w.anon_uuid),
        None => (None, None),
    };
    let worker_predicate = worker_predicate(worker.as_ref());

    // Every branch below reads its rows through the job id the record tables
    // now carry, rather than by joining `tasks` to find out which rows belong
    // to the job — which put the filter on the far side of a join from the
    // sort, so the whole job had to be gathered before the first page existed.
    //
    // The cursor is the other half: it is the last row of the previous page, so
    // the scan starts there instead of counting past it.
    let mut next_cursor = None;
    let items: Vec<serde_json::Value> = match job.job_type {
        JobType::OpeningRack => {
            // `id` is the tiebreaker rather than `rack`: `submitted_at` defaults
            // to now(), which is transaction time, so every record of one batch
            // shares it exactly and it is not a key on its own.
            let (after_time, after_id) = opening_rack_cursor(cursor.as_deref());
            let rows = sqlx::query(&format!(
                "SELECT r.id, r.task_id, r.rack, m.move AS best_move, m.score AS best_score,
                        m.equity AS best_equity, r.num_moves, r.submitted_at,
                        u.username, left(encode(sha256(convert_to(c.claimed_by_anon_uuid::text, 'UTF8')), 'hex'), 16) AS anon_id
                 FROM position_analysis_records r
                 JOIN task_claims c ON c.id = r.task_claim_id
                 LEFT JOIN position_analysis_moves m
                     ON m.record_id = r.id AND m.rank = 1
                 LEFT JOIN users u ON u.id = c.claimed_by_user_id
                 WHERE r.job_id = $1
                   {worker_predicate}
                   AND ($4::timestamptz IS NULL
                        OR (r.submitted_at, r.id) < ($4, $5))
                 ORDER BY r.submitted_at DESC, r.id DESC
                 LIMIT $6",
            ))
            // Planned for the values it is run with, not cached: see
            // `worker_predicate`.
            .persistent(worker.is_none())
            .bind(id)
            .bind(worker_user)
            .bind(worker_anon)
            .bind(after_time)
            .bind(after_id)
            .bind(limit)
            .fetch_all(&state.read_pool)
            .await?;

            if rows.len() as i64 == limit {
                if let Some(last) = rows.last() {
                    next_cursor = Some(super::encode_cursor(&[
                        last.get::<chrono::DateTime<chrono::Utc>, _>("submitted_at")
                            .timestamp_micros()
                            .to_string(),
                        last.get::<i64, _>("id").to_string(),
                    ]));
                }
            }

            rows.into_iter()
                .map(|r| {
                    serde_json::json!({
                        "task_id": r.get::<Uuid, _>("task_id"),
                        "rack": r.get::<String, _>("rack"),
                        "best_move": r.get::<Option<String>, _>("best_move"),
                        "best_score": r.get::<Option<i32>, _>("best_score"),
                        "best_equity": r.get::<Option<f64>, _>("best_equity"),
                        "num_moves": r.get::<i32, _>("num_moves"),
                        "submitted_at": r.get::<chrono::DateTime<chrono::Utc>, _>("submitted_at"),
                        "username": r.get::<Option<String>, _>("username"),
                        "anon_id": r.get::<Option<String>, _>("anon_id"),
                    })
                })
                .collect()
        }
        JobType::Games | JobType::GamePairs => {
            // `game_results` has no serial, so its primary key is the
            // tiebreaker. `tasks` is still joined -- the listing shows the
            // task's seed -- but as a primary-key lookup per returned row
            // rather than as the thing that decides which rows belong to the
            // job.
            let (after_time, after_claim) = game_result_cursor(cursor.as_deref());
            let rows = sqlx::query(&format!(
                "SELECT r.task_claim_id, r.task_id, r.games, r.wins, r.losses, r.ties,
                        r.p1_score_mean, r.p1_score_sd, r.p2_score_mean, r.p2_score_sd,
                        r.divergent_games, r.divergent_wins, r.divergent_losses,
                        r.divergent_ties, r.submitted_at,
                        t.seed, u.username, left(encode(sha256(convert_to(c.claimed_by_anon_uuid::text, 'UTF8')), 'hex'), 16) AS anon_id
                 FROM game_results r
                 JOIN tasks t ON t.id = r.task_id
                 JOIN task_claims c ON c.id = r.task_claim_id
                 LEFT JOIN users u ON u.id = c.claimed_by_user_id
                 WHERE r.job_id = $1
                   {worker_predicate}
                   AND ($4::timestamptz IS NULL
                        OR (r.submitted_at, r.task_claim_id) < ($4, $5))
                 ORDER BY r.submitted_at DESC, r.task_claim_id DESC
                 LIMIT $6",
            ))
            .persistent(worker.is_none())
            .bind(id)
            .bind(worker_user)
            .bind(worker_anon)
            .bind(after_time)
            .bind(after_claim)
            .bind(limit)
            .fetch_all(&state.read_pool)
            .await?;

            if rows.len() as i64 == limit {
                if let Some(last) = rows.last() {
                    next_cursor = Some(super::encode_cursor(&[
                        last.get::<chrono::DateTime<chrono::Utc>, _>("submitted_at")
                            .timestamp_micros()
                            .to_string(),
                        last.get::<Uuid, _>("task_claim_id").to_string(),
                    ]));
                }
            }

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
                        "anon_id": r.get::<Option<String>, _>("anon_id"),
                    })
                })
                .collect()
        }
        JobType::LeaveGeneration => {
            // Newest generation first, and within a generation by rack: the
            // primary key's order, `(job_id, generation, rack)`, so each read is
            // a seek into it and a page costs a page. The feed once ran
            // furthest-from-target first (`occurrence_count`, then `rack`). No
            // index held that order -- one statement for it sorted every
            // progress row the job has, 4.0 s a page at one full-size
            // generation, on a public route -- and an index that did hold it
            // cost 160 MB a generation (see `leave_rack_progress_pick_idx`).
            // By rack, a contributor can also find one. It is read a generation
            // at a time because the two directions differ.
            let (after_generation, after_rack) = leave_cursor(cursor.as_deref());
            let mut generation = match after_generation {
                Some(generation) => Some(generation),
                // One probe of the primary key's far end.
                None => sqlx::query_scalar::<_, Option<i32>>(
                    "SELECT MAX(generation) FROM leave_rack_progress WHERE job_id = $1",
                )
                .bind(id)
                .fetch_one(&state.read_pool)
                .await?,
            };
            // Where in the first generation read to resume; later ones are
            // read from their start. `''` sorts before every rack, which keeps
            // the comparison a bare index condition.
            let mut after = after_rack.unwrap_or_default();

            let mut rows = Vec::new();
            while let Some(current) = generation.filter(|g| *g >= 1) {
                let remaining = limit - rows.len() as i64;
                if remaining <= 0 {
                    break;
                }
                let page = sqlx::query(
                    "SELECT rack, generation, occurrence_count,
                            equity_sum / NULLIF(occurrence_count, 0) AS mean_equity,
                            updated_at
                     FROM leave_rack_progress
                     WHERE job_id = $1 AND generation = $2 AND rack > $3
                     ORDER BY rack ASC
                     LIMIT $4",
                )
                .bind(id)
                .bind(current)
                .bind(std::mem::take(&mut after))
                .bind(remaining)
                .fetch_all(&state.read_pool)
                .await?;
                rows.extend(page);
                generation = Some(current - 1);
            }

            if rows.len() as i64 == limit {
                if let Some(last) = rows.last() {
                    next_cursor = Some(super::encode_cursor(&[
                        last.get::<i32, _>("generation").to_string(),
                        last.get::<String, _>("rack"),
                    ]));
                }
            }

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

    Ok(Json(super::CursorPage { items, total: -1, per_page: limit, next_cursor }))
}

/// Who `?worker=` names: an account, an anonymous worker, or -- a username may
/// happen to be sixteen hex characters -- one of each.
struct WorkerFilter {
    user_id: Option<Uuid>,
    anon_uuid: Option<Uuid>,
}

/// Resolves `?worker=` to the identities it names, or `None` when it names
/// nobody.
///
/// The filter used to be applied to every row of the job instead: `u.username
/// = $2 OR left(encode(sha256(...claimed_by_anon_uuid...)), 16) = $2`, a hash
/// per record on the far side of two joins, which no index can serve. A page
/// for a contributor with few results -- or for a name nobody has -- therefore
/// read the *whole job* looking for fifty rows that were not there: 2.4 seconds
/// at a million records, measured, on a public, unauthenticated, unmetered
/// route, each request holding a pool connection for all of it. Resolved first,
/// the filter is an equality on an indexed column of `task_claims`, and a name
/// that matches nothing never reads the job at all.
///
/// A pseudonym is matched among contributors only (`tasks_completed > 0`, the
/// partial index `/api/workers` reads): an identity with results in any job has
/// completed something, and that set is bounded by work actually done rather
/// than by how many UUIDs were ever minted. The hash cannot be indexed as it
/// is defined -- `convert_to` is not immutable.
async fn resolve_worker(state: &AppState, name: &str) -> AppResult<Option<WorkerFilter>> {
    // Deleted accounts included: the contribution table still lists their
    // results, under the tombstone name, and that name filters like any other.
    // Through the case-insensitive unique index, the only one on the name:
    // names are unique whatever their case, so this is still one account.
    let user_id = sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE lower(username) = lower($1)")
    .bind(name)
    .fetch_optional(&state.read_pool)
    .await?;

    let looks_like_a_pseudonym =
        name.len() == 16 && name.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let anon_uuid = if looks_like_a_pseudonym {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT uuid FROM anonymous_workers
             WHERE tasks_completed > 0
               AND left(encode(sha256(convert_to(uuid::text, 'UTF8')), 'hex'), 16) = $1
             LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&state.read_pool)
        .await?
    } else {
        None
    };

    Ok((user_id.is_some() || anon_uuid.is_some()).then_some(WorkerFilter { user_id, anon_uuid }))
}

/// The feed queries' worker clause, over `$2` (an account) and `$3` (an
/// anonymous worker). Every variant mentions both, typed, so the statement
/// prepares whichever are NULL.
///
/// A filtered feed is sent unprepared (`persistent(false)`), so Postgres plans
/// it for the identity actually asked about. The right plan differs by two
/// orders of magnitude with who that is -- a heavy contributor is found at the
/// head of the job's feed index, a rare one through their own claims -- and a
/// cached generic plan picks one for everybody.
fn worker_predicate(worker: Option<&WorkerFilter>) -> &'static str {
    match worker {
        None => "AND $2::uuid IS NULL AND $3::uuid IS NULL",
        Some(WorkerFilter { user_id: Some(_), anon_uuid: None }) => {
            "AND c.claimed_by_user_id = $2::uuid AND $3::uuid IS NULL"
        }
        Some(WorkerFilter { user_id: None, anon_uuid: Some(_) }) => {
            "AND $2::uuid IS NULL AND c.claimed_by_anon_uuid = $3::uuid"
        }
        Some(_) => "AND (c.claimed_by_user_id = $2::uuid OR c.claimed_by_anon_uuid = $3::uuid)",
    }
}

/// The three cursor shapes this route uses. Each returns `None`s for a missing
/// or unparseable cursor, which the queries read as "start at the beginning" —
/// a cursor is opaque, so a caller cannot be expected to repair one.
fn opening_rack_cursor(
    cursor: Option<&[String]>,
) -> (Option<chrono::DateTime<chrono::Utc>>, Option<i64>) {
    match cursor {
        Some([time, id]) => (micros_to_time(time), id.parse().ok()),
        _ => (None, None),
    }
}

fn game_result_cursor(
    cursor: Option<&[String]>,
) -> (Option<chrono::DateTime<chrono::Utc>>, Option<Uuid>) {
    match cursor {
        Some([time, claim]) => (micros_to_time(time), claim.parse().ok()),
        _ => (None, None),
    }
}

fn leave_cursor(cursor: Option<&[String]>) -> (Option<i32>, Option<String>) {
    match cursor {
        Some([generation, rack]) => (generation.parse().ok(), Some(rack.clone())),
        _ => (None, None),
    }
}

fn micros_to_time(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::from_timestamp_micros(raw.parse().ok()?)
}

/// The full ranked move list for one rack, from `position_analysis_moves`.
async fn rack_lookup(
    state: &AppState,
    job_id: Uuid,
    rack: &str,
) -> AppResult<super::CursorPage<serde_json::Value>> {
    let canonical: String = {
        let mut chars: Vec<char> = rack.trim().to_uppercase().chars().collect();
        chars.sort_unstable();
        chars.into_iter().collect()
    };

    // One probe of `(job_id, rack) WHERE game_index IS NULL` rather than a walk
    // of the job's tasks. `game_index IS NULL` is what keeps an incidentally
    // captured in-game position with the same rack out of an opening-rack
    // lookup.
    //
    // Ordered by record first: under redundancy above 1 a rack has one record
    // per accepted claim, and ordering by rank alone interleaved the lists --
    // two rank-1 rows, then two rank-2 rows -- as if one analysis had ranked
    // every move twice.
    let rows = sqlx::query(
        "SELECT m.rank, m.move, m.score, m.equity
         FROM position_analysis_records r
         JOIN position_analysis_moves m ON m.record_id = r.id
         WHERE r.job_id = $1 AND r.rack = $2 AND r.game_index IS NULL
         ORDER BY r.id ASC, m.rank ASC",
    )
    .bind(job_id)
    .bind(&canonical)
    .fetch_all(&state.read_pool)
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

    // A rack's whole ranked list comes back in one page, so there is nothing to
    // page to.
    let total = items.len() as i64;
    Ok(super::CursorPage { items, total, per_page: total.max(1), next_cursor: None })
}

/// One SSE event per accepted result, carrying the same payload `GET
/// /api/jobs/:id` would return.
async fn job_stream(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let job = load_job(&state, id).await?;
    // Subscribed before the first payload is read: a push published between
    // the two was lost, and on a quiet job nothing followed it.
    let receiver = state.sse.subscribe(id);
    let initial = jobstats::payload(&state.read_pool, &job, state.cfg.stats_cache)
        .await?
        .to_string();

    let updates = tokio_stream::wrappers::BroadcastStream::new(receiver)
        .filter_map(|msg| async move { msg.ok() });

    // Ended when the process is told to stop. The stream has no end of its
    // own, and graceful shutdown waits for every open response: see
    // `state::Shutdown`. The page subscribes again by itself
    // (`frontend/src/lib/sse.ts`).
    let shutdown = state.shutdown.clone();
    let stream = futures::stream::once(async move { initial })
        .chain(updates)
        .map(|payload| Ok(Event::default().event("stats").data(payload)))
        .take_until(async move { shutdown.triggered().await });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

/// Newline-delimited JSON of every record for the job, streamed straight from a
/// cursor so an offline analysis download never buffers the whole job in memory.
///
/// **Admin-only, and capped.** It was public, unpaginated and unmetered, which
/// made one HTTP request enough to start a scan of tens of millions of rows —
/// and the resource it consumes is not CPU but a pool connection, held for as
/// long as the caller keeps reading, out of twenty. Bulk reads are an admin
/// operation now; the public gets `GET /api/jobs/:id/results`, which is
/// paginated. For a completed job this defers to the export, which is the same
/// corpus read once rather than once per caller.
#[derive(Deserialize)]
pub(super) struct StreamQuery {
    /// Games and game-pairs jobs only: stream the positions the job captured
    /// while playing, each with its ranked moves, instead of its result rows.
    #[serde(default)]
    positions: bool,
}

pub(super) async fn job_results_stream(
    State(state): State<AppState>,
    _admin: crate::auth::AdminUser,
    Path(id): Path<Uuid>,
    Query(query): Query<StreamQuery>,
) -> AppResult<axum::response::Response> {
    let job = load_job(&state, id).await?;
    if query.positions && !crate::exports::may_capture_positions(job.job_type) {
        return Err(AppError::bad_request(
            "?positions=true is for games and game-pairs jobs, which can capture positions \
             while playing; an opening-rack job's stream already is its positions",
        ));
    }

    // A completed job's results are immutable, so an export of them is a stable
    // artifact: read it instead of re-scanning. The redirect is what puts the
    // cheap path in front of a caller without them having to know about it.
    if job.status == crate::models::job::JobStatus::Completed {
        if let Some(ready) = crate::exports::newest_ready(&state.pool, id).await? {
            // The positions' own artifact when that is what was asked for. An
            // export with none -- the job captured nothing -- falls through to
            // the scan below, which streams the same nothing.
            let key = if query.positions {
                ready.positions_artifact_key
            } else {
                Some(ready.artifact_key)
            };
            if let Some(key) = key {
                let url = state
                    .artifacts
                    .presigned_get(&key, crate::exports::DOWNLOAD_URL_TTL)
                    .await?;
                return Ok((StatusCode::SEE_OTHER, [(axum::http::header::LOCATION, url)])
                    .into_response());
            }
        }
    }

    // Held for the life of the stream, and released when the response body is
    // dropped — which covers a caller that disconnects half way as well as one
    // that reads to the end.
    let permit = state
        .result_streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::rate_limited(30))?;

    // The main pool, not the display one: this cursor is held for as long as
    // the caller keeps reading, which the display pool's statement timeout
    // exists to forbid. The permit above is what bounds it instead.
    let pool = state.pool.clone();
    let stream = async_stream::stream! {
        let _permit = permit;
        // The export's own queries, so the stream of an active job and the
        // export of a completed one are the same corpus -- an opening-rack
        // record with its ranked moves nested in it, not the record alone.
        let query = if query.positions {
            crate::exports::positions_query()
        } else {
            crate::exports::export_query(job.job_type)
        };

        // A completed leave job may still hold accepted results that no merge
        // has folded into the rows about to be read (`exports::settle`); an
        // active one is a moving target either way, and is left to its sweep.
        if job.status == crate::models::job::JobStatus::Completed {
            if let Err(err) = crate::exports::settle(&pool, &job).await {
                tracing::error!(job_id = %id, error = %err.message, "settling a job before streaming it failed");
            }
        }

        let mut rows = sqlx::query(query).bind(id).fetch(&pool);
        while let Some(row) = rows.next().await {
            match row {
                Ok(row) => {
                    // Already a line of JSON text: the export's queries
                    // serialize in Postgres, so nothing is parsed here.
                    let mut line: String = row.get("row");
                    line.push('\n');
                    yield Ok::<_, std::io::Error>(line);
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
    )
        .into_response())
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
    // `users.tasks_completed` is a running total rather than a count over
    // `task_claims`. The page orders by it, so counting meant computing every
    // user's whole claim history before the LIMIT could apply; the partial
    // index on (tasks_completed DESC, created_at ASC) now serves both.
    let rows = sqlx::query(
        "SELECT u.id, u.username, u.is_admin, u.created_at, u.tasks_completed
         FROM users u
         WHERE u.deleted_at IS NULL
         ORDER BY u.tasks_completed DESC, u.created_at ASC, u.id ASC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.read_pool)
    .await?;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(&state.read_pool)
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
pub(super) struct WorkerListItem {
    user_id: Option<Uuid>,
    /// An anonymous worker's public name; see `auth::public_anon_id`.
    anon_id: Option<String>,
    /// The anonymous worker's UUID, which is its credential: admin listing only.
    #[serde(skip_serializing_if = "Option::is_none")]
    anon_uuid: Option<Uuid>,
    username: Option<String>,
    tasks_completed: i64,
    last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn list_workers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<super::Page<WorkerListItem>>> {
    worker_page(&state, query, false).await
}

/// The same list with anonymous workers' real UUIDs, which banning one needs.
pub(super) async fn list_workers_admin(
    State(state): State<AppState>,
    _admin: crate::auth::AdminUser,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<super::Page<WorkerListItem>>> {
    worker_page(&state, query, true).await
}

async fn worker_page(
    state: &AppState,
    query: PageQuery,
    with_credentials: bool,
) -> AppResult<Json<super::Page<WorkerListItem>>> {
    let (limit, offset) = super::paginate(query.page, query.per_page);

    // Both kinds of contributor in one ranking, each from its own running
    // total. Each arm is an ordered scan of its own partial index, cut off at
    // the end of the requested page, and the two are merged. The arms must be
    // limited themselves: with the LIMIT only outside the UNION, Postgres
    // sorted every contributor of both kinds on each view (the constant NULL
    // column in each arm defeats a merge of the index orders). Within a count,
    // the outer order puts accounts (by id) before anonymous identities (by
    // UUID) -- a NULL sorts last -- which is what each arm's own order gives,
    // so the first `offset + limit` of each arm hold the page.
    //
    // `last_seen_at` here is the last *task finished*, which is what this list
    // has always shown; it is deliberately not `anonymous_workers.last_seen_at`,
    // which any request touches and answers a different question.
    //
    // The identity breaks ties. Contributions tie all the time -- every worker
    // with one task finished -- and ordered by the count alone, Postgres is free
    // to order a tie differently for each LIMIT, so paging through the list
    // showed some contributors twice and others never. A row has exactly one of
    // the two ids, so together they are a total order.
    //
    // The pseudonym is hashed after the page is chosen, for its rows only:
    // computed inside the anonymous arm, it was a SHA-256 of every contributing
    // anonymous identity on every view of a public, unmetered page.
    let rows = sqlx::query(
        "SELECT c.user_id, c.anon_uuid,
                CASE WHEN c.anon_uuid IS NOT NULL
                     THEN left(encode(sha256(convert_to(c.anon_uuid::text, 'UTF8')), 'hex'), 16)
                END AS anon_id,
                c.username, c.tasks_completed, c.last_seen_at
         FROM (
             SELECT * FROM (
                 (SELECT u.id AS user_id, NULL::uuid AS anon_uuid,
                         u.username, u.tasks_completed, u.last_completed_at AS last_seen_at
                  FROM users u WHERE u.tasks_completed > 0
                  ORDER BY u.tasks_completed DESC, u.id
                  LIMIT $3)
                 UNION ALL
                 (SELECT NULL::uuid, w.uuid, NULL::text, w.tasks_completed, w.last_completed_at
                  FROM anonymous_workers w WHERE w.tasks_completed > 0
                  ORDER BY w.tasks_completed DESC, w.uuid
                  LIMIT $3)
             ) contributors
             ORDER BY tasks_completed DESC, user_id, anon_uuid
             LIMIT $1 OFFSET $2
         ) c
         ORDER BY c.tasks_completed DESC, c.user_id, c.anon_uuid",
    )
    .bind(limit)
    .bind(offset)
    .bind(offset.saturating_add(limit))
    .fetch_all(&state.read_pool)
    .await?;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT (SELECT COUNT(*) FROM users WHERE tasks_completed > 0)
              + (SELECT COUNT(*) FROM anonymous_workers WHERE tasks_completed > 0)",
    )
    .fetch_one(&state.read_pool)
    .await?;

    Ok(Json(super::Page {
        items: rows
            .into_iter()
            .map(|r| WorkerListItem {
                user_id: r.get("user_id"),
                anon_id: r.get("anon_id"),
                anon_uuid: if with_credentials { r.get("anon_uuid") } else { None },
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
