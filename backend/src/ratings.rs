//! Rating pools: who is rated, against what evidence, and the snapshots that
//! result.
//!
//! Ratings live entirely in here and in the four `rating_*` tables. Nothing in
//! this module is called while dispatching, claiming, validating or completing
//! a task, and no job decision reads a rating — the match test remains a per-job stopping
//! rule on the job config tables. The coupling is one-way: a fit reads finished
//! `game_results` and writes a snapshot.
//!
//! A fit is a pure function of (pool membership, matching evidence), so it is
//! always recomputed from scratch. See [`crate::stats::bradley_terry`] for why
//! that is the right shape for rating fixed-strength bots.

use crate::error::{AppError, AppResult};
use crate::stats::bradley_terry::{self, Matrix};
use crate::stats::outcomes::{Pentanomial, Sample};
use serde::Serialize;
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
    /// An admin moved the anchor or its rating. The sweep would never notice:
    /// it compares evidence and membership, and neither changed.
    Anchor,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Trigger::Membership => "membership",
            Trigger::Evidence => "evidence",
            Trigger::Manual => "manual",
            Trigger::Anchor => "anchor",
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

/// What a fit reads: the members (by name), the matrix the ratings are fitted
/// to, how much evidence went in, and each head-to-head's evidence for the
/// cross table, keyed by `(i, j)` with `i < j`.
struct Evidence {
    members: Vec<Uuid>,
    matrix: Matrix,
    pairs_used: u64,
    jobs_used: i32,
    head_to_heads: HashMap<(usize, usize), HeadToHeadEvidence>,
}

/// One head-to-head's evidence for the cross table, from the side of its
/// lower-indexed config: the pentanomial summed over every job between the
/// two, a job that seats them the other way round flipped (bucket `k` to
/// `4 − k`), and the spread `game_results` reports for the same rows.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HeadToHeadEvidence {
    pub pentanomial: Pentanomial,
    /// Games behind `spread_sum`: `game_results.games`, summed.
    pub games: i64,
    /// Σ games · (this side's mean score − the other side's), over the same
    /// rows.
    pub spread_sum: f64,
}

impl HeadToHeadEvidence {
    /// Adds one job's results, given from its player 1's side: `flipped` when
    /// player 1 is this head-to-head's other config.
    pub fn add(&mut self, pentanomial: [u64; 5], games: i64, spread_sum: f64, flipped: bool) {
        for (bucket, count) in pentanomial.into_iter().enumerate() {
            let at = if flipped { 4 - bucket } else { bucket };
            self.pentanomial.counts[at] += count;
        }
        self.games += games;
        self.spread_sum += if flipped { -spread_sum } else { spread_sum };
    }

    /// This side's score per game, `(W + ½D) / games`, and its standard
    /// error.
    ///
    /// The error is the pair's, not the game's. A pair is the independent
    /// unit -- its two games share a seed -- and scores `x = k/4` for bucket
    /// `k`, the mean of its two games' scores, so it is already per game.
    /// Over `N` pairs with counts `c_k` and mean `m`:
    ///
    /// ```text
    /// s² = Σ c_k·(k/4)² / N − m²      SE = √(s² / N)
    /// ```
    ///
    /// the plug-in variance the match test uses (`outcomes::Sample`), shown
    /// ×100 as percentage points of win %. Counting games instead,
    /// `√(m(1 − m) / 2N)`, would treat a pair's two games as independent,
    /// which is what pairing exists to undo: pairs that played identically
    /// all score ½ and narrow the error, as they should.
    pub fn score_and_stderr(&self) -> (f64, f64) {
        let sample = Sample::from_pentanomial(&self.pentanomial);
        if sample.n == 0 {
            return (0.5, f64::INFINITY);
        }
        (sample.mean, (sample.variance.max(0.0) / sample.n as f64).sqrt())
    }

    /// This side's average spread per game: `Σ games·(p1_mean − p2_mean) /
    /// Σ games`, each job's from this side.
    pub fn spread(&self) -> f64 {
        if self.games <= 0 {
            return 0.0;
        }
        self.spread_sum / self.games as f64
    }
}

/// One cell of a pool's cross table, from the row config's side, as a fit
/// stores it with its run (`rating_run_residuals`) and the pool's page reads
/// it. A fit stores each head-to-head once; [`CrossCell::mirrored`] is the
/// other side's view of the same games.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CrossCell {
    pub row: Uuid,
    pub col: Uuid,
    pub pairs: f64,
    /// The row config's score per game, `(W + ½D) / games`.
    pub actual: f64,
    /// What the fit's ratings predict `actual` to be; the gap is the model's
    /// residual, where a non-transitive pool shows.
    pub predicted: f64,
    /// The standard error of `actual` ([`HeadToHeadEvidence::score_and_stderr`]).
    pub stderr: f64,
    /// The row config's average spread per game.
    pub spread: f64,
}

