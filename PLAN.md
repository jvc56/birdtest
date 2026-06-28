# birdtest — Project Plan

## High-Level Design

### Overview

birdtest is a crowdsourced word game analysis platform, modeled after Fishnet (which crowdsources chess game analysis for Lichess). Users contribute compute by running tasks locally and submitting results back to the site. Admins define jobs and allocate work; the site aggregates results and presents them on a polished dashboard.

---

### Jobs

Jobs are long-running research goals defined and managed by admins. The following job types are supported:

- **Analyze all possible opening racks**
- **Run simulated games** — Monte Carlo simulation across candidate moves to estimate win percentage.
- **Run simulated game pairs** — Monte Carlo simulation across candidate moves to estimate win percentage, run as matched pairs (same seed, players swapped) to reduce variance.
- **Run static games** — move selection based on highest static equity (score + leave value; no simulation or lookahead).
- **Run static game pairs** — same as static games but run as matched pairs.
- **Analyze a single preendgame**
- **Leave generation**

Each job has a **priority** and a **percentage allocation**. Priority takes precedence: workers are only assigned tasks from lower-priority jobs when no tasks remain at any higher-priority level. **A lower integer value means higher priority** — priority `0` outranks priority `1`. Allocation percentages govern how work is distributed among jobs at the same priority level; active jobs within a tier must have allocations summing to 100% (enforced at the application layer, not via a DB constraint). Jobs, their priorities, and their allocations are managed by admins only.

#### Job Lifecycle Controls

Jobs are created by admins and become active immediately. The following states are supported:

- **active** — workers are assigned tasks from this job normally.
- **inactive** — the job exists and retains all its tasks and results, but workers are not assigned tasks from it. Admins can reactivate it at any time.
- **completed** — all tasks have been completed.

Admins can deactivate, reactivate, purge (clear all results and return tasks to unclaimed), or delete a job at any time.

---

### Tasks

Tasks are the atomic units of work that workers execute. The following task types are supported:

- Analyze a single position
- Play a game with a given seed
- Play a game pair with a given seed
- Play a batch of games or game pairs with a given seed

Each task has a **natural key** (a seed or position ID) that guarantees global uniqueness. Duplicate tasks are prevented at the database level.

#### Task States

Tasks move through an explicit state machine:

```
available → claimed → completed
               ↑          
         (heartbeat timeout on a claim: one slot reopens; task returns to available if it had been at capacity)
```

State is determined by denormalized counters (`accepted_count`, `active_claim_count`) relative to the job's `redundancy` value X:

- **available**: `accepted_count + active_claim_count < redundancy` — open capacity remains; workers can claim a new slot.
- **claimed**: `accepted_count + active_claim_count = redundancy` but `accepted_count < redundancy` — all slots are filled with in-flight claims; waiting on results.
- **completed**: `accepted_count = redundancy` — all X results have been submitted and accepted.

Individual claims are rows in `task_claims`. When a claim's heartbeat times out, that claim row is flipped to `abandoned`, `active_claim_count` is decremented, and if the task was at capacity it returns to **available**. Reclamation is lazy — it runs at the moment the next task is requested, not via a background process.

#### Task Generation

Task generation is job-type-dependent:

- **Pre-populated jobs**: At job creation time, all task requests are generated and inserted into the `tasks` table with `state = 'available'`. Workers then claim from this pool.
- **On-demand jobs**: Tasks are generated and inserted atomically at claim time. The task never passes through `available` — it goes directly to `claimed` in a single transaction.
- Preendgame tasks are pre-populated, but the subdivision algorithm is yet to be defined.

---

### Workflow

1. The worker sends a **task claim** to the server — a minimal message identifying itself and signaling it is ready for work.
2. The system selects a job by priority tier (lowest integer value = highest priority, descending). Within the top available tier, a job is chosen by weighted random draw renormalized proportionally across only the currently **active** jobs in that tier; inactive and completed jobs are excluded and their configured allocation percentages do not count against active jobs' weights at selection time.
3. Expired claims for the selected job are lazily reclaimed: each timed-out `task_claims` row is flipped to `abandoned`, `active_claim_count` is decremented, and tasks that were at capacity return to `available`.
4. The system acquires the next task (pre-populated or on-demand, depending on the job type), inserts a `task_claims` row, increments `active_claim_count`, and issues a claim token (UUID) to the worker.
5. The server responds with the **task request** for that job type.
6. The worker performs the task and submits a **task response** along with the claim token.
7. If the claim token matches a `task_claims` row that is not abandoned, the task response is accepted, a **task record** is stored keyed to the `task_claim_id`, `accepted_count` is incremented, and `active_claim_count` is decremented. When `accepted_count = redundancy` the task is marked **completed**. If the token is stale (the claim was abandoned due to timeout), the submission is silently ignored.

