//! The job type system: one request / response / record triple plus a creation
//! strategy per job type.

use crate::error::AppResult;
use crate::models::job::PlayerConfig;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationStrategy {
    /// All tasks are written at job creation; workers claim from the pool.
    PrePopulated,
    /// Tasks are generated, inserted and claimed atomically at claim time.
    OnDemand,
}

/// Every job type implements this. The trait is used through static dispatch
/// from [`crate::jobs::registry`], where the `JobType` match is exhaustive — a
/// new variant will not compile until all four components exist.
#[allow(async_fn_in_trait)]
pub trait JobHandler {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type Record;

    fn creation_strategy() -> CreationStrategy;

    /// Persist the typed request row alongside the `tasks` row.
    async fn insert_request(conn: &mut PgConnection, task_id: Uuid, req: &Self::Request)
        -> AppResult<()>;

    /// Read back a stored request (pre-populated jobs claim tasks written earlier).
    async fn load_request(conn: &mut PgConnection, task_id: Uuid) -> AppResult<Self::Request>;

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
    pub top_plays: Option<i32>,
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
            top_plays: c.top_plays,
            stopping_pct: c.stopping_pct,
            use_inference: c.use_inference,
            time_limit_secs: c.time_limit_secs,
        }
    }
}

// --- Requests --------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRequest {
    pub lexicon: String,
    pub variant: String,
    /// CGP-encoded board + rack.
    pub position: String,
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
    /// uint64 at the application layer; stored as a signed BIGINT.
    #[serde(with = "seed_as_string")]
    pub seed: u64,
    pub num_games: i32,
    /// True for `game_pairs`: MAGPIE runs both orderings from the same seed.
    pub game_pairs: bool,
    pub player1: PlayerSpec,
    pub player2: PlayerSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveRequest {
    pub lexicon: String,
    pub variant: String,
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
    OpeningRackAnalysis(PositionRequest),
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
    #[serde(default)]
    pub plies: Vec<PlyStats>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PositionAnalysisResponse {
    /// Ranked best-first as MAGPIE emitted them.
    pub moves: Vec<MoveEntry>,
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
pub struct PositionAnalysisRecord {
    pub best_move: String,
    pub best_score: i32,
    pub best_equity: f64,
    pub num_moves: i32,
    pub moves: Vec<MoveEntry>,
}

#[derive(Debug, Clone)]
pub struct GameResultsRecord {
    pub all_games: GameAggregate,
    pub divergent_games: Option<GameAggregate>,
}

#[derive(Debug, Clone)]
pub struct LeaveRecord {
    pub racks: Vec<RackOccurrence>,
}
