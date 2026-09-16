use super::handler::*;
use super::racks::{LetterDistribution, RackIndex};
use super::JobData;
use crate::error::{AppError, AppResult};
use crate::models::job::OpeningRackConfig;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

pub struct OpeningRackHandler;

impl JobHandler for OpeningRackHandler {
    type Request = OpeningRackRequest;
    type Response = PositionAnalysisResponse;
    type Record = PositionAnalysisRecord;

    async fn load_request(conn: &mut PgConnection, task_id: Uuid) -> AppResult<Self::Request> {
        let row = sqlx::query(
            "SELECT r.variant, r.letter_distribution, r.board_layout, r.rack_start,
                    r.rack_count, r.previous_play,
                    r.player_config_id, c.rack_size
             FROM opening_rack_requests r
             JOIN tasks t ON t.id = r.task_id
             JOIN job_opening_rack_config c ON c.job_id = t.job_id
             WHERE r.task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;

        let job_data = super::load_job_data_for_task(&mut *conn, task_id).await?;
        let player = super::load_player_spec(conn, row.get("player_config_id")).await?;
        let rack_start: i64 = row.get("rack_start");
        let rack_count: i32 = row.get("rack_count");
        let rack_size: i32 = row.get("rack_size");

        // The racks are not stored, only the range they came from -- which is
        // what makes a job over millions of racks cheap to create. Expanding
        // is a handful of additions per rack, not a walk over the space.
        Ok(OpeningRackRequest {
            racks: RackRange { rack_size, start: rack_start, count: rack_count }
                .expand(&job_data.letterdist),
            variant: row.get("variant"),
            letter_distribution: row.get("letter_distribution"),
            board_layout: row.get("board_layout"),
            previous_play: row.get("previous_play"),
            bingo_bonus: job_data.bingo_bonus,
            sim_cutoff: job_data.sim_cutoff,
            player,
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
                analysis.num_moves,
                "opening rack",
            )?;
            racks.push(PositionAnalysis::opening_rack(
                analysis.rack.clone(),
                analysis.moves,
                analysis.num_moves,
            ));
        }
        Ok(PositionAnalysisRecord { positions: racks })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        job_id: Uuid,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        // How many ranked moves to keep per rack. Deliberately separate from
        // how many the worker generated or simulated: a simmer may rank
        // hundreds to get the order right while only the leaders are worth
        // storing for every rack in a space of millions.
        let row = sqlx::query(
            "SELECT p.num_plays_recorded
             FROM tasks t
             JOIN job_opening_rack_config c ON c.job_id = t.job_id
             JOIN player_configs p ON p.id = c.player_config_id
             WHERE t.id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
        let num_plays_recorded: i32 = row.get("num_plays_recorded");

        // An opening rack is unique per (claim, rack), so a conflict here would
        // be a duplicate within one submission rather than a redundant claim.
        super::insert_position_analyses(
            conn,
            job_id,
            task_id,
            claim_id,
            &record.positions,
            num_plays_recorded,
            false,
        )
        .await?;

        Ok(())
    }
}

/// Checks an opening-rack submission against the racks the task dispatched.
///
/// The counterpart of the batch-size rule game jobs get
/// ([`super::plausibility::check_against_task`]), and the same rule: what was
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
/// per rack, against a batch capped at 10,000.
pub async fn check_batch_against_task(
    conn: &mut PgConnection,
    task_id: Uuid,
    reported: &[String],
) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT r.rack_start, r.rack_count, c.rack_size
         FROM opening_rack_requests r
         JOIN tasks t ON t.id = r.task_id
         JOIN job_opening_rack_config c ON c.job_id = t.job_id
         WHERE r.task_id = $1",
    )
    .bind(task_id)
    .fetch_one(&mut *conn)
    .await?;

    let job_data = super::load_job_data_for_task(&mut *conn, task_id).await?;
    let expected = RackRange {
        rack_size: row.get("rack_size"),
        start: row.get("rack_start"),
        count: row.get("rack_count"),
    }
    .expand(&job_data.letterdist);