impl CrossCell {
    /// The same games from the column config's side: the score and the
    /// prediction complemented, the spread negated, the error unchanged.
    pub fn mirrored(&self) -> Self {
        Self {
            row: self.col,
            col: self.row,
            pairs: self.pairs,
            actual: 1.0 - self.actual,
            predicted: 1.0 - self.predicted,
            stderr: self.stderr,
            spread: -self.spread,
        }
    }
}

/// Every stored cell and its mirror, so each ordered `(row, col)` of the
/// cross table with games in it has its cell, ordered by row then column.
pub fn both_sides(cells: &[CrossCell]) -> Vec<CrossCell> {
    let mut out: Vec<CrossCell> = cells.iter().flat_map(|c| [*c, c.mirrored()]).collect();
    out.sort_by_key(|c| (c.row, c.col));
    out
}

/// Every head-to-head in a pool, summed from the pentanomial.
///
/// The **pair** is the unit, not the game: the two games of a pair share a
/// seed, so counting them as two independent observations would overstate how
/// much evidence there is. A pair contributes one observation worth its
/// half-point score over four, which puts the score on the same per-game scale
/// the fit's logistic expects while keeping the sample size honest.
///
/// Only `game_pairs` jobs matching the pool's `(variant, letterdist, layout)`
/// scope count, and only where **both** configs are pool members. Plain `games`
/// jobs are excluded on purpose: `-gp` plays both orderings of every seed, so a
/// pair is side-balanced by construction, while an unpaired job is not, and
/// going first is worth real rating points. Pooling unbalanced results would
/// bias every rating in the direction of whoever happened to start more often.
///
/// The cross table's evidence comes from the same rows in the same grouped
/// scan -- each job's spread, games-weighted, beside its pentanomial -- so the
/// table always describes the evidence the ratings were fitted to.
async fn build_matrix(conn: &mut PgConnection, pool_id: Uuid) -> AppResult<Evidence> {
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
         -- One result per task: a task has one slot.
         job_results AS (
             SELECT r.job_id, r.pent_0, r.pent_1, r.pent_2, r.pent_3, r.pent_4,
                    r.games, r.p1_score_mean, r.p2_score_mean
             FROM eligible_jobs e
             JOIN game_results r ON r.job_id = e.job_id
             WHERE r.pent_0 IS NOT NULL
         )
         SELECT e.p1 AS p1, e.p2 AS p2, f.job_id AS job_id,
                COALESCE(SUM(f.pent_0), 0)::bigint AS pent_0,
                COALESCE(SUM(f.pent_1), 0)::bigint AS pent_1,
                COALESCE(SUM(f.pent_2), 0)::bigint AS pent_2,
                COALESCE(SUM(f.pent_3), 0)::bigint AS pent_3,
                COALESCE(SUM(f.pent_4), 0)::bigint AS pent_4,
                COALESCE(SUM(f.games), 0)::bigint AS games,
                COALESCE(SUM(f.games * (f.p1_score_mean - f.p2_score_mean)), 0)::float8
                    AS spread_sum
         FROM job_results f
         JOIN eligible_jobs e ON e.job_id = f.job_id
         GROUP BY e.p1, e.p2, f.job_id",
    )
    .bind(pool_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut total_pairs: u64 = 0;
    let mut jobs = std::collections::HashSet::new();
    let mut head_to_heads: HashMap<(usize, usize), HeadToHeadEvidence> = HashMap::new();
    for row in &rows {
        let p1: Uuid = row.get("p1");
        let p2: Uuid = row.get("p2");
        let (Some(&i), Some(&j)) = (index.get(&p1), index.get(&p2)) else {
            continue;
        };
        // A config against itself says nothing about its rating (the matrix
        // ignores it), so it is not counted as evidence the fit used either.
        if i == j {
            continue;
        }
        let mut pentanomial = [0u64; 5];
        let mut pairs = 0.0;
        let mut score_p1 = 0.0;
        for (bucket, slot) in pentanomial.iter_mut().enumerate() {
            let count = row.get::<i64, _>(format!("pent_{bucket}").as_str());
            *slot = count.max(0) as u64;
            pairs += count as f64;
            score_p1 += count as f64 * (bucket as f64 / 4.0);
        }
        if pairs <= 0.0 {
            continue;
        }
        total_pairs += pairs as u64;
        jobs.insert(row.get::<Uuid, _>("job_id"));
        matrix.add(i, j, pairs, score_p1);
        head_to_heads.entry((i.min(j), i.max(j))).or_default().add(
            pentanomial,
            row.get("games"),
            row.get("spread_sum"),
            i > j,
        );
    }

    Ok(Evidence {
        members,
        matrix,
        pairs_used: total_pairs,
        jobs_used: jobs.len() as i32,
        head_to_heads,
    })
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
///
/// Everything else that changes what a fit reads takes it too: an anchor
/// change, a member's removal (whose anchor check must still hold at its
/// delete), and a pool's deletion, which so waits for a fit in flight rather
/// than having its rows cascaded out from under it.
pub(crate) async fn lock_pool_fit(conn: &mut PgConnection, pool_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2::text))")
        .bind(RATING_LOCK_NAMESPACE)
        .bind(pool_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Marks every pool's newest fit for a refit (`evidence_games = NULL`), for a
/// purge or delete of a job whose results may have been in it: the sweep's
/// cheap check compares a sum of `games_completed`, which a purge and a re-run
/// to the same count leave where it was. Under every pool's fit lock, in pool
/// order: a fit that read the pools before the purge committed would
/// otherwise write its pre-purge sum back over the mark, or become the newest
/// run itself, and the purged results stay in the ratings.
pub async fn mark_every_pool_for_refit(conn: &mut PgConnection) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext(id::text)) FROM rating_pools ORDER BY id")
        .bind(RATING_LOCK_NAMESPACE)
        .execute(&mut *conn)
        .await?;
    // Each pool's newest run, found by a seek into the pool's index --
    // walking every run to find them was most of a second, inside the purge's
    // locks, at a month's runs for a few dozen pools.
    sqlx::query(
        "UPDATE rating_runs r SET evidence_games = NULL
           FROM rating_pools p
           CROSS JOIN LATERAL (SELECT x.id FROM rating_runs x WHERE x.pool_id = p.id
                               ORDER BY x.computed_at DESC, x.id DESC LIMIT 1) newest
          WHERE r.id = newest.id AND r.evidence_games IS NOT NULL",
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Refits a pool from scratch and stores the result as a new run.
///
/// Always a full refit, never a patch: adding or removing a config changes what
/// counts as evidence for *everyone*, and a batch fit has no per-player history
/// to unwind. Cheap enough to do this way — the Newton fit is milliseconds for
/// a hundred-member pool (seconds only for hundreds of members or a long ladder
/// of sweeps, on a blocking thread: see `fit_and_store`), and the query above is
/// one grouped scan.
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
    let run = fit_within(&mut tx, pool_id, trigger, only_if_evidence_changed).await?;
    tx.commit().await?;
    Ok(run)
}

