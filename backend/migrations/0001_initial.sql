-- Users

CREATE TABLE users (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username             TEXT NOT NULL UNIQUE,
    email                TEXT NOT NULL UNIQUE,
    password_hash        TEXT NOT NULL,
    email_confirmed_at   TIMESTAMPTZ,
    is_admin             BOOLEAN NOT NULL DEFAULT FALSE,
    -- Embedded in every session token and compared on every request. Bumped by
    -- a password reset, "sign out everywhere" and account deletion, which is
    -- what revokes every session minted before.
    session_generation   INT NOT NULL DEFAULT 0,
    -- Set when an admin deletes the account. Deletion anonymizes rather than
    -- removes the row: username, email and password are replaced by
    -- tombstones and API keys are deleted, but the account's claims and
    -- results stay, so no donated compute is lost.
    deleted_at           TIMESTAMPTZ,
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
    -- SET NULL, like jobs.created_by: deleting the admin who issued a ban must
    -- neither fail nor lift the ban.
    banned_by   UUID REFERENCES users(id) ON DELETE SET NULL,
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
    -- so "no floor" is not a state worth being able to express. 0.1.0 is the
    -- first MAGPIE version that implements the protocol correctly.
    min_magpie_major INT NOT NULL DEFAULT 0 CHECK (min_magpie_major >= 0),
    min_magpie_minor INT NOT NULL DEFAULT 1 CHECK (min_magpie_minor >= 0),
    min_magpie_patch INT NOT NULL DEFAULT 0 CHECK (min_magpie_patch >= 0),
    -- Every claim ever issued for this job, abandoned and declined ones
    -- included: the deficit the scheduler orders on. Kept as a counter rather
    -- than counted, because counting task_claims on every claim request costs
    -- time proportional to the job's whole history. Only ever incremented,
    -- except by a purge, which deletes the claims it counts.
    claims_issued   BIGINT NOT NULL DEFAULT 0 CHECK (claims_issued >= 0),
    -- Progress totals the dashboard reads, maintained in the submit transaction
    -- rather than counted on read (PLAN.md, "What these reads cost"). Both are
    -- incremented
    -- once per task, on its FIRST accepted result, because that is the row the
    -- reads they replace selected: with redundancy > 1 the later claims of a
    -- task replay the same deterministic work, and summing all of them would
    -- multiply every total by the redundancy.
    --
    -- games_completed counts GAMES for both games and game_pairs; a pairs job's
    -- unit count is half of it, exactly as the read derived it. racks_analyzed
    -- counts distinct opening racks with an accepted analysis, which is a plain
    -- sum because each task covers its own disjoint slice of the rack space.
    --
    -- Neither is authoritative for anything that decides: SPRT still reads
    -- game_results, so a drifted counter shows a wrong number on a page and
    -- cannot stop a job early. A purge zeroes them; a partial restore
    -- recomputes them (RUNBOOK 2.3).
    games_completed BIGINT NOT NULL DEFAULT 0 CHECK (games_completed >= 0),
    racks_analyzed  BIGINT NOT NULL DEFAULT 0 CHECK (racks_analyzed >= 0),
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
    -- carry over -- a clone is a new player config, so it enters a rating pool
    -- with no games and no rating until it plays -- so the UI must show where a
    -- config with no history came from.
    cloned_from_id   UUID REFERENCES player_configs(id),
    -- Simulation parameters (all NULL for a static player)
    max_iterations   INT,                   -- -i1 / -i2
    -- Two pairs of "how much to compute" / "how much to report". MAGPIE
    -- generates plays and plies, then displays a subset of each; birdtest
    -- stores exactly what is displayed.
    num_plies          INT,                 -- plies to simulate    (-pl1 / -pl2)
    num_plies_recorded INT,                 -- plies to report      (shplies)
    num_plays          INT,                 -- plays to simulate    (-np1 / -np2)
    -- plays to report (maxnumdplays). Required: "keep everything" is unbounded
    -- per position, and the worker and the server must agree on the number.
    num_plays_recorded INT NOT NULL CHECK (num_plays_recorded >= 1),
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
    -- SET NULL, like jobs.created_by: a config outlives the admin who made it.
    created_by       UUID REFERENCES users(id) ON DELETE SET NULL,
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
    -- Whether the leave-generating bot plays with a wordmap. Sent to the worker,
    -- which builds one from its .kwg if it does not already have it. A player
    -- setting like any other -- workers assume nothing about wordmaps.
    use_wordmap       BOOLEAN NOT NULL DEFAULT TRUE
);

