-- Users

CREATE TABLE users (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Unique whatever its case: users_username_lower_idx, below, which a
    -- plain UNIQUE here only duplicated (an index write per user update).
    username             TEXT NOT NULL,
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
    -- Completed claims by this account, and when the last one landed. Running
    -- totals rather than a COUNT over task_claims: the contributor lists order
    -- by this, so counting it meant computing every user's whole history before
    -- a LIMIT could apply. Maintained in the submit transaction.
    --
    -- Unlike the counters on `jobs`, this one spans jobs, so it is not enough
    -- to zero it when a job goes: purge_job and delete_job decrement it by what
    -- they are about to destroy. `last_completed_at` is deliberately NOT
    -- rewound by those, since finding the new maximum means the scan the
    -- counter exists to avoid; it only ever moves forward, and is a display
    -- figure.
    tasks_completed      BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    -- The rest of the contribution, kept the same way and given back the same
    -- way: the time each accepted claim was held, claim to submission, in
    -- milliseconds (MAGPIE reports no CPU or thread counts, so this is the
    -- only measure of time there is); and the move generations those claims
    -- performed, as MAGPIE counts and reports them with each result -- every
    -- call to its move generator, in sims, endgame and pre-endgame searches
    -- and autoplay alike, on every thread. Movegens are the measure of work
    -- done, independent of how fast a machine is or how long it held a
    -- claim, and what the contributor list ranks by. Summed from each claim's
    -- own figure (`task_claims.movegens`), every claim counted: unlike a job's
    -- progress, which counts an opening rack once however often consensus has
    -- it analysed again, a contributor did the work of each claim.
    -- Integer milliseconds rather than fractional seconds so that what a purge
    -- takes back is exactly what the submissions added.
    compute_ms           BIGINT NOT NULL DEFAULT 0 CHECK (compute_ms >= 0),
    movegens             BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    last_completed_at    TIMESTAMPTZ,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Serves /api/users, which ranks accounts by contribution. Partial because a
-- deleted account is never listed.
CREATE INDEX users_contribution_idx ON users (tasks_completed DESC, created_at ASC)
    WHERE deleted_at IS NULL;

-- Serve the account half of /api/workers, one per order that list offers --
-- movegens (its default), compute time and tasks -- with the list's own
-- predicate, so whichever it is sorted by it is the same contributors (see
-- anonymous_workers_contribution_idx). A deleted account keeps its place
-- there: its work was done, and it is listed under its anonymized name.
CREATE INDEX users_worker_rank_idx ON users (tasks_completed DESC, id)
    WHERE tasks_completed > 0;
CREATE INDEX users_worker_compute_idx ON users (compute_ms DESC, id)
    WHERE tasks_completed > 0;
CREATE INDEX users_worker_movegens_idx ON users (movegens DESC, id)
    WHERE tasks_completed > 0;

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

-- Confirmation and reset look a token up by its hash, and the stale-account
-- release (and a user's delete) reaches both tables by user through the
-- cascade; neither table is reaped, so each was a sequential scan growing
-- without bound, on unauthenticated routes.
CREATE INDEX email_confirmations_code_idx   ON email_confirmations (code_hash);
CREATE INDEX email_confirmations_user_idx   ON email_confirmations (user_id);
CREATE INDEX password_reset_tokens_hash_idx ON password_reset_tokens (token_hash);
CREATE INDEX password_reset_tokens_user_idx ON password_reset_tokens (user_id);

-- One account per username whatever its case: "Josh" and "josh" side by side
-- on a public leaderboard is an impersonation. Login matches the same way, so
-- whoever registered "Josh" can sign in as "josh"; this index serves it.
CREATE UNIQUE INDEX users_username_lower_idx ON users (lower(username));


CREATE TABLE api_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_hash     TEXT NOT NULL UNIQUE,
    label        TEXT,
    is_active    BOOLEAN NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);
-- A user's keys: the key list, the hundred-key check, and the cascade when a
-- user is deleted or an expired unconfirmed account is released.
CREATE INDEX api_keys_user_idx ON api_keys (user_id);
-- Enforce the 100-key limit per user at the application layer, not via a DB constraint.

-- Workers

CREATE TABLE anonymous_workers (
    uuid          UUID PRIMARY KEY,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Any request from this identity touches this, at most once a minute: it
    -- answers "is this worker still around".
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The contribution counters, mirroring users.tasks_completed, compute_ms,
    -- movegens and last_completed_at; see there for what
    -- each counts, why they are counters and who decrements them.
    -- `last_completed_at` is distinct from `last_seen_at` above: one is the
    -- last task finished, the other is the last request of any kind, and the
    -- contributor list shows the first.
    tasks_completed   BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    compute_ms        BIGINT NOT NULL DEFAULT 0 CHECK (compute_ms >= 0),
    movegens          BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    last_completed_at TIMESTAMPTZ
);

-- Serve the anonymous half of /api/workers, which merges both kinds of
-- identity in one ranking, one per order it offers (users_worker_*_idx is the
-- other half). Partial: an identity that has completed nothing is not a
-- contributor and is not listed, whatever the list is sorted by.
CREATE INDEX anonymous_workers_contribution_idx
    ON anonymous_workers (tasks_completed DESC, uuid) WHERE tasks_completed > 0;
CREATE INDEX anonymous_workers_compute_idx
    ON anonymous_workers (compute_ms DESC, uuid) WHERE tasks_completed > 0;
CREATE INDEX anonymous_workers_movegens_idx
    ON anonymous_workers (movegens DESC, uuid) WHERE tasks_completed > 0;

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

-- One ban per identity. A second row for an identity already banned carries no
-- information -- enforcement is an EXISTS, so the second reason is never read
-- -- and it breaks unban, which deletes by row id: an admin lifts a ban, one
-- row goes, the identity stays banned, and nothing says why. A duplicate is a
-- 409 through the usual unique-violation mapping instead. "Ban again with a
-- different reason" survives as unban-then-ban, which the audit log records as
-- both halves.
CREATE UNIQUE INDEX worker_bans_user_idx ON worker_bans (user_id)
    WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX worker_bans_anon_idx ON worker_bans (anon_uuid)
    WHERE anon_uuid IS NOT NULL;

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
    -- Object-store key for the bytes of the roles the server *builds* from, as
    -- opposed to the ones it parses. A reference wordmap needs the .kwg and a
    -- reference rack info table the .klv2 as well (see `derived_data`), so
    -- those two go to the object store rather than into `content`: a 6 MB
    -- lexicon and a 3.7 MB KLV in a row is a different proposition from a
    -- 489-byte distribution, there are fifty of each per tarball, and nothing
    -- queries their contents.
    --
    -- Keyed by digest, so the same bytes under two paths are one object and a
    -- file unchanged between two tarballs is uploaded once. NULL for every
    -- other role, and for a row whose bytes were never stored -- a derived
    -- build from one of those fails naming the remedy rather than finding a
    -- missing key. Not an equivalence CHECK like `content` above, because
    -- nothing stops a row being written by something other than the import.
    object_key   TEXT,
    imported_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    imported_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    UNIQUE (path, sha256)
);

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
    -- 'nothing_new': staged, every file already known, so there is nothing
    -- to confirm; it goes there straight from 'running' rather than waiting
    -- a day in 'staged' to be expired.
    state          TEXT NOT NULL DEFAULT 'running'
                   CHECK (state IN ('running', 'staged', 'nothing_new', 'confirmed',
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
    -- Likewise for kwg/klv entries, whose bytes go to the object store while
    -- the archive is still in memory. An import the admin then cancels leaves
    -- an object nobody references, which is keyed by digest and so is exactly
    -- what the next import of the same file would have uploaded anyway.
    object_key  TEXT,
    PRIMARY KEY (import_id, path, sha256)
);

-- Derived files: the wordmaps, rack info tables and word info tables the
-- server builds a reference copy of, and the hash a worker has to reproduce.
--
-- None of them is ever shipped -- 179 MB and 1.9 GB for a CSW24 wordmap and
-- rack info table -- so every machine that needs one builds it from files it
-- already has. What travels instead is the SHA-256 the server's own pinned
-- MAGPIE got from the same inputs: a worker builds its own copy and uses it
-- only if the bytes agree, and declines the task otherwise. See README.md,
-- "MAGPIE on the server".
--
-- The key is the whole identity of the file rather than a surrogate, because
-- what makes two derived files the same file is that they were built from the
-- same inputs by the same builder. A wordmap and a word info table depend on a
-- .kwg and the letter distribution they are built against; a rack info table
-- depends on a .klv2 as well, because its entries carry precomputed leave
-- values.
--
-- `builder` is separate from the MAGPIE version on purpose. A CSW24 wordmap
-- built in December 2025 and one built nine months later differ in 72,852,152
-- bytes with the same inputs and the same wordmap format version: the builder
-- changed and the format did not have to. MAGPIE carries WMP_BUILDER_VERSION
-- and RIT_BUILDER_VERSION for exactly this, a test pins their output so a
-- change cannot pass without bumping them, and the server asks the binary it
-- runs (`magpie builders`) rather than being told in configuration.
CREATE TABLE derived_data (
    role          TEXT NOT NULL CHECK (role IN ('wmp','rit','wit')),
    -- What the worker loads the file as. A wordmap's and a word info table's
    -- is its lexicon's name; a rack info table's is '<lexicon>.<leaves>',
    -- because a table belongs to a (.kwg, .klv2) pair and two jobs on CSW24
    -- with different leaves must not share one.
    name          TEXT NOT NULL,
    builder       TEXT NOT NULL,          -- 'wmp-1', 'rit-1', 'wit-1'
    kwg_id        UUID NOT NULL REFERENCES input_data(id),
    -- NULL for a wordmap and a word info table, which are built from the
    -- lexicon alone. The partial
    -- unique indexes below are what make (role, name, builder, kwg, NULL) a key
    -- rather than a duplicate waiting to happen: in a UNIQUE constraint two
    -- NULLs are distinct, so a plain UNIQUE would let a wordmap be queued
    -- twice.
    klv_id        UUID REFERENCES input_data(id),
    letterdist_id UUID NOT NULL REFERENCES input_data(id),
    state         TEXT NOT NULL DEFAULT 'pending'
                  CHECK (state IN ('pending','building','built','failed')),
    -- Set exactly when state = 'built'. The equivalence is a constraint rather
    -- than a convention because dispatch reads this hash: a row that says
    -- 'built' with no hash would be dispatched as if it had one.
    sha256        TEXT CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    bytes         BIGINT CHECK (bytes >= 0),
    -- The instruction-set target the building MAGPIE was compiled for, e.g.
    -- 'nehalem'. Recorded and reported, never compared: measurement says these
    -- builders' output does not depend on it, and a contributor who builds
    -- from source should not be locked out on the strength of a field. If that
    -- ever stops being true, this column is the evidence.
    build_target  TEXT,
    -- Why the last attempt failed, shown to the admin verbatim.
    error         TEXT,
    -- Taken by the builder task when it starts a row, so a second builder does
    -- not start the same three-minute build. A lease rather than a plain flag:
    -- a builder that dies leaves 'building' behind forever otherwise.
    leased_until  TIMESTAMPTZ,
    attempts      INT NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    built_at      TIMESTAMPTZ,
    CONSTRAINT derived_data_built_has_hash CHECK (
        (state = 'built') = (sha256 IS NOT NULL AND bytes IS NOT NULL)
    ),
    -- A wordmap and a word info table are built from the lexicon and the
    -- distribution; a rack info table additionally from the leaves. A wmp or
    -- wit row carrying a klv_id would be claiming a dependency it does not
    -- have.
    CONSTRAINT derived_data_inputs_match_role CHECK (
        (role = 'rit') = (klv_id IS NOT NULL)
    )
);

-- The identity of a derived file, one index per role. Partial indexes because
-- a wordmap's and a word info table's klv_id is NULL and NULLs are distinct in a
-- UNIQUE constraint, which would silently permit duplicate wordmap rows.
CREATE UNIQUE INDEX derived_data_wmp_idx
    ON derived_data (name, builder, kwg_id, letterdist_id)
    WHERE role = 'wmp';
CREATE UNIQUE INDEX derived_data_rit_idx
    ON derived_data (name, builder, kwg_id, klv_id, letterdist_id)
    WHERE role = 'rit';
CREATE UNIQUE INDEX derived_data_wit_idx
    ON derived_data (name, builder, kwg_id, letterdist_id)
    WHERE role = 'wit';

-- The builder task's queue: oldest request first, so a job that has been
-- waiting is not starved by one created since.
CREATE INDEX derived_data_queue_idx
    ON derived_data (requested_at)
    WHERE state IN ('pending','building');

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
    -- What the admin called it, shown first wherever jobs are listed. '' for
    -- a job created without one (through the API; the form asks for it).
    name       TEXT NOT NULL DEFAULT '' CHECK (char_length(name) <= 100),
    job_type   job_type NOT NULL,
    -- Every active job's share of the fleet: the scheduler hands each claim
    -- to the active job furthest behind
    -- `(claims_issued - claims_baseline) / allocation`, and the active jobs
    -- may allocate at most 100% between them. There is no priority: a job
    -- that should get nothing for now is at 0%, and a job that should get
    -- everything is the only one above 0%.
    allocation INT NOT NULL DEFAULT 0 CHECK (allocation BETWEEN 0 AND 100),
    -- Jobs start inactive at 0%. The allocation is the only switch: setting
    -- one above 0% activates a job and setting it to 0% deactivates it, so
    -- `inactive` and 0% are one state rather than two that could disagree --
    -- an active job at 0% was on offer to nobody while every page called it
    -- running. A completed job is not active, so it holds 0% too: nothing
    -- of its old share is kept to come back to (see `jobs_allocation_is_status`).
    status     job_status NOT NULL DEFAULT 'inactive',
    -- SET NULL if the creating admin's account is deleted.
    created_by           UUID REFERENCES users(id) ON DELETE SET NULL,
    -- Settings every job type has, regardless of what it does. The lexicon is
    -- NOT here: it lives on the player (player_configs.kwg_id), because MAGPIE
    -- scopes it per player and two players may run different ones.
    variant       TEXT NOT NULL,                            -- 'classic' | 'wordsmog'; a rules setting, not a file
    letterdist_id UUID NOT NULL REFERENCES input_data(id),  -- one per job: MAGPIE takes one -ld for the whole game
    layout_id     UUID NOT NULL REFERENCES input_data(id),  -- 'standard15' unless a job says otherwise
    -- Run-wide MAGPIE settings every request states: the bingo bonus, part of
    -- every play's score, and the simulation cutoff. Written at creation from
    -- MAGPIE's defaults (backend/src/magpie_defaults.rs) rather than left for
    -- each worker's build to supply, so a task means the same thing on every
    -- MAGPIE release. Leave generation states only the bingo bonus: its bot
    -- does not simulate.
    -- The bonus is bounded because the plausibility rules' score bounds are
    -- absolute and assume an ordinary one: a typo of 5000 for 50 would
    -- refuse every honest batch. Real variants use 0 to 50.
    bingo_bonus   INT NOT NULL CHECK (bingo_bonus BETWEEN 0 AND 500),  -- -bb
    sim_cutoff    DOUBLE PRECISION NOT NULL CHECK (sim_cutoff >= 0 AND sim_cutoff <= 100),  -- -cutoff
    -- Minimum MAGPIE version workers must have to execute tasks for this job,
    -- as sortable parts. Semver in TEXT compares lexically, where '1.10.0' <
    -- '1.9.0' -- a bug that appears only once a minor version reaches double
    -- digits, i.e. long after it is written.
    --
    -- Not nullable: every job pins input data, and a client too old to
    -- understand expected_data contributes unverified rather than declining,
    -- so "no floor" is not a state worth being able to express. 0.1.1 is
    -- the `birdtest-contribute` version the backend image pins; the branch's
    -- version moves whenever a change can alter what a task computes, and the
    -- floor moves with it. The default here is the same value as the server's
    -- MIN_MAGPIE_VERSION, which create_job writes explicitly; the two are kept
    -- equal so a row written any other way (a restore, a hand insert) does not
    -- floor a job below the server.
    min_magpie_major INT NOT NULL DEFAULT 0 CHECK (min_magpie_major >= 0),
    min_magpie_minor INT NOT NULL DEFAULT 1 CHECK (min_magpie_minor >= 0),
    min_magpie_patch INT NOT NULL DEFAULT 1 CHECK (min_magpie_patch >= 0),
    -- Every claim ever issued for this job, abandoned and declined ones
    -- included: the deficit the scheduler orders on. Kept as a counter rather
    -- than counted, because counting task_claims on every claim request costs
    -- time proportional to the job's whole history. Only ever incremented,
    -- except by a purge, which deletes the claims it counts.
    claims_issued   BIGINT NOT NULL DEFAULT 0 CHECK (claims_issued >= 0),
    -- Where this job's share is measured *from*. The scheduler orders on
    -- `(claims_issued - claims_baseline) / allocation`, and the baseline is
    -- reset -- on activation and on an allocation change (a purge leaves the
    -- job inactive, to be reset when it is activated again) -- so that
    -- the job's ratio equals the lowest ratio among the other jobs being
    -- served (see `last_claimed_at` below): it joins at parity and takes its
    -- share from then on.
    --
    -- Without it the deficit was measured over a job's whole life, so a job
    -- activated today beside one that had issued two million claims took
    -- *every* claim until it had issued two million of its own, and the older
    -- job got nothing for as long as that took; a purge (which zeroes
    -- claims_issued) and a raised allocation did the same. May be negative: a
    -- job with no claims yet joining a busy fleet is credited the claims that
    -- put it level.
    claims_baseline BIGINT NOT NULL DEFAULT 0,
    -- When this job last issued a claim; NULL until it has. It rides the
    -- `UPDATE jobs` every claim already makes, so it costs nothing to keep.
    -- `join_at_parity` reads it to tell the jobs that are *being served* from
    -- the ones that are merely on offer: a job whose derived files are still
    -- building, or that the fleet cannot run yet, stands still while the others
    -- climb, and a newcomer put level with *it* then took every claim from the
    -- jobs that were actually running until it had caught up with them.
    last_claimed_at TIMESTAMPTZ,
    -- The match test's verdict a games or game-pairs job was completed on, as
    -- the finish check saw it, with player 1's score interval and the units it
    -- had then: NULL for every other job, for one completed any other way (by
    -- an admin, or at its cap in the claim path), and for one that runs no
    -- test -- there is no verdict to keep, and its `job.completed` audit row
    -- says `reached_target` instead. The live figures are recomputed from
    -- every accepted result, and the claims in flight when a job completes are
    -- still played and accepted -- so without this the page of a job that
    -- decided could drift back to "running" with no record anywhere of the
    -- decision that stopped it. A purge clears it.
    test_decided_status TEXT CHECK (test_decided_status IN
                            ('player1_better', 'player2_better', 'inconclusive')),
    test_decided_lower  DOUBLE PRECISION,
    test_decided_upper  DOUBLE PRECISION,
    test_decided_units  BIGINT,
    -- Tasks of this job a worker stopped at the time limit
    -- (`settings.max_task_seconds`) and handed back, declining them
    -- `time_limit`: the job page says how many, since the cure is a smaller
    -- batch. And how many of those came in a row with no task of the job
    -- completed between, which an accepted result zeroes: at three the job is
    -- set aside -- inactive at 0%, with `set_aside_reason` saying why -- since
    -- a job whose one unit always outlasts the limit would otherwise be
    -- handed out, run for the limit and handed back for ever. Giving it an
    -- allocation again starts the run afresh and clears the reason; a purge
    -- zeroes all three.
    time_limit_declines BIGINT NOT NULL DEFAULT 0 CHECK (time_limit_declines >= 0),
    time_limit_streak   INT NOT NULL DEFAULT 0 CHECK (time_limit_streak >= 0),
    -- Why the server switched the job off, when it did; read only while the
    -- job is inactive.
    set_aside_reason    TEXT,
    -- The invariant above, for every status: active exactly when above 0%,
    -- which holds an inactive or completed job at 0%.
    CONSTRAINT jobs_allocation_is_status CHECK ((status = 'active') = (allocation > 0)),
    CONSTRAINT jobs_test_decided_together CHECK (
        (test_decided_status IS NULL) = (test_decided_lower IS NULL)
        AND (test_decided_status IS NULL) = (test_decided_upper IS NULL)
        AND (test_decided_status IS NULL) = (test_decided_units IS NULL)
    ),
    -- Progress totals the dashboard reads, maintained in the submit transaction
    -- rather than counted on read (PLAN.md, "What these reads cost"), once per
    -- accepted result: a task has one slot, so one result.
    --
    -- games_completed counts GAMES for both games and game_pairs; a pairs job's
    -- unit count is half of it, exactly as the read derived it. racks_analyzed
    -- counts distinct opening racks with an accepted analysis, which is a plain
    -- sum because each task covers its own disjoint slice of the rack space.
    --
    -- Neither is authoritative for anything that decides: the match test still reads
    -- game_results, so a drifted counter shows a wrong number on a page and
    -- cannot stop a job early. A purge zeroes them; a partial restore
    -- recomputes them (RUNBOOK 2.3).
    games_completed BIGINT NOT NULL DEFAULT 0 CHECK (games_completed >= 0),
    racks_analyzed  BIGINT NOT NULL DEFAULT 0 CHECK (racks_analyzed >= 0),
    -- An opening-rack job's racks that need no more analysis (see
    -- job_opening_rack_config's consensus), and those of them settled by
    -- reaching their most analyses without a consensus. Racks settle at their
    -- first analysis when the job wants one, so for such a job the first is
    -- racks_analyzed. The job is done once every rack is settled.
    racks_settled            BIGINT NOT NULL DEFAULT 0 CHECK (racks_settled >= 0),
    racks_without_consensus  BIGINT NOT NULL DEFAULT 0 CHECK (racks_without_consensus >= 0),
    -- Tasks created, and tasks that reached `completed`. The job list shows
    -- both for every job on the page, and counting them meant two COUNT(*)s
    -- over `tasks` per job per page view -- linear in each job's whole history,
    -- on the site's index. Same reasoning and same caveats as the two counters
    -- above: nothing decides anything from them, a purge zeroes them, and a
    -- partial restore recomputes them (RUNBOOK 2.3).
    tasks_total     BIGINT NOT NULL DEFAULT 0 CHECK (tasks_total >= 0),
    tasks_completed BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    -- The move generations the job's accepted claims reported, every claim's
    -- own (`task_claims.movegens`): the job page's figure, and summed by job
    -- type, the Contributors page's. Added by the same submission, in the
    -- same `UPDATE jobs` as the counters above, that adds the claim to its
    -- contributor's `movegens` -- so the jobs' total and the contributors'
    -- agree, and a purge (which zeroes it) or a delete (which takes the row)
    -- takes away exactly what it gives the contributors back. A partial
    -- restore recomputes it (RUNBOOK 2.3).
    movegens        BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    -- When a result was last accepted for the job, to the minute (the
    -- submission that stores one sets it at most once a minute). The job
    -- list's `stalled` flag asks "none in a day"; answered from the claims, it
    -- joined every task of the job to the day's completions -- growing with
    -- the job's whole history, on every list view. Display only; a purge
    -- clears it, a partial restore recomputes it (RUNBOOK 2.3).
    last_completed_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at    TIMESTAMPTZ
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
--   player; 'score' = sort by raw score only, for a static player. A simming player's candidates
--   are the top plays by equity (autoplay generates them so whatever the row says), so a simmer
--   is always 'equity': config creation refuses 'score' for one. Both static and simming
--   players are valid in games/game_pairs jobs.
--
-- Simulation columns are all NULL for a static (no-sim) player.

CREATE TABLE player_configs (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL UNIQUE CHECK (char_length(name) <= 100),  -- e.g. "simmer-NWL23-4ply"
    recorder_type    TEXT NOT NULL,         -- 'best' | 'equity' | 'all'  (-r1 / -r2)
    sort_strategy    TEXT NOT NULL,         -- 'equity' | 'score'  (-s1 / -s2)
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
    -- Every setting a task request states is stated here, never NULL for
    -- "MAGPIE's default": creation writes MAGPIE's value into the row
    -- (backend/src/magpie_defaults.rs), so a config plays the same on every
    -- MAGPIE release, and MAGPIE refuses a request that leaves one out. The
    -- exception is a static player's simulation settings, which are NULL
    -- because nothing reads them; the CHECK at the end of the table holds the
    -- two sets apart.
    --
    -- Two pairs of "how much to compute" / "how much to report". MAGPIE
    -- generates plays and plies, then displays a subset of each; birdtest
    -- stores exactly what is displayed.
    num_plies          INT NOT NULL CHECK (num_plies >= 0),           -- plies to simulate; 0 is static (-pl1 / -pl2)
    num_plies_recorded INT NOT NULL CHECK (num_plies_recorded >= 1),  -- plies to report (shplies)
    -- plays to generate and simulate (-np1 / -np2). Stated for a static player
    -- too: an opening-rack analysis sizes its move list from it.
    num_plays          INT NOT NULL CHECK (num_plays >= 1),
    -- plays to report (maxnumdplays). Required: "keep everything" is unbounded
    -- per position, and the worker and the server must agree on the number.
    num_plays_recorded INT NOT NULL CHECK (num_plays_recorded >= 1),
    -- Simulation parameters (all NULL for a static player, all set for a simmer)
    max_iterations   INT,                   -- -i1 / -i2
    stopping_pct     DOUBLE PRECISION,      -- -sc1 / -sc2 (0–100)
    use_inference    BOOLEAN,               -- -si1 / -si2
    time_limit_secs  INT,                   -- -tl1 / -tl2
    -- The remaining MAGPIE options that can affect how a player plays.
    -- Exhaustive on purpose: MAGPIE takes nothing a request leaves out from its
    -- own defaults, so a setting missing here is a task no worker will run.
    use_wordmap          BOOLEAN NOT NULL,   -- -w1 / -w2
    use_rit               BOOLEAN NOT NULL,  -- rack info table            (-rit1 / -rit2)
    use_wit               BOOLEAN NOT NULL,  -- word info table (-wit1 / -wit2)
    -- More simulation parameters: NULL for a static player, set for a simmer.
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
    -- The same holds of num_plays_recorded and num_plies_recorded above in a
    -- games or game-pairs job that captures positions: MAGPIE reads player
    -- 1's for both seats, so job creation requires the two to agree.
    movegen_margin         DOUBLE PRECISION NOT NULL, -- move-gen equity margin for 'equity' recording (-mmargin)
    -- Endgame and pre-endgame (PEG) solving, read only by games and game-pairs
    -- jobs: an opening rack never reaches a small bag, and a leave job's games
    -- end before one (job creation refuses a leave player that solves).
    --
    -- endgame_plies is the switch for both: 0 solves nothing, because PEG
    -- scores its emptier scenarios with endgame solves. Above 0 the player
    -- solves the endgame to that depth once the bag is empty, and runs PEG
    -- while the bag holds 1..peg_max_bag tiles (0 = no PEG). Neither has a time
    -- limit: the depth and the schedule bound the work, so a player is as
    -- strong on a slow machine as on a fast one. Like a simulation, a solve is
    -- multithreaded and so not reproducible run to run.
    endgame_plies          INT NOT NULL DEFAULT 0,    -- -eplies1 / -eplies2
    peg_max_bag            INT NOT NULL DEFAULT 0,    -- -pegbag1 / -pegbag2
    -- The PEG schedule: NULL when peg_max_bag is 0, all set when it is not.
    peg_stage_top_k        INT[],                     -- survivors per halving stage (-pegtopk1 / -pegtopk2)
    peg_scenario_stride    INT,                       -- 1 = full enumeration (-pegstride1 / -pegstride2)
    peg_opp_model          TEXT,                      -- 'rational' | 'pessimistic' (-pegpess1 / -pegpess2)
    peg_nested             BOOLEAN,                   -- nested lookahead (-pegnested1 / -pegnested2)
    -- The nested lookahead's knobs: set exactly when peg_nested is.
    peg_nested_cand_caps   INT[],                     -- per-level candidate caps (-pegncaps)
    peg_nested_max_depth   INT,                       -- nested pegs before a rollout (-pegndepth)
    peg_nested_strides     INT[],                     -- stride per inner bag size 1..4 (-pegnstrides)
    -- SET NULL, like jobs.created_by: a config outlives the admin who made it.
    created_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A player simulates exactly when it has plies (MAGPIE decides on
    -- num_plies > 0). A simmer states every simulation setting, including the
    -- win% model it loads; a static player states none, since nothing reads
    -- them and a request carrying them would suggest otherwise.
    CONSTRAINT player_configs_simulation_settings CHECK (
        (num_plies > 0
            AND winpct_id IS NOT NULL AND max_iterations IS NOT NULL
            AND stopping_pct IS NOT NULL AND use_inference IS NOT NULL
            AND time_limit_secs IS NOT NULL AND min_play_iterations IS NOT NULL
            AND threshold IS NOT NULL AND sampling_rule IS NOT NULL
            AND inference_margin IS NOT NULL AND utility_w_winpct IS NOT NULL
            AND utility_w_spread IS NOT NULL AND utility_spread_scale IS NOT NULL)
        OR
        (num_plies = 0
            AND winpct_id IS NULL AND max_iterations IS NULL
            AND stopping_pct IS NULL AND use_inference IS NULL
            AND time_limit_secs IS NULL AND min_play_iterations IS NULL
            AND threshold IS NULL AND sampling_rule IS NULL
            AND inference_margin IS NULL AND utility_w_winpct IS NULL
            AND utility_w_spread IS NULL AND utility_spread_scale IS NULL)
    ),
    -- The solver settings nest: PEG needs the endgame, the PEG schedule is
    -- stated exactly when PEG runs, and the nested knobs exactly when nested
    -- lookahead is on. MAGPIE refuses a request that breaks any of these.
    CONSTRAINT player_configs_solver_settings CHECK (
        endgame_plies BETWEEN 0 AND 25
        AND peg_max_bag BETWEEN 0 AND 4
        AND (endgame_plies > 0 OR peg_max_bag = 0)
        AND (
            (peg_max_bag = 0
                AND peg_stage_top_k IS NULL AND peg_scenario_stride IS NULL
                AND peg_opp_model IS NULL AND peg_nested IS NULL)
            OR
            (peg_max_bag > 0
                AND peg_stage_top_k IS NOT NULL AND peg_scenario_stride IS NOT NULL
                AND peg_opp_model IN ('rational', 'pessimistic')
                AND peg_nested IS NOT NULL)
        )
        AND (peg_nested IS TRUE) = (peg_nested_cand_caps IS NOT NULL)
        AND (peg_nested IS TRUE) = (peg_nested_max_depth IS NOT NULL)
        AND (peg_nested IS TRUE) = (peg_nested_strides IS NOT NULL)
    )
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
    total_racks       BIGINT NOT NULL CHECK (total_racks >= 0),
    -- Consensus: a rack is analysed until enough of its analyses agree on its
    -- best move, each analysis a task of its own. Its consensus is the share
    -- of its analyses whose rank-1 move is the most common rank-1 move. It is
    -- settled once it has at least `min_results_per_rack` analyses and its
    -- consensus is at least `consensus_pct`, or once it has
    -- `max_results_per_rack` analyses (settled without consensus), and not
    -- analysed again unless an admin changes these three (the only settings a
    -- job's config may change after creation), which restates every rack.
    -- One and one is one analysis per rack, which is what a static player
    -- gets: its analyses are deterministic and always agree.
    consensus_pct          DOUBLE PRECISION NOT NULL DEFAULT 100
                           CHECK (consensus_pct > 50 AND consensus_pct <= 100),
    min_results_per_rack   INT NOT NULL DEFAULT 1 CHECK (min_results_per_rack >= 1),
    max_results_per_rack   INT NOT NULL DEFAULT 1
                           CHECK (max_results_per_rack >= min_results_per_rack
                                  AND max_results_per_rack <= 100)
);

CREATE TABLE job_game_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    -- Even, as job creation requires: MAGPIE alternates the first mover
    -- within a task, from player 1, so an odd batch gives player 1 the first
    -- move in more of the job's games (KL-87). The default is the least such
    -- batch, so a row written without one is not an odd one.
    games_per_batch     INT NOT NULL DEFAULT 2,
    -- Whether the job runs the match test (stats/match_test.rs): a confidence
    -- interval for player 1's score that stays valid however often it is
    -- checked, the job stopping once it excludes an even score. Off, it plays
    -- max_games and stops, and min_games and confidence_pct are stored at
    -- their defaults and read by nothing. Off by default: a job that only
    -- wants the games played should not be stopped early by a test it did not
    -- ask for.
    test_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    -- Two finish conditions: a decision (looked for from min_games on) OR reaching max_games.
    min_games           INT NOT NULL,   -- the test is not acted on before this many games are complete
    max_games           INT NOT NULL,   -- job auto-completes at this count regardless of the test
    -- How confident a decision is: the interval's coverage, in percent.
    confidence_pct      DOUBLE PRECISION NOT NULL DEFAULT 95
                        CHECK (confidence_pct > 50 AND confidence_pct < 100),
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE,
    -- How MAGPIE spends its threads on a task (MULTI_THREADING_MODE): 'igp'
    -- gives them all to one game at a time, inside its simulation, which makes
    -- an iteration-bounded simulation reproducible; 'pgp' plays games in
    -- parallel, a thread each. Matters only when a player simulates; a job of
    -- static players runs alike in either. Stated on every request.
    threading_mode      TEXT NOT NULL DEFAULT 'igp' CHECK (threading_mode IN ('igp', 'pgp')),
    -- What `validate_job_body` requires, held here too for a row written any
    -- other way (a script, a fixture, a restore). The stopping rule reads the
    -- counts as unsigned: a negative max_games was a cap no job reached, so it
    -- ran for ever, and a batch of 0 had every claim play the same seed.
    CONSTRAINT job_game_config_counts CHECK (
        games_per_batch >= 1 AND max_games >= 1 AND min_games >= 0
        AND (NOT test_enabled OR min_games BETWEEN 1 AND max_games)
    )
);

CREATE TABLE job_game_pair_config (
    job_id              UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- lexicon, variant and letter distribution live on the job now: the first
    -- on the player configs, the other two on `jobs`.
    player1_config_id   UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id   UUID NOT NULL REFERENCES player_configs(id),
    pairs_per_batch     INT NOT NULL DEFAULT 1,
    -- As on job_game_config: off, the job plays max_pairs and stops.
    test_enabled        BOOLEAN NOT NULL DEFAULT FALSE,
    min_pairs           INT NOT NULL,
    max_pairs           INT NOT NULL,
    confidence_pct      DOUBLE PRECISION NOT NULL DEFAULT 95
                        CHECK (confidence_pct > 50 AND confidence_pct < 100),
    -- Keep the position analyses the worker produces while playing. A worker
    -- analyses a position every turn regardless; this decides whether those are
    -- recorded. Off by default: at ~22.5 turns a game it roughly doubles the
    -- rows a job produces.
    capture_positions   BOOLEAN NOT NULL DEFAULT FALSE,
    -- With capture on, keep only each pair's first divergence: both games'
    -- positions at the first turn the two games play different moves, and
    -- nothing from a pair played identically. Before that turn the two games
    -- are the same game; after it they are two different ones, and the turn
    -- itself is where the players disagree.
    capture_first_divergence BOOLEAN NOT NULL DEFAULT FALSE,
    -- As on job_game_config.
    threading_mode      TEXT NOT NULL DEFAULT 'igp' CHECK (threading_mode IN ('igp', 'pgp')),
    CONSTRAINT job_game_pair_config_divergence_needs_capture
        CHECK (capture_positions OR NOT capture_first_divergence),
    -- As job_game_config_counts, in pairs.
    CONSTRAINT job_game_pair_config_counts CHECK (
        pairs_per_batch >= 1 AND max_pairs >= 1 AND min_pairs >= 0
        AND (NOT test_enabled OR min_pairs BETWEEN 1 AND max_pairs)
    )
);

CREATE TABLE job_leave_config (
    job_id         UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    -- The player the leave-generating bot plays as, in both seats: its lexicon
    -- and wordmap setting are the job's. Its leaves are never loaded -- every
    -- generation's come from the server-built KLV artifact, and generation 1's
    -- is a zeroed one -- and nor is its win% model, since the bot plays
    -- statically. So the job's complete data requirement is this player's kwg
    -- plus the job's letterdist_id and layout_id. Job creation refuses a player
    -- that simulates, sorts on anything but equity, or asks for a rack info
    -- table: leave values are measured from static equity play.
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    -- Games each leave-gen task plays over its forced-rack subset.
    num_iterations INT NOT NULL,
    -- The occurrence target every rack must reach before a generation closes,
    -- one per generation: element g (1-based, as generations are numbered) is
    -- generation g's, and the array's length is how many generations the job
    -- runs before it is complete. MAGPIE's `leavegen 100,200,500,…` shape.
    -- `<= ALL` over an array holding a NULL is NULL, which a CHECK passes, so
    -- the NULL test is separate; and it is indexed by generation, so flat and
    -- starting at 1.
    target_rack_counts INT[] NOT NULL CHECK (
        array_ndims(target_rack_counts) = 1
        AND array_lower(target_rack_counts, 1) = 1
        AND cardinality(target_rack_counts) >= 1
        AND 1 <= ALL (target_rack_counts)
        AND array_position(target_rack_counts, NULL) IS NULL
    ),
    -- Size of the forced-rack subset handed to a single task.
    racks_per_task    INT NOT NULL CHECK (racks_per_task >= 1)
);

-- Exports
--
-- A job's results, as one gzipped NDJSON object in the artifact store. Any job
-- can be exported; is_final says whether this is the completed job's corpus,
-- which is immutable and so built once and reused (the results stream
-- redirects to it), or a snapshot of a job still taking results, which is
-- offered as a download labelled with its time and never served in the
-- finished job's place.
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
    -- A games or game-pairs job that captured positions gets a second object
    -- beside its results: the positions, each with its ranked moves, in the
    -- shape an opening-rack export's lines have. All four are NULL for every
    -- other export. A second artifact rather than tagged lines in the first,
    -- so a consumer of a games job's results never meets a line of another
    -- kind.
    positions_artifact_key TEXT,
    positions_bytes        BIGINT,
    positions_sha256       TEXT,
    positions_row_count    BIGINT,
    -- TRUE when the snapshot the export was read in saw the job completed,
    -- with no claim still open and nothing staged: its final corpus. Decided
    -- inside that snapshot (exports::read_snapshot), not when the export was
    -- requested -- a job exported mid-run can complete before its rows are
    -- read, with its last results still landing. FALSE until built, so an
    -- unfinished or unmarked row is never taken for the final one.
    is_final      BOOLEAN NOT NULL DEFAULT FALSE,
    -- When that snapshot was taken, for the page's "Snapshot as of …".
    snapshot_at   TIMESTAMPTZ,
    error         TEXT,
    requested_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    requested_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at  TIMESTAMPTZ
);

