//! The job type system: one request / response / record triple plus a creation
//! strategy per job type.

use super::dispatch::JobTemplate;
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
    /// always the one recorded against the task. The job's immutable half --
    /// its players, its letter distribution, its run-wide settings -- comes
    /// from `template`, read once per process, so only the row that differs
    /// per task is read here.
    async fn load_request(
        conn: &mut PgConnection,
        template: &JobTemplate,
        task_id: Uuid,
    ) -> AppResult<Self::Request>;

    /// Normalize a worker submission into its stored form.
    fn process_response(response: Self::Response) -> AppResult<Self::Record>;

    /// `template` carries the job id, which every record table stores
    /// denormalized so a job's rows can be read without joining through
    /// `tasks`, and the per-player settings that decide how much of a result
    /// to keep.
    async fn insert_record(
        conn: &mut PgConnection,
        template: &JobTemplate,
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
    /// The name this player's rack info table is loaded under, or `None` when
    /// it asks for none.
    ///
    /// A name rather than a boolean alone because a table stores precomputed
    /// leave values, so it belongs to the (lexicon, leaves) pair rather than to
    /// the lexicon. MAGPIE's CLI finds a table by lexicon name, which is how a
    /// player pinning NWL23 words and CSW21 leaves -- a pairing birdtest
    /// accepts on purpose -- would have loaded `NWL23.rit` and ranked every
    /// full rack on NWL23's leaves instead. This is the same name the server
    /// pinned a hash for in `expected_data.derived`.
    pub rit_name: Option<String>,
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
        let (lexicon, leaves) = (kwg_name, klv_name);
        Self {
            name: c.name,
            recorder_type: c.recorder_type,
            sort_strategy: c.sort_strategy,
            lexicon: lexicon.clone(),
            leaves: leaves.clone(),
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
            rit_name: c
                .use_rit
                .then(|| crate::derived::rack_info_table_name(&lexicon, &leaves)),
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
    /// The seed rack `i` of the batch is analysed from, as `seed + i`: the
    /// index of the first rack in the job's rack space, as a decimal string
    /// like [`GameRequest::seed`]. Every task states a seed, this one included,
    /// so a simulation's sampling depends on the task and not on the worker
    /// (the executor used to derive one from the rack's letters).
    #[serde(with = "seed_as_string")]
    pub seed: u64,
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
        // Summed in i64: in i32 three large counts wrap round to a small total
        // (a panic in a debug build), and a tally of two billion wins would
        // pass as ten games.
        self.games >= 0
            && self.wins >= 0
            && self.losses >= 0
            && self.ties >= 0
            && i64::from(self.wins) + i64::from(self.losses) + i64::from(self.ties)
                == i64::from(self.games)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// The server's own claim responses, as `contract-fixtures/` pins them.
    /// Read, never written, here: that directory has its own tests.
    const ASSIGNMENTS: [(&str, &str); 3] = [
        ("games", include_str!("../../../contract-fixtures/assignment-games.json")),
        ("opening_rack", include_str!("../../../contract-fixtures/assignment-opening-rack.json")),
        (
            "leave_generation",
            include_str!("../../../contract-fixtures/assignment-leave-generation.json"),
        ),
    ];

    fn task_request(assignment: &str) -> Value {
        let assignment: Value = serde_json::from_str(assignment).unwrap();
        assignment["task_request"].clone()
    }

    /// The games request as a `game_pairs` job states it. There is no pairs
    /// assignment fixture; the two share `GameRequest`.
    fn game_pairs_request() -> Value {
        let mut request = task_request(ASSIGNMENTS[0].1);
        request["job_type"] = json!("game_pairs");
        request["game_pairs"] = json!(true);
        request
    }

    fn seed_of(request: &TaskRequest) -> u64 {
        match request {
            TaskRequest::OpeningRack(r) => r.seed,
            TaskRequest::Games(r) | TaskRequest::GamePairs(r) => r.seed,
            TaskRequest::LeaveGeneration(r) => r.seed,
        }
    }

    fn set_seed(request: &mut TaskRequest, seed: u64) {
        match request {
            TaskRequest::OpeningRack(r) => r.seed = seed,
            TaskRequest::Games(r) | TaskRequest::GamePairs(r) => r.seed = seed,
            TaskRequest::LeaveGeneration(r) => r.seed = seed,
        }
    }

    /// U-WIRE-1: on every request type the seed crosses as a decimal string,
    /// and seeds past 2^53 -- where a double starts skipping integers -- come
    /// back exact.
    #[test]
    fn seeds_cross_the_wire_as_decimal_strings_without_loss() {
        let seeds = [0, 1, (1u64 << 53) + 1, (1u64 << 63) + 7, u64::MAX];
        for (job_type, assignment) in ASSIGNMENTS {
            let mut request: TaskRequest = serde_json::from_value(task_request(assignment)).unwrap();
            for seed in seeds {
                set_seed(&mut request, seed);
                let wire = serde_json::to_value(&request).unwrap();
                assert_eq!(wire["seed"], Value::String(seed.to_string()), "{job_type}");
                let text = serde_json::to_string(&request).unwrap();
                let back: TaskRequest = serde_json::from_str(&text).unwrap();
                assert_eq!(seed_of(&back), seed, "{job_type}");
            }
        }
        // 2^53 + 1 is the first integer a double cannot hold, which is what a
        // JSON number would have silently rounded it to.
        assert_ne!(((1u64 << 53) + 1) as f64 as u64, (1u64 << 53) + 1);

        // The leave fixture's own seed is above 2^53 as written.
        let leave: TaskRequest = serde_json::from_value(task_request(ASSIGNMENTS[2].1)).unwrap();
        assert_eq!(seed_of(&leave), 18_446_744_073_709_551_557);
    }

    /// U-WIRE-1: a JSON number is refused rather than read, deliberately: no
    /// client should be sending one, and accepting it would be accepting a
    /// value that may already have been rounded.
    #[test]
    fn a_seed_sent_as_a_json_number_is_refused() {
        let mut request = task_request(ASSIGNMENTS[0].1);
        request["seed"] = json!(12345);
        assert!(serde_json::from_value::<TaskRequest>(request.clone()).is_err());
        request["seed"] = json!("-1");
        assert!(serde_json::from_value::<TaskRequest>(request).is_err(), "a uint64");
    }

    /// U-WIRE-2: the tag is `job_type` in snake_case, one per variant, and
    /// every variant -- the pinned assignments plus `game_pairs` -- survives a
    /// round trip unchanged.
    #[test]
    fn task_requests_are_tagged_in_snake_case_and_round_trip() {
        let mut requests: Vec<(&str, Value)> =
            ASSIGNMENTS.iter().map(|(job_type, a)| (*job_type, task_request(a))).collect();
        requests.push(("game_pairs", game_pairs_request()));

        for (job_type, wire) in requests {
            let request: TaskRequest = serde_json::from_value(wire.clone()).unwrap();
            let variant_matches = match (&request, job_type) {
                (TaskRequest::OpeningRack(_), "opening_rack") => true,
                (TaskRequest::Games(r), "games") => !r.game_pairs,
                (TaskRequest::GamePairs(r), "game_pairs") => r.game_pairs,
                (TaskRequest::LeaveGeneration(_), "leave_generation") => true,
                _ => false,
            };
            assert!(variant_matches, "{job_type} decoded as {request:?}");
            let again = serde_json::to_value(&request).unwrap();
            assert_eq!(again["job_type"], json!(job_type));
            assert_eq!(again, wire, "{job_type} does not round-trip");
        }

        // The tag is the only thing that decides the variant: the same body
        // under an unknown or camel-cased tag is refused.
        for tag in ["gamePairs", "GamePairs", "game-pairs", "pairs"] {
            let mut wire = game_pairs_request();
            wire["job_type"] = json!(tag);
            assert!(serde_json::from_value::<TaskRequest>(wire).is_err(), "{tag}");
        }
    }

    fn aggregate_json() -> Value {
        json!({
            "games": 10, "wins": 5, "losses": 4, "ties": 1,
            "p1_score_mean": 420.0, "p1_score_sd": 60.0,
            "p2_score_mean": 415.0, "p2_score_sd": 58.0,
        })
    }

    /// U-WIRE-3: every `#[serde(default)]` field can be left out, all at once,
    /// on every response type that has one.
    #[test]
    fn every_defaulted_response_field_is_optional() {
        let bare_move = json!({ "move": "8D QI", "score": 22, "equity": 30.5 });

        let entry: MoveEntry = serde_json::from_value(bare_move.clone()).unwrap();
        assert_eq!((entry.win_percentage, entry.blended_utility), (None, None));
        assert!(entry.plies.is_empty());

        let analysis: RackAnalysis =
            serde_json::from_value(json!({ "rack": "AEINRST", "moves": [bare_move] })).unwrap();
        assert_eq!(analysis.num_moves, None);

        let position: CapturedPosition = serde_json::from_value(json!({
            "game_index": 0, "turn_number": 0, "rack": "AEINRST",
            "position": "15/15/15/15/15/15/15/15/15/15/15/15/15/15/15 AEINRST/ 0/0 0",
            "num_moves": 40, "moves": [bare_move],
        }))
        .unwrap();
        assert_eq!((position.previous_move, position.previous_move_score), (None, None));

        let results: GameResultsResponse =
            serde_json::from_value(json!({ "all_games": aggregate_json() })).unwrap();
        assert!(results.pentanomial.is_none());
        assert!(results.divergent_games.is_none());
        assert!(results.positions.is_empty());

        // The contrast: a field without a default is required.
        let mut no_score = bare_move.clone();
        no_score.as_object_mut().unwrap().remove("score");
        assert!(serde_json::from_value::<MoveEntry>(no_score).is_err());
        assert!(serde_json::from_value::<GameResultsResponse>(json!({})).is_err());
    }

    /// U-WIRE-4, the compatibility decision: an unknown field anywhere in a
    /// response is **ignored**, not rejected. No response type sets
    /// `deny_unknown_fields`, so a newer client (MAGPIE is released separately
    /// from the server) can add a field and still be accepted by an older
    /// server. The cost is that a misspelled optional field is silently
    /// dropped rather than refused; the required ones still fail loudly.
    #[test]
    fn unknown_response_fields_are_ignored_so_newer_clients_stay_valid() {
        let extra = |mut value: Value| {
            value["field_from_the_future"] = json!({ "any": ["shape"] });
            value
        };
        let ply = extra(json!({ "ply": 0, "bingo_percentage": 1.0, "average_score": 30.0 }));
        let entry = extra(json!({ "move": "8D QI", "score": 22, "equity": 30.5, "plies": [ply] }));
        let position = extra(json!({
            "game_index": 0, "turn_number": 3, "rack": "AEINRST", "position": "cgp",
            "num_moves": 40, "moves": [entry.clone()],
        }));

        let opening = extra(json!({
            "racks": [extra(json!({ "rack": "AEINRST", "moves": [entry], "num_moves": 9 }))],
        }));
        let decoded: PositionAnalysisResponse = serde_json::from_value(opening).unwrap();
        assert_eq!(decoded.racks[0].moves[0].plies.len(), 1);

        let games = extra(json!({
            "all_games": extra(aggregate_json()),
            "pentanomial": [0, 1, 2, 1, 1],
            "divergent_games": extra(aggregate_json()),
            "positions": [position],
        }));
        let decoded: GameResultsResponse = serde_json::from_value(games).unwrap();
        assert_eq!(decoded.positions[0].turn_number, 3);

        let leaves = extra(json!({
            "racks": [extra(json!({ "rack": "AEINRST", "count": 2, "mean": 1.5 }))],
        }));
        let decoded: LeaveResponse = serde_json::from_value(leaves).unwrap();
        assert_eq!(decoded.racks[0].count, 2);
    }

    fn tally(games: i32, wins: i32, losses: i32, ties: i32) -> GameAggregate {
        GameAggregate {
            games,
            wins,
            losses,
            ties,
            p1_score_mean: 420.0,
            p1_score_sd: 60.0,
            p2_score_mean: 415.0,
            p2_score_sd: 58.0,
        }
    }

    /// U-WIRE-5: a tally is consistent when every count is non-negative and
    /// the outcomes sum to the games -- and each way of breaking that fails.
    #[test]
    fn a_game_tally_must_be_non_negative_and_sum_to_its_games() {
        assert!(tally(10, 5, 4, 1).is_consistent());
        assert!(tally(0, 0, 0, 0).is_consistent());
        assert!(tally(3, 0, 0, 3).is_consistent(), "all ties");

        for (broken, why) in [
            (tally(10, 5, 4, 2), "outcomes exceed the games"),
            (tally(10, 5, 4, 0), "outcomes fall short of the games"),
            (tally(-1, 0, 0, -1), "negative games, sum matching"),
            (tally(10, -1, 10, 1), "negative wins, sum matching"),
            (tally(10, 10, -1, 1), "negative losses, sum matching"),
            (tally(10, 5, 6, -1), "negative ties, sum matching"),
            // i32::MAX + i32::MAX + 12 wraps to exactly 10 in 32 bits.
            (tally(10, i32::MAX, i32::MAX, 12), "outcomes that only sum by overflowing"),
            (tally(i32::MAX, i32::MAX, 1, 0), "one past i32::MAX"),
        ] {
            assert!(!broken.is_consistent(), "{why}: {broken:?}");
        }
    }
}
