use super::dispatch::{JobKind, JobTemplate};
use super::handler::*;
use super::racks::{LetterDistribution, RackIndex};
use super::JobData;
use crate::auth::WorkerIdentity;
use crate::error::{AppError, AppResult};
use crate::models::job::{ConsensusSettings, OpeningRackConfig};
use sqlx::{PgConnection, Row};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub struct OpeningRackHandler;

impl JobHandler for OpeningRackHandler {
    type Request = OpeningRackRequest;
    type Response = PositionAnalysisResponse;
    type Record = PositionAnalysisRecord;

    async fn load_request(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
    ) -> AppResult<Self::Request> {
        let JobKind::OpeningRack { player, index, .. } = &template.kind else {
            return Err(template.mismatch("opening_rack"));
        };
        let row = sqlx::query(
            "SELECT variant, letter_distribution, board_layout, rack_start, rack_count, racks,
                    candidates
             FROM opening_rack_requests WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
        let rack_start: i64 = row.get("rack_start");

        Ok(OpeningRackRequest {
            racks: task_racks(index, &row),
            // As dispatched: a reclaim ranks the moves the first claim was
            // sent, whatever the job's X is now.
            candidates: task_candidates(&row),
            seed: rack_start as u64,
            variant: row.get("variant"),
            letter_distribution: row.get("letter_distribution"),
            board_layout: row.get("board_layout"),
            bingo_bonus: template.data.bingo_bonus,
            sim_cutoff: template.data.sim_cutoff,
            player: player.clone(),
        })
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        if response.racks.is_empty() {
            return Err(AppError::bad_request("position analysis returned no racks"));
        }

        let mut racks = Vec::with_capacity(response.racks.len());
        for analysis in response.racks {
            // The moves arrive ranked best-first, so an empty list means the
            // worker analyzed nothing -- there is no best move to record.
            if analysis.moves.is_empty() {
                return Err(AppError::bad_request(format!(
                    "rack {} was analyzed with no moves",
                    analysis.rack
                )));
            }
            super::plausibility::check_rack(&analysis.rack, "opening rack")?;
            // A worker cannot have reported more moves than it says it ranked.
            super::plausibility::check_moves(
                &analysis.moves,
                Some(analysis.num_moves),
                "opening rack",
            )?;
            let rack = PositionAnalysis::opening_rack(
                analysis.rack.clone(),
                analysis.moves,
                analysis.num_moves,
            );
            // An opening rack is analysed statically or by simulation, never
            // by a solver: a solve's statistics on one are a broken client.
            super::plausibility::check_analysis(rack.analysis, &rack.moves, "opening rack")?;
            racks.push(rack);
        }
        Ok(PositionAnalysisRecord { positions: racks })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        // How many ranked moves to keep per rack. Deliberately separate from
        // how many the worker generated or simulated: a simmer may rank
        // hundreds to get the order right while only the leaders are worth
        // storing for every rack in a space of millions. The player config's
        // `num_plays_recorded`, the same number that told the worker how many
        // to report, from the template rather than a join inside the task's
        // row lock.
        let JobKind::OpeningRack { player, .. } = &template.kind else {
            return Err(template.mismatch("opening_rack"));
        };

        // An opening rack is unique per (claim, rack), so a conflict here would
        // be a duplicate within one submission.
        super::insert_position_analyses(
            conn,
            template.job_id,
            task_id,
            claim_id,
            &record.positions,
            super::RecordedCaps::one(player.num_plays_recorded, player.num_plies_recorded),
        )
        .await?;

        Ok(())
    }
}

/// Checks an opening-rack submission against the racks the task dispatched,
/// and a reissue's against the moves it sent.
///
/// The counterpart of the batch-size rule game jobs get
/// ([`super::plausibility::check_batch_size`]), and the same rule: what was
/// asked for was fixed when the task was handed out, so an answer to anything
/// else is answering a question nobody asked. It is stronger here because the
/// request names the racks rather than just how many, so the whole set can be
/// compared rather than its size.
///
/// Without it a submission was unchecked input in both directions. Too few
/// racks and the task still completed, leaving a hole in the rack space that
/// nothing revisits -- the job's own finish condition only asks whether every
/// task completed. Too many, or racks from nowhere, and they were stored as
/// analyses of this job and added to `jobs.racks_analyzed`, the progress
/// counter the dashboard reads.
///
/// A reissue that sent candidates is answered over them alone: a reported
/// move it did not send, or more moves ranked than it sent, is a worker that
/// analysed something else (a build that ignored them ranks every play), and
/// its rank-1 move would vote for a move the pass never weighed.
///
/// Expanding the range costs what dispatching it cost: a handful of additions
/// per rack, against a batch capped at 10,000. One read -- the task's range and
/// candidates -- inside the task's row lock, where each round trip is time
/// every other submission for the task waits.
pub async fn check_batch_against_task(
    conn: &mut PgConnection,
    template: &JobTemplate,
    task_id: Uuid,
    reported: &[PositionAnalysis],
) -> AppResult<()> {
    let JobKind::OpeningRack { index, .. } = &template.kind else {
        return Err(template.mismatch("opening_rack"));
    };
    let row = sqlx::query(
        "SELECT rack_start, rack_count, racks, candidates
         FROM opening_rack_requests WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(&mut *conn)
    .await?;
    let expected = task_racks(index, &row);

    if reported.len() != expected.len() {
        return Err(AppError::bad_request(format!(
            "result analyses {} racks but this task dispatched {}",
            reported.len(),
            expected.len()
        )));
    }
    // Order is not part of the contract, only the set.
    let dispatched: HashSet<&str> = expected.iter().map(String::as_str).collect();
    if let Some(stray) = reported.iter().find(|p| !dispatched.contains(p.rack.as_str())) {
        return Err(AppError::bad_request(format!(
            "result analyses rack {:?}, which this task did not dispatch",
            stray.rack
        )));
    }
    // Equal sizes plus containment is equality only if nothing is listed
    // twice. A duplicate was left for the unique index on (task_claim_id, rack)
    // to refuse, which it did -- as a `409 that already exists`, after the
    // batch had been sent to the database, where every other malformed
    // submission is a `400` that says what is wrong with it.
    let mut seen = HashSet::with_capacity(reported.len());
    if let Some(twice) = reported.iter().find(|p| !seen.insert(p.rack.as_str())) {
        return Err(AppError::bad_request(format!(
            "result analyses rack {:?} twice, so it leaves out a rack this task dispatched",
            twice.rack
        )));
    }

    let Some(candidates) = task_candidates(&row) else {
        return Ok(());
    };
    let sent: HashMap<&str, HashSet<&str>> = expected
        .iter()
        .zip(&candidates)
        .map(|(rack, moves)| (rack.as_str(), moves.iter().map(String::as_str).collect()))
        .collect();
    for position in reported {
        let moves = &sent[position.rack.as_str()];
        if position.num_moves as usize > moves.len() {
            return Err(AppError::bad_request(format!(
                "rack {:?} ranked {} moves, but this task sent it {} candidates",
                position.rack,
                position.num_moves,
                moves.len()
            )));
        }
        if let Some(other) = position.moves.iter().find(|m| !moves.contains(m.play.as_str())) {
            return Err(AppError::bad_request(format!(
                "rack {:?} ranked {:?}, which is not one of the candidates this task sent it",
                position.rack, other.play
            )));
        }
    }
    Ok(())
}

/// A reissue's candidates, from its request row: `None` for a first-pass task
/// and for a reissue made before reissues sent them.
fn task_candidates(row: &sqlx::postgres::PgRow) -> Option<Vec<Vec<String>>> {
    row.get::<Option<sqlx::types::Json<Vec<Vec<String>>>>, _>("candidates").map(|json| json.0)
}

/// How many distinct racks a job over this distribution covers. Recorded at job
/// creation so the scheduler knows when the space is exhausted without
/// re-deriving it on every claim.
pub fn total_racks(distribution: &LetterDistribution, rack_size: i32) -> AppResult<i64> {
    Ok(RackIndex::new(distribution, rack_size as usize)?.total() as i64)
}

/// The racks a task dispatched, from its request row (`rack_start`,
/// `rack_count`, `racks`).
///
/// A task covering a range stores only the range -- which is what makes a job
/// over millions of racks cheap to create. Expanding it is a handful of
/// additions per rack, not a walk over the space, and the table it walks is
/// the template's, built once for the job. A task reissuing racks a consensus
/// still wants lists them.
fn task_racks(index: &RackIndex, row: &sqlx::postgres::PgRow) -> Vec<String> {
    match row.get::<Option<Vec<String>>, _>("racks") {
        Some(racks) => racks,
        None => {
            let rack_start: i64 = row.get("rack_start");
            let rack_count: i32 = row.get("rack_count");
            index.racks_in_range(rack_start as u64, rack_count as u64)
        }
    }
}

/// The oldest MAGPIE that reads a reissue's `candidates`: the floor a job that
/// reissues racks must have (`routes::admin`'s `reissue_floor_problem`).
pub const CANDIDATES_MIN_MAGPIE_VERSION: crate::version::Version =
    crate::version::Version { major: 0, minor: 1, patch: 5 };

/// Claim-time task creation: the next unclaimed slice of the rack space, and
/// once the space is covered, the racks a consensus still wants.
///
/// Slices tile the space the same way game seeds do, so the `(job_id, seed)`
/// unique index resolves two workers racing for the same slice -- the loser
/// retries and takes the next one. The slice's start is also the task's seed:
/// rack `i` of the batch is analysed from `seed + i`, its index in the job's
/// rack space, which is the same on every worker.
///
/// Past the end of the space the cursor goes on stepping by the batch size,
/// and a task there reissues unsettled racks ([`next_reissue`]): its seed is
/// the cursor, so every analysis of a rack is sampled from a seed of its own.
/// A job wanting one analysis per rack has no unsettled racks once its space
/// is covered, so for it the space ending is the end.
///
/// `index` is the job's rack space and `player` its analysing player, both from
/// the job's template. Read here are the seed cursor and the consensus
/// settings, which an admin may have changed since the template was cached:
/// under the dispatch lock, which the edit holds while it writes them.
#[allow(clippy::too_many_arguments)]
pub async fn next_request(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &OpeningRackConfig,
    job_data: &JobData,
    index: &RackIndex,
    player: &PlayerSpec,
    identity: &WorkerIdentity,
) -> AppResult<Option<(i64, OpeningRackRequest)>> {
    let next_start = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(seed) FROM tasks WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?
    .map(|max| max + config.racks_per_batch as i64)
    .unwrap_or(0);

    let (racks, candidates) = if next_start < config.total_racks {
        // The final batch of a job comes up short: the range runs off the end
        // of the space and yields only what exists.
        (index.racks_in_range(next_start as u64, config.racks_per_batch as u64), None)
    } else {
        let consensus = ConsensusSettings::load(conn, job_id).await?;
        if consensus.reissues() {
            let racks = next_reissue(conn, job_id, config, identity).await?;
            let candidates = first_pass_leaders(conn, job_id, &racks, consensus.reissue_top_moves).await?;
            (racks, Some(candidates))
        } else {
            (Vec::new(), None)
        }
    };
    if racks.is_empty() {
        return Ok(None);
    }

    Ok(Some((
        next_start,
        OpeningRackRequest {
            variant: job_data.variant.clone(),
            letter_distribution: job_data.letterdist_name.clone(),
            board_layout: job_data.layout_name.clone(),
            racks,
            seed: next_start as u64,
            bingo_bonus: job_data.bingo_bonus,
            sim_cutoff: job_data.sim_cutoff,
            player: player.clone(),
            candidates,
        },
    )))
}

/// Each rack's candidates for a reissue, in the order of `racks`: the first
/// `top` moves, best first, of the rack's first analysis.
///
/// A rack's first analysis is its earliest record in the job: a rack is only
/// reissued once it has a standing, which its first analysis -- a first-pass
/// task's, ranking every play -- gave it. So racks analysed before reissues
/// sent candidates have them too, from what is already stored. The moves are
/// the ones stored, so a job keeping fewer than `top` per rack
/// (`num_plays_recorded`) sends what it kept.
///
/// One statement, bounded by the batch: the earliest record of each rack
/// through the `(job_id, rack)` index, then its leading moves through the
/// moves' record index.
async fn first_pass_leaders(
    conn: &mut PgConnection,
    job_id: Uuid,
    racks: &[String],
    top: i32,
) -> AppResult<Vec<Vec<String>>> {
    let mut leaders: HashMap<String, Vec<String>> = HashMap::with_capacity(racks.len());
    for (rack, play) in sqlx::query_as::<_, (String, String)>(
        "WITH first AS (
             SELECT DISTINCT ON (a.rack) a.rack, a.id
             FROM position_analysis_records a
             WHERE a.job_id = $1 AND a.game_index IS NULL AND a.rack = ANY($2)
             ORDER BY a.rack, a.id
         )
         SELECT f.rack, m.move
         FROM first f
         JOIN position_analysis_moves m ON m.record_id = f.id AND m.rank <= $3
         ORDER BY f.rack, m.rank",
    )
    .bind(job_id)
    .bind(racks)
    .bind(top)
    .fetch_all(&mut *conn)
    .await?
    {
        leaders.entry(rack).or_default().push(play);
    }
    racks
        .iter()
        .map(|rack| {
            leaders.remove(rack).ok_or_else(|| {
                AppError::internal(format!("rack {rack} is reissued but has no stored analysis"))
            })
        })
        .collect()
}

/// Up to a batch of a consensus job's unsettled racks to analyse again: fewest
/// analyses first, none that another task is analysing now, and preferably
/// none this worker has analysed before -- a consensus of one worker with
/// itself is a weaker one. When too few racks are left that it has not seen
/// (a small fleet, or the last few racks), the batch is filled with ones it
/// has: each analysis has a seed of its own, so it is still a fresh sample,
/// and refusing would leave a rack wanting more analyses than the fleet has
/// workers unsettled for good.
///
/// Under the job's dispatch lock, so the in-flight set cannot change under it
/// -- and so what this costs, every other claim for the job waits on. It is
/// bounded by the batch and by what is open now, not by the job or its
/// history:
///
/// - **The preference looks at a window.** Only the first
///   [`REISSUE_WINDOW_BATCHES`] batches of candidates (unsettled, not in
///   flight, fewest analyses first) are asked whether this identity analysed
///   them, and the unseen among them go first. It used to walk the unsettled
///   racks until it had found a batch this identity had not analysed, a
///   probe of its analyses per rack stepped over. An identity is an account,
///   every machine under it one identity, and once it has analysed the whole
///   first pass -- the owner's own machines, early on -- it has analysed every
///   rack: each of its reissues walked all of them (3.2 million for English)
///   and found nothing before taking any. Outside the window an unseen rack
///   is not looked for; within it, the window is the racks with the fewest
///   analyses, which is where the unseen ones are.
/// - **The preference is a probe per rack, never a hash.** It is asked as a
///   scalar subquery (`LIMIT 1`, then `IS NOT NULL`), not as `EXISTS`: an
///   `EXISTS` may be planned as a hashed subplan, which reads every analysis
///   the identity made in the job (every record of the job, in a generic
///   plan) to answer for the window's racks. Postgres picks it when its
///   statistics understate the records -- a database just restored, which
///   has none, or autovacuum behind -- and it measured 1.2-2.0 s a claim for
///   an identity with 300,000 analyses, 5.3 s with the table never analysed.
///   A scalar subquery is never hashed.
/// - **The in-flight racks are read from what is open.** A task not
///   completed is `available` or `claimed`: the available reissues are found
///   through the queue index, the claimed tasks through the open claims'
///   index (one open claim each, `task_claims_one_slot_idx`; a first-pass
///   task lists no racks). The seed index found every reissue the job had
///   ever made and visited each to skip the completed ones -- a cost that
///   grew with the reissue history, 27-44 ms at 200,000 of them.
/// - **The in-flight racks are a hashed set.** `NOT IN` over the array's
///   `unnest` is a hashed subplan whatever plan the prepared statement
///   settles on. `<> ALL($array)` is hashed only in a custom plan, and a
///   generic one compared each rack stepped over with every rack in flight,
///   which are the very racks at the front of the order.
async fn next_reissue(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &OpeningRackConfig,
    identity: &WorkerIdentity,
) -> AppResult<Vec<String>> {
    // In flight: the racks of every reissue not yet completed -- given back
    // and waiting to go out again (`tasks_queue_idx`), or claimed
    // (`task_claims_open_idx`: the fleet's open claims, which reclaiming
    // already reads on every claim). Not `state <> 'completed'` over the seed
    // index: that visited every reissue the job ever made.
    let in_flight: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT unnest(r.racks) FROM opening_rack_requests r
         WHERE r.racks IS NOT NULL AND r.task_id IN (
             SELECT t.id FROM tasks t
              WHERE t.job_id = $1 AND t.state = 'available' AND t.seed >= $2
             UNION ALL
             SELECT c.task_id FROM task_claims c
              WHERE c.job_id = $1 AND c.state = 'claimed')",
    )
    .bind(job_id)
    .bind(config.total_racks)
    .fetch_all(&mut *conn)
    .await?;

