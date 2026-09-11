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
/// The pool's members and evidence matrix, without fitting. Exposed so the
/// read endpoints can compute residuals against the same matrix the fit used.
pub async fn evidence_matrix(
    conn: &mut PgConnection,
    pool_id: Uuid,
) -> AppResult<(Vec<Uuid>, Matrix)> {
    let (members, matrix, _, _) = build_matrix(conn, pool_id).await?;
    Ok((members, matrix))
}

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

    let rows = sqlx::query(
        "SELECT c.player1_config_id AS p1,
                c.player2_config_id AS p2,
                t.job_id            AS job_id,
                COALESCE(SUM(r.pent_0), 0)::bigint AS pent_0,
                COALESCE(SUM(r.pent_1), 0)::bigint AS pent_1,
                COALESCE(SUM(r.pent_2), 0)::bigint AS pent_2,
                COALESCE(SUM(r.pent_3), 0)::bigint AS pent_3,
                COALESCE(SUM(r.pent_4), 0)::bigint AS pent_4
         -- One result per task: with redundancy > 1 the other accepted
         -- claims replayed the same seeded games, and counting them would
         -- multiply a job's weight in the fit by its redundancy.
         FROM (SELECT DISTINCT ON (task_id) *
               FROM game_results
               WHERE pent_0 IS NOT NULL
               ORDER BY task_id, submitted_at, task_claim_id) r
         JOIN tasks t                ON t.id = r.task_id
         JOIN jobs j                 ON j.id = t.job_id
         JOIN job_game_pair_config c ON c.job_id = j.id
         JOIN rating_pools pool      ON pool.id = $1
         WHERE j.job_type = 'game_pairs'
           AND r.pent_0 IS NOT NULL
           AND j.variant = pool.variant
           AND j.letterdist_id = pool.letterdist_id
           AND j.layout_id = pool.layout_id
           AND c.player1_config_id IN (SELECT player_config_id FROM rating_pool_members WHERE pool_id = $1)
           AND c.player2_config_id IN (SELECT player_config_id FROM rating_pool_members WHERE pool_id = $1)
         GROUP BY c.player1_config_id, c.player2_config_id, t.job_id",
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

/// Refits a pool from scratch and stores the result as a new run.
///
/// Always a full refit, never a patch: adding or removing a config changes what
/// counts as evidence for *everyone*, and a batch fit has no per-player history
/// to unwind. Cheap enough to do this way — the MM iteration is microseconds
/// for a pool of any plausible size, and the query above is one grouped scan.
pub async fn recompute(db: &PgPool, pool_id: Uuid, trigger: Trigger) -> AppResult<Uuid> {
    let mut tx = db.begin().await?;
    let pool = load_pool(&mut tx, pool_id).await?;
    let (members, matrix, pairs_used, jobs_used) = build_matrix(&mut tx, pool_id).await?;

    let anchor_index = members
        .iter()
        .position(|id| *id == pool.anchor_player_config_id)
        .ok_or_else(|| AppError::bad_request("the pool's anchor is not a member of the pool"))?;

    let fit = bradley_terry::fit(&matrix, anchor_index, pool.anchor_rating);

    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO rating_runs
             (pool_id, trigger, method, iterations, converged, pairs_used, jobs_used)
         VALUES ($1, $2, 'bradley_terry_mm', $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(pool_id)
    .bind(trigger.as_str())
    .bind(fit.iterations as i32)
    .bind(fit.converged)
    .bind(pairs_used as i64)
    .bind(jobs_used)
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

    tx.commit().await?;
    Ok(run_id)
}

/// Refits every pool whose evidence has grown since its last run.
///
/// Deliberately a periodic sweep rather than a hook on result submission: a fit
/// is global to a pool, an active job submits thousands of results an hour, and
/// unlike SPRT nothing blocks on the answer. Comparing the pair count against
/// the last run's `pairs_used` avoids needing a dirty flag anywhere.
pub async fn recompute_stale(db: &PgPool) -> AppResult<usize> {
    let pool_ids =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM rating_pools").fetch_all(db).await?;

    let mut recomputed = 0;
    for pool_id in pool_ids {
        let mut conn = db.acquire().await?;
        let (_, _, pairs_used, _) = build_matrix(&mut conn, pool_id).await?;
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT pairs_used FROM rating_runs
             WHERE pool_id = $1 ORDER BY computed_at DESC LIMIT 1",
        )
        .bind(pool_id)
        .fetch_optional(&mut *conn)
        .await?;
        drop(conn);

        if last != Some(pairs_used as i64) {
            recompute(db, pool_id, Trigger::Evidence).await?;
            recomputed += 1;
        }
    }
    Ok(recomputed)
}
