use super::handler::*;
use super::racks::LetterDistribution;
use crate::error::{AppError, AppResult};
use crate::models::job::OpeningRackConfig;
use sqlx::{PgConnection, Row};
use std::path::Path;
use uuid::Uuid;

pub struct OpeningRackHandler;

impl JobHandler for OpeningRackHandler {
    type Request = PositionRequest;
    type Response = PositionAnalysisResponse;
    type Record = PositionAnalysisRecord;

    fn creation_strategy() -> CreationStrategy {
        CreationStrategy::PrePopulated
    }

    async fn insert_request(
        conn: &mut PgConnection,
        task_id: Uuid,
        req: &Self::Request,
    ) -> AppResult<()> {
        // The player config id is not carried on the wire request (the worker
        // gets the flattened spec instead), so it is resolved by name here.
        let player_config_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM player_configs WHERE name = $1",
        )
        .bind(&req.player.name)
        .fetch_one(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO position_requests
                 (task_id, lexicon, variant, rack, previous_play, player_config_id)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(task_id)
        .bind(&req.lexicon)
        .bind(&req.variant)
        .bind(&req.rack)
        .bind(&req.previous_play)
        .bind(player_config_id)
        .execute(conn)
        .await?;
        Ok(())
    }

    async fn load_request(conn: &mut PgConnection, task_id: Uuid) -> AppResult<Self::Request> {
        let row = sqlx::query(
            "SELECT r.lexicon, r.variant, r.rack, r.previous_play, r.player_config_id
             FROM position_requests r WHERE r.task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;

        let player = super::load_player_spec(conn, row.get("player_config_id")).await?;
        Ok(PositionRequest {
            lexicon: row.get("lexicon"),
            variant: row.get("variant"),
            rack: row.get("rack"),
            previous_play: row.get("previous_play"),
            player,
        })
    }

    fn process_response(response: Self::Response) -> AppResult<Self::Record> {
        let best = response
            .moves
            .first()
            .ok_or_else(|| AppError::bad_request("position analysis returned no moves"))?;
        Ok(PositionAnalysisRecord {
            best_move: best.play.clone(),
            best_score: best.score,
            best_equity: best.equity,
            num_moves: response.moves.len() as i32,
            moves: response.moves,
        })
    }

    async fn insert_record(
        conn: &mut PgConnection,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO position_analysis_records
                 (task_claim_id, task_id, best_move, best_score, best_equity, num_moves)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(claim_id)
        .bind(task_id)
        .bind(&record.best_move)
        .bind(record.best_score)
        .bind(record.best_equity)
        .bind(record.num_moves)
        .execute(&mut *conn)
        .await?;

        for (index, entry) in record.moves.iter().enumerate() {
            let move_id = sqlx::query_scalar::<_, i64>(
                "INSERT INTO position_analysis_moves
                     (task_claim_id, task_id, rank, move, score, equity)
                 VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
            )
            .bind(claim_id)
            .bind(task_id)
            .bind((index + 1) as i16)
            .bind(&entry.play)
            .bind(entry.score)
            .bind(entry.equity)
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
        Ok(())
    }
}

/// Job-creation-time enumeration: one task + one `position_requests` row per
/// distinct 7-tile rack drawable from the lexicon's bag.
pub async fn prepopulate(
    conn: &mut PgConnection,
    job_id: Uuid,
    config: &OpeningRackConfig,
    data_path: &Path,
) -> AppResult<i64> {
    let dist = LetterDistribution::load(data_path, &config.lexicon)?;
    let racks = dist.enumerate_racks(7);
    tracing::info!(job_id = %job_id, racks = racks.len(), "pre-populating opening rack tasks");

    // Inserted in chunks: one round trip per rack would take minutes for a full
    // English bag, and a single statement for all of them would be enormous.
    const CHUNK: usize = 500;
    let mut inserted = 0i64;
    for chunk in racks.chunks(CHUNK) {
        let mut task_ids = Vec::with_capacity(chunk.len());
        let mut builder = sqlx::QueryBuilder::new("INSERT INTO tasks (job_id, seed, state) ");
        builder.push_values(chunk.iter(), |mut b, _| {
            b.push_bind(job_id)
                .push_bind(Option::<i64>::None)
                .push("'available'::task_state");
        });
        builder.push(" RETURNING id");
        let rows = builder.build().fetch_all(&mut *conn).await?;
        for row in &rows {
            task_ids.push(row.get::<Uuid, _>("id"));
        }

        let mut builder = sqlx::QueryBuilder::new(
            "INSERT INTO position_requests
                 (task_id, lexicon, variant, rack, previous_play, player_config_id) ",
        );
        builder.push_values(task_ids.iter().zip(chunk.iter()), |mut b, (task_id, rack)| {
            b.push_bind(*task_id)
                .push_bind(config.lexicon.clone())
                .push_bind(config.variant.clone())
                .push_bind(rack.clone())
                // Opening racks sit on an empty board, so there is no prior move.
                .push_bind(Option::<String>::None)
                .push_bind(config.player_config_id);
        });
        builder.build().execute(&mut *conn).await?;
        inserted += chunk.len() as i64;
    }

    Ok(inserted)
}