-- The newest ready export for a job, which is what a download resolves to
-- (the newest final one, for the results stream).
CREATE INDEX job_exports_job_idx ON job_exports (job_id, requested_at DESC);

-- One export of a job at a time. Only the page's disabled button stopped a
-- second, and each holds a pool connection for the whole corpus read.
CREATE UNIQUE INDEX job_exports_one_running_idx ON job_exports (job_id) WHERE state = 'running';

-- Tasks

CREATE TYPE task_state AS ENUM ('available', 'claimed', 'completed');

CREATE TABLE tasks (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id               UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    -- The seed the task's games are played from. Every job type plays games,
    -- so every task has one, stated on its request: games and game pairs seed
    -- their batch from it and step by one per game; an opening-rack task's is
    -- the index of its first rack in the job's rack space, and rack i of the
    -- batch is analysed from seed + i; a leave-generation task's is drawn
    -- when the task is created. Stored as signed int64; interpreted as uint64
    -- at the application layer. For games, pairs and opening racks it is also
    -- the cursor that tiles the job's space, which is what the unique index
    -- below serves.
    seed                 BIGINT NOT NULL,
    state                task_state NOT NULL DEFAULT 'available',
    -- Denormalized counters used by SKIP LOCKED selection; avoids per-candidate
    -- join/aggregate. A task has one slot: it is claimed by one worker at a
    -- time and completed by its one accepted result, so each is 0 or 1 -- and
    -- held to it, so a counter that drifted fails the statement that drifted
    -- it rather than, days later, leaving a job that quietly stops dispatching.
    accepted_count       INT NOT NULL DEFAULT 0 CHECK (accepted_count IN (0, 1)),
    active_claim_count   INT NOT NULL DEFAULT 0 CHECK (active_claim_count IN (0, 1)),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at         TIMESTAMPTZ
);

