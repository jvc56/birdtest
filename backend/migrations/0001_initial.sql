-- Users

CREATE TABLE users (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username             TEXT NOT NULL UNIQUE,
    email                TEXT NOT NULL UNIQUE,
    password_hash        TEXT NOT NULL,
    email_confirmed_at   TIMESTAMPTZ,
    is_admin             BOOLEAN NOT NULL DEFAULT FALSE,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE email_confirmations (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   TEXT NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE password_reset_tokens (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash     TEXT NOT NULL UNIQUE,
    label        TEXT,
    is_active    BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);
-- Enforce the 100-key limit per user at the application layer, not via a DB constraint.

-- Workers

CREATE TABLE anonymous_workers (
    uuid          UUID PRIMARY KEY,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE worker_bans (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID REFERENCES users(id) ON DELETE CASCADE,
    anon_uuid   UUID REFERENCES anonymous_workers(uuid) ON DELETE CASCADE,
    reason      TEXT,
    banned_by   UUID NOT NULL REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT ban_has_single_target CHECK (
        (user_id IS NOT NULL)::int + (anon_uuid IS NOT NULL)::int = 1
    )
);

-- Jobs

CREATE TYPE job_type AS ENUM (
    'opening_rack_analysis',
    'games',
    'game_pairs',
    'leave_generation'
);

CREATE TYPE job_status AS ENUM (
    'active',
    'inactive',
    'completed'
);

CREATE TABLE jobs (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_type   job_type NOT NULL,
    -- Lower value = higher priority. Priority 0 outranks priority 1.
    priority   INT NOT NULL DEFAULT 0,
    -- NULL until the job is first activated; set by the admin at activation time.
    allocation INT CHECK (allocation BETWEEN 0 AND 100),
    -- Number of independent workers that must complete each task. Default 1 = single-claim behavior.
    redundancy INT NOT NULL DEFAULT 1 CHECK (redundancy >= 1),
    -- Jobs start inactive; admin activates with an allocation percentage.
    status     job_status NOT NULL DEFAULT 'inactive',
    -- SET NULL if the creating admin's account is deleted.
    created_by           UUID REFERENCES users(id) ON DELETE SET NULL,
    -- Minimum MAGPIE version workers must have to execute tasks for this job.
    -- NULL = no minimum enforced. Semver string, e.g. "1.4.0".
    min_magpie_version   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at    TIMESTAMPTZ,
    deactivated_at  TIMESTAMPTZ
);

-- Named, reusable player configurations.
-- Each row stores the MAGPIE argument values for one player slot.
-- Rows are immutable once any job references them (enforced at the application layer).
--
-- recorder_type (-r1 / -r2): 'best' = play the top-ranked move (fast, right for autoplay);
--   'equity' = record all moves within mmargin equity of best; 'all' = record every move.
--   For autoplay in birdtest, always use 'best'.
--
-- sort_strategy (-s1 / -s2): 'equity' = sort by equity (score + leave value) — standard static
--   player; 'score' = sort by raw score only. NULL for simming players (sim output determines
--   the move, not a static sort). Both static and simming players are valid in games/game_pairs jobs.
--
-- Simulation columns are all NULL for a static (no-sim) player.

CREATE TABLE player_configs (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL UNIQUE,  -- human-readable label, e.g. "simmer-NWL23-4ply"
    recorder_type    TEXT NOT NULL,         -- 'best' | 'equity' | 'all'  (-r1 / -r2)
    sort_strategy    TEXT,                  -- 'equity' | 'score' | NULL  (-s1 / -s2)
    leaves           TEXT,                  -- leave file name; NULL = lexicon default  (-k1 / -k2)
    -- Simulation parameters (all NULL for a static player)
    max_iterations   INT,                   -- -i1 / -i2
    plies            INT,                   -- -pl1 / -pl2
    top_plays        INT,                   -- -np1 / -np2
    stopping_pct     DOUBLE PRECISION,      -- -sc1 / -sc2 (0–100)
    use_inference    BOOLEAN,               -- -si1 / -si2
    time_limit_secs  DOUBLE PRECISION,      -- -tl1 / -tl2
    created_by       UUID NOT NULL REFERENCES users(id),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Per-job-type config tables (one row per job; replaces the config JSONB column)

CREATE TABLE job_opening_rack_config (
    job_id            UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    lexicon           TEXT NOT NULL,
    variant           TEXT NOT NULL,
    -- The player config used to analyze each rack (may be a simmer or static player).
    player_config_id  UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE job_game_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    lexicon             TEXT NOT NULL,
    variant             TEXT NOT NULL,
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    games_per_batch     INT NOT NULL DEFAULT 1,
    -- Two finish conditions: SPRT significance (evaluated after min_games) OR reaching max_games.
    min_games           INT NOT NULL,   -- SPRT is not evaluated until this many games are complete
    max_games           INT NOT NULL,   -- job auto-completes at this count regardless of SPRT
    -- SPRT parameters (H0: elo_diff = elo_low, H1: elo_diff = elo_high)
    sprt_alpha          DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    sprt_beta           DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    elo_low             DOUBLE PRECISION NOT NULL DEFAULT -10.0,
    elo_high            DOUBLE PRECISION NOT NULL DEFAULT 10.0
);

CREATE TABLE job_game_pair_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    lexicon             TEXT NOT NULL,
    variant             TEXT NOT NULL,
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    pairs_per_batch     INT NOT NULL DEFAULT 1,
    min_pairs           INT NOT NULL,
    max_pairs           INT NOT NULL,
    sprt_alpha          DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    sprt_beta           DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    elo_low             DOUBLE PRECISION NOT NULL DEFAULT -10.0,
    elo_high            DOUBLE PRECISION NOT NULL DEFAULT 10.0
);

CREATE TABLE job_leave_config (
    job_id         UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    lexicon        TEXT NOT NULL,
    variant        TEXT NOT NULL,
    -- Games each leave-gen task plays over its forced-rack subset.
    num_iterations INT NOT NULL,
    -- How many sequential generations this job runs before it is complete.
    generation_count  INT NOT NULL DEFAULT 1 CHECK (generation_count >= 1),
    -- Per-generation occurrence target every rack must reach before the generation closes.
    target_rack_count INT NOT NULL CHECK (target_rack_count >= 1),
    -- Size of the forced-rack subset handed to a single task.
    racks_per_task    INT NOT NULL CHECK (racks_per_task >= 1),
    -- Largest leave size enumerated into the rack universe (leaves are 1..N tiles).
    max_leave_size    INT NOT NULL DEFAULT 6 CHECK (max_leave_size BETWEEN 1 AND 6)
);

-- Tasks

CREATE TYPE task_state AS ENUM ('available', 'claimed', 'completed');

CREATE TABLE tasks (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id               UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    -- Seed for seed-based tasks (games, game pairs). NULL for non-seed tasks.
    seed                 BIGINT,  -- stored as signed int64; interpreted as uint64 at the application layer
    state                task_state NOT NULL DEFAULT 'available',
    -- Denormalized counters used by SKIP LOCKED selection; avoids per-candidate join/aggregate.
    accepted_count       INT NOT NULL DEFAULT 0,
    active_claim_count   INT NOT NULL DEFAULT 0,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at         TIMESTAMPTZ
);

-- Prevent duplicate seed-based tasks within the same job.
CREATE UNIQUE INDEX tasks_seed_unique_idx ON tasks (job_id, seed) WHERE seed IS NOT NULL;

-- Partial indexes to support efficient SKIP LOCKED task selection and timeout reclamation.
CREATE INDEX tasks_queue_idx   ON tasks (job_id, state) WHERE state = 'available';
CREATE INDEX tasks_claimed_idx ON tasks (state) WHERE state = 'claimed';

-- Individual claims (one row per worker claim; up to redundancy concurrent/cumulative rows per task)
--
-- Account deletion is handled at the application layer (not via ON DELETE CASCADE) because
-- task counters (accepted_count, active_claim_count) must be decremented and tasks may need
-- to revert from completed → available. The deletion sequence is:
--   1. For each active/completed claim: update task counters.
--   2. Delete all task records (game_results, etc.) linked to those claims.
--   3. Delete the task_claim rows.
--   4. Delete the user row (cascades to api_keys, email_confirmations, password_reset_tokens).

CREATE TYPE claim_state AS ENUM ('claimed', 'completed', 'abandoned');

CREATE TABLE task_claims (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id              UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    claim_token          UUID NOT NULL,
    state                claim_state NOT NULL DEFAULT 'claimed',
    claimed_by_user_id   UUID REFERENCES users(id),
    claimed_by_anon_uuid UUID REFERENCES anonymous_workers(uuid),
    claimed_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_heartbeat_at    TIMESTAMPTZ,
    completed_at         TIMESTAMPTZ,
    CONSTRAINT claim_has_single_owner CHECK (
        (claimed_by_user_id IS NOT NULL)::int + (claimed_by_anon_uuid IS NOT NULL)::int = 1
    )
);

-- Prevent a single identity from filling more than one non-abandoned slot on the same task.
CREATE UNIQUE INDEX task_claims_user_unique_idx
    ON task_claims (task_id, claimed_by_user_id)
    WHERE state != 'abandoned';
CREATE UNIQUE INDEX task_claims_anon_unique_idx
    ON task_claims (task_id, claimed_by_anon_uuid)
    WHERE state != 'abandoned';

-- Task requests (one-to-one with tasks; inserted in the same transaction as the task row)

CREATE TABLE position_requests (
    task_id           UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon           TEXT NOT NULL,
    variant           TEXT NOT NULL,
    position          TEXT NOT NULL,         -- CGP-encoded board + rack
    previous_play     TEXT,                  -- GCG-encoded previous move; required when inference is enabled; NULL for opening racks
    player_config_id  UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE game_requests (
    task_id           UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon           TEXT NOT NULL,
    variant           TEXT NOT NULL,
    -- seed is also stored on the tasks row; duplicated here for convenience when reading the full request.
    seed              BIGINT NOT NULL,
    num_games         INT NOT NULL DEFAULT 1,
    player1_config_id UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE leave_requests (
    task_id             UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon             TEXT NOT NULL,
    variant             TEXT NOT NULL,
    generation          INT NOT NULL,
    forced_racks        TEXT[] NOT NULL,   -- the rack subset this task must force (see rack_list_create's forceracksfile)
    num_games           INT NOT NULL,      -- denormalized from job_leave_config.num_iterations
    previous_artifact_key TEXT             -- combined KLV from generation - 1; NULL for generation 1
);

-- Live per-rack occurrence progress for the in-progress generation of a leave-gen job.
-- Upserted transactionally on every accepted leave task result; drives both generation-transition
-- detection (all racks >= target) and the live dashboard figure.
CREATE TABLE leave_rack_progress (
    job_id           UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation       INT NOT NULL,
    rack             TEXT NOT NULL,
    occurrence_count BIGINT NOT NULL DEFAULT 0,
    equity_sum       DOUBLE PRECISION NOT NULL DEFAULT 0,  -- occurrence_count-weighted; equity_sum / occurrence_count = mean
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, generation, rack)
);

-- Task records (one per accepted claim; keyed by task_claim_id since redundancy > 1 yields multiple results per task)
-- task_id is denormalized here for efficient job-results queries without joining through task_claims.

CREATE TABLE position_analysis_records (
    task_claim_id   UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    best_move       TEXT NOT NULL,
    best_score      INT NOT NULL,
    best_equity     DOUBLE PRECISION NOT NULL,
    num_moves       INT NOT NULL,
    artifact_key    TEXT,           -- S3 key for full ranked move list
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Top moves stored relationally for querying; full list is in S3 via artifact_key.
CREATE TABLE position_analysis_moves (
    id              BIGSERIAL PRIMARY KEY,
    task_claim_id   UUID NOT NULL REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    rank            SMALLINT NOT NULL,
    move            TEXT NOT NULL,
    score           INT NOT NULL,
    equity          DOUBLE PRECISION NOT NULL
);
CREATE INDEX position_analysis_moves_task_idx ON position_analysis_moves (task_id);

-- Per-ply simulation stats for each candidate move.
CREATE TABLE position_analysis_plies (
    id               BIGSERIAL PRIMARY KEY,
    move_id          BIGINT NOT NULL REFERENCES position_analysis_moves(id) ON DELETE CASCADE,
    ply              SMALLINT NOT NULL,
    bingo_percentage DOUBLE PRECISION NOT NULL,
    average_score    DOUBLE PRECISION NOT NULL,
    UNIQUE (move_id, ply)
);
CREATE INDEX position_analysis_plies_move_idx ON position_analysis_plies (move_id);

-- Shared by games and game pairs: one row per accepted task, holding the aggregate
-- MAGPIE's autoplay reports. Autoplay does not emit individual games -- it reports
-- counts and score moments for a batch, and in `-gp` mode a second such summary
-- covering only the *divergent* pairs: those whose two games did not play
-- identically. A pair that played identically is a guaranteed tie carrying no
-- information, so excluding those is the variance reduction pairing exists to
-- provide, and the divergent aggregate is what SPRT and Glicko are computed from.
CREATE TABLE game_results (
    task_claim_id     UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id           UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,

    -- Every game this task played. Two per pair for a game_pairs task.
    games             INT NOT NULL CHECK (games >= 0),
    wins              INT NOT NULL CHECK (wins >= 0),      -- player 1
    losses            INT NOT NULL CHECK (losses >= 0),
    ties              INT NOT NULL CHECK (ties >= 0),
    p1_score_mean     DOUBLE PRECISION NOT NULL,
    p1_score_sd       DOUBLE PRECISION NOT NULL,
    p2_score_mean     DOUBLE PRECISION NOT NULL,
    p2_score_sd       DOUBLE PRECISION NOT NULL,
    CONSTRAINT game_results_counts_sum CHECK (wins + losses + ties = games),

    -- The divergent subset. NULL for `games` jobs, which do not play pairs.
    divergent_games   INT CHECK (divergent_games >= 0),
    divergent_wins    INT CHECK (divergent_wins >= 0),
    divergent_losses  INT CHECK (divergent_losses >= 0),
    divergent_ties    INT CHECK (divergent_ties >= 0),
    CONSTRAINT game_results_divergent_all_or_nothing CHECK (
        (divergent_games IS NULL AND divergent_wins IS NULL
             AND divergent_losses IS NULL AND divergent_ties IS NULL)
        OR (divergent_games IS NOT NULL AND divergent_wins IS NOT NULL
             AND divergent_losses IS NOT NULL AND divergent_ties IS NOT NULL
             AND divergent_wins + divergent_losses + divergent_ties = divergent_games
             AND divergent_games <= games)
    ),

    submitted_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per accepted leave task (a single worker's forced-rack partition of a generation).
-- The full {rack, count, mean} submission is folded into leave_rack_progress and not kept
-- separately — nothing reads it back, so there's no CSV artifact to reference here.
CREATE TABLE leave_records (
    task_claim_id   UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    rack_count      INT NOT NULL,  -- number of distinct racks in this submission, for audit
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per completed generation: the server-built combined KLV (see Aggregation in
-- "Leave Generation — On-demand, partitioned generations"), not tied to any single task_claim
-- since it's produced by the server from all of that generation's leave_rack_progress rows.
CREATE TABLE leave_generation_artifacts (
    job_id        UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation    INT NOT NULL,
    artifact_key  TEXT NOT NULL,
    completed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, generation)
);

-- Glicko ratings per (player_config, job) pair
-- Used by game_pairs jobs. Each job maintains its own independent rating context for every player config involved.
-- The static bot is seeded at 2000; all other player configs start at the Glicko default of 1500.

CREATE TABLE player_config_ratings (
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    job_id           UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    rating           DOUBLE PRECISION NOT NULL DEFAULT 1500,
    rating_deviation DOUBLE PRECISION NOT NULL DEFAULT 350,  -- RD; shrinks as more pairs are played
    volatility       DOUBLE PRECISION NOT NULL DEFAULT 0.06, -- Glicko-2 σ
    games_played     INT NOT NULL DEFAULT 0,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (player_config_id, job_id)
);

-- Audit log

CREATE TABLE audit_log (
    id              BIGSERIAL PRIMARY KEY,
    action          TEXT NOT NULL,
    actor_user_id   UUID REFERENCES users(id),
    actor_anon_uuid UUID REFERENCES anonymous_workers(uuid),
    target_type     TEXT,
    target_id       TEXT,
    -- Typed extra-context columns (replace JSONB metadata)
    job_id          UUID REFERENCES jobs(id),      -- task/result events
    reason          TEXT,                           -- ban events, etc.
    old_status      TEXT,                           -- status-change events
    new_status      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Supporting indexes for the claim / submit / dashboard paths.

CREATE UNIQUE INDEX task_claims_token_idx     ON task_claims (claim_token);
CREATE INDEX        task_claims_task_idx      ON task_claims (task_id);
CREATE INDEX        task_claims_open_idx      ON task_claims (task_id) WHERE state = 'claimed';
CREATE INDEX        task_claims_user_idx      ON task_claims (claimed_by_user_id);
CREATE INDEX        task_claims_anon_idx      ON task_claims (claimed_by_anon_uuid);
CREATE INDEX        tasks_job_idx             ON tasks (job_id);
CREATE INDEX        game_results_task_idx     ON game_results (task_id);
CREATE INDEX        leave_records_task_idx    ON leave_records (task_id);
CREATE INDEX        position_records_task_idx ON position_analysis_records (task_id);
CREATE INDEX        audit_log_created_idx     ON audit_log (created_at DESC);
CREATE INDEX        audit_log_job_idx         ON audit_log (job_id);

-- Drives claim-time rack selection: "the racks furthest from target in this generation".
CREATE INDEX leave_rack_progress_pick_idx
    ON leave_rack_progress (job_id, generation, occurrence_count);