    let batch = i64::from(config.racks_per_batch);
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT w.rack FROM (
             SELECT p.rack, p.results FROM opening_rack_progress p
             WHERE p.job_id = $1 AND NOT p.settled
               AND p.rack NOT IN (SELECT unnest($2::text[]))
             ORDER BY p.results, p.rack
             LIMIT $6
         ) w
         ORDER BY (SELECT 1 FROM position_analysis_records a
                   JOIN task_claims c ON c.id = a.task_claim_id
                   WHERE a.job_id = $1 AND a.game_index IS NULL AND a.rack = w.rack
                     AND (c.claimed_by_user_id = $3 OR c.claimed_by_anon_uuid = $4)
                   LIMIT 1) IS NOT NULL,
                  w.results, w.rack
         LIMIT $5",
    )
    .bind(job_id)
    .bind(&in_flight)
    .bind(identity.user_id())
    .bind(identity.anon_uuid())
    .bind(batch)
    .bind(batch * REISSUE_WINDOW_BATCHES)
    .fetch_all(&mut *conn)
    .await?)
}

/// How many batches of candidates a reissue asks whether the claiming
/// identity analysed them ([`next_reissue`]): enough that a fleet of a few
/// identities finds a batch it has not seen among the racks with the fewest
/// analyses, few enough that a claim stays a bounded number of probes.
const REISSUE_WINDOW_BATCHES: i64 = 4;