-- Prevent two tasks of one job from playing the same seed: for games, pairs
-- and opening racks that is two workers racing for the same slice of the
-- space, for leave generation a collision between two randomly drawn seeds
-- (which the claim path retries).
CREATE UNIQUE INDEX tasks_seed_unique_idx ON tasks (job_id, seed);

-- Partial indexes to support efficient SKIP LOCKED task selection and timeout
-- reclamation.
--
-- The queue index carries `created_at` rather than `state`, which the partial
-- predicate already fixes: claim-time selection takes the *oldest* available
-- task of a job (`registry::next_available`), so with `state` in the key the
-- planner had to read every available task of the job and sort it.
CREATE INDEX tasks_queue_idx   ON tasks (job_id, created_at) WHERE state = 'available';

-- Individual claims (one row per worker claim: a task's lapsed and declined
-- claims, then the one that completed it)
--
-- claimed_by_user_id carries no ON DELETE clause because a user row is never
-- deleted: account deletion anonymizes it in place (users.deleted_at, and a
-- tombstone username and email) and leaves these rows exactly where they are.
-- Removing them instead would take with them the position analyses keyed to
-- those claims -- an opening-rack batch's, and the in-game positions a games
-- job captured -- and leave-generation occurrences that were folded into
-- per-rack totals and cannot be subtracted back out. See
-- routes::admin::delete_user.

