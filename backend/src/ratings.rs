//! Rating pools: who is rated, against what evidence, and the snapshots that
//! result.
//!
//! Ratings live entirely in here and in the four `rating_*` tables. Nothing in
//! this module is called while dispatching, claiming, validating or completing
//! a task, and no job decision reads a rating — SPRT remains a per-job stopping
//! rule on the job config tables. The coupling is one-way: a fit reads finished
//! `game_results` and writes a snapshot.
//!
//! A fit is a pure function of (pool membership, matching evidence), so it is
//! always recomputed from scratch. See [`crate::stats::bradley_terry`] for why
//! that is the right shape for rating fixed-strength bots.

use crate::error::{AppError, AppResult};
use crate::stats::bradley_terry::{self, Matrix};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

/// Why a run happened. Stored on the run so the ratings page can explain a
/// change without diffing two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// An admin added or removed a player config.
    Membership,
    /// New results arrived.
    Evidence,
    Manual,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Trigger::Membership => "membership",
            Trigger::Evidence => "evidence",
            Trigger::Manual => "manual",
        }
    }
}

pub struct Pool {
    pub anchor_player_config_id: Uuid,
    pub anchor_rating: f64,
}

async fn load_pool(conn: &mut PgConnection, pool_id: Uuid) -> AppResult<Pool> {
    let row = sqlx::query(
        "SELECT anchor_player_config_id, anchor_rating FROM rating_pools WHERE id = $1",
    )
    .bind(pool_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found("rating pool not found"))?;

    Ok(Pool {
        anchor_player_config_id: row.get("anchor_player_config_id"),
        anchor_rating: row.get("anchor_rating"),
    })
}