/// Writes the typed request row for a task: the range it covers, or, for a
/// task reissuing racks a consensus still wants, the racks themselves and the
/// candidates it sends for each.
pub async fn insert_request(
    conn: &mut PgConnection,
    task_id: Uuid,
    config: &OpeningRackConfig,
    job_data: &JobData,
    start: i64,
    request: &OpeningRackRequest,
) -> AppResult<()> {
    let racks = &request.racks;
    let listed = (start >= config.total_racks).then_some(racks);
    sqlx::query(
        "INSERT INTO opening_rack_requests
             (task_id, variant, letter_distribution, board_layout, rack_start,
              rack_count, racks, player_config_id, candidates)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(task_id)
    .bind(&job_data.variant)
    .bind(&job_data.letterdist_name)
    .bind(&job_data.layout_name)
    .bind(start)
    .bind(racks.len() as i32)
    .bind(listed)
    .bind(config.player_config_id)
    .bind(request.candidates.as_ref().map(sqlx::types::Json))
    .execute(conn)
    .await?;
    Ok(())
}

/// What an accepted batch of analyses adds to a job's rack totals: racks
/// analysed for the first time, racks that need no more analysis, and those
/// of them settled at their most analyses without a consensus. The last two
/// are changes, which can be negative: under settings an admin changed while
/// a rack's reissue was out, a rack already settled can come back settled
/// without a consensus, or with one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RackTally {
    pub analysed: i64,
    pub settled: i64,
    pub without_consensus: i64,
}

