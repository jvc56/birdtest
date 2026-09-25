pub mod dispatch;
pub mod game;
pub mod game_pair;
pub mod handler;
pub mod leave_gen;
pub mod opening_rack;
pub mod plausibility;
pub mod racks;
pub mod registry;

use crate::error::AppResult;
use crate::models::job::NamedPlayerConfig;
use handler::{
    GameRequest, GameResultsRecord, MoveEntry, PlayerSpec, PlyStats, PositionAnalysis,
};
use racks::LetterDistribution;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

/// The advisory-lock namespace for dispatch decisions. Postgres advisory locks
/// are a single flat 64-bit space shared by every user of them, so the
/// two-argument form's first key is a namespace and the second identifies the
/// job.
const DISPATCH_LOCK_NAMESPACE: i32 = 1;

/// Serialize one job's dispatch decisions against each other, for the rest of
/// the caller's transaction.
///
/// Every job type needs this, for the same reason: what to hand out next is
/// decided from reads that a concurrent claim's uncommitted writes are
/// invisible to.
///
/// - **Games, game pairs and opening racks** pick the next seed with
///   `MAX(seed)`, so two overlapping claims compute the same one. The
///   `(job_id, seed)` unique index catches that, but only by failing the loser,
///   and `scheduler::claim` gives up after three attempts -- so past three-way
///   contention on one job a worker is told `204` while work exists. The lock
///   costs nothing that was not already being paid: `issue_claim` bumps
///   `jobs.claims_issued`, which takes the job's row lock until commit, so
///   claims against one job already serialize. This only moves the start of
///   that window earlier, turning a lost race into a short wait.
/// - **Leave generation** additionally decides which racks are still out and
///   whether the generation can close; see `leave_gen::next_step`.
///
/// Per job, so claims for other jobs are unaffected, and transaction-scoped, so
/// it is released on commit, on rollback, and on a dropped connection.
/// `hashtext` may collide, which costs two unrelated jobs a little
/// serialization and nothing else.
pub(crate) async fn lock_job_dispatch(conn: &mut PgConnection, job_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2::text))")
        .bind(DISPATCH_LOCK_NAMESPACE)
        .bind(job_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Jobs whose dispatch lock this process is holding for a long time: a leave
/// generation's universe being seeded (tens of seconds), a purge or a delete
/// (minutes, for a large job).
///
/// A claim for such a job skips it without asking the database. The bounded
/// wait of [`try_lock_job_dispatch`] alone was not enough: each waiter holds a
/// pool connection for the whole [`DISPATCH_LOCK_WAIT_MS`], and a job that
/// hands out nothing falls behind its share and so heads every worker's
/// candidate list -- a fleet of idle workers polling every five seconds held
/// the twenty-connection pool on it, and submissions for every other job
/// queued. In-process is enough because the service is a single instance
/// (`desired_count` is validated to at most one); the advisory lock is still
/// what makes the hold safe, and this only spares the wait.
///
/// A purge or a delete also holds every open claim of the job
/// ([`HoldKind::Claims`]), so a submission or decline for one of them is
/// answered at once rather than waiting out its lock timeout on a connection;
/// and if it ends without committing -- the request dropped at the load
/// balancer's timeout, a deadlock -- the job's claims are not reclaimed for a
/// heartbeat timeout afterwards: their heartbeats were skipped while it held
/// them, not missed.
#[derive(Clone, Default)]
pub struct DispatchHolds(std::sync::Arc<std::sync::Mutex<HoldsInner>>);

#[derive(Default)]
struct HoldsInner {
    /// Per job, how many holds of each kind: `[dispatch only, claims]`.
    held: std::collections::HashMap<Uuid, [usize; 2]>,
    /// Jobs whose claims-holding hold ended without committing, and until
    /// when their claims are not reclaimed.
    reclaim_not_before: std::collections::HashMap<Uuid, std::time::Instant>,
    /// Per job, how many claims holds (purges and deletes) have been taken:
    /// what an action that waited on the job's row compares, since the hold
    /// itself can be gone by the time it wakes.
    claims_holds_taken: std::collections::HashMap<Uuid, u64>,
}

/// What a [`DispatchHold`] holds besides the job's dispatch lock.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HoldKind {
    /// Nothing else: a seeding. Submissions go ahead.
    DispatchOnly,
    /// Every open claim of the job as well: a purge or a delete.
    Claims,
}

