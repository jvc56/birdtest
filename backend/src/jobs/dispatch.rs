//! What a job's tasks are built from, read once per job per process.
//!
//! Every claim used to re-read it inside the job's dispatch lock: the job's
//! per-type config row, its letter distribution (parsed again each time), a
//! player config per player (a three-way join each) and `expected_data` (a
//! union over six tables) -- five or six round trips per claim, every one of
//! them time no other worker could be claiming from that job, and the submit
//! path re-read some of the same rows inside the task's row lock. None of it
//! can change: a job's config rows have no update path, player configs are
//! immutable once created, and an `input_data` row cannot be deleted while a
//! job or a player config pins it (the foreign keys refuse). The only thing a
//! job's lifecycle changes is its status, allocation and counters, all of which
//! live on the `jobs` row that every claim still reads.
//!
//! So the template is read the first time a process dispatches from or accepts
//! for a job and kept for the life of the process, like the derived-file
//! answer in [`crate::derived::DerivedCache`]. `delete_job` forgets the entry;
//! a purge changes none of it, since it deletes results and tasks and leaves
//! the configuration alone.

use super::handler::PlayerSpec;
use super::racks::RackIndex;
use super::{ExpectedFile, JobData};
use crate::error::{AppError, AppResult};
use crate::models::job::{GameConfig, GamePairConfig, Job, JobType, LeaveConfig, OpeningRackConfig};
use sqlx::PgConnection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// The per-type half of a template: the config row and the players it names,
/// flattened into the shape a request carries.
pub enum JobKind {
    OpeningRack {
        config: OpeningRackConfig,
        player: PlayerSpec,
        /// The rack space at the job's rack size, so a range is expanded by a
        /// handful of additions rather than by rebuilding the table first.
        index: RackIndex,
    },
    Games {
        config: GameConfig,
        player1: PlayerSpec,
        player2: PlayerSpec,
    },
    GamePairs {
        config: GamePairConfig,
        player1: PlayerSpec,
        player2: PlayerSpec,
    },
    LeaveGeneration {
        config: LeaveConfig,
        /// The lexicon the job's bot plays with, by name, from the row it pins.
        lexicon: String,
    },
}

/// Everything immutable a claim or a submission for one job needs.
pub struct JobTemplate {
    pub job_id: Uuid,
    /// The variant, board, letter distribution and run-wide settings.
    pub data: JobData,
    /// Every file the job's tasks load, with the digest the job pins. Shared
    /// with each assignment rather than copied into it.
    pub expected: Arc<Vec<ExpectedFile>>,
    pub kind: JobKind,
}

impl JobTemplate {
    /// Reads a job's template. One round trip per row it needs, once.
    pub async fn load(conn: &mut PgConnection, job: &Job) -> AppResult<Self> {
        let data = super::load_job_data(conn, job.id).await?;
        let expected = super::expected_data(conn, job).await?;
        let kind = match job.job_type {
            JobType::OpeningRack => {
                let config = sqlx::query_as::<_, OpeningRackConfig>(
                    "SELECT * FROM job_opening_rack_config WHERE job_id = $1",
                )
                .bind(job.id)
                .fetch_one(&mut *conn)
                .await?;
                let player = super::load_player_spec(conn, config.player_config_id).await?;
                let index = RackIndex::new(&data.letterdist, config.rack_size as usize);
                JobKind::OpeningRack { config, player, index }
            }
            JobType::Games => {
                let config =
                    sqlx::query_as::<_, GameConfig>("SELECT * FROM job_game_config WHERE job_id = $1")
                        .bind(job.id)
                        .fetch_one(&mut *conn)
                        .await?;
                let player1 = super::load_player_spec(conn, config.player1_config_id).await?;
                let player2 = super::load_player_spec(conn, config.player2_config_id).await?;
                JobKind::Games { config, player1, player2 }
            }
            JobType::GamePairs => {
                let config = sqlx::query_as::<_, GamePairConfig>(
                    "SELECT * FROM job_game_pair_config WHERE job_id = $1",
                )
                .bind(job.id)
                .fetch_one(&mut *conn)
                .await?;
                let player1 = super::load_player_spec(conn, config.player1_config_id).await?;
                let player2 = super::load_player_spec(conn, config.player2_config_id).await?;
                JobKind::GamePairs { config, player1, player2 }
            }
            JobType::LeaveGeneration => {
                let config =
                    sqlx::query_as::<_, LeaveConfig>("SELECT * FROM job_leave_config WHERE job_id = $1")
                        .bind(job.id)
                        .fetch_one(&mut *conn)
                        .await?;
                let lexicon = super::leave_gen::lexicon_name(conn, config.kwg_id).await?;
                JobKind::LeaveGeneration { config, lexicon }
            }
        };
        Ok(Self { job_id: job.id, data, expected: Arc::new(expected), kind })
    }

    /// The error for a template whose kind does not match what the caller
    /// asked for -- a job row and a config row of different types, which the
    /// job-creation transaction makes impossible and a hand edit does not.
    pub fn mismatch(&self, wanted: &str) -> AppError {
        AppError::internal(format!("job {} is not a {wanted} job", self.job_id))
    }
}

/// The templates of every job this process has dispatched from or accepted a
/// result for.
#[derive(Clone, Default)]
pub struct JobTemplates(Arc<Mutex<HashMap<Uuid, Arc<JobTemplate>>>>);

impl JobTemplates {
    pub fn new() -> Self {
        Self::default()
    }

    /// The template, if this process has already read it.
    pub fn get(&self, job_id: Uuid) -> Option<Arc<JobTemplate>> {
        self.0.lock().expect("job template cache poisoned").get(&job_id).cloned()
    }

    /// The template, read through `conn` the first time. Two callers loading
    /// the same job at once read identical rows, so whichever inserts last
    /// changes nothing.
    pub async fn get_or_load(
        &self,
        conn: &mut PgConnection,
        job: &Job,
    ) -> AppResult<Arc<JobTemplate>> {
        if let Some(template) = self.get(job.id) {
            return Ok(template);
        }
        let template = Arc::new(JobTemplate::load(conn, job).await?);
        self.0
            .lock()
            .expect("job template cache poisoned")
            .insert(job.id, template.clone());
        Ok(template)
    }

    /// Drop a job's entry. Only a deleted job has one that is no longer
    /// wanted; nothing else can make a remembered template wrong.
    pub fn forget(&self, job_id: Uuid) {
        self.0.lock().expect("job template cache poisoned").remove(&job_id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().expect("job template cache poisoned").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forgotten_job_is_read_again() {
        let templates = JobTemplates::new();
        let job = Uuid::new_v4();
        assert!(templates.get(job).is_none());
        assert_eq!(templates.len(), 0);
        templates.forget(job);
        assert_eq!(templates.len(), 0, "forgetting an unknown job is nothing");
    }
}