-- 'declined' is distinct from 'abandoned': one is a worker saying "I cannot do
-- this", the other is a claim that lapsed. Only the first is diagnostic.
CREATE TYPE claim_state AS ENUM ('claimed', 'completed', 'abandoned', 'declined');

CREATE TABLE task_claims (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id              UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    -- The task's job, copied at claim time and never changed (a task never
    -- moves between jobs). Without it "this contributor's claims in this job"
    -- -- the public feed's `?worker=` -- meant every claim the contributor ever
    -- made, or every task of the job: seconds for a heavy contributor. With it,
    -- one range of the identity indexes below. No foreign key of its own:
    -- claims go with their task, and their task with its job.
    job_id               UUID NOT NULL,
    claim_token          UUID NOT NULL,
    state                claim_state NOT NULL DEFAULT 'claimed',
    claimed_by_user_id   UUID REFERENCES users(id),
    claimed_by_anon_uuid UUID REFERENCES anonymous_workers(uuid),
    claimed_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- When the task must be done by: `claimed_at` plus `settings.max_task_seconds`
    -- as it stood when the claim was made, which the assignment told the
    -- worker. Past it and a minute's grace the claim lapses whether or not its
    -- worker still heartbeats, and a result for it is refused: a task whose
    -- one unit outlasts the limit (a deep-sim game pair) would otherwise hold
    -- its slot for as long as its worker lived. The claim path always sets
    -- it; the default, the limit's own default, is for a row written any
    -- other way (a script, a test's fixture).
    deadline_at          TIMESTAMPTZ NOT NULL DEFAULT now() + interval '1 hour',
    last_heartbeat_at    TIMESTAMPTZ,
    completed_at         TIMESTAMPTZ,
    -- The move generations this claim's accepted result reported (the
    -- submission's `movegens`), written when it completes (zero until then,
    -- and for a claim that never does). Its contributor's running total adds
    -- this, so a purge can give back exactly what it added by summing the
    -- claims it is about to delete, without reading the results. Every
    -- claim's own, not the task's first result's.
    movegens             BIGINT NOT NULL DEFAULT 0 CHECK (movegens >= 0),
    -- As reported at claim time. What the fleet is actually running, which is
    -- the evidence for raising a job's floor.
    magpie_version       TEXT,
    CONSTRAINT claim_has_single_owner CHECK (
        (claimed_by_user_id IS NOT NULL)::int + (claimed_by_anon_uuid IS NOT NULL)::int = 1
    )
)
-- Room on each page for a claim's heartbeats. A heartbeat changes only
-- `last_heartbeat_at`, which no index covers, so it can be a HOT update -- an
-- in-page rewrite that touches none of this table's eight indexes -- but only
-- if the row's page has space, and claims are appended, so at the default
-- fillfactor of 100 a claim's first heartbeat found its page full and wrote a
-- new entry into every index: two a minute for every claim in flight.
WITH (fillfactor = 85);