/// Every head-to-head in a pool, summed from the pentanomial.
///
/// The **pair** is the unit, not the game: the two games of a pair share a
/// seed, so counting them as two independent observations would overstate how
/// much evidence there is. A pair contributes one observation worth its
/// half-point score over four, which puts the score on the same per-game scale
/// the Elo formula expects while keeping the sample size honest.
///
/// Only `game_pairs` jobs matching the pool's `(variant, letterdist, layout)`
/// scope count, and only where **both** configs are pool members. Plain `games`
/// jobs are excluded on purpose: `-gp` plays both orderings of every seed, so a
/// pair is side-balanced by construction, while an unpaired job is not, and
/// going first is worth real Elo. Pooling unbalanced results would bias every
/// rating in the direction of whoever happened to start more often.
async fn build_matrix(
    conn: &mut PgConnection,
    pool_id: Uuid,
) -> AppResult<(Vec<Uuid>, Matrix, u64, i32)> {
    let members = sqlx::query_scalar::<_, Uuid>(
        "SELECT m.player_config_id
         FROM rating_pool_members m
         JOIN player_configs p ON p.id = m.player_config_id
         WHERE m.pool_id = $1
         ORDER BY p.name",
    )
    .bind(pool_id)
    .fetch_all(&mut *conn)
    .await?;

    let index: HashMap<Uuid, usize> = members.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut matrix = Matrix::new(members.len());

    // The job filter comes first, and that ordering is the whole cost of this
    // query. Selecting one result per task over *all* of `game_results` and
    // filtering afterwards made every fit -- and every public read of a pool --
    // sort the entire table, for every pool, every two minutes. Narrowing to
    // the pool's own jobs first turns it into an index walk of those jobs'
    // tasks and their results.
    let rows = sqlx::query(
        "WITH eligible_jobs AS (
             SELECT j.id AS job_id,
                    c.player1_config_id AS p1,
                    c.player2_config_id AS p2
             FROM jobs j
             JOIN job_game_pair_config c ON c.job_id = j.id
             JOIN rating_pools pool      ON pool.id = $1
             WHERE j.job_type = 'game_pairs'
               AND j.variant = pool.variant
               AND j.letterdist_id = pool.letterdist_id
               AND j.layout_id = pool.layout_id
               AND c.player1_config_id IN
                   (SELECT player_config_id FROM rating_pool_members WHERE pool_id = $1)
               AND c.player2_config_id IN
                   (SELECT player_config_id FROM rating_pool_members WHERE pool_id = $1)
         ),
         -- One result per task: with redundancy > 1 the other accepted claims
         -- replayed the same seeded games, and counting them would multiply a
         -- job's weight in the fit by its redundancy.
         first_result_per_task AS (
             SELECT DISTINCT ON (r.task_id)
                    r.job_id, r.pent_0, r.pent_1, r.pent_2, r.pent_3, r.pent_4
             FROM eligible_jobs e
             JOIN game_results r ON r.job_id = e.job_id
             WHERE r.pent_0 IS NOT NULL
             ORDER BY r.task_id, r.submitted_at, r.task_claim_id
         )
         SELECT e.p1 AS p1, e.p2 AS p2, f.job_id AS job_id,
                COALESCE(SUM(f.pent_0), 0)::bigint AS pent_0,
                COALESCE(SUM(f.pent_1), 0)::bigint AS pent_1,
                COALESCE(SUM(f.pent_2), 0)::bigint AS pent_2,
                COALESCE(SUM(f.pent_3), 0)::bigint AS pent_3,
                COALESCE(SUM(f.pent_4), 0)::bigint AS pent_4
         FROM first_result_per_task f
         JOIN eligible_jobs e ON e.job_id = f.job_id
         GROUP BY e.p1, e.p2, f.job_id",
    )
    .bind(pool_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut total_pairs: u64 = 0;
    let mut jobs = std::collections::HashSet::new();
    for row in &rows {
        let p1: Uuid = row.get("p1");
        let p2: Uuid = row.get("p2");
        let (Some(&i), Some(&j)) = (index.get(&p1), index.get(&p2)) else {
            continue;
        };
        let mut pairs = 0.0;
        let mut score_p1 = 0.0;
        for bucket in 0..5 {
            let count = row.get::<i64, _>(format!("pent_{bucket}").as_str()) as f64;
            pairs += count;
            score_p1 += count * (bucket as f64 / 4.0);
        }
        if pairs <= 0.0 {
            continue;
        }
        total_pairs += pairs as u64;
        jobs.insert(row.get::<Uuid, _>("job_id"));
        matrix.add(i, j, pairs, score_p1);
    }

    Ok((members, matrix, total_pairs, jobs.len() as i32))
}

/// The advisory-lock namespace for rating fits, distinct from dispatch's.
const RATING_LOCK_NAMESPACE: i32 = 2;