impl DispatchHolds {
    /// Marks the job held until the returned guard is dropped. `grace` is how
    /// long the job's claims are spared reclamation if a [`HoldKind::Claims`]
    /// hold is dropped without [`DispatchHold::committed`].
    pub fn hold(&self, job_id: Uuid, kind: HoldKind, grace: std::time::Duration) -> DispatchHold {
        let mut inner = self.0.lock().expect("dispatch holds poisoned");
        inner.held.entry(job_id).or_insert([0, 0])[kind as usize] += 1;
        DispatchHold { holds: self.clone(), job_id, kind, grace, committed: false }
    }

    /// A [`HoldKind::Claims`] hold, unless one is already held on the job:
    /// the check and the hold under one lock, so of two purges or deletes
    /// arriving together exactly one gets it.
    pub fn try_hold_claims(&self, job_id: Uuid, grace: std::time::Duration) -> Option<DispatchHold> {
        let mut inner = self.0.lock().expect("dispatch holds poisoned");
        let counts = inner.held.entry(job_id).or_insert([0, 0]);
        if counts[HoldKind::Claims as usize] > 0 {
            return None;
        }
        counts[HoldKind::Claims as usize] += 1;
        *inner.claims_holds_taken.entry(job_id).or_insert(0) += 1;
        Some(DispatchHold { holds: self.clone(), job_id, kind: HoldKind::Claims, grace, committed: false })
    }

    /// How many purges or deletes of the job have started in this process.
    pub fn claims_holds_taken(&self, job_id: Uuid) -> u64 {
        self.0
            .lock()
            .expect("dispatch holds poisoned")
            .claims_holds_taken
            .get(&job_id)
            .copied()
            .unwrap_or(0)
    }

    /// Whether claims should skip the job.
    pub fn is_held(&self, job_id: Uuid) -> bool {
        self.0.lock().expect("dispatch holds poisoned").held.contains_key(&job_id)
    }

    /// Whether the job's open claims are held, so a submission or decline for
    /// one would only wait.
    pub fn claims_held(&self, job_id: Uuid) -> bool {
        self.0
            .lock()
            .expect("dispatch holds poisoned")
            .held
            .get(&job_id)
            .is_some_and(|counts| counts[HoldKind::Claims as usize] > 0)
    }

    /// Whether any job's claims are held at all -- the cheap check that lets a
    /// submission skip looking up its job the rest of the time.
    pub fn any_claims_held(&self) -> bool {
        self.0
            .lock()
            .expect("dispatch holds poisoned")
            .held
            .values()
            .any(|counts| counts[HoldKind::Claims as usize] > 0)
    }

    /// `job_ids` less the jobs whose claims are in their post-hold grace.
    pub fn reclaimable(&self, job_ids: &[Uuid]) -> Vec<Uuid> {
        let mut inner = self.0.lock().expect("dispatch holds poisoned");
        let now = std::time::Instant::now();
        inner.reclaim_not_before.retain(|_, until| *until > now);
        job_ids.iter().copied().filter(|id| !inner.reclaim_not_before.contains_key(id)).collect()
    }
}

/// See [`DispatchHolds::hold`].
pub struct DispatchHold {
    holds: DispatchHolds,
    job_id: Uuid,
    kind: HoldKind,
    grace: std::time::Duration,
    committed: bool,
}

impl DispatchHold {
    /// The holder's transaction committed: its claims are gone, not spared.
    pub fn committed(&mut self) {
        self.committed = true;
    }
}

