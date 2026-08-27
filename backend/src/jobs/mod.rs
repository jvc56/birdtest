pub mod game;
pub mod game_pair;
pub mod handler;
pub mod leave_gen;
pub mod opening_rack;
pub mod racks;
pub mod registry;

use crate::error::AppResult;
use crate::models::job::PlayerConfig;
use handler::{GameRequest, GameResultsRecord, PlayerSpec};
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
             (task_id, lexicon, variant, seed, num_games, player1_config_id, player2_config_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(task_id)
    .bind(&req.lexicon)
    .bind(&req.variant)
    .bind(req.seed as i64)
    .bind(req.num_games)
    .bind(p1)
    .bind(p2)
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
        player1,
        player2,
    })
}

pub(crate) async fn insert_game_records(
    conn: &mut PgConnection,
    task_id: Uuid,
    claim_id: Uuid,
    record: &GameResultsRecord,
) -> AppResult<()> {
    let mut builder = sqlx::QueryBuilder::new(
        "INSERT INTO game_records
             (task_claim_id, game_index, task_id, score1, score2, winner, num_turns) ",
    );
    builder.push_values(record.games.iter().enumerate(), |mut b, (index, game)| {
        b.push_bind(claim_id)
            .push_bind(index as i16)
            .push_bind(task_id)
            .push_bind(game.score1)
            .push_bind(game.score2)
            .push_bind(game.winner)
            .push_bind(game.num_turns);
    });
    builder.build().execute(conn).await?;
    Ok(())
}
