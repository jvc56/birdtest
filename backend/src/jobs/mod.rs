pub mod game;
pub mod game_pair;
pub mod handler;
pub mod klv;
pub mod leave_gen;
pub mod opening_rack;
pub mod plausibility;
pub mod racks;
pub mod registry;

use crate::error::AppResult;
use crate::models::job::NamedPlayerConfig;
use handler::{GameRequest, GameResultsRecord, PlayerSpec, PlyStats, PositionAnalysis};
use racks::LetterDistribution;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

/// The advisory-lock namespace for dispatch decisions. Postgres advisory locks
/// are a single flat 64-bit space shared by every user of them, so the
/// two-argument form's first key is a namespace and the second identifies the
/// job.
const DISPATCH_LOCK_NAMESPACE: i32 = 1;

/// Serialize one job's dispatch decisions against each other, for the rest of
/// the caller's transaction.
///
/// Every job type needs this, for the same reason: what to hand out next is
/// decided from reads that a concurrent claim's uncommitted writes are
/// invisible to.
///
/// - **Games, game pairs and opening racks** pick the next seed with
///   `MAX(seed)`, so two overlapping claims compute the same one. The
///   `(job_id, seed)` unique index catches that, but only by failing the loser,
///   and `scheduler::claim` gives up after three attempts -- so past three-way
///   contention on one job a worker is told `204` while work exists. The lock
///   costs nothing that was not already being paid: `issue_claim` bumps
///   `jobs.claims_issued`, which takes the job's row lock until commit, so
///   claims against one job already serialize. This only moves the start of
///   that window earlier, turning a lost race into a short wait.
/// - **Leave generation** additionally decides which racks are still out and
///   whether the generation can close; see `leave_gen::next_step`.
///
/// Per job, so claims for other jobs are unaffected, and transaction-scoped, so
/// it is released on commit, on rollback, and on a dropped connection.
/// `hashtext` may collide, which costs two unrelated jobs a little
/// serialization and nothing else.
pub(crate) async fn lock_job_dispatch(conn: &mut PgConnection, job_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2::text))")
        .bind(DISPATCH_LOCK_NAMESPACE)
        .bind(job_id)
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn load_player_spec(
    conn: &mut PgConnection,
    player_config_id: Uuid,
) -> AppResult<PlayerSpec> {
    let config = sqlx::query_as::<_, NamedPlayerConfig>(&format!(
        "{} WHERE pc.id = $1",
        NamedPlayerConfig::SELECT
    ))
    .bind(player_config_id)
    .fetch_one(conn)
    .await?;
    Ok(config.into())
}

/// What a job pins that is not per-player: the rules variant, the board, and
/// the letter distribution -- the last as the actual bytes of the row, parsed.
///
/// The server enumerates rack universes and builds KLVs from that
/// distribution, so it must be the pinned bytes rather than a file on the
/// server's disk. There is no server-side copy of the data to disagree with.
pub struct JobData {
    pub variant: String,
    pub letterdist_name: String,
    pub letterdist: LetterDistribution,
    /// The pinned board layout's name, which is what the worker's request
    /// states. The bytes stay on the row: nothing server-side reads a layout.
    pub layout_name: String,
}

/// The job settings every request and every rack enumeration is built from.
/// `pub` rather than crate-private so a test can make one claim decision the
/// way the claim path makes it.
pub async fn load_job_data(conn: &mut PgConnection, job_id: Uuid) -> AppResult<JobData> {
    let row = sqlx::query(
        "SELECT j.variant, ld.name AS ld_name, ld.content AS ld_content,
                layout.name AS layout_name
         FROM jobs j
         JOIN input_data ld ON ld.id = j.letterdist_id
         JOIN input_data layout ON layout.id = j.layout_id
         WHERE j.id = $1",
    )
    .bind(job_id)
    .fetch_one(conn)
    .await?;

    // NOT NULL by the role/content equivalence check on input_data: a
    // letterdist row without bytes cannot exist, so this is a schema
    // violation rather than a case to handle.
    let content: Vec<u8> = row.get("ld_content");
    let letterdist_name: String = row.get("ld_name");
    Ok(JobData {
        variant: row.get("variant"),
        letterdist: LetterDistribution::parse(&content, &letterdist_name)?,
        letterdist_name,
        layout_name: row.get("layout_name"),
    })
}

