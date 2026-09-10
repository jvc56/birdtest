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

-- Input data
--
-- Must precede jobs and player_configs: both reference input_data.

-- Every input data file birdtest knows about, identified by content.
--
-- A row is a (path, sha256) pair: the same path with different bytes is a
-- different row, which is the entire point. `tarball_date` records the
-- versioned tarball a row was FIRST seen in -- provenance, not membership. A
-- file unchanged between two tarballs stays one row labelled with the older
-- date, because it is the same bytes and a job pinning it is pinning those
-- bytes regardless of which tarball the contributor installed.
CREATE TABLE input_data (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Path relative to the data root, including basename: 'lexica/NWL23.kwg'.
    path         TEXT NOT NULL,
    -- Derived from `path` at import and stored because dispatch and the client
    -- protocol address files by (role, name), not by path: MAGPIE resolves a
    -- name through its own data_paths search list.
    role         TEXT NOT NULL CHECK (role IN ('kwg','klv','winpct','letterdist','layout')),
    name         TEXT NOT NULL,          -- 'NWL23', 'winpct', 'english', 'standard15'
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    bytes        BIGINT NOT NULL CHECK (bytes >= 0),
    -- YYYYMMDD name of the versioned tarball this content was first imported
    -- from. Text, not DATE: it is the artifact's name, and it appears verbatim
    -- in the message a contributor is told to act on.
    tarball_date TEXT NOT NULL CHECK (tarball_date ~ '^\d{8}$'),
    -- The file's bytes, for the roles the SERVER itself reads. birdtest
    -- enumerates rack universes and builds KLVs from the letter distribution,
    -- so those bytes must be the pinned ones -- there is no server-side disk
    -- copy of the data any more. Lexica stay out: a 15 MB .kwg in a row is a
    -- different proposition and nothing server-side reads one.
    --
    -- The check is an equivalence, not a nullable convenience: a letterdist or
    -- layout row without bytes cannot exist, and a kwg/klv/winpct row with
    -- bytes cannot either. Server-side code therefore has no "fall back to the
    -- filesystem" branch to write.
    content      BYTEA
                 CHECK ((role IN ('letterdist','layout')) = (content IS NOT NULL)),
    imported_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    imported_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    UNIQUE (path, sha256)
);

CREATE INDEX input_data_role_name_idx ON input_data (role, name);

-- Staged imports. Phase 1 (a spawned background task) writes; phase 2 reads and
-- commits. Rows here are proposals, not data -- nothing dispatch or job creation
-- reads. birdtest runs as a single instance, so a task needs no lease and
-- startup may fail any row still 'running'.
CREATE TABLE input_data_imports (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tarball_date   TEXT NOT NULL CHECK (tarball_date ~ '^\d{8}$'),
    commit_sha     TEXT NOT NULL,
    -- NULL until the download completes: the row exists from the moment the
    -- background task is spawned.
    tarball_sha256 TEXT,
    state          TEXT NOT NULL DEFAULT 'running'
                   CHECK (state IN ('running', 'staged', 'confirmed',
                                    'cancelled', 'failed')),
    -- What the poller renders while state = 'running'.
    progress_bytes   BIGINT NOT NULL DEFAULT 0,
    progress_entries INT    NOT NULL DEFAULT 0,
    -- Why it failed, shown verbatim to the admin.
    error          TEXT,
    requested_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    requested_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at   TIMESTAMPTZ
);

CREATE TABLE input_data_import_rows (
    import_id  UUID NOT NULL REFERENCES input_data_imports(id) ON DELETE CASCADE,
    path       TEXT NOT NULL,
    role       TEXT NOT NULL,
    name       TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    bytes      BIGINT NOT NULL,
    -- 'new' | 'known' | 'collision' (same path, different sha256 already known)
    disposition TEXT NOT NULL,
    -- Carried from phase 1 for letterdist/layout entries so confirmation
    -- inserts input_data.content without re-downloading the tarball.
    content     BYTEA,
    PRIMARY KEY (import_id, path, sha256)
);

-- Jobs

