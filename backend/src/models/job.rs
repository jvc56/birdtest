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
    /// What the admin called it; empty for a job created without one.
    pub name: String,
    pub job_type: JobType,
    /// The job's share of the fleet: above 0% exactly when the job is active
    /// (`jobs_allocation_is_status`), so 0% is what `inactive` means and a
    /// completed job holds 0%. There is no priority.
    pub allocation: i32,
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
    /// the jobs being served on activation and on an allocation change
    /// (`scheduler::join_at_parity`; a purge zeroes it with the job inactive);
    /// lifted to parity when a claim passes
    /// the job over for want of a task (`scheduler::lift_passed_over`), on its
    /// first claim after a heartbeat timeout unserved, and on each claim within
    /// `scheduler::JOIN_SETTLE` of joining.
    pub claims_baseline: i64,
    /// When the job last issued a claim. What `scheduler::join_at_parity` reads
    /// to tell a job being served from one that is only on offer, and the
    /// claim path to tell a job returning from a spell unserved.
    pub last_claimed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The match test's verdict the job was completed on, if the finish check
    /// completed it: `player1_better`, `player2_better` or `inconclusive`,
    /// player 1's score interval then, and the units it had. All four or none
    /// -- none for a job that runs no test, which has no verdict to keep.
    pub test_decided_status: Option<String>,
    pub test_decided_lower: Option<f64>,
    pub test_decided_upper: Option<f64>,
    pub test_decided_units: Option<i64>,
    /// Games recorded by the job's accepted results (one per task); the dashboard's
    /// progress numerator, maintained in the submit transaction rather than
    /// summed on read. A pairs job's unit count is half of it.
    pub games_completed: i64,
    /// Distinct opening racks with an accepted analysis, on the same terms.
    pub racks_analyzed: i64,
    /// Opening racks that need no more analysis, and those of them settled at
    /// their most analyses without a consensus.
    pub racks_settled: i64,
    pub racks_without_consensus: i64,
    pub created_at: DateTime<Utc>,
    /// When the job last joined the jobs on offer: its activation, or its
    /// first claim after a spell unserved. The scheduler settles a job for
    /// an hour from it (`scheduler::JOIN_SETTLE`); the ETA's rate is measured
    /// from it.
    pub activated_at: Option<DateTime<Utc>>,
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
    /// Whether the player loads the word info table for its lexicon: a
    /// per-substring letter mask move generation prunes with. The server
    /// builds it and pins its hash, as for the other two derived files.
    pub use_wit: bool,
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
    /// Endgame and pre-endgame solving (see the migration's comment).
    /// `endgame_plies` 0 solves nothing and turns PEG off; the PEG schedule is
    /// `None` unless `peg_max_bag` is above 0, and the nested knobs unless
    /// `peg_nested` is set.
    pub endgame_plies: i32,
    pub peg_max_bag: i32,
    pub peg_stage_top_k: Option<Vec<i32>>,
    pub peg_scenario_stride: Option<i32>,
    pub peg_opp_model: Option<String>,
    pub peg_nested: Option<bool>,
    pub peg_nested_cand_caps: Option<Vec<i32>>,
    pub peg_nested_max_depth: Option<i32>,
    pub peg_nested_strides: Option<Vec<i32>>,
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
    /// A rack is analysed until at least `min_results_per_rack` analyses
    /// agree on its best move in this share (percent), or until it has
    /// `max_results_per_rack` of them. One and one is one analysis per rack.
    pub consensus_pct: f64,
    pub min_results_per_rack: i32,
    pub max_results_per_rack: i32,
}

impl OpeningRackConfig {
    /// The settings that decide when a rack is settled, as the job was created.
    /// An admin may change them since (`PATCH /api/admin/jobs/:id/consensus`),
    /// so anything that acts on them reads [`ConsensusSettings::load`] instead.
    pub fn consensus(&self) -> ConsensusSettings {
        ConsensusSettings {
            consensus_pct: self.consensus_pct,
            min_results_per_rack: self.min_results_per_rack,
            max_results_per_rack: self.max_results_per_rack,
        }
    }
}

/// An opening-rack job's consensus settings: a rack is analysed until at
/// least `min_results_per_rack` analyses agree on its best move in
/// `consensus_pct` percent of them, or until it has `max_results_per_rack`.
///
/// The one part of a job's configuration that changes after creation, so it
/// is never taken from the cached [`crate::jobs::dispatch::JobTemplate`]: a
/// claim reads it under the job's dispatch lock and a submission under its
/// claim's lock, both of which the edit holds while it writes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, sqlx::FromRow)]
pub struct ConsensusSettings {
    pub consensus_pct: f64,
    pub min_results_per_rack: i32,
    pub max_results_per_rack: i32,
}

