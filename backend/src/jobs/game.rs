use super::handler::*;
use crate::error::{AppError, AppResult};
use crate::models::job::GameConfig;
use sqlx::{PgConnection, Row};
use std::path::Path;
use uuid::Uuid;

pub struct GameHandler;

/// Counts a worker reports must be internally consistent before they are
/// stored; the DB has the same constraint, but a 400 is a better answer than a
/// constraint violation.
/// Validates captured positions against the batch that produced them and
/// converts them for storage.
///
/// `games_in_batch` bounds `game_index`: a submission is otherwise unbounded
/// input written straight into the largest table in the schema, and the batch
/// size is the only thing that legitimately constrains it.
pub(super) fn validate_positions(
    positions: Vec<CapturedPosition>,
    games_in_batch: i32,
) -> AppResult<Vec<PositionAnalysis>> {
    // A game cannot plausibly run this long; the bound exists so a malformed
    // turn number cannot masquerade as a valid one.
    const MAX_TURNS_PER_GAME: i16 = 400;

    let mut out = Vec::with_capacity(positions.len());
    for position in positions {
        if position.game_index < 0 || position.game_index as i32 >= games_in_batch {
            return Err(AppError::bad_request(format!(
                "captured position names game {} but the batch has {games_in_batch}",
                position.game_index
            )));
        }
        if position.turn_number < 0 || position.turn_number > MAX_TURNS_PER_GAME {
            return Err(AppError::bad_request(format!(
                "captured position has an implausible turn number {}",
                position.turn_number
            )));
        }
        if position.moves.is_empty() {
            return Err(AppError::bad_request(
                "a captured position must carry at least one ranked move",
            ));
        }
        out.push(PositionAnalysis {
            rack: position.rack,
            position: Some(position.position),
            game_index: Some(position.game_index),
            turn_number: Some(position.turn_number),
            num_moves: position.num_moves,
            moves: position.moves,
        });
    }
    Ok(out)
}

pub(super) fn validate_aggregate(aggregate: &GameAggregate, field: &str) -> AppResult<()> {
    if !aggregate.is_consistent() {
        return Err(AppError::bad_request(format!(
            "{field}: wins + losses + ties must equal games, and all must be non-negative"
        )));
    }
    Ok(())
}

impl JobHandler for GameHandler {
    type Request = GameRequest;
    type Response = GameResultsResponse;
    type Record = GameResultsRecord;



    async fn load_request(conn: &mut PgConnection, task_id: Uuid,
                          _data_path: &Path) -> AppResult<Self::Request> {
        super::load_game_request(conn, task_id, false).await
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        validate_aggregate(&response.all_games, "all_games")?;
        if response.all_games.games == 0 {
            return Err(AppError::bad_request("a games result must contain at least one game"));
        }
        // A plain `games` job does not play pairs, so there is no divergent
        // subset to report; ignore it rather than storing something meaningless.
        let positions = validate_positions(response.positions, response.all_games.games)?;
        Ok(GameResultsRecord {
            all_games: response.all_games,
            divergent_games: None,
            positions,
        })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        super::insert_game_results(conn, task_id, claim_id, record).await
    }
}

/// On-demand task creation for a `games` job.
///
/// MAGPIE plays seeds S..S+N-1 for a batch of N starting at S, so consecutive
/// tasks are spaced `games_per_batch` apart and the seed space tiles with
/// neither gaps nor overlaps. Two workers racing here both compute the same
/// next seed; the `(job_id, seed)` unique index makes one of them lose, and the
/// loser retries.
pub async fn next_request(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &GameConfig,
) -> AppResult<(i64, GameRequest)> {
    let next_seed = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(seed) FROM tasks WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?
    .map(|max| max + config.games_per_batch as i64)
    .unwrap_or(1);

    let player1 = super::load_player_spec(conn, config.player1_config_id).await?;
    let player2 = super::load_player_spec(conn, config.player2_config_id).await?;

    Ok((
        next_seed,
        GameRequest {
            lexicon: config.lexicon.clone(),
            variant: config.variant.clone(),
            seed: next_seed as u64,
            num_games: config.games_per_batch,
            game_pairs: false,
            capture_positions: config.capture_positions,
            capture_top_moves: config.capture_top_moves,
            player1,
            player2,
        },
    ))
}

pub(super) async fn load_game_request_row(
    conn: &mut PgConnection,
    task_id: Uuid,
) -> AppResult<sqlx::postgres::PgRow> {
    Ok(sqlx::query(
        "SELECT lexicon, variant, seed, num_games, player1_config_id, player2_config_id,
                capture_positions, capture_top_moves
         FROM game_requests WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(conn)
    .await?)
}

pub(super) fn seed_from_row(row: &sqlx::postgres::PgRow) -> u64 {
    row.get::<i64, _>("seed") as u64
}
