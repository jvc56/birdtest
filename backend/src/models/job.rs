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
    pub priority: i32,
    pub allocation: Option<i32>,
    pub redundancy: i32,
    pub status: JobStatus,
    pub created_by: Option<Uuid>,
    pub min_magpie_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
    pub deactivated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PlayerConfig {
    pub id: Uuid,
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
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OpeningRackConfig {
    pub job_id: Uuid,
    pub lexicon: String,
    pub variant: String,
    pub player_config_id: Uuid,
    pub racks_per_batch: i32,
    pub rack_size: i32,
    pub top_moves_stored: i32,
    pub total_racks: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GameConfig {
    pub job_id: Uuid,
    pub lexicon: String,
    pub variant: String,
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
    /// Ranked moves kept per captured position.
    pub capture_top_moves: i32
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GamePairConfig {
    pub job_id: Uuid,
    pub lexicon: String,
    pub variant: String,
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
    /// Ranked moves kept per captured position.
    pub capture_top_moves: i32
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LeaveConfig {
    pub job_id: Uuid,
    pub lexicon: String,
    pub variant: String,
    pub num_iterations: i32,
    pub generation_count: i32,
    pub target_rack_count: i32,
    pub racks_per_task: i32,
    pub max_leave_size: i32,
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