-- A task has one slot: one claim holds it, or one claim completed it, never
-- two of either and never both -- a completed task is not handed out again.
-- The claim path keeps it so (a task is selected only while `available`,
-- under its job's dispatch lock and its own row lock), and the counters on
-- `tasks` and the job's `tasks_completed` count on it; this makes a breach,
-- by any two identities, fail at the statement rather than surface as
-- a counter gone wrong. A task has any number of lapsed and declined claims,
-- so those are outside it: a worker that declined a task for missing data and
-- then fixed its data has to be able to claim that task again.
--
-- It replaces two per-identity indexes from when a task had several slots and
-- the thing to stop was one identity taking two of them; one slot covers that
-- case too.
CREATE UNIQUE INDEX task_claims_one_slot_idx
    ON task_claims (task_id) WHERE state IN ('claimed', 'completed');

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
-- (job_id, reported_at): the job list asks whether a job has a gap reported in
-- the last 24 hours, and the admin view groups a job's gaps, so both start at
-- the job. Ordered by time within it, the first question stops at the newest
-- row rather than walking every gap the job ever had.
CREATE INDEX worker_data_gaps_job_idx ON worker_data_gaps (job_id, reported_at DESC);
-- The cascade from a claim. A purge deletes every claim of a job, and without
-- this the lookup was a sequential scan of this table per deleted claim.
CREATE INDEX worker_data_gaps_claim_idx ON worker_data_gaps (claim_id);

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
    -- batch of a job may be short. A task reissuing racks a consensus still
    -- wants (`racks` below) has its seed here instead: the cursor past the end
    -- of the rack space, so rack i of it is analysed from a seed no earlier
    -- analysis used.
    rack_start        BIGINT NOT NULL CHECK (rack_start >= 0),
    rack_count        INT NOT NULL CHECK (rack_count >= 1),
    -- The racks themselves, for a task reissuing racks a consensus still
    -- wants: they are scattered over the space rather than a range of it.
    -- NULL for a task covering a range, which is every task of a job wanting
    -- one analysis per rack.
    racks             TEXT[] CHECK (racks IS NULL OR cardinality(racks) = rack_count),
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
    -- Game pairs only: of the captured positions, keep only each pair's first
    -- divergence (job_game_pair_config.capture_first_divergence).
    capture_first_divergence BOOLEAN NOT NULL DEFAULT FALSE,
    -- seed is also stored on the tasks row; duplicated here for convenience when reading the full request.
    seed              BIGINT NOT NULL,
    num_games         INT NOT NULL DEFAULT 1,
    player1_config_id UUID NOT NULL REFERENCES player_configs(id),
    player2_config_id UUID NOT NULL REFERENCES player_configs(id)
);