---

### Workers

Workers are the clients that perform tasks and submit results. Two types are supported:

- **Anonymous workers**: Identified by a UUID generated by the worker client on first run and sent with every request. Contributions are tracked and displayed per UUID, shown under the label "Anonymous" with the UUID as the distinguishing identifier.
- **Authenticated workers**: Identified by an API key tied to a user account. Contributions are tracked per user.

#### Worker Integrity and Anomaly Detection

- **Chi-square testing per worker** — statistical test that flags workers submitting results that deviate significantly from the population. Detects buggy or malicious clients without needing to trust any individual submission.
- **Worker ban list** — a persistent table of banned worker identities; banned workers cannot claim or submit tasks. Meaningful for authenticated workers; for anonymous workers, banning targets the UUID.
- **Redundant task execution** — each job specifies a redundancy value X; X independent workers must each complete the task. All X results are stored independently. No consensus or agreement check is performed at submission time — reconciliation is a downstream analysis question deferred past v1. The only active integrity mechanism at submission time is chi-square anomaly detection per worker.

---

### Worker Client

Contributors run a client program that loops continuously: it sends a **task claim** to the server, receives a **task request**, executes the work, and submits a **task response**. The client handles authentication (API key or anonymous UUID) and heartbeating automatically. See the [Worker Client](#worker-client-1) section for technical details.

---

### Statistical Result Evaluation

For game-pair jobs, results are evaluated using the Sequential Probability Ratio Test (SPRT): testing stops as soon as statistical significance is reached rather than running a fixed number of pairs, saving compute by terminating early when the outcome is clear.

Game results are also aggregated into persistent ELO ratings for each bot or configuration being tested. The dashboard displays an estimated time to completion for active jobs based on current throughput and SPRT progress.

---

### User Accounts

Users can create an account to track their contributions. Account creation requires:

- Username
- Password (minimum strength enforced at registration time)
- Email address (used for account confirmation and password reset)

A confirmation code is sent to the email address on registration. Users can generate one or more API keys from their account, which are used to authenticate task submissions.

API keys are stored as hashes (never raw values) in the database. The raw key is shown to the user exactly once at generation time.

**v1 account scope**: The sole v1 purpose of a user account is to generate an API token, which attributes task submissions to that account instead of an anonymous UUID. No other feature is gated behind registration. Anonymous workers can complete tasks fully, with no account or API token required.

---

### Security

All state-mutating endpoints are protected by CSRF tokens tied to the session. API keys and session tokens are handled as described in the Auth tech stack note below.

---

### Dashboard

The site hosts a polished dashboard where users can view job status and results. The dashboard shows:

- Current status of all active jobs
- Simple per-job statistics queried directly from raw result data (no pre-aggregation for v1)
- Per-worker contribution stats: tasks completed, uptime, error rate

Raw result data is queryable via a public API with pagination and filtering by username, success status, and time control. A streaming download endpoint allows offline analysis of completed job results.

#### Audit Log

Every significant action (task claimed, result submitted, job created, user banned) is written to an append-only log table for debugging and accountability.

---

### Tech Stack

| Concern | Decision |
|---|---|
| Web framework | Axum |
| DB access | SQLx |
| Database | RDS Postgres |
| Compute | ECS with Fargate |
| Task queue | Postgres-based (SKIP LOCKED) |
| Frontend framework | SvelteKit |
| Frontend hosting | ECS (same service as backend), S3 + CloudFront later |
| Styling | Tailwind CSS |
| Component library | shadcn-svelte |
| Charts | LayerCake |
| Auth | Roll your own (Axum + Argon2 + Paseto) |
| Email | AWS SES |
| Secrets | AWS SSM Parameter Store |
| Artifact storage | AWS S3 |
| Infrastructure as Code | Terraform |

#### Key Technology Notes

**Postgres task queue**: Tasks are claimed using `SELECT ... FOR UPDATE SKIP LOCKED`, which allows concurrent workers to claim tasks without lock contention. Timeout reclamation is lazy and runs at claim time.

**Claim tokens**: Each claim issues a UUID token. Workers must submit this token with their results. Stale tokens (from timed-out claims) are silently rejected.

**ECS with Fargate**: The Axum backend and SvelteKit frontend are served from the same ECS service initially. The frontend will be split out to S3 + CloudFront in a later phase.

**Auth**: Sessions use Paseto tokens stored in httpOnly cookies. API keys are random strings hashed with Argon2 before storage.

**Secrets**: Database credentials, signing keys, and SES credentials are stored in AWS SSM Parameter Store and injected into the ECS task at runtime.

---

### Design Decisions & Rationale

| Decision | Rationale |
|---|---|
| Priority-then-allocation job scheduling | Priority gives admins hard ordering guarantees; allocation within a tier gives proportional distribution without needing to touch priorities |
| Admin-only job management | Avoids abuse prevention and quota complexity in v1 |
| Natural key uniqueness for tasks | Prevents duplicate work at the DB level, not just app logic |
| Lazy timeout reclamation | No background process needed; simpler to operate |
| Claim token for stale result rejection | Race-condition-free; no timestamp comparison needed |
| Anonymous workers identified by UUID | Enables per-worker contribution tracking and result filtering without requiring account creation |
| No pre-aggregation for dashboard v1 | Simple stats don't require it; avoids premature optimization |
| AWS throughout | Learning goals; avoids future migration pain; production-grade from day one |

---

## Low-Level Design

### Request Handling

The core of birdtest is the task claim endpoint — the sequence that runs every time a worker asks for work.

1. **Auth and verification**: The server reads the worker identity from request headers (`Authorization: Bearer <api-key>` for authenticated workers, `X-Worker-UUID` for anonymous workers). It verifies the worker is not banned and upserts the worker record (`users` or `anonymous_workers`).

2. **Job selection**: The server filters to active jobs (excluding inactive and completed), then selects by priority tier — lowest integer value first (priority `0` outranks priority `1`). Within the top available tier, a job is chosen by weighted random draw renormalized proportionally across the active jobs in that tier; allocation percentages of inactive/completed jobs do not factor in.

3. **Lazy reclamation**: Before acquiring a task, any claimed tasks for the selected job whose `last_heartbeat_at` (or `claimed_at`, if no heartbeat has been received yet) exceeds the heartbeat timeout are returned to `available`.

4. **Task acquisition** — strategy-dependent:
   - **Pre-populated**: `SELECT ... FOR UPDATE SKIP LOCKED` on `available` tasks for the selected job. If none remain, fall through to the next job in priority order.
   - **On-demand**: Generate the next task request for the selected job type and insert + claim it atomically in a single transaction.

5. **Response**: The server serializes the job-type-specific task request and returns it to the worker along with the claim token.

### Task Claim

A task claim is the message a worker sends to initiate the exchange. It carries no job-type-specific payload — the server decides the assignment. The worker's identity and auth are conveyed via request headers; the body is empty.

The server responds with the task request for the assigned job type and a claim token the worker must include when submitting its result.

### Job Type System

The core architectural pattern is a **job type registry**: a closed set of job types where each type defines four components. Adding a new job type requires implementing all four; the compiler enforces completeness via exhaustive matching.

The stored form of a processed task response is called a **task record** throughout this document.

### The Four Components

Each job type defines:

| Component | Description |
|---|---|
| **Task request** | Serialized and sent to the worker when it claims a task. Contains everything the worker needs to perform the work. |
| **Task response** | Deserialized from the worker's submission. The raw output of the work, validated on receipt. |
| **Task record** | The normalized form stored in a typed record table (one table per record type). Derived from the response; may omit fields, recompute derived values, or canonicalize formats. |
| **Creation strategy** | How tasks for this job type are generated: **pre-populated** or **on-demand** (see below). |

### Task Request Types

A task request is inserted into a typed request table at task creation time (in the same transaction as the `tasks` row). For pre-populated jobs all requests are written at job creation; for on-demand jobs the request is written at claim time.

Some request types are shared across job types:

| Type | Used by |
|---|---|
| `PositionRequest` | Opening rack analysis, preendgame analysis |
| `SeedRequest` | Simulated games, simulated game pairs |
| `StaticGameRequest` | Static games, static game pairs |
| `LeaveRequest` | Leave generation |

### Task Response Types

A task response is what the worker submits after completing a task. It is validated on receipt and then transformed into a task record for storage. Response types may differ from their corresponding request types (e.g., a single seed request may yield a batch of game results).

| Type | Used by |
|---|---|
| `PositionAnalysisResponse` | Opening rack analysis, preendgame analysis |
| `GameResultsResponse` | Simulated games, static games |
| `GamePairResultsResponse` | Simulated game pairs, static game pairs |
| `LeaveResponse` | Leave generation |

### Task Record Types

A task record is the normalized form stored in a typed table after a response is accepted. It may omit raw fields, recompute derived values, or canonicalize formats. Task record types may be shared when the stored shape is the same regardless of how the task was generated.

| Type | Used by |
|---|---|
| `PositionAnalysisRecord` | Opening rack analysis, preendgame analysis |
| `GameResultsRecord` | Simulated games, static games |
| `GamePairResultsRecord` | Simulated game pairs, static game pairs |
| `LeaveRecord` | Leave generation |

### Creation Strategies

**Pre-populated**: At job creation time the server generates all task requests and inserts them into the `tasks` table with `state = 'available'`. Workers then claim from this pool. Suitable for finite, enumerable work spaces — opening rack analysis, static games, preendgame analysis.

**On-demand**: No tasks are inserted at job creation. When a worker requests a task, the server generates the next task request, then inserts and claims it atomically in a single transaction, issuing a claim token. The task never passes through `available` state. Suitable for seed-based work where the space is effectively unbounded — simulated games, simulated game pairs, leave generation.

### Rust Implementation

Each job type is a struct implementing the `JobHandler` trait:

```rust
pub trait JobHandler {
    type Request;   // maps to a typed request table row
    type Response: DeserializeOwned;
    type Record;    // maps to a typed record table row

    fn creation_strategy() -> CreationStrategy;
    async fn insert_request(pool: &PgPool, task_id: Uuid, req: &Self::Request) -> Result<()>;
    fn make_request(task_id: Uuid) -> Self::Request;
    fn process_response(task_id: Uuid, response: Self::Response) -> Result<Self::Record>;
    async fn insert_record(pool: &PgPool, record: &Self::Record) -> Result<()>;
}

pub enum CreationStrategy {
    PrePopulated,
    OnDemand,
}
```

A top-level `JobType` enum dispatches to each concrete handler. The compiler enforces exhaustiveness on all match arms, so no case can be silently forgotten.

**Adding a new job type requires exactly four steps:**

1. Add a variant to the `JobType` enum and to the `job_type` Postgres enum (migration).
2. Create a handler struct and implement `JobHandler` with its three associated types.
3. Add the variant to `JobType`'s match arms — the compiler will reject a build that omits it.
4. Add a migration inserting/selecting from the new typed request and record tables. No other changes required — the compiler enforces all match arms are handled.

---

## Worker Client

Contributors run a client program that polls for tasks, executes them, and submits results. The client handles authentication (API key or anonymous UUID), heartbeating, and the request/result cycle automatically.

### Responsibilities

1. On startup: load or generate a persistent worker UUID; optionally load an API key from config.
2. Send a **task claim** to `/api/worker/task`.
3. Deserialize the **task request** from the response and execute the appropriate work (run the word game analysis tool).
4. Send periodic heartbeats to `/api/worker/heartbeat` while working.
5. Submit a **task response** to `/api/worker/result` with the claim token.
6. Loop.

### Language

**Python.** Chosen for ease of setup across contributor machines — no compilation step required. The client is lightweight and shells out to the word game engine for all computation, so raw client performance is not a concern.

### Engine Dependency (MAGPIE)

The worker client shells out to **MAGPIE** ([github.com/jvc56/MAGPIE](https://github.com/jvc56/MAGPIE)) for the actual word game computation. For v1, the contributor or admin supplies a path to a local MAGPIE directory containing an already-compiled MAGPIE executable. The worker client does **not** build, install, fetch, or manage MAGPIE — it only invokes the binary at the given path.

The MAGPIE directory path is a required configuration value for the worker client, provided via a config file or CLI flag (e.g. `--magpie-dir /path/to/MAGPIE`).

---

## API

All endpoints return JSON. State-mutating endpoints require a valid CSRF token. Worker endpoints accept either an `Authorization: Bearer <api-key>` header (authenticated workers) or no auth header plus an `X-Worker-UUID` header (anonymous workers).

### Worker API

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/worker/task` | Send a task claim. Returns a job-type-specific task request and claim token. |
| `POST` | `/api/worker/heartbeat` | Keep-alive ping for a claimed task. Updates `last_heartbeat_at`. |
| `POST` | `/api/worker/result` | Submit the result for a claimed task. Requires the claim token. |

### Auth API

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/auth/register` | Create a new user account. Sends a confirmation email. |
| `POST` | `/api/auth/login` | Create a session. Returns a Paseto token in an httpOnly cookie. |
| `POST` | `/api/auth/logout` | End the current session. |
| `POST` | `/api/auth/confirm-email` | Confirm email address using the code from the confirmation email. |
| `POST` | `/api/auth/reset-password/request` | Send a password reset email. |
| `POST` | `/api/auth/reset-password/confirm` | Apply a password reset using the token from the reset email. |

### Account API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/me` | Current user info and contribution stats. |
| `GET` | `/api/me/api-keys` | List API keys (labels and metadata only; hashes are never returned). |
| `POST` | `/api/me/api-keys` | Generate a new API key. Returns the raw key exactly once. |
| `DELETE` | `/api/me/api-keys/:id` | Revoke an API key. |

### Admin API

All Admin API endpoints require the requesting user to have `is_admin = TRUE`. Requests from non-admin authenticated users or anonymous workers are rejected with `403 Forbidden`.

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/admin/jobs` | Create a new job. Becomes active immediately. |
| `POST` | `/api/admin/jobs/:id/deactivate` | Set a job to inactive. Workers will no longer be assigned tasks from it. |
| `POST` | `/api/admin/jobs/:id/activate` | Set an inactive job back to active. |
| `POST` | `/api/admin/jobs/:id/purge` | Purge all results for a job and return tasks to unclaimed. |
| `DELETE` | `/api/admin/jobs/:id` | Delete a job and all its tasks. |
| `GET` | `/api/admin/workers` | List all workers with contribution stats and ban status. |
| `POST` | `/api/admin/workers/ban` | Ban a worker by user ID or anonymous UUID. |
| `DELETE` | `/api/admin/workers/ban/:id` | Remove a ban. |
| `GET` | `/api/admin/audit-log` | Query the audit log with filtering and pagination. |

### Public API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/jobs` | List jobs with status and summary stats. Paginated. |
| `GET` | `/api/jobs/:id` | Job detail, configuration, and aggregate statistics. |
| `GET` | `/api/jobs/:id/results` | Paginated task records for a job. Filterable by worker. |
| `GET` | `/api/jobs/:id/results/stream` | Streaming download of all task records for offline analysis. |
| `GET` | `/api/workers` | Contributor stats, paginated. |

---

## Schema

```sql
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
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);

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
    'simulated_games',
    'simulated_game_pairs',
    'static_games',
    'static_game_pairs',
    'preendgame_analysis',
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
    allocation INT NOT NULL CHECK (allocation BETWEEN 0 AND 100),
    -- Number of independent workers that must complete each task. Default 1 = single-claim behavior.
    redundancy INT NOT NULL DEFAULT 1 CHECK (redundancy >= 1),
    status     job_status NOT NULL DEFAULT 'active',
    config     JSONB NOT NULL DEFAULT '{}',
    created_by      UUID NOT NULL REFERENCES users(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    deactivated_at  TIMESTAMPTZ
);

-- Tasks

CREATE TYPE task_state AS ENUM ('available', 'claimed', 'completed');

CREATE TABLE tasks (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id               UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    natural_key          TEXT NOT NULL UNIQUE,
    state                task_state NOT NULL DEFAULT 'available',
    -- Denormalized counters used by SKIP LOCKED selection; avoids per-candidate join/aggregate.
    accepted_count       INT NOT NULL DEFAULT 0,
    active_claim_count   INT NOT NULL DEFAULT 0,
    completed_at         TIMESTAMPTZ
);

-- Partial indexes to support efficient SKIP LOCKED task selection and timeout reclamation.
CREATE INDEX tasks_queue_idx   ON tasks (job_id, state) WHERE state = 'available';
CREATE INDEX tasks_claimed_idx ON tasks (state) WHERE state = 'claimed';

-- Individual claims (one row per worker claim; up to redundancy concurrent/cumulative rows per task)

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
    task_id     UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon     TEXT NOT NULL,
    variant     TEXT NOT NULL,
    position    TEXT NOT NULL   -- encoded board + rack (e.g. GCG format)
);

CREATE TABLE seed_requests (
    task_id         UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon         TEXT NOT NULL,
    variant         TEXT NOT NULL,
    seed            BIGINT NOT NULL,
    num_games       INT NOT NULL DEFAULT 1,
    player1_type    TEXT NOT NULL,
    player2_type    TEXT NOT NULL
);

CREATE TABLE static_game_requests (
    task_id         UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon         TEXT NOT NULL,
    variant         TEXT NOT NULL,
    game_gcg        TEXT NOT NULL,  -- full game record in GCG format
    player1_type    TEXT NOT NULL,
    player2_type    TEXT NOT NULL
);

CREATE TABLE leave_requests (
    task_id     UUID PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    lexicon     TEXT NOT NULL,
    variant     TEXT NOT NULL,
    iteration   INT NOT NULL
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

CREATE TABLE game_records (
    task_claim_id   UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    score1          INT NOT NULL,
    score2          INT NOT NULL,
    winner          SMALLINT NOT NULL CHECK (winner IN (0, 1, 2)),  -- 0 = draw
    num_turns       INT NOT NULL,
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE game_pair_records (
    task_claim_id   UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    score1_game1    INT NOT NULL,
    score2_game1    INT NOT NULL,
    score1_game2    INT NOT NULL,
    score2_game2    INT NOT NULL,
    winner          SMALLINT NOT NULL CHECK (winner IN (0, 1, 2)),  -- 0 = tie
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Full leave tables are large binary data; store in S3 and reference by key.
CREATE TABLE leave_records (
    task_claim_id   UUID PRIMARY KEY REFERENCES task_claims(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    artifact_key    TEXT NOT NULL,
    submitted_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ELO ratings

CREATE TABLE elo_ratings (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_type  TEXT NOT NULL,
    entity_id    TEXT NOT NULL,
    rating       DOUBLE PRECISION NOT NULL DEFAULT 1500,
    games_played INT NOT NULL DEFAULT 0,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (entity_type, entity_id)
);

-- Audit log

CREATE TABLE audit_log (
    id              BIGSERIAL PRIMARY KEY,
    action          TEXT NOT NULL,
    actor_user_id   UUID REFERENCES users(id),
    actor_anon_uuid UUID REFERENCES anonymous_workers(uuid),
    target_type     TEXT,
    target_id       TEXT,
    metadata        JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

---

## Action Items

- Define the preendgame task subdivision algorithm.
- Define the dashboard scope for each job type.
- Define exact `natural_key` composition per task type (must guarantee global uniqueness without collisions across jobs).
- Clarify whether CSRF protection is scoped to session-cookie endpoints only (excluding the bearer-token/anon-UUID worker API).
- Define rate limiting / abuse prevention for public unauthenticated endpoints (registration, anonymous worker claim/result).
- Decide deployment mechanics: single container vs. sidecar containers for Axum + SvelteKit on ECS; migration tooling; observability (logging/metrics/tracing).
- Define exact `config` JSONB shape per job type, and SPRT/ELO parameters (alpha, beta, ELO bounds, K-factor) and where they are configured.
- Define what triggers automatic job completion for SPRT-driven job types (who evaluates SPRT significance, and when).

---

## Possible Future Improvements (from fishtest)

### Worker Client Features

- **Self-updating worker binary** — worker checks a version endpoint on startup, downloads a newer version if available (with hash verification), and restarts itself. Reduces manual update burden on contributors.
- **Fleet mode** — a flag that makes the worker exit cleanly on error or empty queue, enabling orchestrators (systemd, Docker, CI) to manage its lifecycle.
- **Global artifact cache** — multiple workers on the same machine or network share downloaded dictionaries and bot binaries rather than each fetching independently.
- **Hardware-aware binary selection** — workers report CPU capabilities and download or compile the appropriate binary variant for their architecture.

### Configuration

- **Per-job-type (or per-job) heartbeat timeout** — the heartbeat timeout window is a single global constant for v1. A future improvement could make it configurable per job type or per individual job.

### Scaling

- **Primary/secondary server split** — one instance owns task scheduling and mutations; read-only instances serve the dashboard. Eliminates concurrent scheduling conflicts under high worker load.