impl ConsensusSettings {
    /// Whether a rack may be analysed more than once: whether racks are
    /// reissued once the rack space is covered.
    pub fn reissues(&self) -> bool {
        self.max_results_per_rack > 1
    }

    /// The job's settings as they stand now.
    pub async fn load(conn: &mut sqlx::PgConnection, job_id: Uuid) -> crate::error::AppResult<Self> {
        Ok(sqlx::query_as::<_, Self>(
            "SELECT consensus_pct, min_results_per_rack, max_results_per_rack
             FROM job_opening_rack_config WHERE job_id = $1",
        )
        .bind(job_id)
        .fetch_one(conn)
        .await?)
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GameConfig {
    pub job_id: Uuid,
    pub player1_config_id: Uuid,
    pub player2_config_id: Uuid,
    pub games_per_batch: i32,
    /// Whether the job runs the match test. Off, it plays `max_games` and
    /// stops, and `min_games` and `confidence_pct` are stored defaults nothing
    /// reads.
    pub test_enabled: bool,
    pub min_games: i32,
    pub max_games: i32,
    pub confidence_pct: f64,
    /// Keep the position analyses produced while playing. Off by default: at
    /// ~22.5 turns a game it roughly doubles the rows a job produces.
    pub capture_positions: bool,
    /// How MAGPIE spends its threads: `igp` or `pgp` (see
    /// [`crate::jobs::handler::GameRequest::threading_mode`]).
    pub threading_mode: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GamePairConfig {
    pub job_id: Uuid,
    pub player1_config_id: Uuid,
    pub player2_config_id: Uuid,
    pub pairs_per_batch: i32,
    /// Whether the job runs the match test. Off, it plays `max_pairs` and
    /// stops, and `min_pairs` and `confidence_pct` are stored defaults nothing
    /// reads.
    pub test_enabled: bool,
    pub min_pairs: i32,
    pub max_pairs: i32,
    pub confidence_pct: f64,
    /// Keep the position analyses produced while playing. Off by default: at
    /// ~22.5 turns a game it roughly doubles the rows a job produces.
    pub capture_positions: bool,
    /// With `capture_positions`, keep only each pair's first divergence: both
    /// games' positions at the first turn they play different moves, and
    /// nothing from a pair played identically.
    pub capture_first_divergence: bool,
    /// As on [`GameConfig::threading_mode`].
    pub threading_mode: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LeaveConfig {
    pub job_id: Uuid,
    /// The player the leave-generating bot plays as, in both seats. Its lexicon
    /// and wordmap setting are the job's; its leaves are never loaded, since
    /// every generation plays the server's KLV. Job creation holds it to static
    /// equity play without a rack info table.
    pub player_config_id: Uuid,
    pub num_iterations: i32,
    /// The occurrence target every rack must reach before each generation
    /// closes, one per generation: its length is how many generations the job
    /// runs. MAGPIE's own `leavegen 100,200,500,…` takes the same list; a
    /// generation's leaves are only as good as the counts behind them, so later
    /// generations, playing better leaves, are worth sampling harder.
    pub target_rack_counts: Vec<i32>,
    pub racks_per_task: i32,
}

impl LeaveConfig {
    /// How many generations the job runs before it is complete.
    pub fn generation_count(&self) -> i32 {
        self.target_rack_counts.len() as i32
    }

    /// The occurrence target of `generation` (1-based, as generations are
    /// numbered). The schema guarantees at least one target; a generation past
    /// the last is never opened, and reads the last one's.
    pub fn target_for(&self, generation: i32) -> i64 {
        let last = self.target_rack_counts.len().saturating_sub(1);
        let index = usize::try_from(generation - 1).unwrap_or(0).min(last);
        self.target_rack_counts.get(index).copied().map_or(1, i64::from)
    }
}

/// `games` and `game_pairs` share every field of the match test; the only
/// difference is whether the unit of observation is a game or a pair.
/// Normalizing to one shape here keeps the test and dashboard code from
/// branching on job type.
#[derive(Debug, Clone)]
pub struct TestParams {
    /// Off, the job plays `max_units` and stops: nothing else here is read.
    pub enabled: bool,
    pub min_units: i32,
    pub max_units: i32,
    pub confidence_pct: f64,
}

impl From<&GameConfig> for TestParams {
    fn from(c: &GameConfig) -> Self {
        Self {
            enabled: c.test_enabled,
            min_units: c.min_games,
            max_units: c.max_games,
            confidence_pct: c.confidence_pct,
        }
    }
}

impl From<&GamePairConfig> for TestParams {
    fn from(c: &GamePairConfig) -> Self {
        Self {
            enabled: c.test_enabled,
            min_units: c.min_pairs,
            max_units: c.max_pairs,
            confidence_pct: c.confidence_pct,
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

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATION: &str = include_str!("../../migrations/0001_initial.sql");

    /// The labels of `CREATE TYPE <name> AS ENUM (...)`, in declaration order.
    fn enum_labels(name: &str) -> Vec<String> {
        let header = format!("CREATE TYPE {name} AS ENUM (");
        let start = MIGRATION.find(&header).expect("enum not declared") + header.len();
        let body = &MIGRATION[start..start + MIGRATION[start..].find(')').unwrap()];
        body.split(',').map(|label| label.trim().trim_matches('\'').to_string()).collect()
    }

    /// The text sqlx binds for a value of a Postgres enum.
    fn pg_text<T: for<'q> sqlx::Encode<'q, sqlx::Postgres>>(value: &T) -> String {
        let mut buffer = sqlx::postgres::PgArgumentBuffer::default();
        let _ = value.encode_by_ref(&mut buffer).unwrap();
        String::from_utf8(buffer.to_vec()).unwrap()
    }

    const JOB_TYPES: [JobType; 4] =
        [JobType::OpeningRack, JobType::Games, JobType::GamePairs, JobType::LeaveGeneration];

    /// U-WIRE-6: the wire name of each job type, the text sqlx binds for it
    /// and the migration's enum label are one string, and serde reads it back.
    /// A variant renamed on one side only would pass the type checker and
    /// fail every query or every worker.
    #[test]
    fn job_types_have_one_name_on_the_wire_and_in_postgres() {
        let labels = enum_labels("job_type");
        assert_eq!(labels, ["opening_rack", "games", "game_pairs", "leave_generation"]);
        assert_eq!(labels.len(), JOB_TYPES.len(), "a variant with no label, or the reverse");

        for (job_type, label) in JOB_TYPES.iter().zip(&labels) {
            let wire = serde_json::to_value(job_type).unwrap();
            assert_eq!(wire, serde_json::Value::String(label.clone()));
            assert_eq!(pg_text(job_type), *label);
            let back: JobType = serde_json::from_value(wire).unwrap();
            assert_eq!(back, *job_type);
        }
        assert!(serde_json::from_str::<JobType>("\"GamePairs\"").is_err());
    }

    /// U-WIRE-6's sibling on the same table: job statuses, which the admin
    /// API and the scheduler both read.
    #[test]
    fn job_statuses_have_one_name_on_the_wire_and_in_postgres() {
        let labels = enum_labels("job_status");
        let statuses = [JobStatus::Active, JobStatus::Inactive, JobStatus::Completed];
        assert_eq!(labels.len(), statuses.len());
        for (status, label) in statuses.iter().zip(&labels) {
            assert_eq!(serde_json::to_value(status).unwrap(), serde_json::Value::String(label.clone()));
            assert_eq!(pg_text(status), *label);
        }
    }

    /// U-WIRE-7: the match test's settings come out of a games config and a
    /// pairs config the same way, with only the unit renamed. Every value is
    /// distinct, so a swapped field (min for max) shows.
    #[test]
    fn test_params_read_the_same_settings_from_games_and_pairs() {
        let (job_id, p1, p2) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let games = GameConfig {
            job_id,
            player1_config_id: p1,
            player2_config_id: p2,
            games_per_batch: 7,
            test_enabled: true,
            min_games: 100,
            max_games: 5000,
            confidence_pct: 97.5,
            capture_positions: false,
            threading_mode: "igp".into(),
        };
        let pairs = GamePairConfig {
            job_id,
            player1_config_id: p1,
            player2_config_id: p2,
            pairs_per_batch: 7,
            test_enabled: true,
            min_pairs: 100,
            max_pairs: 5000,
            confidence_pct: 97.5,
            capture_positions: false,
            capture_first_divergence: false,
            threading_mode: "igp".into(),
        };

        let fields = |p: TestParams| (p.enabled, p.min_units, p.max_units, p.confidence_pct);
        let expected = (true, 100, 5000, 97.5);
        assert_eq!(fields(TestParams::from(&games)), expected);
        assert_eq!(fields(TestParams::from(&pairs)), expected);
    }
}
