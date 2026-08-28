use super::handler::*;
use super::racks::{LetterDistribution, RackIndex};
use crate::error::{AppError, AppResult};
use crate::models::job::OpeningRackConfig;
use sqlx::{PgConnection, Row};
use std::path::Path;
use uuid::Uuid;

pub struct OpeningRackHandler;

impl JobHandler for OpeningRackHandler {
    type Request = OpeningRackRequest;
    type Response = PositionAnalysisResponse;
    type Record = PositionAnalysisRecord;

    async fn load_request(conn: &mut PgConnection, task_id: Uuid,
                          data_path: &Path) -> AppResult<Self::Request> {
        let row = sqlx::query(
            "SELECT r.lexicon, r.variant, r.rack_start, r.rack_count, r.previous_play,
                    r.player_config_id, c.rack_size
             FROM opening_rack_requests r
             JOIN tasks t ON t.id = r.task_id
             JOIN job_opening_rack_config c ON c.job_id = t.job_id
             WHERE r.task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;

        let player = super::load_player_spec(conn, row.get("player_config_id")).await?;
        let lexicon: String = row.get("lexicon");
        let rack_start: i64 = row.get("rack_start");
        let rack_count: i32 = row.get("rack_count");
        let rack_size: i32 = row.get("rack_size");

        // The racks are not stored, only the range they came from -- which is
        // what makes a job over millions of racks cheap to create. Expanding
        // is a handful of additions per rack, not a walk over the space.
        Ok(OpeningRackRequest {
            racks: RackRange {
                lexicon: lexicon.clone(),
                rack_size,
                start: rack_start,
                count: rack_count,
            }
            .expand(data_path)?,
            lexicon,
            variant: row.get("variant"),
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
            "SELECT c.top_moves_stored, p.max_iterations IS NOT NULL AS simming
             FROM tasks t
             JOIN job_opening_rack_config c ON c.job_id = t.job_id
             JOIN player_configs p ON p.id = c.player_config_id
             WHERE t.id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
        let top_moves_stored: i32 = row.get("top_moves_stored");
        let simming: bool = row.get("simming");

        // An opening rack is unique per (claim, rack), so a conflict here would
        // be a duplicate within one submission rather than a redundant claim.
        super::insert_position_analyses(conn, task_id, claim_id, &record.positions,
                                        top_moves_stored, false)
            .await?;

        // A static player produces no per-ply statistics, so only a simming
        // config stores them.
        if !simming {
            return Ok(());
        }
        for position in &record.positions {
            for (index, entry) in
                position.moves.iter().take(top_moves_stored as usize).enumerate()
            {
                if entry.plies.is_empty() {
                    continue;
                }
                let move_id = sqlx::query_scalar::<_, i64>(
                    "SELECT m.id FROM position_analysis_moves m
                     JOIN position_analysis_records r ON r.id = m.record_id
                     WHERE r.task_claim_id = $1 AND r.rack = $2 AND m.rank = $3",
                )
                .bind(claim_id)
                .bind(&position.rack)
                .bind((index + 1) as i16)
                .fetch_one(&mut *conn)
                .await?;

                for ply in &entry.plies {
                    sqlx::query(
                        "INSERT INTO position_analysis_plies
                             (move_id, ply, bingo_percentage, average_score)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT (move_id, ply) DO NOTHING",
                    )
                    .bind(move_id)
                    .bind(ply.ply)
                    .bind(ply.bingo_percentage)
                    .bind(ply.average_score)
                    .execute(&mut *conn)
                    .await?;
                }
            }
        }
        Ok(())
    }
}

/// A contiguous slice of the rack space, which is what a task actually is.
pub struct RackRange {
    pub lexicon: String,
    pub rack_size: i32,
    pub start: i64,
    pub count: i32,
}

impl RackRange {
    pub fn expand(&self, data_path: &Path) -> AppResult<Vec<String>> {
        let distribution = LetterDistribution::load(data_path, &self.lexicon)?;
        let index = RackIndex::new(&distribution, self.rack_size as usize);
        Ok(index.racks_in_range(self.start as u64, self.count as u64))
    }
}

/// How many distinct racks a job over this lexicon covers. Recorded at job
/// creation so the scheduler knows when the space is exhausted without
/// re-deriving it on every claim.
pub fn total_racks(
    data_path: &Path,
    lexicon: &str,
    rack_size: i32,
) -> AppResult<i64> {
    let distribution = LetterDistribution::load(data_path, lexicon)?;
    Ok(RackIndex::new(&distribution, rack_size as usize).total() as i64)
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
    data_path: &Path,
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
        lexicon: config.lexicon.clone(),
        rack_size: config.rack_size,
        start: next_start,
        count: config.racks_per_batch,
    };
    let racks = range.expand(data_path)?;
    if racks.is_empty() {
        return Ok(None);
    }

    let player = super::load_player_spec(conn, config.player_config_id).await?;
    Ok(Some((
        next_start,
        OpeningRackRequest {
            lexicon: config.lexicon.clone(),
            variant: config.variant.clone(),
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
    start: i64,
    count: usize,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO opening_rack_requests
             (task_id, lexicon, variant, rack_start, rack_count, previous_play,
              player_config_id)
         VALUES ($1, $2, $3, $4, $5, NULL, $6)",
    )
    .bind(task_id)
    .bind(&config.lexicon)
    .bind(&config.variant)
    .bind(start)
    .bind(count as i32)
    .bind(config.player_config_id)
    .execute(conn)
    .await?;
    Ok(())
}