CREATE TYPE job_type AS ENUM (
    'opening_rack',
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
    -- Settings every job type has, regardless of what it does. The lexicon is
    -- NOT here: it lives on the player (player_configs.kwg_id), because MAGPIE
    -- scopes it per player and two players may run different ones.
    variant       TEXT NOT NULL,                            -- 'classic' | 'wordsmog'; a rules setting, not a file
    letterdist_id UUID NOT NULL REFERENCES input_data(id),  -- one per job: MAGPIE takes one -ld for the whole game
    layout_id     UUID NOT NULL REFERENCES input_data(id),  -- 'standard15' unless a job says otherwise
    -- Minimum MAGPIE version workers must have to execute tasks for this job,
    -- as sortable parts. Semver in TEXT compares lexically, where '1.10.0' <
    -- '1.9.0' -- a bug that appears only once a minor version reaches double
    -- digits, i.e. long after it is written.
    --
    -- Not nullable: every job pins input data, and a client too old to
    -- understand expected_data contributes unverified rather than declining,
    -- so "no floor" is not a state worth being able to express. 0.0.1 is a
    -- placeholder for the MAGPIE release implementing the check.
    min_magpie_major INT NOT NULL DEFAULT 0 CHECK (min_magpie_major >= 0),
    min_magpie_minor INT NOT NULL DEFAULT 0 CHECK (min_magpie_minor >= 0),
    min_magpie_patch INT NOT NULL DEFAULT 1 CHECK (min_magpie_patch >= 0),
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
    -- The files this player loads, pinned by content rather than named.
    --
    -- kwg_id and klv_id are NOT NULL: there is no job lexicon left to fall back
    -- to, and "NULL = the lexicon default" was exactly the implicit name-based
    -- resolution this design removes.
    --
    -- winpct_id stays nullable, but NULL now means "this player never loads a
    -- win% model" -- true of every static player, since MAGPIE only reads one
    -- through config_load_win_pcts. Validated against the sim columns at job
    -- creation: a simming player must have one, a static player must not.
    kwg_id           UUID NOT NULL REFERENCES input_data(id),   -- (-l1 / -l2)
    klv_id           UUID NOT NULL REFERENCES input_data(id),   -- (-k1 / -k2)
    winpct_id        UUID REFERENCES input_data(id),            -- (-winpct)
    -- The config this one was cloned from, for a data update. Ratings do NOT
    -- carry over -- player_config_ratings is keyed by config and ratings are
    -- only comparable on identical data -- so the UI must show where a config
    -- with no history came from.
    cloned_from_id   UUID REFERENCES player_configs(id),
    -- Simulation parameters (all NULL for a static player)
    max_iterations   INT,                   -- -i1 / -i2
    -- Two pairs of "how much to compute" / "how much to report". MAGPIE
    -- generates plays and plies, then displays a subset of each; birdtest
    -- stores exactly what is displayed.
    num_plies          INT,                 -- plies to simulate    (-pl1 / -pl2)
    num_plies_recorded INT,                 -- plies to report      (shplies)
    num_plays          INT,                 -- plays to simulate    (-np1 / -np2)
    num_plays_recorded INT,                 -- plays to report      (maxnumdplays)
    stopping_pct     DOUBLE PRECISION,      -- -sc1 / -sc2 (0–100)
    use_inference    BOOLEAN,               -- -si1 / -si2
    time_limit_secs  INT,                   -- -tl1 / -tl2
    -- The remaining MAGPIE options that can affect how a player plays.
    -- Exhaustive on purpose: anything not stated here falls back to whatever
    -- value a worker's own MAGPIE process happens to have, which can differ
    -- across workers and silently produce non-comparable data.
    use_wordmap          BOOLEAN,            -- -w1 / -w2
    use_rit               BOOLEAN,           -- rack info table            (-rit1 / -rit2)
    min_play_iterations   INT,               -- -mi1 / -mi2
    threshold             TEXT,              -- 'none' | 'gk16'            (-th1 / -th2)
    sampling_rule         TEXT,              -- 'round_robin' | 'top_two_ids' (-sa1 / -sa2)
    inference_margin       DOUBLE PRECISION, -- -im1 / -im2
    utility_w_winpct       DOUBLE PRECISION, -- blended-utility weight on win%     (-uwin1 / -uwin2)
    utility_w_spread       DOUBLE PRECISION, -- blended-utility weight on spread   (-uspread1 / -uspread2)
    utility_spread_scale   DOUBLE PRECISION, -- blended-utility spread scale       (-uspreadscale1 / -uspreadscale2)
    -- Options that are one shared MAGPIE setting for the whole run rather
    -- than per-player. Stored here anyway (duplicated on both players'
    -- rows in a job, validated equal at job-creation time) so this table
    -- stays the single, exhaustive source of what a job asked MAGPIE for.
    movegen_margin         DOUBLE PRECISION, -- move-gen equity margin for 'equity' recording (-mmargin)
    created_by       UUID NOT NULL REFERENCES users(id),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Per-job-type config tables (one row per job; replaces the config JSONB column)

CREATE TABLE job_opening_rack_config (
    job_id            UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player config, the other two on `jobs`.
    -- The player config used to analyze each rack (may be a simmer or static player).
    player_config_id  UUID NOT NULL REFERENCES player_configs(id),
    -- Racks handed out per task. One rack per task means one claim/submit round
    -- trip per rack, and the worker rate limit alone would then cap a worker at
    -- well under a rack per second against a space of millions.
    racks_per_batch   INT NOT NULL DEFAULT 500 CHECK (racks_per_batch >= 1),
    rack_size         INT NOT NULL DEFAULT 7 CHECK (rack_size BETWEEN 1 AND 7),
    -- Size of the rack space, computed at job creation. Tasks address ranges of
    -- it, so this is what tells the scheduler when the job is exhausted.
    total_racks       BIGINT NOT NULL CHECK (total_racks >= 0)
);

CREATE TABLE job_game_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
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
    elo_high            DOUBLE PRECISION NOT NULL DEFAULT 10.0,
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE job_game_pair_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    pairs_per_batch     INT NOT NULL DEFAULT 1,
    min_pairs           INT NOT NULL,
    max_pairs           INT NOT NULL,
    sprt_alpha          DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    sprt_beta           DOUBLE PRECISION NOT NULL DEFAULT 0.05,
    elo_low             DOUBLE PRECISION NOT NULL DEFAULT -10.0,
    elo_high            DOUBLE PRECISION NOT NULL DEFAULT 10.0,
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE job_leave_config (
    job_id         UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- The one place a lexicon still sits on a job: leave generation has a
    -- single bot and no player_configs row to hold it. It needs no klv_id
    -- (every generation's leaves come from the server-built KLV artifact, and
    -- generation 1's is a zeroed one) and no winpct_id (the bot plays
    -- statically). Its complete data requirement is this plus the job's
    -- letterdist_id and layout_id.
    kwg_id         UUID NOT NULL REFERENCES input_data(id),
    -- Games each leave-gen task plays over its forced-rack subset.
    num_iterations INT NOT NULL,
    -- How many sequential generations this job runs before it is complete.
    generation_count  INT NOT NULL DEFAULT 1 CHECK (generation_count >= 1),
    -- Per-generation occurrence target every rack must reach before the generation closes.
    target_rack_count INT NOT NULL CHECK (target_rack_count >= 1),
    -- Size of the forced-rack subset handed to a single task.
    racks_per_task    INT NOT NULL CHECK (racks_per_task >= 1),
    -- Largest leave size enumerated into the rack universe (leaves are 1..N tiles).
    max_leave_size    INT NOT NULL DEFAULT 6 CHECK (max_leave_size BETWEEN 1 AND 6),
    -- Whether the leave-generating bot plays with a wordmap. Sent to the worker,
    -- which builds one from its .kwg if it does not already have it. A player
    -- setting like any other -- workers assume nothing about wordmaps.
    use_wordmap       BOOLEAN NOT NULL DEFAULT TRUE
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

-- 'declined' is distinct from 'abandoned': one is a worker saying "I cannot do
-- this", the other is a claim that lapsed. Only the first is diagnostic.
CREATE TYPE claim_state AS ENUM ('claimed', 'completed', 'abandoned', 'declined');

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
    -- As reported at claim time. What the fleet is actually running, which is
    -- the evidence for raising a job's floor.
    magpie_version       TEXT,
    CONSTRAINT claim_has_single_owner CHECK (
        (claimed_by_user_id IS NOT NULL)::int + (claimed_by_anon_uuid IS NOT NULL)::int = 1
    )
);

-- Prevent a single identity from filling more than one live slot on the same
-- task. 'declined' must be excluded alongside 'abandoned': a worker that
-- declined a task for missing data and then fixed its data has to be able to
-- claim that task again.
CREATE UNIQUE INDEX task_claims_user_unique_idx
    ON task_claims (task_id, claimed_by_user_id)
    WHERE state NOT IN ('abandoned', 'declined');
CREATE UNIQUE INDEX task_claims_anon_unique_idx
    ON task_claims (task_id, claimed_by_anon_uuid)
    WHERE state NOT IN ('abandoned', 'declined');

-- What a worker said it was missing when it declined. The server records gaps
-- for humans; it does not route on them (the client sends its own unsupported
-- set with each claim, which makes that state self-correcting).
CREATE TABLE worker_data_gaps (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id       UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    claim_id     UUID NOT NULL REFERENCES task_claims(id) ON DELETE CASCADE,
    role         TEXT NOT NULL,
    name         TEXT NOT NULL,
    expected     TEXT NOT NULL,
    actual       TEXT,                    -- NULL = file absent
    reported_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX worker_data_gaps_job_idx ON worker_data_gaps (job_id, role, name);

-- Task requests (one-to-one with tasks; inserted in the same transaction as the task row)

-- One row per task, covering a contiguous range of the rack space. The racks
-- themselves are not stored: they are unranked from `rack_start` on demand,
-- which is what lets a job over millions of racks be created in constant time.
--
-- Named for opening racks rather than positions: the request is specifically a
-- set of opening racks, and there is no general position-analysis job. What
-- comes back from analyzing one *is* a position analysis, which is why the
-- record tables below keep that name.
CREATE TABLE opening_rack_requests (
    task_id           UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    -- No lexicon column: the player config carries it.
    variant           TEXT NOT NULL,
    letter_distribution TEXT NOT NULL,
    -- Index of the first rack in this batch, and how many it covers. The final
    -- batch of a job may be short.
    rack_start        BIGINT NOT NULL CHECK (rack_start >= 0),
    rack_count        INT NOT NULL CHECK (rack_count >= 1),
    previous_play     TEXT,                  -- GCG-encoded previous move; required when inference is enabled; NULL for opening racks
    player_config_id  UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE game_requests (
    task_id           UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    -- No lexicon column: each player config carries its own.
    variant           TEXT NOT NULL,
    letter_distribution TEXT NOT NULL,
    -- Denormalized from the job config, like everything else here, so the
    -- request a re-dispatched task replays is exactly the one it was given.
    capture_positions BOOLEAN NOT NULL DEFAULT FALSE,
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
    letter_distribution TEXT NOT NULL,
    generation          INT NOT NULL,
    forced_racks        TEXT[] NOT NULL,   -- the rack subset this task must force (passed to MAGPIE's rack_list_create)
    num_games           INT NOT NULL,      -- denormalized from job_leave_config.num_iterations
    -- Combined KLV from the previous generation. Never NULL: generation 1 reads
    -- the server-built zeroed KLV at generation-0, so every generation fetches
    -- its leaves the same way and the client has no first-generation branch.
    previous_artifact_key TEXT NOT NULL,
    use_wordmap         BOOLEAN NOT NULL   -- denormalized from job_leave_config.use_wordmap
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

-- One analysed position per row, whatever produced it.
--
-- Opening rack jobs write one per rack. Games and game-pairs jobs write one per
-- turn when `capture_positions` is on: a worker analyses a position on every
-- turn anyway, and keeping those makes a job a corpus of analysed positions as
-- well as an Elo measurement.
--
-- The request that produced these is job-type-specific -- opening_rack_requests
-- or game_requests -- but what comes back is a position analysis either way,
-- which is why both share this table.
CREATE TABLE position_analysis_records (
    -- Surrogate, because the natural key differs by source: an opening rack is
    -- unique per (claim, rack), while an in-game position recurs at the same
    -- rack across turns and games.
    id              BIGSERIAL PRIMARY KEY,
    task_claim_id   UUID NOT NULL REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    rack            TEXT NOT NULL,
    -- CGP of the position analysed. NULL for an opening rack, where the board
    -- is empty by definition and the rack is the whole position.
    position        TEXT,
    -- In-game positions only: which game of the batch, and which turn of it.
    game_index      SMALLINT,
    turn_number     SMALLINT,
    -- The move played on the previous turn of this game, and its score.
    -- NULL for turn 0 of a game (nothing preceded it) and for opening racks.
    previous_move       TEXT,
    previous_move_score INT,
    -- How many moves the worker ranked, which is generally far more than the
    -- stored moves. The one thing about the analysis those cannot tell you,
    -- since they are truncated.
    --
    -- The best move, its score and its equity are deliberately *not* stored
    -- here: they are the rank 1 row in position_analysis_moves, and duplicating
    -- them is a second copy to keep consistent for no gain.
    num_moves       INT NOT NULL,
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT position_analysis_in_game_together CHECK (
        (game_index IS NULL AND turn_number IS NULL)
        OR (game_index IS NOT NULL AND turn_number IS NOT NULL)
    )
);

-- Games are seeded and deterministic, so redundant claims replay identical
-- games and would capture identical positions. Keying on the task rather than
-- the claim makes the first accepted claim the one that lands and the rest
-- no-ops, so redundancy still verifies the *result* without multiplying the
-- corpus.
CREATE UNIQUE INDEX position_analysis_records_in_game_idx
    ON position_analysis_records (task_id, game_index, turn_number)
    WHERE game_index IS NOT NULL;

-- Opening racks keep their natural key: one analysis per rack per claim, so
-- redundant claims each record their own and can be compared.
CREATE UNIQUE INDEX position_analysis_records_rack_idx
    ON position_analysis_records (task_claim_id, rack)
    WHERE game_index IS NULL;

CREATE INDEX position_analysis_records_task_idx
    ON position_analysis_records (task_id, rack);

-- The top `num_plays_recorded` moves per position, from the player config that
-- produced them. Storing every move the worker ranked would be untenable:
-- a job over the full English 7-tile space is roughly 3.2 million racks, and a
-- 40,000-pair job with capture on is 1.8 million positions.
CREATE TABLE position_analysis_moves (
    id              BIGSERIAL PRIMARY KEY,
    record_id       BIGINT NOT NULL REFERENCES position_analysis_records(id) ON DELETE CASCADE,
    -- Denormalized so job-wide aggregates need not join through the record.
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    rank            SMALLINT NOT NULL,
    move            TEXT NOT NULL,
    score           INT NOT NULL,
    equity          DOUBLE PRECISION NOT NULL,
    -- The simulated win percentage. NULL for a static player, which ranks on
    -- equity alone and simulates nothing.
    win_percentage  DOUBLE PRECISION,
    -- Mean win%+spread blend in [0, 1] (see the player config's
    -- utility_w_winpct/utility_w_spread/utility_spread_scale), sometimes used
    -- to rank moves instead of equity or raw win percentage. NULL for a
    -- static player, same as win_percentage.
    blended_utility DOUBLE PRECISION
);
CREATE INDEX position_analysis_moves_record_idx
    ON position_analysis_moves (record_id, rank);
-- The dashboard's aggregates are all over best moves, which are read from here
-- rather than duplicated onto the record. A partial index keeps that a scan of
-- one row per position rather than of every stored move.
CREATE INDEX position_analysis_moves_best_idx
    ON position_analysis_moves (task_id) INCLUDE (move, equity)
    WHERE rank = 1;

-- Per-ply simulation stats for each candidate move. Only populated for simming
-- player configs; a static player has no per-ply statistics to record.
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
    -- SHA-256 of the KLV bytes as first written. The object store holds the
    -- only copy of these bytes, and an artifact is the one piece of state that
    -- can be silently overwritten -- by a restore that replays a generation
    -- transition against fewer results, or by a rebuild under a changed
    -- klv::build. Recording the hash is what turns that from invisible into a
    -- query; the ON CONFLICT DO NOTHING on insert means the row keeps the
    -- FIRST hash, so a later mismatch is evidence rather than an overwrite.
    sha256        TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
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

-- Backups
--
-- Written by scripts/backup.sh (and the restore drill), never by the server:
-- the backend reads this table for the admin dashboard and has no ability to
-- perform or delete a backup. See PLAN.md, "Making backups visible".
--
-- Insert-only, failures included: a run that broke leaves an ok = false row,
-- so the admin page shows a failure rather than a gap that reads as "nothing
-- happened". Nothing in the request path reads this table.
--
-- Restoring the database restores its own backup history, which is
-- momentarily confusing and harmless: the rows describe backups that do still
-- exist in the bucket.
CREATE TABLE backups (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind           TEXT NOT NULL CHECK (kind IN ('pg_dump', 'rds_snapshot')),
    -- Key prefix within the backup bucket ('pg/2026-09-07T03-00-00Z'), NULL
    -- for a snapshot; snapshot_id is the mirror of it. Exactly one is set.
    s3_key         TEXT,
    snapshot_id    TEXT,
    started_at     TIMESTAMPTZ NOT NULL,
    finished_at    TIMESTAMPTZ NOT NULL,
    dump_bytes     BIGINT CHECK (dump_bytes >= 0),
    -- Per-table exact counts at dump time. What a restore is verified against
    -- (PLAN.md, "Verifying a restore"), and what makes a silently truncated dump
    -- detectable without restoring it.
    row_counts     JSONB NOT NULL,
    sha256         TEXT CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    ok             BOOLEAN NOT NULL,
    CONSTRAINT backups_has_single_location CHECK (
        (s3_key IS NOT NULL)::int + (snapshot_id IS NOT NULL)::int = 1
    )
);

-- The admin page asks for the most recent runs, and the staleness figure asks
-- for the most recent successful one.
CREATE INDEX backups_finished_idx ON backups (finished_at DESC);

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