/// Refits a pool inside the caller's transaction, which must already hold
/// the pool's fit lock ([`lock_pool_fit`]).
///
/// For a change that must commit together with its refit: an anchor change
/// stored without its run would leave the page showing ratings on the old
/// scale beside the new anchor until someone pressed Recompute, since the
/// sweep never notices an anchor move.
pub(crate) async fn recompute_within(
    conn: &mut PgConnection,
    pool_id: Uuid,
    trigger: Trigger,
) -> AppResult<Uuid> {
    fit_within(conn, pool_id, trigger, false)
        .await?
        .ok_or_else(|| AppError::internal("an unconditional refit stored nothing"))
}

/// Read, fit, write, on a connection whose transaction holds the fit lock.
/// Returns `None` when the sweep's check finds nothing new; the caller commits
/// either way (what that path writes is the refreshed evidence sum).
async fn fit_within(
    tx: &mut PgConnection,
    pool_id: Uuid,
    trigger: Trigger,
    only_if_evidence_changed: bool,
) -> AppResult<Option<Uuid>> {
    let pool = load_pool(&mut *tx, pool_id).await?;

    // The cheap question first: have the pool's jobs completed any games, or
    // its members changed, since the last run? `games_completed` is each
    // job's running total of games played, kept by the submission that
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
            return Ok(None);
        }
    }

    let Evidence { members, matrix, pairs_used, jobs_used, head_to_heads } =
        build_matrix(&mut *tx, pool_id).await?;

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
                return Ok(None);
            }
        }
    }

    let anchor_index = members
        .iter()
        .position(|id| *id == pool.anchor_player_config_id)
        .ok_or_else(|| AppError::bad_request("the pool's anchor is not a member of the pool"))?;

    // On a blocking thread, not the runtime's: a fit is a Cholesky factorisation
    // a step, a quarter of a second at 400 members and more on a long ladder,
    // and the service has one vCPU, so run inline it stalled every request that
    // thread was serving. The pool's lock and this transaction are held
    // meanwhile, as before.
    let anchor_rating = pool.anchor_rating;
    let (fit, matrix) = tokio::task::spawn_blocking(move || {
        (bradley_terry::fit(&matrix, anchor_index, anchor_rating), matrix)
    })
    .await
    .map_err(|err| {
        tracing::error!(%pool_id, "the rating fit for this pool did not finish");
        AppError::task_failed("the rating fit", err)
    })?;

    let run_id: Uuid = sqlx::query_scalar(
        // `computed_at` is the moment of the insert, under the pool's lock,
        // rather than the column's `now()` default -- the transaction's start,
        // taken before the lock. A fit that began first and got the lock
        // second would otherwise be stamped older than the run it superseded,
        // and the page, which reads the newest, would show the older one.
        "INSERT INTO rating_runs
             (pool_id, trigger, method, iterations, converged, pairs_used, jobs_used,
              evidence_games, computed_at)
         VALUES ($1, $2, 'bradley_terry_newton', $3, $4, $5, $6, $7, clock_timestamp())
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

    // The cross table -- each head-to-head's score, its error, its spread and
    // what the ratings predict -- stored with the run rather than computed
    // when the pool is viewed. Built here it costs nothing: the same grouped
    // scan the fit needs already read it. Built on a view it was that scan
    // again on every load of a public, unauthenticated page (see
    // `rating_run_residuals`), and it would describe evidence that has moved
    // on from the ratings beside it.
    let residuals = fit.residuals(&matrix);
    if !residuals.is_empty() {
        let mut rows = Vec::with_capacity(residuals.len());
        let mut cols = Vec::with_capacity(residuals.len());
        let mut pairs = Vec::with_capacity(residuals.len());
        let mut actual = Vec::with_capacity(residuals.len());
        let mut predicted = Vec::with_capacity(residuals.len());
        let mut stderr = Vec::with_capacity(residuals.len());
        let mut spread = Vec::with_capacity(residuals.len());
        for residual in &residuals {
            // `residuals` walks `i < j`, the evidence's own key order.
            let evidence =
                head_to_heads.get(&(residual.i, residual.j)).copied().unwrap_or_default();
            rows.push(members[residual.i]);
            cols.push(members[residual.j]);
            pairs.push(residual.games);
            actual.push(residual.actual);
            predicted.push(residual.predicted);
            stderr.push(evidence.score_and_stderr().1);
            spread.push(evidence.spread());
        }
        sqlx::query(
            "INSERT INTO rating_run_residuals
                 (run_id, row_player_config_id, col_player_config_id, pairs, actual, predicted,
                  stderr, spread)
             SELECT $1, * FROM UNNEST($2::uuid[], $3::uuid[], $4::float8[], $5::float8[],
                                      $6::float8[], $7::float8[], $8::float8[])",
        )
        .bind(run_id)
        .bind(&rows)
        .bind(&cols)
        .bind(&pairs)
        .bind(&actual)
        .bind(&predicted)
        .bind(&stderr)
        .bind(&spread)
        .execute(&mut *tx)
        .await?;
    }

    Ok(Some(run_id))
}