CREATE TABLE leave_requests (
    task_id             UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    -- No lexicon or wordmap column: the player config carries both.
    variant             TEXT NOT NULL,
    letter_distribution TEXT NOT NULL,
    board_layout        TEXT NOT NULL,
    generation          INT NOT NULL,
    -- The seed the task's games are played from, chosen when the task is
    -- created, so a reissued task replays it. Stored as signed int64 and
    -- interpreted as uint64, like tasks.seed. Without it a worker seeded from
    -- its own process state, which differs by machine.
    seed                BIGINT NOT NULL,
    forced_racks        TEXT[] NOT NULL,   -- the rack subset this task must force (passed to MAGPIE's rack_list_create)
    num_games           INT NOT NULL,      -- denormalized from job_leave_config.num_iterations
    -- Combined KLV from the previous generation. Never NULL: generation 1 reads
    -- the server-built zeroed KLV at generation-0, so every generation fetches
    -- its leaves the same way and the client has no first-generation branch.
    previous_artifact_key TEXT NOT NULL,
    player_config_id    UUID NOT NULL REFERENCES player_configs(id)
);

-- An opening-rack job's racks: how many analyses each has, its most common
-- rank-1 move and how many analyses ranked it first, and whether it is
-- settled (see job_opening_rack_config). Every opening-rack job keeps them,
-- even one wanting one analysis per rack, which settles each rack at its
-- first: an admin may change its consensus settings later, and its reissues
-- then start from these rows. A rack has a row from its first analysis; the
-- racks still to be reissued are its unsettled rows, fewest analyses first.
CREATE TABLE opening_rack_progress (
    job_id     UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    rack       TEXT NOT NULL,
    results    INT NOT NULL CHECK (results >= 1),
    top_move   TEXT NOT NULL,
    top_count  INT NOT NULL CHECK (top_count BETWEEN 1 AND results),
    settled    BOOLEAN NOT NULL,
    -- Settled by reaching the job's most analyses, its analyses still split.
    without_consensus BOOLEAN NOT NULL CHECK (settled OR NOT without_consensus),
    PRIMARY KEY (job_id, rack)
);

-- What a reissue picks from: a job's unsettled racks, fewest analyses first.
CREATE INDEX opening_rack_progress_unsettled_idx
    ON opening_rack_progress (job_id, results, rack) WHERE NOT settled;

-- Per-rack occurrence progress for each generation of a leave-gen job, one row per
-- full 7-tile rack the distribution can draw (3,199,724 for English), seeded at zero when
-- the generation opens. Updated by `leave_gen::merge_staged`, which folds the accepted
-- results staged in `leave_rack_staging` below -- not by each submission; see there for why.
-- Drives claim-time rack selection and generation-transition detection (all racks >= target,
-- nothing in flight, nothing staged). Leave values are derived from these full-rack means as
-- MAGPIE's rack_list_write_to_klv does.
CREATE TABLE leave_rack_progress (
    job_id           UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation       INT NOT NULL,
    rack             TEXT NOT NULL,
    occurrence_count BIGINT NOT NULL DEFAULT 0,
    equity_sum       DOUBLE PRECISION NOT NULL DEFAULT 0,  -- occurrence_count-weighted; equity_sum / occurrence_count = mean
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, generation, rack)
);

-- Accepted leave results waiting to be folded into `leave_rack_progress`: one
-- row per accepted task, its racks, counts and equity sums as three parallel
-- arrays (compressed and stored out of line, so a 200,000-rack submission is a
-- few megabytes and one insert).
--
-- A submission used to fold itself: an UPDATE over every rack its games drew,
-- scattered uniformly across the 3.2 million rows above. Measured, that was
-- 2.5-5.5 s inside the transaction the worker waits on, 147-409 MB of WAL per
-- fold (the first touch of a page after a checkpoint writes the whole page),
-- and no HOT update ever, because `occurrence_count` is indexed. Nothing needs
-- the per-rack totals that promptly -- selection needs them roughly, closing a
-- generation and building its KLV need them exactly but only then -- so a
-- submission appends here and `leave_gen::merge_staged` folds everything
-- staged in one pass: every half hour, when a claim finds the generation
-- nearly done, and always before a generation closes.
--
-- `task_id` deliberately has no foreign key. A purge deletes every task of a
-- job, Postgres runs a cascade once per deleted row, and there is no index on
-- this column to serve one -- the shape that made two earlier purges scan a
-- table per task. A purge deletes a job's staged rows itself, by `job_id`;
-- deleting the job cascades through the index below.
CREATE TABLE leave_rack_staging (
    id          BIGSERIAL PRIMARY KEY,
    job_id      UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation  INT NOT NULL,
    -- Whose result this is. Claim-time selection holds the racks this task
    -- forced out of play until the merge says what they reached.
    task_id     UUID NOT NULL,
    racks       TEXT[] NOT NULL,
    counts      BIGINT[] NOT NULL,
    equity_sums DOUBLE PRECISION[] NOT NULL,  -- count * mean, per rack
    staged_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT leave_rack_staging_parallel_arrays CHECK (
        cardinality(racks) = cardinality(counts)
        AND cardinality(racks) = cardinality(equity_sums)
    )
);
-- What a merge takes, what selection excludes, and the cascade from `jobs`.
CREATE INDEX leave_rack_staging_job_idx ON leave_rack_staging (job_id, generation);

-- One row per generation a leave job has opened: what the dashboard shows.
--
-- `tasks_completed` and `games_played` are live, bumped in the submit
-- transaction. The rest is a summary of `leave_rack_progress` as of
-- `merged_at`, recomputed by each merge while the rows are warm; counted on
-- read it was a pass over the whole generation on every detail view and every
-- live push. Seeded with `racks_total` when the generation's universe is
-- written, so there is a denominator before the first merge.
CREATE TABLE leave_generation_progress (
    job_id          UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation      INT NOT NULL,
    tasks_completed BIGINT NOT NULL DEFAULT 0 CHECK (tasks_completed >= 0),
    games_played    BIGINT NOT NULL DEFAULT 0 CHECK (games_played >= 0),
    racks_total     BIGINT NOT NULL DEFAULT 0,
    racks_at_target BIGINT NOT NULL DEFAULT 0,
    -- The rack furthest from target, and its count. NULL until a merge.
    min_rack        TEXT,
    min_rack_count  BIGINT,
    merged_at       TIMESTAMPTZ,
    PRIMARY KEY (job_id, generation)
);

-- Where a leave generation's selection sweep has got to: the last rack handed
-- out in the lap under way. A row with a rack exists exactly while a lap has
-- racks left to hand out: the task that takes the last of them deletes it.
--
-- While many racks are below target, racks are handed out in primary-key order
-- from this cursor rather than lowest count first. Everything behind the cursor
-- has been handed out this lap and nothing ahead of it has -- a lap only starts
-- with no claim of the generation in flight and nothing staged -- so a claim
-- needs no list of what is out, and selection costs the same however much is
-- staged. Lowest-count-first could not say that: between merges the racks of
-- every staged result are exactly the lowest, and each claim hashed and
-- skipped all of them inside the job's dispatch lock.
--
-- Once few racks are below target the generation turns, at a lap's boundary,
-- to lowest count first, and stays there: a row with a NULL cursor_rack
-- (`leave_gen::at_boundary`).
--
-- Read and written only under that lock. A row that goes missing (a purge, a
-- partial restore) is a lap not started, which waits for what is in flight and
-- staged before it selects anything; nothing is handed out twice.
CREATE TABLE leave_selection_cursors (
    job_id      UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    generation  INT NOT NULL,
    cursor_rack TEXT,
    PRIMARY KEY (job_id, generation)
);

-- Task records (one per accepted claim, keyed by task_claim_id)
-- task_id is denormalized here for efficient job-results queries without joining through task_claims.

-- One analysed position per row, whatever produced it.
--
-- Opening rack jobs write one per rack. Games and game-pairs jobs write one per
-- turn when `capture_positions` is on: a worker analyses a position on every
-- turn anyway, and keeping those makes a job a corpus of analysed positions as
-- well as a measurement of strength.
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
    -- The task, for the in-game dedup key below. Deliberately not a foreign
    -- key: a record goes with its claim (above) or its job (below), both of
    -- which cascade through their own indexes, so a cascade from the task
    -- only added an index entry and a foreign-key probe per record -- the same
    -- pattern `position_analysis_moves.task_id` and the staging table's were
    -- removed for.
    task_id         UUID NOT NULL,
    -- Denormalized from the task. Every read of a job's records -- the public
    -- results feed, the rack lookup, the admin stream, the export -- filtered
    -- on the job and could only reach it through `tasks`, which put the filter
    -- on the far side of a join from the sort and made the whole job the unit
    -- of work. With the column here they are index scans.
    job_id          UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
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
    -- The move played from this position, and its score: the one chosen this
    -- turn, which need not be the top-ranked move below (a simmer's pick, or
    -- a solver's), and which no later row holds when only a pair's first
    -- divergence is kept. Every in-game position has it; an opening rack,
    -- from which nothing is played, never does.
    played_move         TEXT,
    played_move_score   INT,
    -- How the move played from this position was chosen: by static equity, a
    -- simulation, a pre-endgame solve or an endgame solve. Decides which of
    -- its moves' statistics are set: win_percentage and per-ply rows for a
    -- simulation, mean_spread and fidelity_plies for a solve. An opening rack
    -- is 'static' or 'sim'.
    analysis            TEXT NOT NULL CHECK (analysis IN ('static', 'sim', 'peg', 'endgame')),
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
    ),
    CONSTRAINT position_analysis_played_in_game CHECK (
        (played_move IS NULL) = (game_index IS NULL)
        AND (played_move_score IS NULL) = (played_move IS NULL)
    )
);

-- One position per turn of a task's game. It is also how a random saved
-- position is drawn
-- (`/api/jobs/:id/positions/random`): a random turn of one task's games, a
-- few hundred entries at most, where `ORDER BY random()` read the whole job.
CREATE UNIQUE INDEX position_analysis_records_in_game_idx
    ON position_analysis_records (task_id, game_index, turn_number)
    WHERE game_index IS NOT NULL;

-- Opening racks keep their natural key: one analysis per rack per claim. A
-- rack an opening-rack job analyses more than once, to reach a consensus,
-- is in several tasks, each with its own claim.
CREATE UNIQUE INDEX position_analysis_records_rack_idx
    ON position_analysis_records (task_claim_id, rack)
    WHERE game_index IS NULL;

-- The public results feed, which is newest-first within a job and paginated by
-- keyset. `id` is in the index because it is the cursor's tiebreaker:
-- `submitted_at` defaults to now(), which is transaction time, so every record
-- of one batch shares it exactly and it is not a key on its own.
CREATE INDEX position_analysis_records_feed_idx
    ON position_analysis_records (job_id, submitted_at DESC, id DESC);

