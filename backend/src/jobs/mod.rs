pub mod game;
pub mod game_pair;
pub mod handler;
pub mod leave_gen;
pub mod opening_rack;
pub mod racks;
pub mod registry;

use crate::error::AppResult;
use crate::models::job::PlayerConfig;
use handler::{GameRequest, GameResultsRecord, PlayerSpec, PositionAnalysis};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

pub(crate) async fn load_player_spec(
    conn: &mut PgConnection,
    player_config_id: Uuid,
) -> AppResult<PlayerSpec> {
    let config = sqlx::query_as::<_, PlayerConfig>("SELECT * FROM player_configs WHERE id = $1")
        .bind(player_config_id)
        .fetch_one(conn)
        .await?;
    Ok(config.into())
}

/// Player configs are immutable and uniquely named, so a request carrying the
/// flattened spec can be mapped back to its row by name.
async fn player_config_id_by_name(conn: &mut PgConnection, name: &str) -> AppResult<Uuid> {
    Ok(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM player_configs WHERE name = $1")
            .bind(name)
            .fetch_one(conn)
            .await?,
    )
}

pub(crate) async fn insert_game_request(
    conn: &mut PgConnection,
    task_id: Uuid,
    req: &GameRequest,
) -> AppResult<()> {
    let p1 = player_config_id_by_name(conn, &req.player1.name).await?;
    let p2 = player_config_id_by_name(conn, &req.player2.name).await?;
    sqlx::query(
        "INSERT INTO game_requests
             (task_id, lexicon, variant, seed, num_games, player1_config_id,
              player2_config_id, capture_positions, letter_distribution)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(task_id)
    .bind(&req.lexicon)
    .bind(&req.variant)
    .bind(req.seed as i64)
    .bind(req.num_games)
    .bind(p1)
    .bind(p2)
    .bind(req.capture_positions)
    .bind(&req.letter_distribution)
    .execute(conn)
    .await?;
    Ok(())
}

pub(crate) async fn load_game_request(
    conn: &mut PgConnection,
    task_id: Uuid,
    game_pairs: bool,
) -> AppResult<GameRequest> {
    let row = game::load_game_request_row(conn, task_id).await?;
    let player1 = load_player_spec(conn, row.get("player1_config_id")).await?;
    let player2 = load_player_spec(conn, row.get("player2_config_id")).await?;
    Ok(GameRequest {
        lexicon: row.get("lexicon"),
        variant: row.get("variant"),
        seed: game::seed_from_row(&row),
        num_games: row.get("num_games"),
        game_pairs,
        capture_positions: row.get("capture_positions"),
        letter_distribution: row.get("letter_distribution"),
        player1,
        player2,
    })
}

/// Writes analysed positions and their top-ranked moves.
///
/// Shared by opening rack jobs (one position per rack) and by games jobs with
/// capture on (one per turn). `on_conflict_ignore` is set for in-game positions:
/// games are deterministic, so redundant claims replay identical games, and the
/// first accepted claim is the one that lands.
pub(crate) async fn insert_position_analyses(
    conn: &mut PgConnection,
    task_id: Uuid,
    claim_id: Uuid,
    positions: &[PositionAnalysis],
    top_moves: i32,
    on_conflict_ignore: bool,
) -> AppResult<()> {
    for position in positions {
        let insert = if on_conflict_ignore {
            "INSERT INTO position_analysis_records
                 (task_claim_id, task_id, rack, position, game_index, turn_number, num_moves)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT DO NOTHING
             RETURNING id"
        } else {
            "INSERT INTO position_analysis_records
                 (task_claim_id, task_id, rack, position, game_index, turn_number, num_moves)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id"
        };

        let record_id = sqlx::query_scalar::<_, i64>(insert)
            .bind(claim_id)
            .bind(task_id)
            .bind(&position.rack)
            .bind(&position.position)
            .bind(position.game_index)
            .bind(position.turn_number)
            .bind(position.num_moves)
            .fetch_optional(&mut *conn)
            .await?;

        // Absent means another claim already recorded this position, so its
        // moves are already there too.
        let Some(record_id) = record_id else { continue };

        let kept: Vec<_> = position.moves.iter().take(top_moves as usize).collect();
        if kept.is_empty() {
            continue;
        }
        let mut builder = sqlx::QueryBuilder::new(
            "INSERT INTO position_analysis_moves
                 (record_id, task_id, rank, move, score, equity, win_percentage) ",
        );
        builder.push_values(kept.iter().enumerate(), |mut b, (index, entry)| {
            b.push_bind(record_id)
                .push_bind(task_id)
                .push_bind((index + 1) as i16)
                .push_bind(entry.play.clone())
                .push_bind(entry.score)
                .push_bind(entry.equity)
                // NULL for a static player, which simulates nothing.
                .push_bind(entry.win_percentage);
        });
        // Returned in insertion order, so the ids line up with `kept` and the
        // per-ply rows can be attached without looking each move back up.
        builder.push(" RETURNING id");
        let move_ids: Vec<i64> = builder
            .build_query_scalar()
            .fetch_all(&mut *conn)
            .await?;

        for (move_id, entry) in move_ids.iter().zip(kept.iter()) {
            // Only a simming player produces per-ply statistics; for a static
            // player this is empty and nothing is written.
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

pub(crate) async fn insert_game_results(
    conn: &mut PgConnection,
    task_id: Uuid,
    claim_id: Uuid,
    record: &GameResultsRecord,
) -> AppResult<()> {
    let all = &record.all_games;
    let divergent = record.divergent_games.as_ref();
    sqlx::query(
        "INSERT INTO game_results
             (task_claim_id, task_id, games, wins, losses, ties,
              p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd,
              divergent_games, divergent_wins, divergent_losses, divergent_ties)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(claim_id)
    .bind(task_id)
    .bind(all.games)
    .bind(all.wins)
    .bind(all.losses)
    .bind(all.ties)
    .bind(all.p1_score_mean)
    .bind(all.p1_score_sd)
    .bind(all.p2_score_mean)
    .bind(all.p2_score_sd)
    .bind(divergent.map(|d| d.games))
    .bind(divergent.map(|d| d.wins))
    .bind(divergent.map(|d| d.losses))
    .bind(divergent.map(|d| d.ties))
    .execute(&mut *conn)
    .await?;

    // Deterministic games mean redundant claims replay identical positions, so
    // the first accepted claim records them and the rest are no-ops.
    // How many ranked moves to keep: the player config's num_plays_recorded,
    // which is also what told the worker how many to report.
    let top_moves = sqlx::query_scalar::<_, Option<i32>>(
        "SELECT COALESCE(p1.num_plays_recorded, p2.num_plays_recorded)
         FROM game_requests r
         JOIN player_configs p1 ON p1.id = r.player1_config_id
         JOIN player_configs p2 ON p2.id = r.player2_config_id
         WHERE r.task_id = $1",
    )
    .bind(task_id)
    .fetch_one(&mut *conn)
    .await?
    .unwrap_or(i32::MAX);

    insert_position_analyses(conn, task_id, claim_id, &record.positions, top_moves, true).await
}