-- Exports
--
-- A completed job's results, as one gzipped NDJSON object in the artifact
-- store. Only completed jobs can be exported, and that is what makes the
-- artifact worth having: a completed job's results are immutable, so an export
-- is built once and reused, where an export of an active job would be stale as
-- it was written.
--
-- Shaped like input_data_imports, and for the same reason: a long operation an
-- admin starts, polls, and then acts on. birdtest runs as a single instance, so
-- the task needs no lease and startup may fail any row still 'running'.
CREATE TABLE job_exports (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id        UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    state         TEXT NOT NULL DEFAULT 'running'
                  CHECK (state IN ('running', 'ready', 'failed')),
    -- NULL until the upload completes: the row exists from the moment the
    -- background task is spawned.
    artifact_key  TEXT,
    bytes         BIGINT,
    sha256        TEXT,
    -- Rows written. Recorded so a later mismatch against the job is visible
    -- rather than silent -- the same reason the KLV artifacts carry a digest.
    row_count     BIGINT,
    error         TEXT,
    requested_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at  TIMESTAMPTZ
);

-- The newest ready export for a job, which is what a download resolves to.
CREATE INDEX job_exports_job_idx ON job_exports (job_id, requested_at DESC);

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
-- claimed_by_user_id carries no ON DELETE clause because a user row is never
-- deleted: account deletion anonymizes it in place (users.deleted_at, and a
-- tombstone username and email) and leaves these rows exactly where they are.
-- Removing them instead would take with them the captured in-game positions
-- keyed to those claims -- including the ones other redundant claims
-- deduplicated against, which nothing else holds -- and leave-generation
-- occurrences that were folded into per-rack totals and cannot be subtracted
-- back out. See routes::admin::delete_user.

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
    -- The job's pinned layout, by name. Stated on the request for the same
    -- reason the distribution is: a worker must play on the board the job
    -- pins, not on whatever board its own settings last loaded.
    board_layout      TEXT NOT NULL,
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
    board_layout      TEXT NOT NULL,
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
    board_layout        TEXT NOT NULL,
    generation          INT NOT NULL,
    forced_racks        TEXT[] NOT NULL,   -- the rack subset this task must force (passed to MAGPIE's rack_list_create)
    num_games           INT NOT NULL,      -- denormalized from job_leave_config.num_iterations
    -- Combined KLV from the previous generation. Never NULL: generation 1 reads
    -- the server-built zeroed KLV at generation-0, so every generation fetches
    -- its leaves the same way and the client has no first-generation branch.
    previous_artifact_key TEXT NOT NULL,
    use_wordmap         BOOLEAN NOT NULL   -- denormalized from job_leave_config.use_wordmap
);

-- Live per-rack occurrence progress for each generation of a leave-gen job, one row per
-- full 7-tile rack the distribution can draw (3,199,724 for English), seeded at zero when
-- the generation opens. Updated transactionally on every accepted leave task result; drives
-- both generation-transition detection (all racks >= target) and the live dashboard figure.
-- Leave values are derived from these full-rack means as MAGPIE's rack_list_write_to_klv does.
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
    -- A second cascade path: moves already go with their record, which goes
    -- with its task, but deleting a task reaches these directly too. It was
    -- added to let job-wide aggregates skip the record join; there are no such
    -- aggregates now, and it is kept for the cascade rather than for reads.
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
-- Every read of a best move goes through its record: the results listing joins
-- `record_id` and filters `rank = 1`, and a rack lookup reads a record's whole
-- ranked list. There is deliberately no job-wide index on `(task_id) WHERE
-- rank = 1`: one existed for a dashboard aggregate over every best move of a
-- job, that aggregate is gone (the panel shows progress only), and the index
-- cost maintenance on every move insert into a table that runs to tens of
-- millions of rows.
CREATE INDEX position_analysis_moves_record_idx
    ON position_analysis_moves (record_id, rank);

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