pub(crate) async fn load_job_data_for_task(
    conn: &mut PgConnection,
    task_id: Uuid,
) -> AppResult<JobData> {
    let job_id = sqlx::query_scalar::<_, Uuid>("SELECT job_id FROM tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(&mut *conn)
        .await?;
    load_job_data(conn, job_id).await
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
             (task_id, variant, seed, num_games, player1_config_id,
              player2_config_id, capture_positions, letter_distribution,
              board_layout)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(task_id)
    .bind(&req.variant)
    .bind(req.seed as i64)
    .bind(req.num_games)
    .bind(p1)
    .bind(p2)
    .bind(req.capture_positions)
    .bind(&req.letter_distribution)
    .bind(&req.board_layout)
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
        variant: row.get("variant"),
        seed: game::seed_from_row(&row),
        num_games: row.get("num_games"),
        game_pairs,
        capture_positions: row.get("capture_positions"),
        letter_distribution: row.get("letter_distribution"),
        board_layout: row.get("board_layout"),
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
                 (task_claim_id, task_id, rack, position, game_index, turn_number,
                  previous_move, previous_move_score, num_moves)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT DO NOTHING
             RETURNING id"
        } else {
            "INSERT INTO position_analysis_records
                 (task_claim_id, task_id, rack, position, game_index, turn_number,
                  previous_move, previous_move_score, num_moves)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id"
        };

        let record_id = sqlx::query_scalar::<_, i64>(insert)
            .bind(claim_id)
            .bind(task_id)
            .bind(&position.rack)
            .bind(&position.position)
            .bind(position.game_index)
            .bind(position.turn_number)
            .bind(&position.previous_move)
            .bind(position.previous_move_score)
            .bind(position.num_moves)
            .fetch_optional(&mut *conn)
            .await?;

        // Absent means another claim already recorded this position, so its
        // moves are already there too.
        let Some(record_id) = record_id else { continue };

        // `top_moves` is the config's num_plays_recorded, at least 1 by
        // constraint; clamped anyway rather than trusting the cast.
        let kept: Vec<_> = position.moves.iter().take(top_moves.max(0) as usize).collect();
        if kept.is_empty() {
            continue;
        }

        // Chunked because Postgres caps a statement at 65,535 bind parameters,
        // and a large num_plays_recorded can keep more moves than one
        // statement can carry.
        let mut move_ids: Vec<i64> = Vec::with_capacity(kept.len());
        for (chunk_index, chunk) in kept.chunks(MOVE_ROWS_PER_STATEMENT).enumerate() {
            let rank_offset = chunk_index * MOVE_ROWS_PER_STATEMENT;
            let mut builder = sqlx::QueryBuilder::new(
                "INSERT INTO position_analysis_moves
                     (record_id, task_id, rank, move, score, equity, win_percentage,
                      blended_utility) ",
            );
            builder.push_values(chunk.iter().enumerate(), |mut b, (index, entry)| {
                b.push_bind(record_id)
                    .push_bind(task_id)
                    .push_bind((rank_offset + index + 1) as i16)
                    .push_bind(entry.play.clone())
                    .push_bind(entry.score)
                    .push_bind(entry.equity)
                    // NULL for a static player, which simulates nothing.
                    .push_bind(entry.win_percentage)
                    .push_bind(entry.blended_utility);
            });
            // Returned in insertion order, so the ids line up with `kept` and
            // the per-ply rows can be attached without looking each move up.
            builder.push(" RETURNING id");
            let ids: Vec<i64> = builder.build_query_scalar().fetch_all(&mut *conn).await?;
            move_ids.extend(ids);
        }

        // Only a simming player produces per-ply statistics; for a static
        // player this is empty and nothing is written. One statement per
        // position rather than one per ply: a simmed opening-rack batch is
        // racks x moves x plies rows, and a round trip each was the slowest
        // part of accepting it.
        let plies: Vec<(i64, &PlyStats)> = move_ids
            .iter()
            .zip(kept.iter())
            .flat_map(|(move_id, entry)| entry.plies.iter().map(move |ply| (*move_id, ply)))
            .collect();
        for chunk in plies.chunks(PLY_ROWS_PER_STATEMENT) {
            let mut builder = sqlx::QueryBuilder::new(
                "INSERT INTO position_analysis_plies
                     (move_id, ply, bingo_percentage, average_score) ",
            );
            builder.push_values(chunk.iter(), |mut b, (move_id, ply)| {
                b.push_bind(*move_id)
                    .push_bind(ply.ply)
                    .push_bind(ply.bingo_percentage)
                    .push_bind(ply.average_score);
            });
            builder.push(" ON CONFLICT (move_id, ply) DO NOTHING");
            builder.build().execute(&mut *conn).await?;
        }
    }
    Ok(())
}

/// Rows per multi-row insert, keeping each statement well under Postgres's
/// 65,535-parameter ceiling (8 and 4 binds per row respectively).
const MOVE_ROWS_PER_STATEMENT: usize = 4_000;
const PLY_ROWS_PER_STATEMENT: usize = 8_000;