-- The rack lookup (`?rack=`), which is the branch the site actually uses. One
-- probe rather than one per task of the job. Opening racks only: an
-- incidentally-captured in-game position is not an opening-rack analysis.
CREATE INDEX position_analysis_records_job_rack_idx
    ON position_analysis_records (job_id, rack) WHERE game_index IS NULL;

-- The positions search (`/api/jobs/:id/positions?rack=`): a game's positions
-- with one rack, newest first. In-game positions only, so the index costs a
-- job nothing unless it captures.
CREATE INDEX position_analysis_records_game_rack_idx
    ON position_analysis_records (job_id, rack, id) WHERE game_index IS NOT NULL;

-- The cascade from a claim (`task_claim_id ... ON DELETE CASCADE`). The
-- partial unique index on (task_claim_id, rack) above cannot serve it: a plain
-- equality on task_claim_id does not imply `game_index IS NULL`, so the
-- planner never uses a partial index for it. Without this a purge or a job
-- delete -- which removes every claim of the job, and Postgres runs the
-- cascade once per deleted row -- scanned this whole table once per claim: a
-- full English opening-rack job is ~6,400 claims over ~3.2 million records,
-- thousands of sequential scans inside one transaction holding the job's
-- dispatch lock and every open claim's row. One entry per record; the moves
-- below cascade from the record through their own index.
CREATE INDEX position_analysis_records_claim_idx
    ON position_analysis_records (task_claim_id);