/// Where a rack stands after its analyses: how many there are, its most
/// common rank-1 move (the alphabetically first of a tie), how many ranked it
/// first and how many the runner-up, whether it is settled and whether
/// without a consensus.
#[derive(Debug, Clone, PartialEq)]
pub struct RackStanding {
    pub results: i32,
    pub top_move: String,
    pub top_count: i32,
    pub runner_up_count: i32,
    pub settled: bool,
    pub without_consensus: bool,
}

/// A rack's standing from how many of its analyses ranked each move first,
/// its first analysis's among them: settled once the most common rank-1 move
/// leads the runner-up by `win_margin` (2-0 settles at two analyses, 3-1 at
/// four), or once it has `max_results_per_rack` analyses, without a
/// consensus if the lead is still short. [`restate_racks`] is the same rule
/// in SQL, on the counts this returns.
///
/// A job's first analysis of each rack is counted like any other: its rank-1
/// move is the first of the candidates every later pass ranks.
pub fn standing(config: &ConsensusSettings, firsts: &HashMap<String, i32>) -> Option<RackStanding> {
    let results: i32 = firsts.values().sum();
    let (top_move, top_count) = firsts
        .iter()
        .max_by(|(a_move, a), (b_move, b)| a.cmp(b).then_with(|| b_move.cmp(a_move)))?;
    let runner_up_count =
        firsts.iter().filter(|(play, _)| *play != top_move).map(|(_, n)| *n).max().unwrap_or(0);
    // A job wanting one analysis per rack seeks no agreement: its racks are
    // settled, and agreed, at their first, as they always were. Analysed
    // more than once before its maximum was lowered, a rack whose analyses
    // split is still without a consensus.
    let agreed = top_count - runner_up_count >= config.win_margin
        || (!config.reissues() && runner_up_count == 0);
    let settled = agreed || results >= config.max_results_per_rack;
    Some(RackStanding {
        results,
        top_move: top_move.clone(),
        top_count: *top_count,
        runner_up_count,
        settled,
        without_consensus: settled && !agreed,
    })
}