/// Serialize one pool's fits against each other, for the rest of the caller's
/// transaction.
///
/// A fit is read-then-write over state an admin can change underneath it: it
/// reads membership and evidence, then writes a run stamped `now()`. Two fits
/// running at once therefore interleave, and the one that *started* first can
/// commit last -- so the newest `rating_runs` row, which is what the ratings
/// page reads, can be the one built from the older membership. Removing a
/// config and seeing it come straight back in the fit is the visible symptom,
/// and it lasts until the next sweep happens to disagree with the stored
/// `pairs_used`.
///
/// Taken before anything is read, so the read and the write are one atomic
/// decision. Per pool, so pools never wait on each other, and
/// transaction-scoped, so it is released on commit, on rollback, and on a
/// dropped connection.
async fn lock_pool_fit(conn: &mut PgConnection, pool_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2::text))")
        .bind(RATING_LOCK_NAMESPACE)
        .bind(pool_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Refits a pool from scratch and stores the result as a new run.
///
/// Always a full refit, never a patch: adding or removing a config changes what
/// counts as evidence for *everyone*, and a batch fit has no per-player history
/// to unwind. Cheap enough to do this way — the MM iteration is microseconds
/// for a pool of any plausible size, and the query above is one grouped scan.
pub async fn recompute(db: &PgPool, pool_id: Uuid, trigger: Trigger) -> AppResult<Uuid> {
    fit_and_store(db, pool_id, trigger, false)
        .await?
        .ok_or_else(|| AppError::internal("an unconditional refit stored nothing"))
}

/// The body of a fit: lock, read, fit, write, commit.
///
/// `only_if_evidence_changed` is the sweep's path -- it compares the evidence
/// it just read against the last run's and stores nothing when they agree,
/// which is what keeps a quiet pool from accumulating identical snapshots.
/// Reusing this rather than checking first and refitting after is what stops
/// the sweep building the (expensive) matrix twice per tick.
async fn fit_and_store(
    db: &PgPool,
    pool_id: Uuid,
    trigger: Trigger,
    only_if_evidence_changed: bool,
) -> AppResult<Option<Uuid>> {
    let mut tx = db.begin().await?;
    lock_pool_fit(&mut tx, pool_id).await?;
    let pool = load_pool(&mut tx, pool_id).await?;

    // The cheap question first: have the pool's jobs completed any games, or
    // its members changed, since the last run? `games_completed` is each
    // job's first-result-per-task running total, kept by the submission that
    // stores the result, so an unchanged sum is unchanged evidence. Read
    // before the matrix: a result committed in between makes the stored sum
    // short of the fit, and the next sweep refits, which is the safe side.
    let evidence_games: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(j.games_completed), 0)::bigint
         FROM jobs j
         JOIN job_game_pair_config c ON c.job_id = j.id
         JOIN rating_pools pool      ON pool.id = $1
         WHERE j.job_type = 'game_pairs'
           AND j.variant = pool.variant
           AND j.letterdist_id = pool.letterdist_id
           AND j.layout_id = pool.layout_id
           AND c.player1_config_id IN
               (SELECT player_config_id FROM rating_pool_members WHERE pool_id = $1)
           AND c.player2_config_id IN
               (SELECT player_config_id FROM rating_pool_members WHERE pool_id = $1)",
    )
    .bind(pool_id)
    .fetch_one(&mut *tx)
    .await?;
    if only_if_evidence_changed {
        let last: Option<(Option<i64>, Vec<Uuid>)> = sqlx::query_as(
            "SELECT r.evidence_games,
                    ARRAY(SELECT p.player_config_id FROM player_config_ratings p
                           WHERE p.run_id = r.id ORDER BY p.player_config_id)
             FROM rating_runs r
             WHERE r.pool_id = $1 ORDER BY r.computed_at DESC, r.id DESC LIMIT 1",
        )
        .bind(pool_id)
        .fetch_optional(&mut *tx)
        .await?;
        let members: Vec<Uuid> = sqlx::query_scalar(
            "SELECT player_config_id FROM rating_pool_members WHERE pool_id = $1
             ORDER BY player_config_id",
        )
        .bind(pool_id)
        .fetch_all(&mut *tx)
        .await?;
        if last == Some((Some(evidence_games), members)) {
            tx.rollback().await?;
            return Ok(None);
        }
    }

    let (members, matrix, pairs_used, jobs_used) = build_matrix(&mut tx, pool_id).await?;

    if only_if_evidence_changed {
        // The evidence is the pairs *and* who is in the pool. Compared on the
        // pairs alone, a membership change whose own refit never ran -- the
        // request dropped, or the fit failing after the membership committed
        // -- was never repaired when the config added or removed had no pairs
        // in the pool, since the count did not move.
        let last: Option<(Uuid, i64, Vec<Uuid>)> = sqlx::query_as(
            "SELECT r.id, r.pairs_used,
                    ARRAY(SELECT p.player_config_id FROM player_config_ratings p
                           WHERE p.run_id = r.id ORDER BY p.player_config_id)
             FROM rating_runs r
             WHERE r.pool_id = $1 ORDER BY r.computed_at DESC, r.id DESC LIMIT 1",
        )
        .bind(pool_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mut current = members.clone();
        current.sort();
        if let Some((run_id, pairs, last_members)) = last {
            if pairs == pairs_used as i64 && last_members == current {
                // The same evidence under a sum that moved (a run from before
                // the sum was recorded, a hand-repaired counter): recorded, so
                // the next sweep's cheap check answers instead of this build.
                sqlx::query("UPDATE rating_runs SET evidence_games = $2 WHERE id = $1")
                    .bind(run_id)
                    .bind(evidence_games)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                return Ok(None);
            }
        }
    }

    let anchor_index = members
        .iter()
        .position(|id| *id == pool.anchor_player_config_id)
        .ok_or_else(|| AppError::bad_request("the pool's anchor is not a member of the pool"))?;

    let fit = bradley_terry::fit(&matrix, anchor_index, pool.anchor_rating);

    let run_id: Uuid = sqlx::query_scalar(
        // `computed_at` is the moment of the insert, under the pool's lock,
        // rather than the column's `now()` default -- the transaction's start,
        // taken before the lock. A fit that began first and got the lock
        // second would otherwise be stamped older than the run it superseded,
        // and the page, which reads the newest, would show the older one.
        "INSERT INTO rating_runs
             (pool_id, trigger, method, iterations, converged, pairs_used, jobs_used,
              evidence_games, computed_at)
         VALUES ($1, $2, 'bradley_terry_mm', $3, $4, $5, $6, $7, clock_timestamp())
         RETURNING id",
    )
    .bind(pool_id)
    .bind(trigger.as_str())
    .bind(fit.iterations as i32)
    .bind(fit.converged)
    .bind(pairs_used as i64)
    .bind(jobs_used)
    .bind(evidence_games)
    .fetch_one(&mut *tx)
    .await?;

    for (i, player_config_id) in members.iter().enumerate() {
        let rated = fit.ratings[i];
        let is_anchor = i == anchor_index;
        sqlx::query(
            "INSERT INTO player_config_ratings
                 (run_id, player_config_id, rating, stderr, pairs_played,
                  connected_to_anchor, is_anchor)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(run_id)
        .bind(player_config_id)
        .bind(rated.rating)
        .bind(if rated.stderr.is_finite() { rated.stderr } else { f64::MAX })
        .bind(rated.games as i64)
        .bind(is_anchor || fit.is_rateable(i))
        .bind(is_anchor)
        .execute(&mut *tx)
        .await?;
    }

    // Stored with the run rather than recomputed when the pool is viewed: a
    // view then costs a read instead of a rebuild of the evidence matrix, and
    // the residuals describe the evidence this fit actually used.
    let residuals = fit.residuals(&matrix);
    if !residuals.is_empty() {
        let mut rows = Vec::with_capacity(residuals.len());
        let mut cols = Vec::with_capacity(residuals.len());
        let mut pairs = Vec::with_capacity(residuals.len());
        let mut actual = Vec::with_capacity(residuals.len());
        let mut predicted = Vec::with_capacity(residuals.len());
        for residual in &residuals {
            rows.push(members[residual.i]);
            cols.push(members[residual.j]);
            pairs.push(residual.games);
            actual.push(residual.actual);
            predicted.push(residual.predicted);
        }
        sqlx::query(
            "INSERT INTO rating_run_residuals
                 (run_id, row_player_config_id, col_player_config_id, pairs, actual, predicted)
             SELECT $1, * FROM UNNEST($2::uuid[], $3::uuid[], $4::float8[], $5::float8[],
                                      $6::float8[])",
        )
        .bind(run_id)
        .bind(&rows)
        .bind(&cols)
        .bind(&pairs)
        .bind(&actual)
        .bind(&predicted)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(Some(run_id))
}

/// Refits every pool whose evidence has grown since its last run.
///
/// Deliberately a periodic sweep rather than a hook on result submission: a fit
/// is global to a pool, an active job submits thousands of results an hour, and
/// unlike SPRT nothing blocks on the answer. Comparing the pair count against
/// the last run's `pairs_used` avoids needing a dirty flag anywhere.
///
/// One pool's failure does not stop the others. A fit can fail on state an
/// admin can reach -- a pool whose anchor is no longer a member is the obvious
/// one -- and propagating that ended the whole sweep at the first such pool, so
/// every pool ordered after it silently stopped being refit for as long as the
/// misconfiguration lasted. Each pool is logged and skipped instead, and the
/// count returned is of the fits that actually ran.
pub async fn recompute_stale(db: &PgPool) -> AppResult<usize> {
    let pool_ids =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM rating_pools").fetch_all(db).await?;

    let mut recomputed = 0;
    for pool_id in pool_ids {
        match recompute_if_stale(db, pool_id).await {
            Ok(true) => recomputed += 1,
            Ok(false) => {}
            Err(err) => tracing::error!(
                %pool_id, error = %err.message, "refitting a rating pool failed; skipping it"
            ),
        }
    }
    Ok(recomputed)
}

/// Whether this pool's evidence has grown since its last run, and a refit if so.
///
/// One pass, not two: the staleness check and the fit read the same matrix
/// inside the same transaction. Checking first and then calling `recompute`
/// built it twice, and it is the most expensive query in the module.
async fn recompute_if_stale(db: &PgPool, pool_id: Uuid) -> AppResult<bool> {
    Ok(fit_and_store(db, pool_id, Trigger::Evidence, true).await?.is_some())
}

/// How long a pool keeps every run. Inside this window the run-by-run diff is
/// what answers "why did this rating change?". A pool with an active job is
/// refit every two minutes, so the window holds ~21,600 runs, each with a
/// rating row per member and a residual row per head-to-head.
pub const RUN_FULL_RESOLUTION: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Runs deleted per statement by [`thin_old_runs`]. Each takes its rating and
/// residual rows with it -- a couple of hundred rows per run for a pool of
/// twenty -- so a batch is a bounded transaction rather than the whole backlog
/// in one, and the first pass over a long-lived pool holds no lock for long.
const THIN_BATCH: i64 = 1_000;

/// Thins every pool's runs older than [`RUN_FULL_RESOLUTION`] to the last run
/// of each UTC day, keeping each pool's first run whatever its day.
///
/// Without this, runs grow for the life of a pool: a twenty-member pool with an
/// active job stores ~720 runs a day with ~210 residual rows each, ~150,000
/// rows a day that nothing reads once the run is no longer the newest. Past
/// the window a day is the resolution the history chart draws at anyway -- it
/// thins to 500 points over the pool's whole life -- so the chart keeps its
/// shape and its ends and only runs it never showed are deleted. Deleting
/// everything past the window would have started the chart's past at the
/// window's edge instead.
///
/// The newest run -- what the ratings page shows -- always survives, however
/// long the pool has been quiet: it is the last run of its day. Ratings and
/// residuals go with their run by cascade. No fit lock is taken: a fit inserts
/// a run stamped `now()`, which is never inside the window this deletes from.
///
/// Returns how many runs were deleted.
pub async fn thin_old_runs(db: &PgPool) -> AppResult<u64> {
    let mut deleted = 0;
    loop {
        let batch = sqlx::query(
            "WITH old AS (
                 SELECT id,
                        row_number() OVER (
                            PARTITION BY pool_id, (computed_at AT TIME ZONE 'UTC')::date
                            ORDER BY computed_at DESC, id DESC
                        ) AS n_in_day,
                        row_number() OVER (
                            PARTITION BY pool_id ORDER BY computed_at ASC, id ASC
                        ) AS n_in_pool
                 FROM rating_runs
                 WHERE computed_at < now() - make_interval(secs => $1)
             )
             DELETE FROM rating_runs
             WHERE id IN (SELECT id FROM old WHERE n_in_day > 1 AND n_in_pool > 1 LIMIT $2)",
        )
        .bind(RUN_FULL_RESOLUTION.as_secs_f64())
        .bind(THIN_BATCH)
        .execute(db)
        .await?
        .rows_affected();
        deleted += batch;
        if batch < THIN_BATCH as u64 {
            return Ok(deleted);
        }
    }
}
