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
            "SELECT variant, letter_distribution, board_layout, rack_start, rack_count, racks
             FROM opening_rack_requests WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
        let rack_start: i64 = row.get("rack_start");

        Ok(OpeningRackRequest {
            racks: task_racks(index, &row),
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
            player.num_plays_recorded,
            player.num_plies_recorded,
        )
        .await?;

        Ok(())
    }
}

/// Checks an opening-rack submission against the racks the task dispatched.
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
/// Expanding the range costs what dispatching it cost: a handful of additions
/// per rack, against a batch capped at 10,000. One read -- the task's range --
/// inside the task's row lock, where each round trip is time every other
/// submission for the task waits.
pub async fn check_batch_against_task(
    conn: &mut PgConnection,
    template: &JobTemplate,
    task_id: Uuid,
    reported: &[String],
) -> AppResult<()> {
    let JobKind::OpeningRack { index, .. } = &template.kind else {
        return Err(template.mismatch("opening_rack"));
    };
    let row = sqlx::query(
        "SELECT rack_start, rack_count, racks FROM opening_rack_requests WHERE task_id = $1",
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
    let dispatched: std::collections::HashSet<&str> =
        expected.iter().map(String::as_str).collect();
    if let Some(stray) = reported.iter().find(|rack| !dispatched.contains(rack.as_str())) {
        return Err(AppError::bad_request(format!(
            "result analyses rack {stray:?}, which this task did not dispatch"
        )));
    }
    // Equal sizes plus containment is equality only if nothing is listed
    // twice. A duplicate was left for the unique index on (task_claim_id, rack)
    // to refuse, which it did -- as a `409 that already exists`, after the
    // batch had been sent to the database, where every other malformed
    // submission is a `400` that says what is wrong with it.
    let mut seen = std::collections::HashSet::with_capacity(reported.len());
    if let Some(twice) = reported.iter().find(|rack| !seen.insert(rack.as_str())) {
        return Err(AppError::bad_request(format!(
            "result analyses rack {twice:?} twice, so it leaves out a rack this task dispatched"
        )));
    }
    Ok(())
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

    let racks = if next_start < config.total_racks {
        // The final batch of a job comes up short: the range runs off the end
        // of the space and yields only what exists.
        index.racks_in_range(next_start as u64, config.racks_per_batch as u64)
    } else if ConsensusSettings::load(conn, job_id).await?.reissues() {
        next_reissue(conn, job_id, config, identity).await?
    } else {
        Vec::new()
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
        },
    )))
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
/// task reissuing racks a consensus still wants, the racks themselves.
pub async fn insert_request(
    conn: &mut PgConnection,
    task_id: Uuid,
    config: &OpeningRackConfig,
    job_data: &JobData,
    start: i64,
    racks: &[String],
) -> AppResult<()> {
    let listed = (start >= config.total_racks).then_some(racks);
    sqlx::query(
        "INSERT INTO opening_rack_requests
             (task_id, variant, letter_distribution, board_layout, rack_start,
              rack_count, racks, player_config_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(task_id)
    .bind(&job_data.variant)
    .bind(&job_data.letterdist_name)
    .bind(&job_data.layout_name)
    .bind(start)
    .bind(racks.len() as i32)
    .bind(listed)
    .bind(config.player_config_id)
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
/// common rank-1 move (the alphabetically first of a tie) and how many ranked
/// it first, whether it is settled and whether without a consensus.
#[derive(Debug, Clone, PartialEq)]
pub struct RackStanding {
    pub results: i32,
    pub top_move: String,
    pub top_count: i32,
    pub settled: bool,
    pub without_consensus: bool,
}

/// A rack's standing from how many of its analyses ranked each move first.
/// [`restate_racks`] is the same rule in SQL.
pub fn standing(config: &ConsensusSettings, firsts: &HashMap<String, i32>) -> Option<RackStanding> {
    let results: i32 = firsts.values().sum();
    let (top_move, top_count) = firsts
        .iter()
        .max_by(|(a_move, a), (b_move, b)| a.cmp(b).then_with(|| b_move.cmp(a_move)))?;
    let agreed = results >= config.min_results_per_rack
        && f64::from(*top_count) * 100.0 >= config.consensus_pct * f64::from(results);
    let settled = agreed || results >= config.max_results_per_rack;
    Some(RackStanding {
        results,
        top_move: top_move.clone(),
        top_count: *top_count,
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
        rows.settled.push(now.settled);
        rows.without_consensus.push(now.without_consensus);
    }
    sqlx::query(
        "INSERT INTO opening_rack_progress
             (job_id, rack, results, top_move, top_count, settled, without_consensus)
         SELECT $1, * FROM unnest($2::text[], $3::int[], $4::text[], $5::int[], $6::bool[], $7::bool[])
         ON CONFLICT (job_id, rack) DO UPDATE
         SET results = EXCLUDED.results, top_move = EXCLUDED.top_move,
             top_count = EXCLUDED.top_count, settled = EXCLUDED.settled,
             without_consensus = EXCLUDED.without_consensus",
    )
    .bind(job_id)
    .bind(rows.racks)
    .bind(rows.results)
    .bind(rows.top_moves)
    .bind(rows.top_counts)
    .bind(rows.settled)
    .bind(rows.without_consensus)
    .execute(&mut *conn)
    .await?;
    Ok(tally)
}

/// A job's racks restated under new consensus settings, and its rack totals
/// with them: raising the fewest or most analyses, or the share that must
/// agree, unsettles racks, and lowering them settles racks. One statement
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
                    results >= $2 AND top_count::float8 * 100.0 >= $3 * results::float8 AS agreed,
                    results >= $4 AS at_most
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
    .bind(config.min_results_per_rack)
    .bind(config.consensus_pct)
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

    fn consensus(pct: f64, min: i32, max: i32) -> ConsensusSettings {
        ConsensusSettings { consensus_pct: pct, min_results_per_rack: min, max_results_per_rack: max }
    }

    fn firsts(counts: &[(&str, i32)]) -> HashMap<String, i32> {
        counts.iter().map(|(m, n)| (m.to_string(), *n)).collect()
    }

    /// A rack settles once it has its fewest analyses and they agree in the
    /// share asked for -- and not before, however much they agree.
    #[test]
    fn a_rack_settles_at_its_minimum_once_its_analyses_agree() {
        let config = consensus(80.0, 3, 7);
        let two = standing(&config, &firsts(&[("8D AA", 2)])).unwrap();
        assert!(!two.settled, "two analyses are fewer than three, agreed or not");
        let three = standing(&config, &firsts(&[("8D AA", 3)])).unwrap();
        assert_eq!((three.results, three.top_count, three.settled), (3, 3, true));
        assert!(!three.without_consensus);
        // 4 of 5 is 80%: enough.
        assert!(standing(&config, &firsts(&[("8D AA", 4), ("8D BB", 1)])).unwrap().settled);
        // 3 of 4 is 75%: not.
        assert!(!standing(&config, &firsts(&[("8D AA", 3), ("8D BB", 1)])).unwrap().settled);
    }

    /// A rack whose analyses never agree enough stops at its most, settled
    /// without a consensus, on its most common move.
    #[test]
    fn a_split_rack_settles_at_its_maximum_without_a_consensus() {
        let config = consensus(80.0, 3, 7);
        let split = standing(&config, &firsts(&[("8D AA", 4), ("8D BB", 3)])).unwrap();
        assert_eq!(split.results, 7);
        assert!(split.settled && split.without_consensus);
        assert_eq!(split.top_move, "8D AA");
    }

    /// A tie names the alphabetically first move, so the same analyses always
    /// read the same.
    #[test]
    fn a_tie_names_the_first_move_alphabetically() {
        let config = consensus(80.0, 2, 4);
        let tie = standing(&config, &firsts(&[("8D BB", 1), ("8D AA", 1)])).unwrap();
        assert_eq!(tie.top_move, "8D AA");
        assert!(!tie.settled);
    }

    /// One analysis per rack settles at the first.
    #[test]
    fn one_analysis_per_rack_settles_at_the_first() {
        let config = consensus(100.0, 1, 1);
        assert!(!config.reissues());
        let one = standing(&config, &firsts(&[("8D AA", 1)])).unwrap();
        assert!(one.settled && !one.without_consensus);
    }
}