    if reported.len() != expected.len() {
        return Err(AppError::bad_request(format!(
            "result analyses {} racks but this task dispatched {}",
            reported.len(),
            expected.len()
        )));
    }
    // Order is not part of the contract, only the set. Duplicates within the
    // submission are already refused by the unique index on
    // (task_claim_id, rack), so equal sizes plus containment is equality.
    let dispatched: std::collections::HashSet<&str> =
        expected.iter().map(String::as_str).collect();
    if let Some(stray) = reported.iter().find(|rack| !dispatched.contains(rack.as_str())) {
        return Err(AppError::bad_request(format!(
            "result analyses rack {stray:?}, which this task did not dispatch"
        )));
    }
    Ok(())
}

/// A contiguous slice of the rack space, which is what a task actually is.
pub struct RackRange {
    pub rack_size: i32,
    pub start: i64,
    pub count: i32,
}

impl RackRange {
    /// Expands against the job's pinned letter distribution -- the same bytes
    /// the worker is checked against, so the racks handed out and the bag they
    /// are drawn from can never be enumerated from different alphabets.
    pub fn expand(&self, distribution: &LetterDistribution) -> Vec<String> {
        let index = RackIndex::new(distribution, self.rack_size as usize);
        index.racks_in_range(self.start as u64, self.count as u64)
    }
}

/// How many distinct racks a job over this distribution covers. Recorded at job
/// creation so the scheduler knows when the space is exhausted without
/// re-deriving it on every claim.
pub fn total_racks(distribution: &LetterDistribution, rack_size: i32) -> i64 {
    RackIndex::new(distribution, rack_size as usize).total() as i64
}

/// Claim-time task creation: the next unclaimed slice of the rack space.
///
/// Slices tile the space the same way game seeds do, so the `(job_id, seed)`
/// unique index resolves two workers racing for the same slice -- the loser
/// retries and takes the next one.
pub async fn next_request(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &OpeningRackConfig,
    job_data: &JobData,
) -> AppResult<Option<(i64, OpeningRackRequest)>> {
    let next_start = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(seed) FROM tasks WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?
    .map(|max| max + config.racks_per_batch as i64)
    .unwrap_or(0);

    if next_start >= config.total_racks {
        return Ok(None);
    }

    let range = RackRange {
        rack_size: config.rack_size,
        start: next_start,
        count: config.racks_per_batch,
    };
    let racks = range.expand(&job_data.letterdist);
    if racks.is_empty() {
        return Ok(None);
    }

    let player = super::load_player_spec(conn, config.player_config_id).await?;
    Ok(Some((
        next_start,
        OpeningRackRequest {
            variant: job_data.variant.clone(),
            letter_distribution: job_data.letterdist_name.clone(),
            board_layout: job_data.layout_name.clone(),
            racks,
            previous_play: None,
            bingo_bonus: job_data.bingo_bonus,
            sim_cutoff: job_data.sim_cutoff,
            player,
        },
    )))
}

/// Writes the typed request row for a task, storing the range rather than the
/// racks it expands to.
pub async fn insert_range(
    conn: &mut PgConnection,
    task_id: Uuid,
    config: &OpeningRackConfig,
    job_data: &JobData,
    start: i64,
    count: usize,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO opening_rack_requests
             (task_id, variant, letter_distribution, board_layout, rack_start,
              rack_count, previous_play, player_config_id)
         VALUES ($1, $2, $3, $4, $5, $6, NULL, $7)",
    )
    .bind(task_id)
    .bind(&job_data.variant)
    .bind(&job_data.letterdist_name)
    .bind(&job_data.layout_name)
    .bind(start)
    .bind(count as i32)
    .bind(config.player_config_id)
    .execute(conn)
    .await?;
    Ok(())
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

    /// Builds that predate the field reported everything they ranked, so the
    /// list's own length is the honest answer for them.
    #[test]
    fn an_absent_ranked_count_falls_back_to_the_reported_list() {
        let record = process(vec![analysis("AEINRST", 4, None)]).unwrap();
        assert_eq!(record.positions[0].num_moves, 4);
    }

    /// A worker cannot report more moves than it says it generated.
    #[test]
    fn a_ranked_count_below_the_reported_list_is_refused() {
        assert!(process(vec![analysis("AEINRST", 5, Some(2))]).is_err());
    }
}