pub(crate) async fn insert_game_results(
    conn: &mut PgConnection,
    task_id: Uuid,
    claim_id: Uuid,
    record: &GameResultsRecord,
) -> AppResult<()> {
    let all = &record.all_games;
    let divergent = record.divergent_games.as_ref();
    let pentanomial = record.pentanomial.as_ref();
    let bucket = |i: usize| pentanomial.map(|p| p[i] as i32);
    sqlx::query(
        "INSERT INTO game_results
             (task_claim_id, task_id, games, wins, losses, ties,
              p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd,
              pent_0, pent_1, pent_2, pent_3, pent_4,
              divergent_games, divergent_wins, divergent_losses, divergent_ties)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)",
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
    .bind(bucket(0))
    .bind(bucket(1))
    .bind(bucket(2))
    .bind(bucket(3))
    .bind(bucket(4))
    .bind(divergent.map(|d| d.games))
    .bind(divergent.map(|d| d.wins))
    .bind(divergent.map(|d| d.losses))
    .bind(divergent.map(|d| d.ties))
    .execute(&mut *conn)
    .await?;

    // Deterministic games mean redundant claims replay identical positions, so
    // the first accepted claim records them and the rest are no-ops.
    // How many ranked moves to keep: player 1's num_plays_recorded, which is
    // also the one MAGPIE reads to decide how many to report.
    let top_moves = sqlx::query_scalar::<_, i32>(
        "SELECT p1.num_plays_recorded
         FROM game_requests r
         JOIN player_configs p1 ON p1.id = r.player1_config_id
         WHERE r.task_id = $1",
    )
    .bind(task_id)
    .fetch_one(&mut *conn)
    .await?;

    insert_position_analyses(conn, task_id, claim_id, &record.positions, top_moves, true).await
}

/// One file a task needs, as the assignment states it.
///
/// `role` and `name` are what the client resolves through its own data path
/// search; `path` and `tarball_date` exist for the message it prints when the
/// file is missing or wrong.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExpectedFile {
    pub role: String,
    pub name: String,
    pub path: String,
    pub sha256: String,
    pub bytes: i64,
    pub tarball_date: String,
}

/// Every file the job's tasks will actually load, deduplicated.
///
/// This is a query, not an inference engine: the job pins its letter
/// distribution and board, and its players pin their own lexicon, leaves and
/// win% model. Two players on the same lexicon contribute one `kwg` entry, not
/// two; a static player contributes no `winpct` entry at all, because MAGPIE
/// never opens one for it.
pub async fn expected_data(
    conn: &mut PgConnection,
    job: &crate::models::job::Job,
) -> AppResult<Vec<ExpectedFile>> {
    use crate::models::job::JobType;

    let mut ids: Vec<Uuid> = vec![job.letterdist_id, job.layout_id];

    let player_ids: Vec<Uuid> = match job.job_type {
        JobType::OpeningRack => sqlx::query_scalar(
            "SELECT player_config_id FROM job_opening_rack_config WHERE job_id = $1",
        )
        .bind(job.id)
        .fetch_all(&mut *conn)
        .await?,
        JobType::Games => sqlx::query_scalar(
            "SELECT unnest(ARRAY[player1_config_id, player2_config_id])
             FROM job_game_config WHERE job_id = $1",
        )
        .bind(job.id)
        .fetch_all(&mut *conn)
        .await?,
        JobType::GamePairs => sqlx::query_scalar(
            "SELECT unnest(ARRAY[player1_config_id, player2_config_id])
             FROM job_game_pair_config WHERE job_id = $1",
        )
        .bind(job.id)
        .fetch_all(&mut *conn)
        .await?,
        JobType::LeaveGeneration => {
            // No player config: one bot, and its lexicon sits on the job
            // config. No klv either -- every generation's leaves are a
            // server-built artifact, generation 1's being a zeroed one.
            let kwg_id: Uuid =
                sqlx::query_scalar("SELECT kwg_id FROM job_leave_config WHERE job_id = $1")
                    .bind(job.id)
                    .fetch_one(&mut *conn)
                    .await?;
            ids.push(kwg_id);
            Vec::new()
        }
    };

    if !player_ids.is_empty() {
        let rows = sqlx::query(
            "SELECT kwg_id, klv_id, winpct_id FROM player_configs WHERE id = ANY($1)",
        )
        .bind(&player_ids)
        .fetch_all(&mut *conn)
        .await?;
        for row in rows {
            ids.push(row.get("kwg_id"));
            ids.push(row.get("klv_id"));
            if let Some(winpct) = row.get::<Option<Uuid>, _>("winpct_id") {
                ids.push(winpct);
            }
        }
    }

    let rows = sqlx::query(
        "SELECT role, name, path, sha256, bytes, tarball_date
         FROM input_data WHERE id = ANY($1)
         ORDER BY role, name",
    )
    .bind(&ids)
    .fetch_all(conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ExpectedFile {
            role: row.get("role"),
            name: row.get("name"),
            path: row.get("path"),
            sha256: row.get("sha256"),
            bytes: row.get("bytes"),
            tarball_date: row.get("tarball_date"),
        })
        .collect())
}
