//! The job type system: one request / response / record triple plus a creation
//! strategy per job type.

use crate::error::AppResult;
use crate::models::job::NamedPlayerConfig;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
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
    async fn load_request(conn: &mut PgConnection, task_id: Uuid) -> AppResult<Self::Request>;

    /// Normalize a worker submission into its stored form.
    fn process_response(response: Self::Response) -> AppResult<Self::Record>;

    /// `job_id` is passed rather than looked up: every record table carries it
    /// denormalized so a job's rows can be read without joining through
    /// `tasks`, and the caller has the job in hand already.
    async fn insert_record(
        conn: &mut PgConnection,
        job_id: Uuid,
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
///
/// Every setting that can change a result is stated. The `Option`s left are
/// the simulation settings, null for a static player (`num_plies` 0), which
/// never reads them; MAGPIE refuses a simmer, or any player, that leaves out
/// one it needs rather than supplying its own build's default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerSpec {
    pub name: String,
    pub recorder_type: String,
    pub sort_strategy: String,
    /// The lexicon and leaves this player loads. Required, not overrides:
    /// every player names its own files, and there is no job-level lexicon
    /// left to fall back to.
    pub lexicon: String,
    pub leaves: String,
    pub max_iterations: Option<i32>,
    pub num_plies: i32,
    pub num_plies_recorded: i32,
    pub num_plays: i32,
    pub num_plays_recorded: i32,
    pub stopping_pct: Option<f64>,
    pub use_inference: Option<bool>,
    pub time_limit_secs: Option<i32>,
    pub use_wordmap: bool,
    pub use_rit: bool,
    pub min_play_iterations: Option<i32>,
    pub threshold: Option<String>,
    pub sampling_rule: Option<String>,
    pub inference_margin: Option<f64>,
    pub utility_w_winpct: Option<f64>,
    pub utility_w_spread: Option<f64>,
    pub utility_spread_scale: Option<f64>,
    /// `None` for a static player, which never loads a win% model.
    pub win_pct_model: Option<String>,
    pub movegen_margin: f64,
}

impl From<NamedPlayerConfig> for PlayerSpec {
    fn from(named: NamedPlayerConfig) -> Self {
        let NamedPlayerConfig { config: c, kwg_name, klv_name, winpct_name } = named;
        Self {
            name: c.name,
            recorder_type: c.recorder_type,
            sort_strategy: c.sort_strategy,
            lexicon: kwg_name,
            leaves: klv_name,
            max_iterations: c.max_iterations,
            num_plies: c.num_plies,
            num_plies_recorded: c.num_plies_recorded,
            num_plays: c.num_plays,
            num_plays_recorded: c.num_plays_recorded,
            stopping_pct: c.stopping_pct,
            use_inference: c.use_inference,
            time_limit_secs: c.time_limit_secs,
            use_wordmap: c.use_wordmap,
            use_rit: c.use_rit,
            min_play_iterations: c.min_play_iterations,
            threshold: c.threshold,
            sampling_rule: c.sampling_rule,
            inference_margin: c.inference_margin,
            utility_w_winpct: c.utility_w_winpct,
            utility_w_spread: c.utility_w_spread,
            utility_spread_scale: c.utility_spread_scale,
            win_pct_model: winpct_name,
            movegen_margin: c.movegen_margin,
        }
    }
}