-- Shared by games and game pairs: one row per accepted claim, holding the
-- aggregate MAGPIE's autoplay reports. Autoplay does not emit individual games
-- -- it reports counts and score moments for a batch, and in `-gp` mode also
-- the pentanomial: how many completed pairs ended in each of the five possible
-- pair outcomes. The pentanomial is what SPRT and the rating fits read; the
-- divergent summary alongside it is a diagnostic only.
--
-- With redundancy > 1 a task has several rows here, one per accepted claim,
-- and because games are seeded and deterministic they describe the *same*
-- games. Every aggregate that treats rows as observations (SPRT, progress,
-- ratings) therefore reads one row per task -- the first accepted -- or it
-- would count each game `redundancy` times.
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

    -- The pentanomial: how many completed pairs ended in each of the five
    -- outcomes, indexed by player 1's half-point score across the pair, so
    -- pent_0 is "player 1 lost both games" and pent_4 is "won both". NULL for
    -- `games` jobs, which do not play pairs.
    --
    -- This -- not the divergent subset below -- is what SPRT and the ratings
    -- read. The pair is the independent unit of a paired run, and *every* pair
    -- belongs in the sample: a pair whose two games played identically is a
    -- guaranteed 1-1 tie, lands in pent_2, and is exactly the observation that
    -- says "these two are hard to tell apart". Dropping those conditions the
    -- sample on its own outcome and inflates the apparent difference without
    -- bound.
    pent_0            INT CHECK (pent_0 >= 0),
    pent_1            INT CHECK (pent_1 >= 0),
    pent_2            INT CHECK (pent_2 >= 0),
    pent_3            INT CHECK (pent_3 >= 0),
    pent_4            INT CHECK (pent_4 >= 0),
    CONSTRAINT game_results_pentanomial_all_or_nothing CHECK (
        (pent_0 IS NULL AND pent_1 IS NULL AND pent_2 IS NULL
             AND pent_3 IS NULL AND pent_4 IS NULL)
        OR (pent_0 IS NOT NULL AND pent_1 IS NOT NULL AND pent_2 IS NOT NULL
             AND pent_3 IS NOT NULL AND pent_4 IS NOT NULL
             -- The pentanomial and the game counts are two views of the same
             -- games, so they must agree on both the count and the outcome:
             -- one pair per two games, and the same half-point total for
             -- player 1 either way. A worker that miscounts fails here rather
             -- than silently biasing a rating pool.
             AND (pent_0 + pent_1 + pent_2 + pent_3 + pent_4) * 2 = games
             AND pent_1 + 2 * pent_2 + 3 * pent_3 + 4 * pent_4 = 2 * wins + ties)
    ),

    -- The divergent subset: pairs whose two games did not play identically.
    -- Kept as a *diagnostic* -- it says how often two configs actually differ,
    -- which is worth showing -- and deliberately not used as a statistical
    -- sample. NULL for `games` jobs.
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

-- One row per generation transition that has been *started*, claimed by the
-- worker request that found the generation complete.
--
-- A transition folds millions of leave_rack_progress rows into a KLV and
-- uploads it, which takes tens of seconds and cannot run inside the claim
-- transaction, since a proxy timeout would abandon it part-way. That leaves a
-- window in which a second claim would find the
-- same "every rack at target, nothing in flight" state and start the same
-- transition again, duplicating all of it. The primary key is what makes that
-- impossible: the deciding claim transaction commits this row under the job's
-- advisory lock, and any other claim that sees a live row is told there is no
-- work yet instead.
--
-- `started_at` exists for the crash case. If the process dies mid-transition
-- the row stays behind with no artifact to show for it, and the job would stall
-- forever on a transition nobody is running; a claim that finds a row older
-- than the takeover timeout with no artifact restarts it (see
-- leave_gen::next_step). `completed_at` is set when the artifact row is
-- written, so a stalled or repeated transition is a query rather than a guess.
CREATE TABLE leave_generation_transitions (
    job_id       UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation   INT NOT NULL,
    started_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    -- How many times this generation's transition has been started. Above 1
    -- means a takeover happened, which is worth seeing.
    attempts     INT NOT NULL DEFAULT 1 CHECK (attempts >= 1),
    PRIMARY KEY (job_id, generation)
);

-- Ratings
--
-- Ratings are siloed from job control flow entirely: nothing below is read
-- while dispatching, claiming, validating or completing a task, and nothing
-- above (jobs, the two game config tables, game_results) mentions a rating.
-- The coupling runs one way -- the fit reads finished game_results -- so a
-- rating can never affect whether a job stops. SPRT stays on the job config
-- tables where it belongs: it is a per-job stopping rule, not a measurement.

-- A rating pool is a set of player configs whose ratings are comparable, plus
-- the game conditions that make them so.
--
-- Scoped by (variant, letterdist, layout) because a rating is only meaningful
-- against fixed conditions: pooling a wordsmog job with a classic one, or two
-- different letter distributions, produces a number describing no game anyone
-- played. Only game_pairs jobs matching a pool's scope are eligible evidence
-- for it. (Lexicon is deliberately *not* part of the scope: it lives on the
-- player config, and two configs on different lexicons playing each other is a
-- meaningful comparison.)
CREATE TABLE rating_pools (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL UNIQUE,
    variant       TEXT NOT NULL,
    letterdist_id UUID NOT NULL REFERENCES input_data(id),
    layout_id     UUID NOT NULL REFERENCES input_data(id),
    -- The fixed point every other rating is measured against. Ratings are only
    -- identifiable up to an additive constant, so exactly one player config
    -- must be pinned; the static bot at 2000 is the convention.
    anchor_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    anchor_rating DOUBLE PRECISION NOT NULL DEFAULT 2000,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (variant, letterdist_id, layout_id, name)
);