impl Drop for DispatchHold {
    fn drop(&mut self) {
        let mut inner = self.holds.0.lock().expect("dispatch holds poisoned");
        if let Some(counts) = inner.held.get_mut(&self.job_id) {
            counts[self.kind as usize] -= 1;
            if counts == &[0, 0] {
                inner.held.remove(&self.job_id);
            }
        }
        if self.kind == HoldKind::Claims && !self.committed {
            inner
                .reclaim_not_before
                .insert(self.job_id, std::time::Instant::now() + self.grace);
        }
    }
}

/// How long a claim waits for a job's dispatch lock before giving up on that
/// job and trying the next one.
///
/// Ordinary contention is milliseconds -- a claim transaction is a handful of
/// indexed statements -- so this is never reached in normal operation. It
/// exists for the one holder that is not ordinary: seeding a leave-generation
/// generation's rack universe is millions of rows and tens of seconds, and the
/// task that seeds it holds this very lock throughout. Without a bound,
/// every other claim for that job blocks for the duration *while holding a
/// pool connection*, and the pool is twenty -- so one slow claim on one job
/// stalls submissions and the dashboard for the whole server. With it, the
/// waiting workers are told there is nothing here right now and go elsewhere.
const DISPATCH_LOCK_WAIT_MS: u32 = 2_000;

/// Take the job's dispatch lock, giving up after [`DISPATCH_LOCK_WAIT_MS`].
///
/// `false` means another claim holds it: this job has nothing to offer *right
/// now*, which is exactly what `Acquired::NoWork` says. The caller must not
/// issue further statements on this connection, since the timed-out statement
/// aborted the transaction; every caller returns straight away and the claim
/// path rolls back.
pub(crate) async fn try_lock_job_dispatch(
    conn: &mut PgConnection,
    job_id: Uuid,
) -> AppResult<bool> {
    sqlx::query(&format!("SET LOCAL lock_timeout = '{DISPATCH_LOCK_WAIT_MS}ms'"))
        .execute(&mut *conn)
        .await?;
    let taken = sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2::text))")
        .bind(DISPATCH_LOCK_NAMESPACE)
        .bind(job_id)
        .execute(&mut *conn)
        .await;
    match taken {
        Ok(_) => {
            // The bound covers the wait for *this* lock only; the rest of the
            // claim transaction takes ordinary row locks and should wait for
            // them as it always has.
            sqlx::query("SET LOCAL lock_timeout = DEFAULT")
                .execute(&mut *conn)
                .await?;
            Ok(true)
        }
        Err(err) => {
            let err: crate::error::AppError = err.into();
            if err.db_code.as_deref() == Some(crate::error::LOCK_NOT_AVAILABLE) {
                tracing::debug!(%job_id, "another claim holds this job's dispatch lock");
                Ok(false)
            } else {
                Err(err)
            }
        }
    }
}

/// Take the job's dispatch lock only if nobody holds it, without waiting.
///
/// For background work that any later claim will ask for again if it does not
/// happen now: waiting would hold a pool connection for as long as the holder
/// runs, and the holder may be another copy of the same work.
pub(crate) async fn try_lock_job_dispatch_now(
    conn: &mut PgConnection,
    job_id: Uuid,
) -> AppResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_xact_lock($1, hashtext($2::text))")
        .bind(DISPATCH_LOCK_NAMESPACE)
        .bind(job_id)
        .fetch_one(conn)
        .await?)
}