/// Racks' standings as columns, for one `unnest` upsert.
#[derive(Default)]
struct StandingColumns {
    racks: Vec<String>,
    results: Vec<i32>,
    top_moves: Vec<String>,
    top_counts: Vec<i32>,
    runner_up_counts: Vec<i32>,
    settled: Vec<bool>,
    without_consensus: Vec<bool>,
}

/// Records what an accepted batch says about each of its racks' consensus,
/// and returns what it adds to the job's rack totals. Runs after the batch's
/// analyses are stored, in the same transaction, so they count.
///
/// Each rack's standing is recomputed from its analyses' rank-1 moves -- at
/// most `max_results_per_rack` of them a rack, through the `(job_id, rack)`
/// index -- and written to `opening_rack_progress`. Every job keeps these
/// rows, even one wanting a single analysis per rack, whose racks all settle
/// at their first: its settings may be changed to seek a consensus later
/// ([`restate_racks`]), and then its reissues start from them, and a job
/// changed back to one analysis counts reissues still in flight against them
/// rather than as racks new to the job.
///
/// The settings are read here, not taken from the job's template: the
/// submission holds its claim's lock, which an edit of them takes first, so
/// this reads them as they stand once any edit has committed.
pub async fn record_consensus(
    conn: &mut PgConnection,
    job_id: Uuid,
    racks: &[String],
) -> AppResult<RackTally> {
    let config = ConsensusSettings::load(conn, job_id).await?;
    // Locked: a rack is analysed by one task at a time (`next_reissue`), so
    // nothing else should be writing these rows, and this makes sure of it.
    let before: HashMap<String, (bool, bool)> = sqlx::query_as::<_, (String, bool, bool)>(
        "SELECT rack, settled, without_consensus FROM opening_rack_progress
         WHERE job_id = $1 AND rack = ANY($2) FOR UPDATE",
    )
    .bind(job_id)
    .bind(racks)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(rack, settled, without)| (rack, (settled, without)))
    .collect();

    let mut firsts: HashMap<String, HashMap<String, i32>> = HashMap::new();
    for (rack, play, count) in sqlx::query_as::<_, (String, String, i64)>(
        "SELECT a.rack, m.move, COUNT(*)
         FROM position_analysis_records a
         JOIN position_analysis_moves m ON m.record_id = a.id AND m.rank = 1
         WHERE a.job_id = $1 AND a.game_index IS NULL AND a.rack = ANY($2)
         GROUP BY a.rack, m.move",
    )
    .bind(job_id)
    .bind(racks)
    .fetch_all(&mut *conn)
    .await?
    {
        firsts.entry(rack).or_default().insert(play, count as i32);
    }

    let mut tally = RackTally::default();
    let mut rows = StandingColumns::default();
    let unique: HashSet<&String> = racks.iter().collect();
    for rack in unique {
        let Some(now) = firsts.get(rack).and_then(|f| standing(&config, f)) else {
            return Err(AppError::internal(format!("rack {rack} has no stored analysis")));
        };
        let (was_settled, was_without) = match before.get(rack) {
            Some(&was) => was,
            None => {
                tally.analysed += 1;
                (false, false)
            }
        };
        tally.settled += i64::from(now.settled) - i64::from(was_settled);
        tally.without_consensus += i64::from(now.without_consensus) - i64::from(was_without);
        rows.racks.push(rack.clone());
        rows.results.push(now.results);
        rows.top_moves.push(now.top_move);
        rows.top_counts.push(now.top_count);
        rows.runner_up_counts.push(now.runner_up_count);
        rows.settled.push(now.settled);
        rows.without_consensus.push(now.without_consensus);
    }
    sqlx::query(
        "INSERT INTO opening_rack_progress
             (job_id, rack, results, top_move, top_count, runner_up_count, settled,
              without_consensus)
         SELECT $1, * FROM unnest($2::text[], $3::int[], $4::text[], $5::int[], $6::int[],
                                  $7::bool[], $8::bool[])
         ON CONFLICT (job_id, rack) DO UPDATE
         SET results = EXCLUDED.results, top_move = EXCLUDED.top_move,
             top_count = EXCLUDED.top_count, runner_up_count = EXCLUDED.runner_up_count,
             settled = EXCLUDED.settled, without_consensus = EXCLUDED.without_consensus",
    )
    .bind(job_id)
    .bind(rows.racks)
    .bind(rows.results)
    .bind(rows.top_moves)
    .bind(rows.top_counts)
    .bind(rows.runner_up_counts)
    .bind(rows.settled)
    .bind(rows.without_consensus)
    .execute(&mut *conn)
    .await?;
    Ok(tally)
}