-- Which player configs are rated in a pool. Membership is the admin's lever:
-- not every player config belongs in a rating, and a config that is added or
-- removed causes the whole pool to be refit rather than patched, since a batch
-- fit has no per-player history to unwind.
--
-- Removal is soft (the row goes, the games stay in game_results), so
-- re-adding a config costs nothing but a recompute. Note that removing a
-- config also removes its games as *evidence*, which moves everyone else's
-- rating -- that is correct, not a bug, and the reason a removal triggers a
-- full refit.
CREATE TABLE rating_pool_members (
    pool_id          UUID NOT NULL REFERENCES rating_pools(id) ON DELETE CASCADE,
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    added_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    added_by         UUID REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (pool_id, player_config_id)
);

-- One fit. Ratings are snapshotted per run rather than mutated in place, which
-- is what makes "why did this rating change?" answerable and gives the ratings
-- page a time axis at no extra cost.
CREATE TABLE rating_runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_id       UUID NOT NULL REFERENCES rating_pools(id) ON DELETE CASCADE,
    computed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Why this run happened: 'membership' (an admin added or removed a config),
    -- 'evidence' (new results arrived), or 'manual'.
    trigger       TEXT NOT NULL,
    method        TEXT NOT NULL DEFAULT 'bradley_terry_mm',
    -- Fit provenance. A run that did not converge is still stored and still
    -- displayed, flagged: hiding it would leave the page silently stale.
    iterations    INT NOT NULL,
    converged     BOOLEAN NOT NULL,
    -- How much evidence went in, so a run can be compared to its predecessor
    -- without re-reading game_results.
    pairs_used    BIGINT NOT NULL,
    jobs_used     INT NOT NULL
);

CREATE INDEX rating_runs_pool_idx ON rating_runs (pool_id, computed_at DESC);

-- The ratings themselves: one row per player config per run. This is the only
-- table in the schema that holds a rating.
CREATE TABLE player_config_ratings (
    run_id           UUID NOT NULL REFERENCES rating_runs(id) ON DELETE CASCADE,
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    rating           DOUBLE PRECISION NOT NULL,
    -- Approximate Elo standard error. Wide bars are the honest signal that a
    -- config has barely played, or has only played opponents far from its own
    -- strength; the page shows them next to the rating for that reason.
    stderr           DOUBLE PRECISION NOT NULL,
    pairs_played     BIGINT NOT NULL,
    -- FALSE when no chain of games connects this config to the pool's anchor.
    -- Ratings are identifiable only relative to the anchor, so such a config's
    -- number comes from the fit's prior alone and means nothing; it is shown as
    -- unrated rather than as a confident 1500.
    connected_to_anchor BOOLEAN NOT NULL,
    is_anchor        BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (run_id, player_config_id)
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

-- No foreign keys, deliberately. The log is append-only and has to outlive
-- what it describes: the census rows written by delete_job and delete_user
-- exist precisely to be read after the job or user is gone. A foreign key
-- here either blocks those deletions outright (NO ACTION -- every job has a
-- job.created row, every user a user.registered row) or rewrites history
-- (SET NULL / CASCADE).
CREATE TABLE audit_log (
    id              BIGSERIAL PRIMARY KEY,
    action          TEXT NOT NULL,
    actor_user_id   UUID,
    actor_anon_uuid UUID,
    target_type     TEXT,
    target_id       TEXT,
    -- Typed extra-context columns (replace JSONB metadata)
    job_id          UUID,                           -- task/result events
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
-- (task_id, submitted_at) rather than task_id alone: the per-task "first
-- accepted result" read that every aggregate uses orders on both.
CREATE INDEX        game_results_task_idx     ON game_results (task_id, submitted_at);
CREATE INDEX        leave_records_task_idx    ON leave_records (task_id);
CREATE INDEX        position_records_task_idx ON position_analysis_records (task_id);
CREATE INDEX        audit_log_created_idx     ON audit_log (created_at DESC);
CREATE INDEX        audit_log_job_idx         ON audit_log (job_id);

-- Drives claim-time rack selection: "the racks furthest from target in this generation".
CREATE INDEX leave_rack_progress_pick_idx
    ON leave_rack_progress (job_id, generation, occurrence_count);