/// Mark a job completed because its finish condition was met -- unless it was
/// purged after the evidence for that decision was read.
///
/// The finish check reads a job's results and then writes its status, holding
/// no lock across the two: one held across the read would stall that job's
/// claims or submissions for an aggregate over its whole history. A purge that
/// landed in between left the job `completed` with every result the check had
/// seen deleted, and a completed job cannot be reactivated, so the admin's
/// restart was undone for good.
///
/// `claims_issued` is the witness. It only ever grows, except that a purge
/// zeroes it, and the caller reads it before reading the results -- so a purge
/// in between leaves it below what was observed, and the update does nothing.
/// Returns whether the job was completed.
///
/// `decided` is the SPRT verdict the check completed a games job on, with the
/// units it had, and is stored with the completion (`jobs.sprt_decided_*`).
pub async fn complete_unless_purged(
    pool: &sqlx::PgPool,
    job_id: Uuid,
    observed_claims_issued: i64,
    decided: Option<(crate::stats::sprt::SprtResult, u64)>,
) -> AppResult<bool> {
    let status = decided.map(|(sprt, _)| sprt.status.as_str());
    Ok(sqlx::query(
        "UPDATE jobs SET status = 'completed',
                         sprt_decided_status = $3, sprt_decided_llr = $4, sprt_decided_units = $5
         WHERE id = $1 AND status = 'active' AND claims_issued >= $2",
    )
    .bind(job_id)
    .bind(observed_claims_issued)
    .bind(status)
    .bind(decided.map(|(sprt, _)| sprt.llr))
    .bind(decided.map(|(_, units)| units as i64))
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub(crate) async fn load_player_spec(
    conn: &mut PgConnection,
    player_config_id: Uuid,
) -> AppResult<PlayerSpec> {
    let config = sqlx::query_as::<_, NamedPlayerConfig>(&format!(
        "{} WHERE pc.id = $1",
        NamedPlayerConfig::SELECT
    ))
    .bind(player_config_id)
    .fetch_one(conn)
    .await?;
    Ok(config.into())
}

/// What a job pins that is not per-player: the rules variant, the board, and
/// the letter distribution -- the last as the actual bytes of the row, parsed.
///
/// The server enumerates rack universes and builds KLVs from that
/// distribution, so it must be the pinned bytes rather than a file on the
/// server's disk. There is no server-side copy of the data to disagree with.
pub struct JobData {
    pub variant: String,
    pub letterdist_name: String,
    pub letterdist: LetterDistribution,
    /// The pinned board layout's name, which is what the worker's request
    /// states. The bytes stay on the row: nothing server-side reads a layout.
    pub layout_name: String,
    /// Run-wide MAGPIE settings every request states.
    pub bingo_bonus: i32,
    pub sim_cutoff: f64,
}

/// The job settings every request and every rack enumeration is built from.
/// `pub` rather than crate-private so a test can make one claim decision the
/// way the claim path makes it.
pub async fn load_job_data(conn: &mut PgConnection, job_id: Uuid) -> AppResult<JobData> {
    let row = sqlx::query(
        "SELECT j.variant, j.bingo_bonus, j.sim_cutoff, ld.name AS ld_name,
                ld.content AS ld_content, layout.name AS layout_name
         FROM jobs j
         JOIN input_data ld ON ld.id = j.letterdist_id
         JOIN input_data layout ON layout.id = j.layout_id
         WHERE j.id = $1",
    )
    .bind(job_id)
    .fetch_one(conn)
    .await?;

    // NOT NULL by the role/content equivalence check on input_data: a
    // letterdist row without bytes cannot exist, so this is a schema
    // violation rather than a case to handle.
    let content: Vec<u8> = row.get("ld_content");
    let letterdist_name: String = row.get("ld_name");
    Ok(JobData {
        variant: row.get("variant"),
        letterdist: LetterDistribution::parse(&content, &letterdist_name)?,
        letterdist_name,
        layout_name: row.get("layout_name"),
        bingo_bonus: row.get("bingo_bonus"),
        sim_cutoff: row.get("sim_cutoff"),
    })
}

/// Writes the typed request row for a game or game-pair task.
///
/// The player ids are passed in rather than looked up from the names on the
/// request: the caller read them out of the job's config a moment ago, so
/// resolving them again was two round trips per claim, inside the job's
/// dispatch lock, to recover something already in hand.
pub(crate) async fn insert_game_request(
    conn: &mut PgConnection,
    task_id: Uuid,
    req: &GameRequest,
    player1_config_id: Uuid,
    player2_config_id: Uuid,
) -> AppResult<()> {
    let (p1, p2) = (player1_config_id, player2_config_id);
    sqlx::query(
        "INSERT INTO game_requests
             (task_id, variant, seed, num_games, player1_config_id,
              player2_config_id, capture_positions, letter_distribution,
              board_layout)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(task_id)
    .bind(&req.variant)
    .bind(req.seed as i64)
    .bind(req.num_games)
    .bind(p1)
    .bind(p2)
    .bind(req.capture_positions)
    .bind(&req.letter_distribution)
    .bind(&req.board_layout)
    .execute(conn)
    .await?;
    Ok(())
}

/// Reads a game or game-pair task's request back. The row carries what differs
/// per task; the players and the run-wide settings are the job's, from its
/// template.
pub(crate) async fn load_game_request(
    conn: &mut PgConnection,
    template: &dispatch::JobTemplate,
    task_id: Uuid,
    game_pairs: bool,
) -> AppResult<GameRequest> {
    let (player1, player2) = match &template.kind {
        dispatch::JobKind::Games { player1, player2, .. }
        | dispatch::JobKind::GamePairs { player1, player2, .. } => (player1, player2),
        _ => return Err(template.mismatch("games")),
    };
    let row = game::load_game_request_row(conn, task_id).await?;
    Ok(GameRequest {
        variant: row.get("variant"),
        seed: game::seed_from_row(&row),
        num_games: row.get("num_games"),
        game_pairs,
        capture_positions: row.get("capture_positions"),
        bingo_bonus: template.data.bingo_bonus,
        sim_cutoff: template.data.sim_cutoff,
        letter_distribution: row.get("letter_distribution"),
        board_layout: row.get("board_layout"),
        player1: player1.clone(),
        player2: player2.clone(),
    })
}

/// Writes analysed positions and their top-ranked moves.
///
/// Shared by opening rack jobs (one position per rack) and by games jobs with
/// capture on (one per turn). `on_conflict_ignore` is set for in-game positions:
/// games are deterministic, so redundant claims replay identical games, and the
/// first accepted claim is the one that lands.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_position_analyses(
    conn: &mut PgConnection,
    job_id: Uuid,
    task_id: Uuid,
    claim_id: Uuid,
    positions: &[PositionAnalysis],
    top_moves: i32,
    top_plies: i32,
    on_conflict_ignore: bool,
) -> AppResult<()> {
    use std::collections::HashMap;

    if positions.is_empty() {
        return Ok(());
    }

    // Everything below is written in multi-row statements rather than a
    // statement per position, and that is the difference between a submission
    // costing a handful of round trips and costing one per rack. An
    // opening-rack task carries `racks_per_batch` positions -- 500 by default
    // and up to 10,000 -- so row at a time meant thousands of round trips
    // inside the submit transaction, holding the task's row lock for all of
    // them. The batch sizes keep each statement well under Postgres's
    // 65,535-parameter ceiling.
    let mut record_ids: Vec<Option<i64>> = vec![None; positions.len()];
    for (chunk_index, chunk) in positions.chunks(RECORD_ROWS_PER_STATEMENT).enumerate() {
        let base = chunk_index * RECORD_ROWS_PER_STATEMENT;
        let mut builder = sqlx::QueryBuilder::new(
            "INSERT INTO position_analysis_records
                 (task_claim_id, task_id, job_id, rack, position, game_index,
                  turn_number, previous_move, previous_move_score, num_moves) ",
        );
        builder.push_values(chunk.iter(), |mut b, position| {
            b.push_bind(claim_id)
                .push_bind(task_id)
                .push_bind(job_id)
                .push_bind(position.rack.clone())
                .push_bind(position.position.clone())
                .push_bind(position.game_index)
                .push_bind(position.turn_number)
                .push_bind(position.previous_move.clone())
                .push_bind(position.previous_move_score)
                .push_bind(position.num_moves);
        });

        if on_conflict_ignore {
            // In-game positions: another claim of the same task replayed the
            // same deterministic games and may already have recorded some of
            // these, so what comes back is a subset and has to be matched up
            // rather than zipped. `(game_index, turn_number)` is what the
            // partial unique index is on, so it identifies the row.
            builder.push(" ON CONFLICT DO NOTHING RETURNING id, game_index, turn_number");
            let rows = builder.build().fetch_all(&mut *conn).await?;
            let mut by_position: HashMap<(i16, i16), usize> = HashMap::new();
            for (offset, position) in chunk.iter().enumerate() {
                if let (Some(game), Some(turn)) = (position.game_index, position.turn_number) {
                    by_position.insert((game, turn), base + offset);
                }
            }
            for row in rows {
                let (game, turn): (Option<i16>, Option<i16>) =
                    (row.get("game_index"), row.get("turn_number"));
                if let (Some(game), Some(turn)) = (game, turn) {
                    if let Some(&index) = by_position.get(&(game, turn)) {
                        record_ids[index] = Some(row.get("id"));
                    }
                }
            }
        } else {
            // Opening racks: every row lands or the statement fails, and a
            // multi-row insert returns its rows in the order they were given,
            // so the ids line up with the chunk.
            builder.push(" RETURNING id");
            let ids: Vec<i64> = builder.build_query_scalar().fetch_all(&mut *conn).await?;
            for (offset, id) in ids.into_iter().enumerate() {
                record_ids[base + offset] = Some(id);
            }
        }
    }

    // `top_moves` is the config's num_plays_recorded, at least 1 by
    // constraint; clamped anyway rather than trusting the cast, and to i16
    // because that is what `rank` is stored as.
    let kept = top_moves.clamp(0, i16::MAX as i32) as usize;
    // Every move to write, across every position, with the record it belongs
    // to and its rank within that record. A record with no id was already
    // written by another claim, so its moves are there too and are skipped.
    let mut pending: Vec<(i64, i16, &MoveEntry)> = Vec::new();
    for (index, position) in positions.iter().enumerate() {
        let Some(record_id) = record_ids[index] else { continue };
        for (rank, entry) in position.moves.iter().take(kept).enumerate() {
            pending.push((record_id, (rank + 1) as i16, entry));
        }
    }

    let mut move_ids: Vec<i64> = Vec::with_capacity(pending.len());
    for chunk in pending.chunks(MOVE_ROWS_PER_STATEMENT) {
        let mut builder = sqlx::QueryBuilder::new(
            "INSERT INTO position_analysis_moves
                 (record_id, rank, move, score, equity, win_percentage,
                  blended_utility) ",
        );
        builder.push_values(chunk.iter(), |mut b, (record_id, rank, entry)| {
            b.push_bind(*record_id)
                .push_bind(*rank)
                .push_bind(entry.play.clone())
                .push_bind(entry.score)
                .push_bind(entry.equity)
                // NULL for a static player, which simulates nothing.
                .push_bind(entry.win_percentage)
                .push_bind(entry.blended_utility);
        });
        // Returned in insertion order, so the ids line up with `pending` and
        // the per-ply rows can be attached without looking each move up.
        builder.push(" RETURNING id");
        let ids: Vec<i64> = builder.build_query_scalar().fetch_all(&mut *conn).await?;
        move_ids.extend(ids);
    }

    // Only a simming player produces per-ply statistics; for a static player
    // this is empty and nothing is written. A simmed opening-rack batch is
    // racks x moves x plies rows, which is why they go out in batches too.
    //
    // Kept to the plies the config records, the way moves are kept to the
    // plays it records: `num_plies_recorded` is what told the worker how many
    // to report, and nothing bounded what it sent -- a 64 MB body could hold
    // tens of thousands of ply rows per move.
    let plies: Vec<(i64, &PlyStats)> = move_ids
        .iter()
        .zip(pending.iter())
        .flat_map(|(move_id, (_, _, entry))| {
            entry
                .plies
                .iter()
                .filter(|ply| i32::from(ply.ply) < top_plies)
                .map(move |ply| (*move_id, ply))
        })
        .collect();
    for chunk in plies.chunks(PLY_ROWS_PER_STATEMENT) {
        let mut builder = sqlx::QueryBuilder::new(
            "INSERT INTO position_analysis_plies
                 (move_id, ply, bingo_percentage, average_score) ",
        );
        builder.push_values(chunk.iter(), |mut b, (move_id, ply)| {
            b.push_bind(*move_id)
                .push_bind(ply.ply)
                .push_bind(ply.bingo_percentage)
                .push_bind(ply.average_score);
        });
        builder.push(" ON CONFLICT (move_id, ply) DO NOTHING");
        builder.build().execute(&mut *conn).await?;
    }
    Ok(())
}

/// Rows per multi-row insert, keeping each statement well under Postgres's
/// 65,535-parameter ceiling (10, 7 and 4 binds per row respectively).
const RECORD_ROWS_PER_STATEMENT: usize = 2_000;
const MOVE_ROWS_PER_STATEMENT: usize = 4_000;
const PLY_ROWS_PER_STATEMENT: usize = 8_000;

pub(crate) async fn insert_game_results(
    conn: &mut PgConnection,
    template: &dispatch::JobTemplate,
    task_id: Uuid,
    claim_id: Uuid,
    record: &GameResultsRecord,
) -> AppResult<()> {
    let job_id = template.job_id;
    let all = &record.all_games;
    let divergent = record.divergent_games.as_ref();
    let pentanomial = record.pentanomial.as_ref();
    let bucket = |i: usize| pentanomial.map(|p| p[i] as i32);
    // `submitted_at` is what every "first accepted result per task" read orders
    // on, so it is the time of this insert, taken under the task's row lock,
    // rather than the column's `now()` default: that is when the transaction
    // *began*, and a submission that began first but reached the lock second
    // -- accepted second -- would read as the first, so the aggregates would
    // use one copy while the running totals had counted the other.
    sqlx::query(
        "INSERT INTO game_results
             (task_claim_id, task_id, job_id, games, wins, losses, ties,
              p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd,
              pent_0, pent_1, pent_2, pent_3, pent_4,
              divergent_games, divergent_wins, divergent_losses, divergent_ties,
              submitted_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,
                 clock_timestamp())",
    )
    .bind(claim_id)
    .bind(task_id)
    .bind(job_id)
    .bind(all.games)
    .bind(all.wins)
    .bind(all.losses)
    .bind(all.ties)
    .bind(all.p1_score_mean)
    .bind(all.p1_score_sd)
    .bind(all.p2_score_mean)
    .bind(all.p2_score_sd)
    .bind(bucket(0))
    .bind(bucket(1))
    .bind(bucket(2))
    .bind(bucket(3))
    .bind(bucket(4))
    .bind(divergent.map(|d| d.games))
    .bind(divergent.map(|d| d.wins))
    .bind(divergent.map(|d| d.losses))
    .bind(divergent.map(|d| d.ties))
    .execute(&mut *conn)
    .await?;

    // Deterministic games mean redundant claims replay identical positions, so
    // the first accepted claim records them and the rest are no-ops.
    //
    // A job without capture submits no positions, and that is every games job
    // by default.
    if record.positions.is_empty() {
        return Ok(());
    }
    // How many ranked moves to keep: player 1's num_plays_recorded, which is
    // also the one MAGPIE reads to decide how many to report. From the job's
    // template: it is a setting of an immutable player config.
    //
    // Plies are kept to the larger of the two players' `num_plies_recorded`: a
    // position is either player's, and a player that reports fewer (a static
    // one reports none) is not truncated by the other's cap.
    let (top_moves, top_plies) = match &template.kind {
        dispatch::JobKind::Games { player1, player2, .. }
        | dispatch::JobKind::GamePairs { player1, player2, .. } => (
            player1.num_plays_recorded,
            player1.num_plies_recorded.max(player2.num_plies_recorded),
        ),
        _ => return Err(template.mismatch("games")),
    };

    insert_position_analyses(
        conn,
        job_id,
        task_id,
        claim_id,
        &record.positions,
        top_moves,
        top_plies,
        true,
    )
    .await
}

/// One file a task needs, as the assignment states it.
///
/// `role` and `name` are what the client resolves through its own data path
/// search; `path` and `tarball_date` exist for the message it prints when the
/// file is missing or wrong.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExpectedFile {
    pub role: String,
    pub name: String,
    pub path: String,
    pub sha256: String,
    pub bytes: i64,
    pub tarball_date: String,
}

/// Every file the job's tasks will actually load, deduplicated.
///
/// This is a query, not an inference engine: the job pins its letter
/// distribution and board, and its players pin their own lexicon, leaves and
/// win% model. Two players on the same lexicon contribute one `kwg` entry, not
/// two; a static player contributes no `winpct` entry at all, because MAGPIE
/// never opens one for it.
pub async fn expected_data(
    conn: &mut PgConnection,
    job: &crate::models::job::Job,
) -> AppResult<Vec<ExpectedFile>> {
    // One query, not three, and run once per job per process: the answer is
    // fixed at job creation, so it is part of the job's template
    // (`dispatch::JobTemplate`) rather than read on every claim. The union
    // also removes the match on `job_type` that used to choose between them:
    // a job type simply has no row in the config tables it does not use, so
    // the branches contribute nothing rather than needing to be skipped.
    let rows = sqlx::query(
        "WITH players AS (
             SELECT unnest(ARRAY[player1_config_id, player2_config_id]) AS id
             FROM job_game_config WHERE job_id = $1
             UNION
             SELECT unnest(ARRAY[player1_config_id, player2_config_id])
             FROM job_game_pair_config WHERE job_id = $1
             UNION
             SELECT player_config_id FROM job_opening_rack_config WHERE job_id = $1
         ),
         ids AS (
             SELECT j.letterdist_id AS id FROM jobs j WHERE j.id = $1
             UNION SELECT j.layout_id FROM jobs j WHERE j.id = $1
             -- Leave generation has one bot and no player_configs row, so its
             -- lexicon sits on the job config. It needs no klv (every
             -- generation's leaves are a server-built artifact) and no winpct
             -- (the bot plays statically).
             UNION SELECT c.kwg_id FROM job_leave_config c WHERE c.job_id = $1
             UNION SELECT pc.kwg_id FROM player_configs pc JOIN players p ON p.id = pc.id
             UNION SELECT pc.klv_id FROM player_configs pc JOIN players p ON p.id = pc.id
             -- NULL for a static player, which never opens a win% model; it
             -- joins to nothing and so contributes no entry.
             UNION SELECT pc.winpct_id FROM player_configs pc JOIN players p ON p.id = pc.id
         )
         SELECT d.role, d.name, d.path, d.sha256, d.bytes, d.tarball_date
         FROM input_data d JOIN ids ON ids.id = d.id
         ORDER BY d.role, d.name",
    )
    .bind(job.id)
    .fetch_all(conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ExpectedFile {
            role: row.get("role"),
            name: row.get("name"),
            path: row.get("path"),
            sha256: row.get("sha256"),
            bytes: row.get("bytes"),
            tarball_date: row.get("tarball_date"),
        })
        .collect())
}

#[cfg(test)]
mod holds_tests {
    use super::*;

    #[test]
    fn only_one_claims_hold_is_taken_at_a_time() {
        let holds = DispatchHolds::default();
        let job = Uuid::new_v4();
        let grace = std::time::Duration::from_secs(1);
        let seeding = holds.hold(job, HoldKind::DispatchOnly, grace);
        let first = holds.try_hold_claims(job, grace).expect("a seeding does not hold claims");
        assert!(holds.try_hold_claims(job, grace).is_none());
        assert!(holds.try_hold_claims(Uuid::new_v4(), grace).is_some(), "per job");
        drop(first);
        assert!(holds.try_hold_claims(job, grace).is_some());
        drop(seeding);
        // Counted, so an action that waited on the job's row can tell a purge
        // came and went meanwhile.
        assert_eq!(holds.claims_holds_taken(job), 2);
    }
}