// --- Requests --------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpeningRackRequest {
    /// No top-level lexicon: `player` carries the one it loads.
    pub variant: String,
    /// Stated by the job rather than inferred from the lexicon name.
    pub letter_distribution: String,
    /// The board the job pins, by name. Stated for the same reason as the
    /// distribution: the worker must play on the layout whose digest it just
    /// verified, not on whatever its own settings last loaded.
    pub board_layout: String,
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
    /// Run-wide settings from the job, stated so no worker supplies its own
    /// build's default: the bingo bonus, and the simulation cutoff.
    pub bingo_bonus: i32,
    pub sim_cutoff: f64,
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
    /// No top-level lexicon: each player carries the one it loads, and the two
    /// may differ.
    pub variant: String,
    /// Stated by the job rather than inferred from the lexicon name.
    pub letter_distribution: String,
    /// See [`OpeningRackRequest::board_layout`].
    pub board_layout: String,
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
    /// See [`OpeningRackRequest::bingo_bonus`].
    pub bingo_bonus: i32,
    pub sim_cutoff: f64,
    pub player1: PlayerSpec,
    pub player2: PlayerSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveRequest {
    pub lexicon: String,
    pub variant: String,
    /// Stated by the job rather than inferred from the lexicon name.
    pub letter_distribution: String,
    /// See [`OpeningRackRequest::board_layout`].
    pub board_layout: String,
    pub generation: i32,
    /// The seed the task's games are played from, as a decimal string like
    /// [`GameRequest::seed`]. Chosen by the server when the task is created and
    /// stored with it, so a reissued task replays the seed it was first given.
    /// Without it the worker seeded leave generation from its own state -- the
    /// process start time, a `-seed` in its settings, or the seed of the last
    /// games task it ran -- so what a task played depended on the machine.
    #[serde(with = "seed_as_string")]
    pub seed: u64,
    pub forced_racks: Vec<String>,
    /// Combined KLV from the previous generation. Always present: generation 1
    /// reads the server-built zeroed KLV stored at generation 0, so every
    /// generation fetches its leaves the same way and the client has no
    /// first-generation branch.
    pub previous_artifact_key: String,
    /// The task plays this many games and stops. The generation's rack target
    /// is deliberately not sent: every rack a game touches counts toward the
    /// generation's totals, not just this task's forced subset, so stopping
    /// early at the forced racks' target would discard coverage the server
    /// would have folded in. The target stays server-only state.
    pub num_games: i32,
    /// Leave generation has one bot rather than a player pair, so its wordmap
    /// setting sits on the request instead of on a player spec.
    pub use_wordmap: bool,
    /// The job's bingo bonus. No cutoff: the leave-generating bot plays
    /// statically.
    pub bingo_bonus: i32,
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
    /// Mean win%+spread blend in [0, 1], sometimes used to rank moves instead
    /// of equity or raw win percentage. Absent for a static player, same as
    /// win_percentage.
    #[serde(default)]
    pub blended_utility: Option<f64>,
    #[serde(default)]
    pub plies: Vec<PlyStats>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RackAnalysis {
    pub rack: String,
    /// Ranked best-first as MAGPIE emitted them, truncated to the player
    /// config's `num_plays_recorded` -- the same number the server keeps.
    pub moves: Vec<MoveEntry>,
    /// How many moves were ranked before that truncation, which is the one
    /// thing the stored moves cannot recover. Optional because it was added
    /// after the first `birdtest-contribute` builds: absent, the reported list
    /// is all there was, which is what those builds sent.
    #[serde(default)]
    pub num_moves: Option<i32>,
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
    /// The move played on the previous turn of this game, and its score.
    /// Absent on the first turn of a game.
    #[serde(default)]
    pub previous_move: Option<String>,
    #[serde(default)]
    pub previous_move_score: Option<i32>,
    /// How many moves were ranked, before truncation to `num_plays_recorded`.
    pub num_moves: i32,
    pub moves: Vec<MoveEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameResultsResponse {
    /// Every game the task played. For game pairs that is two per pair.
    pub all_games: GameAggregate,
    /// Completed pairs bucketed by player 1's half-point score across the
    /// pair: index 0 is "lost both", 2 is "split", 4 is "won both". Required
    /// for game pairs, absent for plain games.
    ///
    /// This is the sample a paired job is evaluated on. The pair is the
    /// independent unit — the two games share a seed — and every pair counts,
    /// including the identically-played ones, which are 1-1 ties in bucket 2.
    #[serde(default)]
    pub pentanomial: Option<[i64; 5]>,
    /// The divergent subset: pairs whose two games did not play identically.
    /// Optional for game pairs, absent for plain games.
    ///
    /// A diagnostic only — it says how often two configs actually differ.
    /// Deliberately not a statistical sample: selecting the pairs that produced
    /// a result conditions on the outcome, which makes a hairline difference
    /// look enormous.
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
    /// The move played on the previous turn of this game, and its score.
    /// `None` for turn 0 of a game and for opening racks.
    pub previous_move: Option<String>,
    pub previous_move_score: Option<i32>,
    /// How many moves the worker ranked, which is generally far more than the
    /// number kept in `moves`. The only part of the analysis the stored moves
    /// cannot recover, since they are truncated.
    pub num_moves: i32,
    /// Truncated by the caller to the job's cap. The best move is simply the
    /// first of these, so it is not carried separately.
    pub moves: Vec<MoveEntry>,
}

impl PositionAnalysis {
    /// An opening rack: no board, no game, no turn, no previous move.
    ///
    /// `num_moves` is what the worker says it ranked, which is generally more
    /// than it reported. A client that does not send it reported everything it
    /// ranked, so the list's own length is the honest answer.
    pub fn opening_rack(rack: String, moves: Vec<MoveEntry>, num_moves: Option<i32>) -> Self {
        Self {
            rack,
            position: None,
            game_index: None,
            turn_number: None,
            previous_move: None,
            previous_move_score: None,
            num_moves: num_moves.unwrap_or(moves.len() as i32),
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
    /// See [`GameResultsResponse::pentanomial`]. `None` for plain games jobs.
    pub pentanomial: Option<[i64; 5]>,
    pub divergent_games: Option<GameAggregate>,
    pub positions: Vec<PositionAnalysis>,
}

#[derive(Debug, Clone)]
pub struct LeaveRecord {
    pub racks: Vec<RackOccurrence>,
}
