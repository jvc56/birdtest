//! The job type system: one request / response / record triple plus a creation
//! strategy per job type.

use crate::error::AppResult;
use crate::models::job::PlayerConfig;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use std::path::Path;
use uuid::Uuid;

/// Every job type implements this. The trait is used through static dispatch
/// from [`crate::jobs::registry`], where the `JobType` match is exhaustive — a
/// new variant will not compile until all four components exist.
#[allow(async_fn_in_trait)]
pub trait JobHandler {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type Record;

    /// Read back a stored request. A task whose claim lapsed is re-dispatched
    /// through here rather than regenerated, so the request a worker sees is
    /// always the one recorded against the task.
    ///
    /// `data_path` is only meaningful to opening rack analysis, which stores a
    /// range of the rack space rather than the racks themselves and needs the
    /// letter distribution to expand it.
    async fn load_request(conn: &mut PgConnection, task_id: Uuid,
                          data_path: &Path) -> AppResult<Self::Request>;

    /// Normalize a worker submission into its stored form.
    fn process_response(response: Self::Response) -> AppResult<Self::Record>;

    async fn insert_record(
        conn: &mut PgConnection,
        task_id: Uuid,
        claim_id: Uuid,
        record: &Self::Record,
    ) -> AppResult<()>;
}

// ---------------------------------------------------------------------------
// Shared wire types
// ---------------------------------------------------------------------------

/// A player configuration flattened into the form the worker passes to MAGPIE.
/// Denormalized into every request so a worker never needs a second round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerSpec {
    pub name: String,
    pub recorder_type: String,
    pub sort_strategy: Option<String>,
    pub leaves: Option<String>,
    pub max_iterations: Option<i32>,
    pub plies: Option<i32>,
    pub num_plies_recorded: Option<i32>,
    pub num_plays: Option<i32>,
    pub num_plays_recorded: Option<i32>,
    pub stopping_pct: Option<f64>,
    pub use_inference: Option<bool>,
    pub time_limit_secs: Option<f64>,
}

impl From<PlayerConfig> for PlayerSpec {
    fn from(c: PlayerConfig) -> Self {
        Self {
            name: c.name,
            recorder_type: c.recorder_type,
            sort_strategy: c.sort_strategy,
            leaves: c.leaves,
            max_iterations: c.max_iterations,
            plies: c.plies,
            num_plies_recorded: c.num_plies_recorded,
            num_plays: c.num_plays,
            num_plays_recorded: c.num_plays_recorded,
            stopping_pct: c.stopping_pct,
            use_inference: c.use_inference,
            time_limit_secs: c.time_limit_secs,
        }
    }
}

// --- Requests --------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpeningRackRequest {
    pub lexicon: String,
    pub variant: String,
    /// Stated by the job rather than inferred from the lexicon name.
    pub letter_distribution: String,
    /// A batch of racks. An opening rack is by definition the start of the
    /// game, so only the letters cross the wire -- MAGPIE assumes the empty
    /// starting board.
    ///
    /// Batching matters here more than anywhere else: the rack space runs to
    /// millions, and one rack per task would spend a claim/submit round trip
    /// on each, which the per-worker rate limit alone caps at well under a
    /// rack per second.
    pub racks: Vec<String>,
    pub previous_play: Option<String>,
    pub player: PlayerSpec,
}