/// Refits every pool whose evidence has grown since its last run.
///
/// Deliberately a periodic sweep rather than a hook on result submission: a fit
/// is global to a pool, an active job submits thousands of results an hour, and
/// unlike the match test nothing blocks on the answer. Comparing the pair count against
/// the last run's `pairs_used` avoids needing a dirty flag anywhere.
///
/// One pool's failure does not stop the others. A fit can fail on state an
/// admin can reach -- a pool whose anchor is no longer a member is the obvious
/// one -- and propagating that ended the whole sweep at the first such pool, so
/// every pool ordered after it silently stopped being refit for as long as the
/// misconfiguration lasted. Each pool is logged and skipped instead, and the
/// count returned is of the fits that actually ran.
///
/// A pool an admin deleted after the list was read is skipped without a word:
/// its fit finds no pool (the delete waits for a fit already under way, so
/// this is the only way the two meet), and that is not a failure.
pub async fn recompute_stale(db: &PgPool) -> AppResult<usize> {
    let pool_ids =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM rating_pools").fetch_all(db).await?;

    let mut recomputed = 0;
    for pool_id in pool_ids {
        match recompute_if_stale(db, pool_id).await {
            Ok(true) => recomputed += 1,
            Ok(false) => {}
            Err(err) if err.status == axum::http::StatusCode::NOT_FOUND => {
                tracing::debug!(%pool_id, "a rating pool was deleted mid-sweep; skipping it")
            }
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
/// rows a day that nothing reads once the run is no longer the newest but the
/// history endpoint, which thins to 500 points over the pool's whole life
/// anyway; a run a day past the window keeps its series' shape and its ends.
/// Deleting everything past the window would have started the history at the
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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-12, "{got} != {want}");
    }

    /// U-RATE-1: a head-to-head's two jobs, one with each config as player 1,
    /// summed from the first config's side. A seats first in a job of 16
    /// pairs (pentanomial [1, 3, 7, 3, 2], 32 games, A 10 points a game ahead)
    /// and second in one of 4 ([2, 1, 1, 0, 0] for B, 8 games, B 6 ahead),
    /// which from A's side is [0, 0, 1, 1, 2] and 6 behind.
    ///
    /// Summed, [1, 3, 8, 4, 4] over N = 20 pairs: a mean of 47/80 = 0.5875,
    /// a second moment of 27/64, a variance of 27/64 − (47/80)² = 491/6400,
    /// and a standard error of √(491/6400 / 20) = √(491/128000) =
    /// 0.0619349457091874…; the spread is (32·10 − 8·6) / 40 = 6.8.
    #[test]
    fn a_head_to_head_sums_both_seatings_from_one_side() {
        let mut a_side = HeadToHeadEvidence::default();
        a_side.add([1, 3, 7, 3, 2], 32, 32.0 * 10.0, false);
        a_side.add([2, 1, 1, 0, 0], 8, 8.0 * 6.0, true);
        assert_eq!(a_side.pentanomial.counts, [1, 3, 8, 4, 4]);
        let (score, stderr) = a_side.score_and_stderr();
        close(score, 47.0 / 80.0);
        close(stderr, 0.061_934_945_709_187_48);
        close(stderr, (491.0f64 / 128_000.0).sqrt());
        close(a_side.spread(), 6.8);

        // Which is (W + ½D) / games over the same forty games: A's
        // half-points, 47 of the 80 they hold.
        assert_eq!(a_side.pentanomial.half_points(), 47);

        // Built from B's side, it is the mirror image: the same error, the
        // score complemented, the spread negated.
        let mut b_side = HeadToHeadEvidence::default();
        b_side.add([1, 3, 7, 3, 2], 32, 32.0 * 10.0, true);
        b_side.add([2, 1, 1, 0, 0], 8, 8.0 * 6.0, false);
        let (b_score, b_stderr) = b_side.score_and_stderr();
        close(b_score, 1.0 - score);
        close(b_stderr, stderr);
        close(b_side.spread(), -6.8);
    }

    /// U-RATE-2: the error is the pairs', not the games'. Twenty pairs that
    /// all split say the two are level and nothing else, so the error is 0,
    /// where counting the forty games as independent coin flips would give
    /// √(¼ / 40) ≈ 7.9 points of win %.
    #[test]
    fn identical_pairs_narrow_the_error_rather_than_widen_it() {
        let mut level = HeadToHeadEvidence::default();
        level.add([0, 0, 20, 0, 0], 40, 0.0, false);
        assert_eq!(level.score_and_stderr(), (0.5, 0.0));
        assert_eq!(level.spread(), 0.0);
        // Nothing played: no score to speak of, and no finite error.
        assert_eq!(HeadToHeadEvidence::default().score_and_stderr(), (0.5, f64::INFINITY));
    }

    /// U-RATE-3: the page's cross table has each head-to-head from both
    /// sides, the second the first's mirror, ordered by row then column.
    #[test]
    fn a_stored_cell_is_served_from_both_sides() {
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let ab = CrossCell {
            row: a,
            col: b,
            pairs: 20.0,
            actual: 0.5875,
            predicted: 0.56,
            stderr: 0.0619,
            spread: 6.8,
        };
        let bc = CrossCell {
            row: b,
            col: c,
            pairs: 4.0,
            actual: 0.25,
            predicted: 0.3,
            stderr: 0.1,
            spread: -12.5,
        };
        let cells = both_sides(&[bc, ab]);
        let order: Vec<(Uuid, Uuid)> = cells.iter().map(|c| (c.row, c.col)).collect();
        assert_eq!(order, [(a, b), (b, a), (b, c), (c, b)]);
        let ba = cells[1];
        assert_eq!((ba.pairs, ba.stderr), (20.0, 0.0619));
        close(ba.actual, 1.0 - 0.5875);
        close(ba.predicted, 1.0 - 0.56);
        assert_eq!(ba.spread, -6.8);
        let back = ba.mirrored();
        assert_eq!((back.row, back.col, back.spread), (a, b, 6.8), "mirrored twice is the cell");
        close(back.actual, ab.actual);
    }
}