/// A job's racks restated under new consensus settings, and its rack totals
/// with them: raising the most analyses or the lead that settles a rack
/// unsettles racks, and lowering them settles racks. One statement
/// over the job's progress rows -- [`standing`]'s rule on the counts they
/// keep, writing only the rows whose standing changes -- then the totals
/// counted from them. `racks_analyzed` is untouched: no rack gains or loses an
/// analysis.
///
/// In the edit's transaction, under the locks that keep every claim and
/// submission of the job out (see `update_consensus` in routes/admin.rs).
/// Returns how many racks are now unsettled.
pub async fn restate_racks(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &ConsensusSettings,
) -> AppResult<i64> {
    sqlx::query(
        "WITH s AS (
             SELECT rack,
                    (top_count - runner_up_count >= $2
                     OR ($3 <= 1 AND runner_up_count = 0)) AS agreed,
                    results >= $3 AS at_most
             FROM opening_rack_progress WHERE job_id = $1
         )
         UPDATE opening_rack_progress p
         SET settled = s.agreed OR s.at_most,
             without_consensus = s.at_most AND NOT s.agreed
         FROM s
         WHERE p.job_id = $1 AND p.rack = s.rack
           AND (p.settled, p.without_consensus)
               IS DISTINCT FROM (s.agreed OR s.at_most, s.at_most AND NOT s.agreed)",
    )
    .bind(job_id)
    .bind(config.win_margin)
    .bind(config.max_results_per_rack)
    .execute(&mut *conn)
    .await?;
    let (settled, without_consensus, unsettled): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE settled), COUNT(*) FILTER (WHERE without_consensus),
                COUNT(*) FILTER (WHERE NOT settled)
         FROM opening_rack_progress WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query("UPDATE jobs SET racks_settled = $2, racks_without_consensus = $3 WHERE id = $1")
        .bind(job_id)
        .bind(settled)
        .bind(without_consensus)
        .execute(&mut *conn)
        .await?;
    Ok(unsettled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis(rack: &str, moves: usize, num_moves: Option<i32>) -> serde_json::Value {
        serde_json::json!({
            "rack": rack,
            "num_moves": num_moves,
            "moves": (0..moves).map(|i| serde_json::json!({
                "move": format!("8D PLAY{i}"), "score": 30, "equity": 32.5,
            })).collect::<Vec<_>>(),
        })
    }

    fn process(racks: Vec<serde_json::Value>) -> AppResult<PositionAnalysisRecord> {
        let response: PositionAnalysisResponse =
            serde_json::from_value(serde_json::json!({ "racks": racks })).unwrap();
        OpeningRackHandler::process_response(response)
    }

    /// The stored moves are truncated, so `num_moves` is the only record of how
    /// many the worker actually ranked. MAGPIE now caps the reported list at
    /// the job's `num_plays_recorded` and states the full count alongside it.
    #[test]
    fn a_ranked_count_larger_than_the_reported_list_is_kept() {
        let record = process(vec![analysis("AEINRST", 3, Some(412))]).unwrap();
        assert_eq!(record.positions[0].num_moves, 412);
        assert_eq!(record.positions[0].moves.len(), 3);
    }

    /// Every MAGPIE the version floor admits states the count, so an analysis
    /// without one is not a response to read a count into.
    #[test]
    fn an_analysis_without_its_ranked_count_is_malformed() {
        let mut rack = analysis("AEINRST", 4, None);
        rack.as_object_mut().unwrap().remove("num_moves");
        let response = serde_json::json!({ "racks": [rack] });
        let error = serde_json::from_value::<PositionAnalysisResponse>(response).unwrap_err();
        assert!(error.to_string().contains("num_moves"), "{error}");
    }

    /// A worker cannot report more moves than it says it generated.
    #[test]
    fn a_ranked_count_below_the_reported_list_is_refused() {
        assert!(process(vec![analysis("AEINRST", 5, Some(2))]).is_err());
    }

    fn consensus(max: i32, margin: i32) -> ConsensusSettings {
        ConsensusSettings { max_results_per_rack: max, reissue_top_moves: 10, win_margin: margin }
    }

    fn firsts(counts: &[(&str, i32)]) -> HashMap<String, i32> {
        counts.iter().map(|(m, n)| (m.to_string(), *n)).collect()
    }

    /// A rack settles once one move leads the runner-up by the margin, the
    /// first analysis counted: 2-0 at two analyses, 3-1 at four, 4-2 at six.
    /// 1-0 and 2-1 are a lead of one.
    #[test]
    fn a_rack_settles_once_one_move_leads_by_the_margin() {
        let config = consensus(7, 2);
        for counts in [&[("8D AA", 2)][..], &[("8D AA", 3), ("8D BB", 1)], &[("8D AA", 4), ("8D BB", 2)]] {
            let now = standing(&config, &firsts(counts)).unwrap();
            assert!(now.settled && !now.without_consensus, "{counts:?}");
            assert_eq!(now.top_move, "8D AA");
        }
        let one = standing(&config, &firsts(&[("8D AA", 1)])).unwrap();
        assert_eq!((one.results, one.runner_up_count, one.settled), (1, 0, false));
        let close = standing(&config, &firsts(&[("8D AA", 2), ("8D BB", 1)])).unwrap();
        assert_eq!((close.top_count, close.runner_up_count, close.settled), (2, 1, false));
        // The runner-up is the second-highest count, however many moves split
        // the rest.
        let spread = standing(&config, &firsts(&[("8D AA", 3), ("8D BB", 1), ("8D CC", 1)])).unwrap();
        assert_eq!((spread.runner_up_count, spread.settled), (1, true));
    }

    /// A rack still within the margin at its most analyses settles without a
    /// consensus, on its most common move -- or, tied, the alphabetically
    /// first, so the same analyses always read the same.
    #[test]
    fn a_close_rack_settles_at_its_maximum_without_a_consensus() {
        let config = consensus(7, 2);
        let split = standing(&config, &firsts(&[("8D AA", 4), ("8D BB", 3)])).unwrap();
        assert_eq!(split.results, 7);
        assert!(split.settled && split.without_consensus);
        assert_eq!(split.top_move, "8D AA");
        let tie = standing(&config, &firsts(&[("8D BB", 3), ("8D AA", 3), ("8D CC", 1)])).unwrap();
        assert_eq!((tie.top_move.as_str(), tie.runner_up_count), ("8D AA", 3));
        assert!(tie.settled && tie.without_consensus);
        let short = standing(&config, &firsts(&[("8D BB", 1), ("8D AA", 1)])).unwrap();
        assert_eq!(short.top_move, "8D AA");
        assert!(!short.settled);
    }

    /// One analysis per rack settles, agreed, at the first: it seeks no
    /// agreement, whatever its margin, and nothing is reissued.
    #[test]
    fn one_analysis_per_rack_settles_at_the_first() {
        for margin in [1, 2, 5] {
            let config = consensus(1, margin);
            assert!(!config.reissues());
            let one = standing(&config, &firsts(&[("8D AA", 1)])).unwrap();
            assert!(one.settled && !one.without_consensus, "margin {margin}");
        }
        // Analysed twice before the maximum came down to one: two that agree
        // are agreed, two that split are not.
        let config = consensus(1, 5);
        assert!(!standing(&config, &firsts(&[("8D AA", 2)])).unwrap().without_consensus);
        let split = standing(&config, &firsts(&[("8D AA", 1), ("8D BB", 1)])).unwrap();
        assert!(split.settled && split.without_consensus);
    }
}
