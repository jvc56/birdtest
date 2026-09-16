use super::dispatch::JobTemplate;
use super::handler::*;
use super::JobData;
use crate::error::{AppError, AppResult};
use crate::stats::sprt::Pentanomial;
use crate::models::job::GamePairConfig;
use sqlx::PgConnection;
use uuid::Uuid;

pub struct GamePairHandler;

impl JobHandler for GamePairHandler {
    type Request = GameRequest;
    /// Identical to `games`: a pair is two plain per-game results, one per
    /// ordering. Pair-level outcome is derived at read time, never stored.
    type Response = GameResultsResponse;
    type Record = GameResultsRecord;



    async fn load_request(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
    ) -> AppResult<Self::Request> {
        super::load_game_request(conn, template, task_id, true).await
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        super::game::validate_aggregate(&response.all_games, "all_games")?;
        super::plausibility::check_game_aggregate(&response.all_games, "all_games")?;

        // Every pair is two games, so an odd total means the worker ran
        // something other than what was asked for.
        if response.all_games.games == 0 || response.all_games.games % 2 != 0 {
            return Err(AppError::bad_request(
                "a game_pairs result must contain an even, non-zero number of games (two per pair)",
            ));
        }

        // The pentanomial is what the job is actually evaluated on, so a result
        // without it cannot be scored at all.
        let pentanomial = response
            .pentanomial
            .ok_or_else(|| AppError::bad_request("a game_pairs result must report a pentanomial"))?;
        if pentanomial.iter().any(|&count| count < 0) {
            return Err(AppError::bad_request("pentanomial counts must be non-negative"));
        }

        // The pentanomial and the game aggregate describe the same games from
        // two directions, so they have to agree on both. Checking here rather
        // than trusting the worker is what stops a miscounting client from
        // quietly biasing every rating pool its jobs feed: the numbers are
        // individually plausible and only inconsistent with each other.
        let counts = Pentanomial { counts: std::array::from_fn(|i| pentanomial[i] as u64) };
        if counts.pairs() * 2 != response.all_games.games as u64 {
            return Err(AppError::bad_request(
                "pentanomial pair count must be exactly half the games played",
            ));
        }
        let expected_half_points =
            2 * response.all_games.wins as u64 + response.all_games.ties as u64;
        if counts.half_points() != expected_half_points {
            return Err(AppError::bad_request(
                "pentanomial disagrees with the game counts about player 1's score",
            ));
        }

        // The divergent subset is a diagnostic rather than a sample, so it is
        // still validated, still stored, and no longer required: a client that
        // reports the pentanomial has already said everything the test needs.
        let divergent = response.divergent_games;
        if let Some(divergent) = divergent.as_ref() {
            super::game::validate_aggregate(divergent, "divergent_games")?;
            super::plausibility::check_game_aggregate(divergent, "divergent_games")?;
            if divergent.games % 2 != 0 || divergent.games > response.all_games.games {
                return Err(AppError::bad_request(
                    "divergent_games must be even and no larger than the total games played",
                ));
            }
        }

        let positions =
            super::game::validate_positions(response.positions, response.all_games.games)?;
        Ok(GameResultsRecord {
            all_games: response.all_games,
            pentanomial: Some(pentanomial),
            divergent_games: divergent,
            positions,
        })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        super::insert_game_results(conn, template, task_id, claim_id, record).await
    }
}

/// Same seed-tiling scheme as `games`, with `pairs_per_batch` as the stride,
/// and the players from the job's template the same way.
pub async fn next_request(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &GamePairConfig,
    job_data: &JobData,
    player1: &PlayerSpec,
    player2: &PlayerSpec,
) -> AppResult<Option<(i64, GameRequest)>> {
    let next_seed = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(seed) FROM tasks WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_one(&mut *conn)
    .await?
    .map(|max| max + config.pairs_per_batch as i64)
    .unwrap_or(1);

    // Seeds count pairs here, so the same cap applies in pairs.
    if super::game::past_the_cap(next_seed, config.max_pairs) {
        return Ok(None);
    }

    Ok(Some((
        next_seed,
        GameRequest {
            variant: job_data.variant.clone(),
            letter_distribution: job_data.letterdist_name.clone(),
            board_layout: job_data.layout_name.clone(),
            seed: next_seed as u64,
            num_games: config.pairs_per_batch,
            game_pairs: true,
            capture_positions: config.capture_positions,
            bingo_bonus: job_data.bingo_bonus,
            sim_cutoff: job_data.sim_cutoff,
            player1: player1.clone(),
            player2: player2.clone(),
        },
    )))
}