/// `seed` crosses the wire as a decimal string, not a JSON number.
///
/// It is a full `uint64`, and JSON numbers are doubles — any client parsing
/// with a conventional JSON library would silently lose precision above 2^53.
/// A string costs nothing and the client parses it where it needs an integer.
mod seed_as_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(seed: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&seed.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRequest {
    pub lexicon: String,
    pub variant: String,
    /// Stated by the job rather than inferred from the lexicon name.
    pub letter_distribution: String,
    /// uint64 at the application layer; stored as a signed BIGINT.
    #[serde(with = "seed_as_string")]
    pub seed: u64,
    pub num_games: i32,
    /// True for `game_pairs`: MAGPIE runs both orderings from the same seed.
    pub game_pairs: bool,
    /// Whether to keep the position analyses produced while playing. The worker
    /// analyses a position every turn regardless; this decides whether it
    /// reports them. How many ranked moves come back per position is the
    /// player config's `num_plays_recorded`.
    pub capture_positions: bool,
    pub player1: PlayerSpec,
    pub player2: PlayerSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveRequest {
    pub lexicon: String,
    pub variant: String,
    /// Stated by the job rather than inferred from the lexicon name.
    pub letter_distribution: String,
    pub generation: i32,
    pub forced_racks: Vec<String>,
    /// Combined KLV from the previous generation; NULL for generation 1, where
    /// the worker falls back to the lexicon's default leaves.
    pub previous_artifact_key: Option<String>,
    pub num_games: i32,
}

/// What actually goes over the wire to the worker. Internally tagged so the
/// client can dispatch on `task_request["job_type"]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "job_type", rename_all = "snake_case")]
pub enum TaskRequest {
    OpeningRack(OpeningRackRequest),
    Games(GameRequest),
    GamePairs(GameRequest),
    LeaveGeneration(LeaveRequest),
}

// --- Responses -------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PlyStats {
    pub ply: i16,
    pub bingo_percentage: f64,
    pub average_score: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MoveEntry {
    #[serde(rename = "move")]
    pub play: String,
    pub score: i32,
    pub equity: f64,
    /// The simulated win percentage. Absent for a static player, which ranks on
    /// equity alone and simulates nothing.
    #[serde(default)]
    pub win_percentage: Option<f64>,
    #[serde(default)]
    pub plies: Vec<PlyStats>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RackAnalysis {
    pub rack: String,
    /// Ranked best-first as MAGPIE emitted them. The server keeps only the
    /// leading `num_plays_recorded`.
    pub moves: Vec<MoveEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PositionAnalysisResponse {
    /// One entry per rack in the request.
    pub racks: Vec<RackAnalysis>,
}

/// One `autoplay` summary line.
///
/// MAGPIE reports a batch of games as counts and score moments, not as
/// individual games — this is that report, with player 1 as the reference:
///
/// ```text
/// autoplay games <total> <p1_wins> <p1_losses> <p1_ties> <p1_firsts>
///                <p1_score_mean> <p1_score_sd> <p2_score_mean> <p2_score_sd> ...
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GameAggregate {
    pub games: i32,
    pub wins: i32,
    pub losses: i32,
    pub ties: i32,
    pub p1_score_mean: f64,
    pub p1_score_sd: f64,
    pub p2_score_mean: f64,
    pub p2_score_sd: f64,
}

impl GameAggregate {
    pub(super) fn is_consistent(&self) -> bool {
        self.games >= 0
            && self.wins >= 0
            && self.losses >= 0
            && self.ties >= 0
            && self.wins + self.losses + self.ties == self.games
    }
}

/// Shared by games and game pairs.
/// One position analysed during a game, when capture is on.
#[derive(Debug, Clone, Deserialize)]
pub struct CapturedPosition {
    /// Which game of the batch, and which turn of it.
    pub game_index: i16,
    pub turn_number: i16,
    pub rack: String,
    /// CGP of the position as it stood before the move was played.
    pub position: String,
    /// How many moves were ranked, before truncation to `num_plays_recorded`.
    pub num_moves: i32,
    pub moves: Vec<MoveEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameResultsResponse {
    /// Every game the task played. For game pairs that is two per pair.
    pub all_games: GameAggregate,
    /// The divergent subset: pairs whose two games did not play identically.
    /// Required for game pairs, absent for plain games. Pairs that played
    /// identically are guaranteed ties carrying no information, so the
    /// divergent subset is where a pairs job's signal lives.
    #[serde(default)]
    pub divergent_games: Option<GameAggregate>,
    /// Empty unless the job asked for capture, which keeps every existing
    /// client valid.
    #[serde(default)]
    pub positions: Vec<CapturedPosition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RackOccurrence {
    pub rack: String,
    pub count: i64,
    pub mean: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LeaveResponse {
    /// Every rack that occurred during the batch, forced or not — racks the
    /// games happen to draw naturally count toward their target too.
    pub racks: Vec<RackOccurrence>,
}

// --- Records ---------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PositionAnalysis {
    pub rack: String,
    /// CGP of the position. `None` for an opening rack, where the board is
    /// empty by definition.
    pub position: Option<String>,
    /// In-game positions only: which game of the batch, and which turn of it.
    pub game_index: Option<i16>,
    pub turn_number: Option<i16>,
    /// How many moves the worker ranked, which is generally far more than the
    /// number kept in `moves`. The only part of the analysis the stored moves
    /// cannot recover, since they are truncated.
    pub num_moves: i32,
    /// Truncated by the caller to the job's cap. The best move is simply the
    /// first of these, so it is not carried separately.
    pub moves: Vec<MoveEntry>,
}

impl PositionAnalysis {
    /// An opening rack: no board, no game, no turn.
    pub fn opening_rack(rack: String, moves: Vec<MoveEntry>) -> Self {
        Self {
            rack,
            position: None,
            game_index: None,
            turn_number: None,
            num_moves: moves.len() as i32,
            moves,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PositionAnalysisRecord {
    pub positions: Vec<PositionAnalysis>,
}

#[derive(Debug, Clone)]
pub struct GameResultsRecord {
    pub all_games: GameAggregate,
    pub divergent_games: Option<GameAggregate>,
    pub positions: Vec<PositionAnalysis>,
}

#[derive(Debug, Clone)]
pub struct LeaveRecord {
    pub racks: Vec<RackOccurrence>,
}
