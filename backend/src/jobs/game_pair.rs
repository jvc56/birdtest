use super::dispatch::JobTemplate;
use super::handler::*;
use super::JobData;
use crate::error::{AppError, AppResult};
use crate::stats::outcomes::Pentanomial;
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
        // No bucket can hold more pairs than there were games. Bounding every
        // count is also what keeps the cross-checks below in range: summed as
        // u64, counts near 2^63 wrapped round to agreeing with the game tally
        // (and panicked in a debug build).
        let games = i64::from(response.all_games.games);
        if pentanomial.iter().any(|&count| count < 0 || count > games) {
            return Err(AppError::bad_request(
                "pentanomial counts must be non-negative, and none can exceed the games played",
            ));
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
        // And about the draws. A pair scoring one half-point (bucket 1) is a
        // loss and a draw, one scoring three (bucket 3) a win and a draw, and a
        // pair scoring two is a win and a loss or two draws. So the ties are
        // buckets 1 and 3 plus an even number from bucket 2, at most all of it.
        // `[0,1,0,1,0]` beside two wins, two losses and no ties has the right
        // pair count and score, and is two pairs that each needed a draw.
        let draws_in_even_pairs = response.all_games.ties as i64 - pentanomial[1] - pentanomial[3];
        if draws_in_even_pairs < 0
            || draws_in_even_pairs % 2 != 0
            || draws_in_even_pairs / 2 > pentanomial[2]
        {
            return Err(AppError::bad_request(
                "pentanomial disagrees with the game counts about the draws",
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

/// A pairs result that keeps only first divergences
/// (`capture_first_divergence`): from each pair whose games diverged, exactly
/// two positions, one per game, at one turn, on one board, with one rack to
/// play from -- before that turn the two games are the same game, so the two
/// players face the same position with the same tiles -- and from a pair
/// played identically, none. So the pairs with positions are the divergent
/// ones, and there are as many as `divergent_games` says.
///
/// Every pair is checked rather than sampled: a pair with one position, two at
/// different turns, or two different positions is a worker keeping something
/// other than what the job asked for, and is exactly what the job's page would
/// show side by side as if it were a disagreement.
pub(super) fn check_first_divergences(
    positions: &[PositionAnalysis],
    divergent_games: Option<&GameAggregate>,
) -> AppResult<()> {
    let divergent_games = divergent_games.ok_or_else(|| {
        AppError::bad_request(
            "a result that keeps first divergences must report divergent_games, which says how \
             many pairs diverged",
        )
    })?;
    // Each pair's first position seen, by pair. `validate_positions` has
    // already refused two positions for one turn of one game.
    let mut first: std::collections::HashMap<i16, &PositionAnalysis> =
        std::collections::HashMap::new();
    let mut complete = 0i64;
    for position in positions {
        let (Some(game), Some(turn)) = (position.game_index, position.turn_number) else {
            return Err(AppError::bad_request("a captured position names no game and turn"));
        };
        let pair = game / 2;
        let Some(other) = first.get(&pair) else {
            first.insert(pair, position);
            continue;
        };
        if other.game_index == Some(game) {
            return Err(AppError::bad_request(format!(
                "game {game} has two first-divergence positions; a pair keeps one turn"
            )));
        }
        if other.turn_number != Some(turn) {
            return Err(AppError::bad_request(format!(
                "pair {pair}'s first-divergence positions are at turns {} and {turn}, not one turn",
                other.turn_number.unwrap_or_default()
            )));
        }
        if board_of(&other.position) != board_of(&position.position) || other.rack != position.rack
        {
            return Err(AppError::bad_request(format!(
                "pair {pair}'s first-divergence positions are different positions: before they \
                 diverge the two games are one game, with the same board and tiles"
            )));
        }
        complete += 1;
    }
    if first.len() as i64 != complete {
        return Err(AppError::bad_request(
            "a pair kept one first-divergence position; it keeps both games' or neither",
        ));
    }
    if complete * 2 != i64::from(divergent_games.games) {
        return Err(AppError::bad_request(format!(
            "{complete} pairs kept a first divergence, and divergent_games says {} diverged",
            divergent_games.games / 2
        )));
    }
    Ok(())
}

/// The board of a CGP: its first field. The rest -- the racks, the scores --
/// sits in each game's own seat order, and the two games of a pair seat their
/// first mover differently.
fn board_of(position: &Option<String>) -> Option<&str> {
    position.as_deref().map(|cgp| cgp.split(' ').next().unwrap_or(cgp))
}

/// Same seed scheme as `games`, with `pairs_per_batch` as the stride,
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
            capture_first_divergence: config.capture_first_divergence,
            bingo_bonus: job_data.bingo_bonus,
            sim_cutoff: job_data.sim_cutoff,
            player1: player1.clone(),
            player2: player2.clone(),
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(game: i16, turn: i16, board: &str, rack: &str, scores: &str) -> PositionAnalysis {
        PositionAnalysis {
            rack: rack.to_string(),
            position: Some(format!("{board} {scores} 0 lex NWL23;")),
            game_index: Some(game),
            turn_number: Some(turn),
            previous_move: None,
            previous_move_score: None,
            played_move: Some("8D PLAY".into()),
            played_move_score: Some(10),
            num_moves: 1,
            analysis: Analysis::Static,
            moves: Vec::new(),
            inference: None,
        }
    }

    fn divergent(pairs: i32) -> GameAggregate {
        GameAggregate {
            games: pairs * 2, wins: pairs, losses: pairs, ties: 0,
            p1_score_mean: 400.0, p1_score_sd: 50.0, p2_score_mean: 400.0, p2_score_sd: 50.0,
        }
    }

    /// U-PAIRS-DIV-1: from each divergent pair both games' positions at one
    /// turn, the same board and tiles (the racks and scores after the board are
    /// in each game's seat order, and differ), and nothing from the others.
    #[test]
    fn first_divergences_are_both_games_at_one_turn_of_one_position() {
        let board = "15/15/15/15/15/15/15/7CAT5/15/15/15/15/15/15/15";
        let positions = [
            kept(0, 3, board, "AEINRST", "AEINRST/DOG 10 20"),
            kept(1, 3, board, "AEINRST", "DOG/AEINRST 20 10"),
            // Pair 1 played identically: nothing. Pair 2 diverged at turn 0.
            kept(5, 0, "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "EEIOUUV", "x"),
            kept(4, 0, "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15", "EEIOUUV", "y"),
        ];
        check_first_divergences(&positions, Some(&divergent(2))).unwrap();
        // Nothing diverged, nothing kept.
        check_first_divergences(&[], Some(&divergent(0))).unwrap();

        let refused = |positions: &[PositionAnalysis], pairs: Option<i32>| {
            check_first_divergences(positions, divergent_games(pairs).as_ref())
                .unwrap_err()
                .message
        };
        fn divergent_games(pairs: Option<i32>) -> Option<GameAggregate> {
            pairs.map(divergent)
        }
        let other_board = "15/15/15/15/15/15/15/7COT5/15/15/15/15/15/15/15";
        assert!(refused(&positions[..1], Some(1)).contains("keeps both games' or neither"));
        assert!(refused(&positions, Some(3)).contains("divergent_games says 3 diverged"));
        assert!(refused(&positions, None).contains("must report divergent_games"));
        let other_turn = [positions[0].clone(), kept(1, 4, board, "AEINRST", "s")];
        assert!(refused(&other_turn, Some(1)).contains("turns 3 and 4"));
        let other_position = [positions[0].clone(), kept(1, 3, other_board, "AEINRST", "s")];
        assert!(refused(&other_position, Some(1)).contains("different positions"));
        let other_rack = [positions[0].clone(), kept(1, 3, board, "AEINRSU", "s")];
        assert!(refused(&other_rack, Some(1)).contains("different positions"));
        let one_game_twice = [positions[0].clone(), kept(0, 4, board, "AEINRST", "s")];
        assert!(refused(&one_game_twice, Some(1)).contains("game 0 has two"));
    }

    /// A pentanomial that agrees with its tally only by overflowing. Summed as
    /// u64, `i64::MAX + 2` pairs doubled wrap round to exactly the 2 games
    /// reported, and the half-points (2 x 2) match two wins -- so a release
    /// build accepted it and stored a bucket of 2^63 pairs for the match test to read,
    /// while a debug build panicked. No bucket can exceed the games played.
    #[test]
    fn a_pentanomial_that_only_agrees_by_overflowing_is_rejected() {
        let response: GameResultsResponse = serde_json::from_value(serde_json::json!({
            "all_games": {
                "games": 2, "wins": 2, "losses": 0, "ties": 0,
                "p1_score_mean": 420.0, "p1_score_sd": 60.0,
                "p2_score_mean": 400.0, "p2_score_sd": 55.0,
            },
            "pentanomial": [i64::MAX, 0, 2, 0, 0],
        }))
        .unwrap();
        let error = GamePairHandler::process_response(response).unwrap_err();
        assert!(error.message.contains("none can exceed the games played"), "{}", error.message);
    }
}
