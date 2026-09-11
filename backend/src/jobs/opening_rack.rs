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
            // `RackAnalysis` carries no generated-move count, so there is
            // nothing to check the list length against here.
            super::plausibility::check_moves(&analysis.moves, None, "opening rack")?;
            racks.push(PositionAnalysis::opening_rack(
                analysis.rack.clone(),
                analysis.moves,
            ));
        }
        Ok(PositionAnalysisRecord { positions: racks })
    }

    async fn insert_record(
        conn: &mut PgConnection,
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
        super::insert_position_analyses(conn, task_id, claim_id, &record.positions,
                                        num_plays_recorded, false)
            .await?;

        Ok(())
    }
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