-- The top `num_plays_recorded` moves per position, from the player config that
-- produced them. Storing every move the worker ranked would be untenable:
-- a job over the full English 7-tile space is roughly 3.2 million racks, and a
-- 40,000-pair job with capture on is 1.8 million positions.
CREATE TABLE position_analysis_moves (
    id              BIGSERIAL PRIMARY KEY,
    -- The record is the only parent. A `task_id` column used to sit here as
    -- well, with its own ON DELETE CASCADE, kept "for the cascade" after the
    -- job-wide aggregates that read it were removed. That cascade was the
    -- problem: the column had no index, so deleting a task -- which a purge or
    -- a job delete does once per task -- scanned this whole table to find the
    -- moves to cascade, and this is the largest table in the schema. A full
    -- English opening-rack job is some 6,400 tasks over 32 million move rows,
    -- which made its purge thousands of sequential scans of the table, hours
    -- inside one transaction holding the job's dispatch lock. The record's
    -- cascade already reaches every move through the index below.
    record_id       BIGINT NOT NULL REFERENCES position_analysis_records(id) ON DELETE CASCADE,
    rank            SMALLINT NOT NULL,
    move            TEXT NOT NULL,
    score           INT NOT NULL,
    equity          DOUBLE PRECISION NOT NULL,
    -- How many times a simulation played the move out (its share of the
    -- simulation's iterations, most going to the leaders). 0 for a move nothing
    -- simulated; NULL from a client that did not say.
    iterations      BIGINT CHECK (iterations >= 0),
    -- The win percentage: a simulation's, or a pre-endgame solve's over every
    -- way the bag can be drawn. NULL for a static or endgame analysis.
    win_percentage  DOUBLE PRECISION,
    -- Mean win%+spread blend in [0, 1] (see the player config's
    -- utility_w_winpct/utility_w_spread/utility_spread_scale), sometimes used
    -- to rank moves instead of equity or raw win percentage. NULL for a
    -- static player, same as win_percentage.
    blended_utility DOUBLE PRECISION,
    -- A pre-endgame or endgame solve's projected final spread for the mover,
    -- in points, and the endgame depth the move was ranked at (a PEG move's
    -- deepest tier; 0 is PEG's greedy seed). NULL for a static or simulated
    -- analysis.
    mean_spread     DOUBLE PRECISION,
    fidelity_plies  SMALLINT
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
    move_id          BIGINT NOT NULL REFERENCES position_analysis_moves(id) ON DELETE CASCADE,
    ply              SMALLINT NOT NULL,
    bingo_percentage DOUBLE PRECISION NOT NULL,
    average_score    DOUBLE PRECISION NOT NULL,
    -- The natural key, and the index the cascade from moves uses: move_id is
    -- its leading column, so there is deliberately no second index on it.
    -- There was a BIGSERIAL `id` beside it that nothing referenced or read --
    -- every reader goes through move_id and the insert conflicts on this key
    -- -- at some 30 bytes a row between the column and its index: 2 to 5 GB
    -- for one simming opening-rack job's tens of millions of plies.
    PRIMARY KEY (move_id, ply)
);

-- What a simming player inferred of the opponent's leave before it simmed a
-- captured position: how many distinct leaves the inference found, how many it
-- drew in all, their mean equity, and the most drawn of them -- at most ten
-- `{leave, draws, equity}` objects, most drawn first. Only a simulated in-game
-- position past a game's first turn, whose opponent did not pass, has one; an
-- opening rack never does (there is no opponent move to infer from).
CREATE TABLE position_analysis_inference (
    record_id      BIGINT PRIMARY KEY
                   REFERENCES position_analysis_records(id) ON DELETE CASCADE,
    num_leaves     BIGINT NOT NULL CHECK (num_leaves >= 0),
    total_draws    BIGINT NOT NULL CHECK (total_draws >= 0),
    average_equity DOUBLE PRECISION NOT NULL,
    leaves         JSONB NOT NULL CHECK (jsonb_typeof(leaves) = 'array'
                                         AND jsonb_array_length(leaves) <= 10)
);

-- Shared by games and game pairs: one row per accepted claim, holding the
-- aggregate MAGPIE's autoplay reports. Autoplay does not emit individual games
-- -- it reports counts and score moments for a batch, and in `-gp` mode also
-- the pentanomial: how many completed pairs ended in each of the five possible
-- pair outcomes. The pentanomial is what the match test and the rating fits read; the
-- divergent summary alongside it is a diagnostic only.
CREATE TABLE game_results (
    task_claim_id     UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id           UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    -- Denormalized from the task; see position_analysis_records.job_id.
    job_id            UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,

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
    -- This -- not the divergent subset below -- is what the match test and the ratings
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
             AND pent_1 + 2 * pent_2 + 3 * pent_3 + 4 * pent_4 = 2 * wins + ties
             -- And on the draws: a pair scoring one or three half-points holds
             -- exactly one, a pair scoring two holds none or two.
             AND ties - pent_1 - pent_3 BETWEEN 0 AND 2 * pent_2
             AND (ties - pent_1 - pent_3) % 2 = 0)
    ),

    -- The divergent subset: pairs whose two games did not play identically.
    -- Kept as a *diagnostic* -- it says how often two configs actually differ,
    -- and how they fare where they do, which the job page shows as a match
    -- score of its own -- and deliberately not used as a statistical sample.
    -- Each player's mean score over the subset's games, as MAGPIE reports it
    -- (0 when no pair diverged). NULL for `games` jobs.
    divergent_games   INT CHECK (divergent_games >= 0),
    divergent_wins    INT CHECK (divergent_wins >= 0),
    divergent_losses  INT CHECK (divergent_losses >= 0),
    divergent_ties    INT CHECK (divergent_ties >= 0),
    divergent_p1_score_mean DOUBLE PRECISION,
    divergent_p2_score_mean DOUBLE PRECISION,
    CONSTRAINT game_results_divergent_all_or_nothing CHECK (
        (divergent_games IS NULL AND divergent_wins IS NULL
             AND divergent_losses IS NULL AND divergent_ties IS NULL
             AND divergent_p1_score_mean IS NULL AND divergent_p2_score_mean IS NULL)
        OR (divergent_games IS NOT NULL AND divergent_wins IS NOT NULL
             AND divergent_losses IS NOT NULL AND divergent_ties IS NOT NULL
             AND divergent_p1_score_mean IS NOT NULL AND divergent_p2_score_mean IS NOT NULL
             AND divergent_wins + divergent_losses + divergent_ties = divergent_games
             AND divergent_games <= games)
    ),

    submitted_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per accepted leave task (a single worker's forced-rack partition of a generation).
-- The full {rack, count, mean} submission is staged in leave_rack_staging, folded into
-- leave_rack_progress by the next merge, and not kept after that — nothing reads it back,
-- so there's no CSV artifact to reference here.
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
    -- transition against fewer results, or by a rebuild under a changed KLV
    -- builder (`builder`, below). Recording the hash is what turns that from
    -- invisible into a query; the ON CONFLICT DO NOTHING on insert means the
    -- row keeps the FIRST hash, so a later mismatch is evidence rather than an
    -- overwrite.
    sha256        TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    -- SHA-256 of the bytes the object store holds *now*, when they are not
    -- the bytes first written; NULL while they are. Set by every
    -- `rebuild_artifacts` check from the object it wrote, or found and could
    -- account for. Workers are sent this (or `sha256` when it is NULL) and
    -- refuse bytes that do not match, so it has to follow the object: a
    -- rebuild under a changed builder wrote new bytes, the row kept the old
    -- hash, and every task of the next generation failed its check on every
    -- worker. `sha256` stays the first hash, as the evidence it is.
    served_sha256 TEXT CHECK (served_sha256 ~ '^[0-9a-f]{64}$'),
    -- The MAGPIE KLV builder that wrote these bytes ('klv-1').
    --
    -- MAGPIE builds these artifacts, so an upgrade can legitimately change the
    -- bytes for the same leave values. Without knowing which builder wrote an
    -- artifact, `rebuild-artifacts` could only report "differs" -- and the
    -- first MAGPIE upgrade after a restore drill would read as data loss. With
    -- it, a rebuild under a different builder says so, and only two artifacts
    -- from the *same* builder disagreeing is evidence of anything.
    builder       TEXT NOT NULL,
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
-- rating can never affect whether a job stops. The match test stays on the job config
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
    name          TEXT NOT NULL UNIQUE CHECK (char_length(name) <= 100),
    variant       TEXT NOT NULL,
    letterdist_id UUID NOT NULL REFERENCES input_data(id),
    layout_id     UUID NOT NULL REFERENCES input_data(id),
    -- The fixed point every other rating is measured against. Ratings are only
    -- identifiable up to an additive constant, so exactly one player config
    -- must be pinned; the static bot at 2000 is the convention.
    anchor_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    anchor_rating DOUBLE PRECISION NOT NULL DEFAULT 2000,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
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
--
-- Kept in full for a month, then thinned to the last run of each UTC day, the
-- pool's first run aside (ratings::thin_old_runs, hourly). A pool with an
-- active job takes a run every two minutes, and past a month nothing reads a
-- run but the history endpoint, which returns at most 500 over the pool's
-- life. The ratings and residuals below go with their run.
CREATE TABLE rating_runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_id       UUID NOT NULL REFERENCES rating_pools(id) ON DELETE CASCADE,
    computed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Why this run happened: 'membership' (an admin added or removed a config),
    -- 'evidence' (new results arrived), 'manual', or 'anchor' (an admin moved
    -- the anchor or its rating).
    trigger       TEXT NOT NULL,
    method        TEXT NOT NULL DEFAULT 'bradley_terry_newton',
    -- Fit provenance. A run that did not converge is still stored and still
    -- displayed, flagged: hiding it would leave the page silently stale.
    iterations    INT NOT NULL,
    converged     BOOLEAN NOT NULL,
    -- How much evidence went in, so a run can be compared to its predecessor
    -- without re-reading game_results.
    pairs_used    BIGINT NOT NULL,
    jobs_used     INT NOT NULL,
    -- The pool's eligible jobs' `games_completed`, summed, as of the fit: what
    -- the sweep compares before deciding to build the evidence matrix at all.
    -- Building it to find nothing had changed was the sweep's whole cost, for
    -- every pool every two minutes. NULL on a run that did not record it,
    -- which the next sweep refits.
    evidence_games BIGINT
);

CREATE INDEX rating_runs_pool_idx ON rating_runs (pool_id, computed_at DESC);

-- The ratings themselves: one row per player config per run. This is the only
-- table in the schema that holds a rating.
CREATE TABLE player_config_ratings (
    run_id           UUID NOT NULL REFERENCES rating_runs(id) ON DELETE CASCADE,
    player_config_id UUID NOT NULL REFERENCES player_configs(id),
    rating           DOUBLE PRECISION NOT NULL,
    -- Approximate standard error, in rating points. Wide bars are the honest
    -- signal that a config has barely played, or has only played opponents
    -- far from its own strength; the page shows them next to the rating for
    -- that reason.
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

-- The cross table of one fit, and its residuals: for every head-to-head with
-- games in it, once (the row is the config whose name sorts first), the score
-- that happened, its standard error and the average spread, beside the score
-- the fit's ratings predict. The page mirrors each row for the other side.
-- Stored with the run rather than recomputed on each view of the pool, which
-- rebuilt the pool's evidence matrix -- a grouped scan over every paired
-- result it counts -- on every public page view. Stored, they also describe
-- the evidence this fit used, not evidence that has moved on since.
CREATE TABLE rating_run_residuals (
    run_id               UUID NOT NULL REFERENCES rating_runs(id) ON DELETE CASCADE,
    row_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    col_player_config_id UUID NOT NULL REFERENCES player_configs(id),
    pairs                DOUBLE PRECISION NOT NULL,
    actual               DOUBLE PRECISION NOT NULL,  -- the row config's score rate
    predicted            DOUBLE PRECISION NOT NULL,
    -- The standard error of `actual`, from the pairs' score variance in the
    -- summed pentanomial (ratings::HeadToHeadEvidence::score_and_stderr).
    stderr               DOUBLE PRECISION NOT NULL,
    -- The row config's average spread per game: Σ games·(its mean score − the
    -- other's) / Σ games over the same game_results rows.
    spread               DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (run_id, row_player_config_id, col_player_config_id)
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

-- Settings an admin changes at run time (`/admin/settings`), as one row. The
-- deployment's own settings are environment variables, which take a deploy
-- to change; these take effect at the next claim.
CREATE TABLE settings (
    -- Always TRUE: the primary key and its check are what make it one row.
    id                BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    -- The longest a task may run. Every claim is given it (the assignment's
    -- `max_task_seconds`) and keeps its own deadline, claim time plus this as
    -- it stood then, so a change applies to claims made after it. A claim
    -- past its deadline and a minute's grace is reclaimed even while its
    -- worker heartbeats, and its result refused (`task_claims.deadline_at`).
    -- Ten minutes at the least, a day at the most. The floor is not the
    -- shortest batch worth a claim but the first claim on a machine: it may
    -- build the job's rack info table first, a minute to three that cannot
    -- be stopped part-way (and is kept for every task after it), so a limit
    -- near that would stop the task that paid for it, every time, on every
    -- new machine. The API refuses what this refuses (`routes::admin`); a
    -- test that wants a claim past its deadline moves the claim's
    -- `deadline_at`, not this.
    max_task_seconds  INT NOT NULL DEFAULT 3600 CHECK (max_task_seconds BETWEEN 600 AND 86400),
    -- Who changed them last, and when; NULL until anyone has. SET NULL, like
    -- jobs.created_by: the settings outlive the admin.
    updated_by        UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO settings DEFAULT VALUES;

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
-- Completed claims by time. The ETA (`jobstats::estimate_eta`, on every
-- detail view and live push) asks "how many of this job's claims completed
-- in the last hour" (the job list's `stalled` flag reads
-- `jobs.last_completed_at` instead), and no index on task_claims leads with the job, so
-- the alternative plan walks every task of the job and every claim of each
-- -- the job's whole history, for a question about its last hour. Through
-- this index the scan is bounded by the fleet's recent completions instead,
-- whatever the job's age -- as long as the query's bound is one the planner
-- can read: a constant `now() - interval '1 hour'`, not a parameter.
CREATE INDEX        task_claims_completed_idx ON task_claims (completed_at DESC)
    WHERE state = 'completed';
-- Each covers only its own kind of identity: a claim has exactly one, and
-- indexing the other kind's claims under a NULL key nothing looks up was dead
-- weight -- nearly half of each index. A lookup by `= $n` implies the
-- `IS NOT NULL`, so every reader still uses them.
-- Keyed by identity, job and completion time: an identity's completed claims
-- in one job, newest first, are one backward range -- the results feed's
-- `?worker=` reads a page through them whatever the contributor's share of the
-- job. Only completed claims have a `completed_at`; the rest sit at the NULL
-- end, outside the range. A lookup by identity alone still uses the leading
-- column. The completion time adds nothing to a claim's updates: completing
-- one changes `state`, which the open-claims index's predicate reads, so that
-- update was never a HOT one, and a heartbeat touches neither.
--
-- Each carries the claim's `movegens`, so a contributor's work by job type
-- (`GET /api/workers/*/:id/movegens`) is an index-only walk of their range,
-- grouped by the job it is already ordered by, rather than a heap read per
-- claim they ever made. It is written by that same completing update, whose
-- index entries are new ones anyway.
CREATE INDEX        task_claims_user_idx      ON task_claims (claimed_by_user_id, job_id, completed_at)
    INCLUDE (movegens) WHERE claimed_by_user_id IS NOT NULL;
CREATE INDEX        task_claims_anon_idx      ON task_claims (claimed_by_anon_uuid, job_id, completed_at)
    INCLUDE (movegens) WHERE claimed_by_anon_uuid IS NOT NULL;
-- There is no (job_id, state) index. The job-scoped reads of `tasks` -- the
-- detail page's counts by state, the census -- are served by
-- `tasks_seed_unique_idx (job_id, seed)` and the heap. The opening-rack finish
-- check asks for the job's first task by seed there, and for a task not
-- completed through `tasks_queue_idx` (available) and `task_claims_open_idx`
-- (claimed), not by state. A state index cost an entry on every task insert
-- and every state change, on the claim and submit paths, for no reader that
-- needed it.
-- For the cascade from `tasks` alone: no reader looks a result up by its task.
-- It was (task_id, submitted_at) for the per-task "first accepted result" read
-- every aggregate made while a task could have more than one; with one slot a
-- task has one result, and the aggregates sum the job's (`jobstats`).
CREATE INDEX        game_results_task_idx     ON game_results (task_id);
-- The public results feed for a games or game-pairs job, keyset-paginated like
-- the opening-rack one. `task_claim_id` is the primary key and so the
-- tiebreaker, since `game_results` has no serial.
CREATE INDEX        game_results_feed_idx
    ON game_results (job_id, submitted_at DESC, task_claim_id DESC);
CREATE INDEX        leave_records_task_idx    ON leave_records (task_id);
CREATE INDEX        audit_log_created_idx     ON audit_log (created_at DESC);
CREATE INDEX        audit_log_job_idx         ON audit_log (job_id);

-- Drives claim-time rack selection once few racks remain below target: "the
-- racks furthest from target in this generation" (`leave_gen::furthest_below_target`).
--
-- On `occurrence_count` alone, and selection orders on it alone. Counts tie in
-- their millions -- every rack of a generation starts at zero, and most of the
-- 3.2 million full racks are rare enough to stay there until they are forced --
-- so an ORDER BY that also broke ties by rack could not be served by this index
-- and every leave claim sorted the generation to find its few hundred racks
-- (4.9 s a claim at full size). Carrying `rack` in the key fixed that at a
-- price: measured on a full English generation, 180 MB where this index is
-- 22 MB, because keys that are nearly all equal deduplicate and unique keys
-- cannot. Nothing needs the tie broken, so the order gave it up instead; while
-- many racks are below target, selection does not read this index at all (it
-- sweeps the primary key, see `leave_selection_cursors`).
CREATE INDEX leave_rack_progress_pick_idx
    ON leave_rack_progress (job_id, generation, occurrence_count);
