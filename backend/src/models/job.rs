use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "job_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    OpeningRack,
    Games,
    GamePairs,
    LeaveGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "job_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Inactive,
    Completed,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub job_type: JobType,
    /// The job's share of the fleet while active; `None` until first
    /// activated. There is no priority: 0% is what `inactive` means.
    pub allocation: Option<i32>,
    pub redundancy: i32,
    pub status: JobStatus,
    pub created_by: Option<Uuid>,
    /// Rules setting, not a file: 'classic' | 'wordsmog'.
    pub variant: String,
    /// One per job -- MAGPIE takes one `-ld` for the whole game, and two
    /// players cannot draw from different bags. Same for the board.
    pub letterdist_id: Uuid,
    pub layout_id: Uuid,
    /// Run-wide MAGPIE settings every request states, written from
    /// [`crate::magpie_defaults`] when the job is created.
    pub bingo_bonus: i32,
    pub sim_cutoff: f64,
    /// The floor as sortable parts. Semver in `TEXT` compares lexically, where
    /// `'1.10.0' < '1.9.0'`.
    pub min_magpie_major: i32,
    pub min_magpie_minor: i32,
    pub min_magpie_patch: i32,
    /// Every claim ever issued for this job; the scheduler's deficit
    /// numerator. See `scheduler::candidate_jobs`.
    pub claims_issued: i64,
    /// Where the job's share is measured from: the scheduler orders on
    /// `(claims_issued - claims_baseline) / allocation`. Reset to parity with
    /// the other jobs offering work on activation, on an allocation change and
    /// on a purge; see `scheduler::join_at_parity`.
    pub claims_baseline: i64,
    /// When the job last issued a claim. What `scheduler::join_at_parity` reads
    /// to tell a job being served from one that is only on offer.
    pub last_claimed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Games recorded by the first accepted result of each task; the dashboard's
    /// progress numerator, maintained in the submit transaction rather than
    /// summed on read. A pairs job's unit count is half of it.
    pub games_completed: i64,
    /// Distinct opening racks with an accepted analysis, on the same terms.
    pub racks_analyzed: i64,
    pub created_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
    pub deactivated_at: Option<DateTime<Utc>>,
}

impl Job {
    /// The floor as the assignment states it. Stored as three integers so it
    /// compares numerically; rendered here only for the wire.
    pub fn min_magpie_version(&self) -> crate::version::Version {
        crate::version::Version::new(
            self.min_magpie_major,
            self.min_magpie_minor,
            self.min_magpie_patch,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PlayerConfig {
    pub id: Uuid,
    pub name: String,
    pub recorder_type: String,
    pub sort_strategy: String,
    /// The files this player loads, pinned by content. `winpct_id` is `None`
    /// for a static player, which never loads a win% model at all.
    pub kwg_id: Uuid,
    pub klv_id: Uuid,
    pub winpct_id: Option<Uuid>,
    /// Set when this config was cloned onto newer data. Ratings do not carry
    /// over, so the UI shows the lineage instead.
    pub cloned_from_id: Option<Uuid>,
    /// Every setting a request states is stated here, filled from
    /// [`crate::magpie_defaults`] at creation. The `Option`s are simulation
    /// settings: `None` for a static player (`num_plies` 0), which never reads
    /// them, and `Some` for every simmer -- a CHECK holds the two apart.
    pub max_iterations: Option<i32>,
    /// Plies to simulate (0 for a static player), and how many to report back.
    pub num_plies: i32,
    pub num_plies_recorded: i32,
    /// Plays to generate and simulate, and how many of them to report back.
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
    /// A shared MAGPIE setting, not really per-player, but stored here anyway
    /// so this table is the exhaustive source of what a job asked for; a
    /// job's two player configs must agree on it (validated at creation).
    pub movegen_margin: f64,
    /// `None` once the admin who created it has been deleted.
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OpeningRackConfig {
    pub job_id: Uuid,
    pub player_config_id: Uuid,
    pub racks_per_batch: i32,
    pub rack_size: i32,
    pub total_racks: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GameConfig {
    pub job_id: Uuid,
    pub player1_config_id: Uuid,
    pub player2_config_id: Uuid,
    pub games_per_batch: i32,
    pub min_games: i32,
    pub max_games: i32,
    pub sprt_alpha: f64,
    pub sprt_beta: f64,
    pub elo_low: f64,
    pub elo_high: f64,
    /// Keep the position analyses produced while playing. Off by default: at
    /// ~22.5 turns a game it roughly doubles the rows a job produces.
    pub capture_positions: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GamePairConfig {
    pub job_id: Uuid,
    pub player1_config_id: Uuid,
    pub player2_config_id: Uuid,
    pub pairs_per_batch: i32,
    pub min_pairs: i32,
    pub max_pairs: i32,
    pub sprt_alpha: f64,
    pub sprt_beta: f64,
    pub elo_low: f64,
    pub elo_high: f64,
    /// Keep the position analyses produced while playing. Off by default: at
    /// ~22.5 turns a game it roughly doubles the rows a job produces.
    pub capture_positions: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LeaveConfig {
    pub job_id: Uuid,
    /// Leave generation has one bot and no `player_configs` row to hold its
    /// lexicon, so this is the one place a lexicon still sits on a job.
    pub kwg_id: Uuid,
    pub num_iterations: i32,
    pub generation_count: i32,
    pub target_rack_count: i32,
    pub racks_per_task: i32,
    pub use_wordmap: bool,
}

/// `games` and `game_pairs` share every SPRT-relevant field; the only difference
/// is whether the unit of observation is a game or a pair. Normalizing to one
/// shape here keeps the SPRT and dashboard code from branching on job type.
#[derive(Debug, Clone)]
pub struct SprtParams {
    pub min_units: i32,
    pub max_units: i32,
    pub alpha: f64,
    pub beta: f64,
    pub elo_low: f64,
    pub elo_high: f64,
}

impl From<&GameConfig> for SprtParams {
    fn from(c: &GameConfig) -> Self {
        Self {
            min_units: c.min_games,
            max_units: c.max_games,
            alpha: c.sprt_alpha,
            beta: c.sprt_beta,
            elo_low: c.elo_low,
            elo_high: c.elo_high,
        }
    }
}

impl From<&GamePairConfig> for SprtParams {
    fn from(c: &GamePairConfig) -> Self {
        Self {
            min_units: c.min_pairs,
            max_units: c.max_pairs,
            alpha: c.sprt_alpha,
            beta: c.sprt_beta,
            elo_low: c.elo_low,
            elo_high: c.elo_high,
        }
    }
}

/// A player config with the names of the files it pins, which is what crosses
/// the wire: MAGPIE's command-line surface takes names, and the digests that
/// pin the bytes travel separately in `expected_data`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NamedPlayerConfig {
    #[sqlx(flatten)]
    pub config: PlayerConfig,
    pub kwg_name: String,
    pub klv_name: String,
    pub winpct_name: Option<String>,
}

impl NamedPlayerConfig {
    /// The join every caller needs; `{}` is a predicate on `pc`.
    pub const SELECT: &'static str = "
        SELECT pc.*, kwg.name AS kwg_name, klv.name AS klv_name,
               wp.name AS winpct_name
        FROM player_configs pc
        JOIN input_data kwg ON kwg.id = pc.kwg_id
        JOIN input_data klv ON klv.id = pc.klv_id
        LEFT JOIN input_data wp ON wp.id = pc.winpct_id";
}
