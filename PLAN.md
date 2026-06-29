# birdtest — Project Plan

## High-Level Design

### Overview

birdtest is a crowdsourced word game analysis platform, modeled after Fishnet (which crowdsources chess game analysis for Lichess). Users contribute compute by running tasks locally and submitting results back to the site. Admins define jobs and allocate work; the site aggregates results and presents them on a polished dashboard.

---

### Jobs

Jobs are long-running research goals defined and managed by admins. The following job types are supported:

- **Analyze all possible opening racks**
- **Run games** — autoplay using any player configuration; supports pure static players (no simulation), simming players, or any mix.
- **Run game pairs** — same as games but run as matched pairs (same seed, players swapped) to reduce variance.
- **Leave generation**

Each job has a **priority** and a **percentage allocation**. Priority takes precedence: workers are only assigned tasks from lower-priority jobs when no tasks remain at any higher-priority level. **A lower integer value means higher priority** — priority `0` outranks priority `1`. Allocation percentages govern how work is distributed among jobs at the same priority level; active jobs within a tier must have allocations summing to 100% (enforced at the application layer, not via a DB constraint). Jobs, their priorities, and their allocations are managed by admins only.

#### Job Lifecycle Controls

Jobs are created by admins and become active immediately. The following states are supported:

- **active** — workers are assigned tasks from this job normally.
- **inactive** — the job exists and retains all its tasks and results, but workers are not assigned tasks from it. Admins can reactivate it at any time.
- **completed** — all tasks have been completed, either automatically when the finish condition is met or manually by an admin.

Jobs are created in the **inactive** state. Allocation is not set at creation time — it is supplied by the admin when they activate the job. This keeps the allocation budget coherent: an admin reviews the full set of active jobs, decides the new job's share, and activates it with a specific percentage in a single action.

Admins can deactivate, reactivate, purge (clear all results and return tasks to unclaimed), force-complete, or delete a job at any time.

---

### Tasks

Tasks are the atomic units of work that workers execute. The following task types are supported:

- Analyze a single position
- Play a game with a given seed
- Play a game pair with a given seed
- Play a batch of games or game pairs with a given seed

All game-based tasks (games, game pairs) are identified by a **seed** — a `uint64` value. The combination of `(job_id, seed)` must be unique; duplicate tasks are prevented at the database level. Non-seed tasks (opening rack analysis, leave generation) are deduplicated by their request content via the typed request tables.

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

---

### Workflow

1. The worker sends a **task claim** to the server — a minimal message identifying itself and signaling it is ready for work.
2. The system selects a job by priority tier (lowest integer value = highest priority, descending). Within the top available tier, the job chosen is the one **most behind its configured allocation share** — specifically, the active job with the lowest ratio of `tasks_dispatched / allocation`, where `tasks_dispatched` is the total non-abandoned claims ever issued for that job. Ties are broken by job creation order (oldest first). This is a deterministic deficit-based selection; no randomness is involved.
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

For game and game-pair jobs, results are evaluated using the Sequential Probability Ratio Test (SPRT). A job has two finish conditions:

1. **SPRT significance**: once `min_games` (or `min_pairs`) have been completed, SPRT is evaluated on every submitted result. The job auto-completes as soon as the LLR crosses the significance boundary.
2. **Hard cap**: the job auto-completes when `max_games` (or `max_pairs`) is reached, regardless of SPRT outcome.

SPRT is evaluated inline on every result submission (no background sweep). The server flips the job to `completed` automatically when either condition is met.

For all game pair jobs, Glicko ratings are automatically computed after every game pair result. Ratings are keyed by `(player_config_id, job_id)` — each job maintains its own independent rating table for the player configs involved. The static bot is seeded at 2000; all other player configs start at the Glicko default of 1500. The dashboard displays the current Glicko snapshot for each player config in the job. The dashboard also displays estimated time to completion for active jobs based on current throughput and SPRT progress.

---

### User Accounts

Users can create an account to track their contributions. Account creation requires:

- Username
- Password (minimum strength enforced at registration time)
- Email address (used for account confirmation and password reset)

A confirmation code is sent to the email address on registration. Users can generate one or more API keys from their account, which are used to authenticate task submissions.

API keys are stored as hashes (never raw values) in the database. The raw key is shown to the user exactly once at generation time. Users may hold up to **100 API keys**. Each key can be independently marked **active** or **inactive** — only active keys are accepted for worker authentication. This lets contributors rotate or temporarily disable a key without deleting it.

**v1 account scope**: The sole v1 purpose of a user account is to generate an API token, which attributes task submissions to that account instead of an anonymous UUID. No other feature is gated behind registration. Anonymous workers can complete tasks fully, with no account or API token required.

#### Account Creation Flow

1. User fills out the registration form (`/register`) with username, email, and password.
2. The server validates: username and email are unique; password meets minimum strength (checked server-side using a strength scoring library). Returns `400` with field-level errors on failure.
3. Password is hashed with Argon2 and stored. A confirmation code is generated, hashed, and stored in `email_confirmations`. The raw code is emailed to the user via SES.
4. The frontend redirects to `/register/check-email` — a static holding page instructing the user to check their inbox. No session is created yet.
5. The user clicks the confirmation link in the email, which lands on `/confirm-email?code=<raw-code>`. The page auto-submits the code to `POST /api/auth/confirm-email`.
6. The server hashes the submitted code and matches it against `email_confirmations`. On success, `email_confirmed_at` is set. The user is redirected to `/login`.

Email is confirmed before the first login. Logging in without a confirmed email returns `403` with a message indicating confirmation is required.

#### Login Flow

1. User submits the login form (`/login`) with username and password.
2. The server looks up the user by username. If not found or password doesn't verify, returns `401` (same message for both — no username enumeration).
3. If email is unconfirmed, returns `403` with a prompt to check their inbox.
4. On success: a Paseto token is generated and set as an `httpOnly`, `Secure`, `SameSite=Strict` cookie. The response body carries basic user info (username, `is_admin`).
5. The frontend redirects to `/account` (or to the page the user was trying to access before being redirected to login).

#### Password Reset Flow

1. User clicks "Forgot password?" on `/login` and is taken to `/reset-password`.
2. User enters their email address and submits. The server always returns `200` regardless of whether the email is registered — no account enumeration.
3. If the email matches a confirmed account, the server generates a reset token, hashes it, stores it in `password_reset_tokens` with an expiry, and emails the raw token link via SES.
4. The user clicks the link, landing on `/reset-password/confirm?token=<raw-token>`. The page shows a new-password form.
5. On submit, `POST /api/auth/reset-password/confirm` validates the token (hash match, not expired, not already used), sets `used_at`, hashes and stores the new password, and invalidates all existing sessions for that user.
6. The user is redirected to `/login` with a success message.

---

### Security

**CSRF**: CSRF protection applies to session-cookie-backed endpoints only (Auth API, Account API, Admin API). Worker endpoints (`/api/worker/*`) use bearer tokens or the `X-Worker-UUID` header — neither is sent automatically by browsers, so they are not susceptible to CSRF and are exempt.

**Rate limiting**: Public unauthenticated endpoints are protected against abuse with per-IP (and per-UUID for worker endpoints) rate limiting enforced at the Axum middleware layer using the `governor` crate (token bucket algorithm). Rate-limited responses return `429 Too Many Requests` with a `Retry-After` header. Specific limits (TBD):

| Endpoint | Limit |
|---|---|
| `POST /api/auth/register` | 10 / hour / IP |
| `POST /api/worker/task` | 1 / second / worker identity (UUID for anonymous workers, user ID for authenticated workers) |
| `POST /api/worker/result` | 1 / second / worker identity |
| `POST /api/worker/heartbeat` | 1 / second / worker identity |

For v1, rate limit state is held in-memory (resets on process restart). A persistent backend can be added later for cross-instance coordination.

API keys and session tokens are handled as described in the Auth tech stack note below.

---

### Dashboard

The dashboard has two levels: a **job list page** and a **job detail page** per job.

Live updates are delivered via **Server-Sent Events (SSE)**. The client subscribes to a per-job SSE stream; the server pushes a new event whenever a task result is accepted. SSE is one-way (server → client) and sufficient since the client never needs to send data over the live connection.

#### Job List Page

Shows all jobs with: job type, status, priority, allocation, and a completion counter (tasks completed / total, or games completed / max for on-demand jobs).

#### Job Detail Page — Common Elements (all job types)

- Job metadata: type, status, config summary, created by, created at.
- Completion progress.
- **Per-worker contribution table**: worker identity (username or anonymous UUID), tasks completed for this job. Sorted by tasks completed descending.

#### Job Detail Page — By Job Type

**Games / Game pairs**

- SPRT status text: one of `running`, `passed (H1 accepted)`, `failed (H0 accepted)`, or `terminated at max games`.
- Current Glicko snapshot (game pairs only): rating and rating deviation for each player config.
- Running result counts and percentages: wins / losses / draws for player 1.

**Opening rack analysis**

- Aggregate statistics across all analyzed racks: total racks analyzed, average best equity, distribution of best-move types.
- Search input: enter a rack string to look up its analysis. Returns the full ranked move list (all N plays that were evaluated) for that rack, sourced from `position_analysis_moves`.

**Leave generation**

- Current generation number and the configured per-generation minimum rack target (e.g., "Generation 3 — target: 500 occurrences per rack").
- The rack with the fewest occurrences in the current generation and its current count — shows how far the generation is from completing.

---

Raw result data is queryable via a public API with pagination and filtering by worker. A streaming download endpoint allows offline analysis of completed job results.

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
| Component library | shadcn-svelte (dark mode only; Tailwind `darkMode: 'class'` with `dark` always applied to root) |
| Charts | LayerCake |
| Live updates | Server-Sent Events (SSE) via Axum |
| Auth | Roll your own (Axum + Argon2 + Paseto) |
| Email | AWS SES |
| Secrets | AWS SSM Parameter Store |
| Artifact storage | AWS S3 |
| Infrastructure as Code | Terraform |

#### Key Technology Notes

**Postgres task queue**: Tasks are claimed using `SELECT ... FOR UPDATE SKIP LOCKED`, which allows concurrent workers to claim tasks without lock contention. Timeout reclamation is lazy and runs at claim time.

**Claim tokens**: Each claim issues a UUID token. Workers must submit this token with their results. Stale tokens (from timed-out claims) are silently rejected.

**ECS with Fargate**: Two containers share a single ECS task definition — one running the Axum backend, one running an Nginx container serving the SvelteKit static build. The frontend will be split out to S3 + CloudFront in a later phase.

**Database migrations**: `sqlx migrate run` executes at container startup before the server accepts connections. No separate migration runner needed.

**Observability**: Structured JSON logs via `tracing` + `tracing-subscriber` (JSON formatter). No metrics or distributed tracing for v1.

**Auth**: Sessions use Paseto tokens stored in httpOnly cookies. API keys are random strings hashed with Argon2 before storage.

**Secrets**: Database credentials, signing keys, and SES credentials are stored in AWS SSM Parameter Store and injected into the ECS task at runtime.

**Live dashboard updates**: Each job detail page subscribes to `GET /api/jobs/:id/stream` (SSE). The server pushes a lightweight event after every accepted task result for that job, carrying the updated aggregate stats. The client merges the event into its local state without a full page reload. Axum supports SSE natively via `axum::response::sse`.

---

### Design Decisions & Rationale

| Decision | Rationale |
|---|---|
| Priority-then-allocation job scheduling | Priority gives admins hard ordering guarantees; allocation within a tier gives proportional distribution without needing to touch priorities |
| Admin-only job management | Avoids abuse prevention and quota complexity in v1 |
| Seed uniqueness for seed-based tasks | `(job_id, seed)` unique index prevents duplicate work at the DB level; uint64 seed stored as signed BIGINT, reinterpreted at the application layer |
| No JSONB in schema | All `config` and audit `metadata` are expanded into typed columns and per-job-type config tables; avoids schema-less data and keeps queries typed |
| Lazy timeout reclamation | No background process needed; simpler to operate |
| Claim token for stale result rejection | Race-condition-free; no timestamp comparison needed |
| Anonymous workers identified by UUID | Enables per-worker contribution tracking and result filtering without requiring account creation |
| No pre-aggregation for dashboard v1 | Simple stats don't require it; avoids premature optimization |
| AWS throughout | Learning goals; avoids future migration pain; production-grade from day one |
| Glicko instead of ELO | Glicko models rating uncertainty via rating deviation; static bot seeded at 2000 |
| Named `player_configs` table | Reusable across jobs; maps directly to MAGPIE per-player arguments (`-r1`/`-r2`, `-s1`/`-s2`, etc.); **immutable once created** — no update endpoint exists; deletion only if no job references the config |
| Frontend dark mode only | Single theme simplifies the component library configuration; no light/dark toggle in v1 |
| Deficit-based job selection | Deterministic; guarantees long-run allocation accuracy regardless of claim timing; no randomness means reproducible behavior and no starvation |
| Seed gap of batch size | Prevents two tasks from covering overlapping game seeds; `next_seed = MAX(seed) + batch_size` so seeds tile without gaps or overlaps |
| Glicko ratings keyed by (player_config, job) | Each job is an independent experiment; pooling ratings across jobs would conflate different experimental conditions |
| Two finish conditions for SPRT jobs | `min_games`/`min_pairs` prevents early false-positive termination; `max_games`/`max_pairs` bounds compute cost |
| Jobs created inactive | Allocation is set at activation time, not creation, so the admin reviews the full active job set and assigns percentages as a single deliberate act |
| API keys active/inactive toggle | Lets contributors rotate or temporarily suspend a key without losing it; only active keys accepted for auth |
| Account deletion is app-layer, not CASCADE | Task counters must be decremented and tasks may revert state; a DB-level cascade cannot update denormalized counters |
| SPRT evaluated on every result submission | No background sweep needed; keeps the system simple in v1 |
| Two containers per ECS task | Axum backend + Nginx for SvelteKit static files; cleaner than co-mingling in one process |

---

## Low-Level Design

### Request Handling

The core of birdtest is the task claim endpoint — the sequence that runs every time a worker asks for work.

1. **Auth and verification**: The server reads the worker identity from request headers (`Authorization: Bearer <api-key>` for authenticated workers, `X-Worker-UUID` for anonymous workers). It verifies the worker is not banned and upserts the worker record (`users` or `anonymous_workers`).

2. **Job selection**: The server filters to active jobs (excluding inactive and completed), then selects by priority tier — lowest integer value first (priority `0` outranks priority `1`). Within the top available tier, the server selects the job with the lowest ratio of `tasks_dispatched / allocation` — the job most behind its configured share. `tasks_dispatched` is the total number of non-abandoned claims ever issued for the job (a count that only goes up). Ties break on `created_at ASC`. This is implemented as a single SQL `ORDER BY` query; no randomness is involved.

   ```sql
   SELECT j.id
   FROM jobs j
   WHERE j.status = 'active' AND j.priority = $min_priority
   ORDER BY
     (SELECT COUNT(*) FROM task_claims tc
      JOIN tasks t ON t.id = tc.task_id
      WHERE t.job_id = j.id AND tc.state != 'abandoned')::float
     / NULLIF(j.allocation, 0) ASC,
     j.created_at ASC
   LIMIT 1
   ```

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
| `PositionRequest` | Opening rack analysis |
| `GameRequest` | Games, game pairs |
| `LeaveRequest` | Leave generation |

### Task Response Types

A task response is what the worker submits after completing a task. It is validated on receipt and then transformed into a task record for storage. Response types may differ from their corresponding request types (e.g., a single seed request may yield a batch of game results).

| Type | Used by |
|---|---|
| `PositionAnalysisResponse` | Opening rack analysis |
| `GameResultsResponse` | Games |
| `GamePairResultsResponse` | Game pairs |
| `LeaveResponse` | Leave generation |

### Task Record Types

A task record is the normalized form stored in a typed table after a response is accepted. It may omit raw fields, recompute derived values, or canonicalize formats. Task record types may be shared when the stored shape is the same regardless of how the task was generated.

| Type | Used by |
|---|---|
| `PositionAnalysisRecord` | Opening rack analysis |
| `GameResultsRecord` | Games |
| `GamePairResultsRecord` | Game pairs |
| `LeaveRecord` | Leave generation |

### Creation Strategies

**Pre-populated**: At job creation time the server generates all task requests and inserts them into the `tasks` table with `state = 'available'`. Workers then claim from this pool. Suitable for finite, enumerable work spaces — opening rack analysis.

**On-demand**: No tasks are inserted at job creation. When a worker requests a task, the server generates the next task request, then inserts and claims it atomically in a single transaction, issuing a claim token. The task never passes through `available` state. Suitable for seed-based work where the space is effectively unbounded — games, game pairs, leave generation.

### Per-Job-Type Creation Details

#### Opening Rack Analysis — Pre-populated

At job creation the server enumerates every distinct 7-tile multiset drawable from the lexicon's tile bag and inserts one task + one `position_requests` row per rack, all in a single transaction (or a streaming batch insert).

Steps:
1. Load the tile distribution for the job's `lexicon` (letter counts, blank count).
2. Enumerate all distinct 7-tile multisets via combinatorial iteration over the tile multiset. Each rack is represented as a canonical string of sorted tile characters (e.g., `"AABCELT"`, blanks as `"?"`).
3. For each rack, encode it as a CGP position string for an empty board with that rack (the format MAGPIE accepts for `cg` / position analysis).
4. Within one transaction: `INSERT INTO tasks (job_id, seed, state, ...) VALUES ($job_id, NULL, 'available', ...)` then `INSERT INTO position_requests (task_id, lexicon, variant, position, previous_play, player_config_id) VALUES (...)`. `previous_play` is NULL for all opening racks (empty board, no prior move). `player_config_id` is copied from `job_opening_rack_config`. Repeat for all racks.

Uniqueness is enforced by the position content (the same rack on an empty board always produces the same CGP string). The letter distribution files in `data/letterdistributions/` are used to enumerate the bag. The total task count for a standard English Scrabble bag is on the order of several hundred thousand.

---

#### Games — On-demand

Each task represents one batch of games (`games_per_batch` from the job config) played starting at a given seed. MAGPIE uses seeds S, S+1, …, S+N−1 for a batch starting at seed S with batch size N. To prevent two tasks from overlapping on the same game seeds, consecutive task seeds are spaced `games_per_batch` apart.

At claim time (all in one transaction):
1. Compute next seed: `SELECT COALESCE(MAX(seed) + $games_per_batch, 1) FROM tasks WHERE job_id = $job_id`. This yields seed 1 for the first task, then `1 + games_per_batch`, `1 + 2*games_per_batch`, etc. The insert in step 3 will conflict on the unique seed index if two workers race; the loser retries.
2. `INSERT INTO tasks (job_id, seed, state, active_claim_count) VALUES ($job_id, $next_seed, 'claimed', 1) RETURNING id`.
3. `INSERT INTO game_requests (task_id, lexicon, variant, seed, num_games, player1_config_id, player2_config_id)` — denormalize all values from the job config so the worker receives a self-contained request.
4. `INSERT INTO task_claims (task_id, claim_token, state, claimed_by_...)`.
5. Return the request + claim token to the worker.

SPRT and finish-condition checks run during result submission, not at claim time.

---

#### Game Pairs — On-demand

Same as games, except the batch size is `pairs_per_batch` from the job config. Each task seed is spaced `pairs_per_batch` apart: `SELECT COALESCE(MAX(seed) + $pairs_per_batch, 1) FROM tasks WHERE job_id = $job_id`. The job type signals MAGPIE to run both orderings (p1/p2 then p2/p1) with the same seed in a single invocation. Results are `GamePairResultsResponse`. SPRT is evaluated on pair outcomes. Glicko ratings are automatically updated after each submitted result.

---

#### Leave Generation — On-demand, sequential generations

Leave generation has sequential phases: generation N must complete before generation N+1 begins. Within a generation, work is done by a single worker running the full generation (multiple parallel workers per generation is not supported in v1). Each task represents one generation.

**State**: The job tracks which generation is currently in progress via the completed task count. The output of each generation (a leave file) is stored in S3 and referenced by `leave_records.artifact_key`. The next generation's task receives the previous generation's artifact key as input.

At claim time:
1. Count completed `leave_generation` tasks for this job → `completed_generations`.
2. If `completed_generations == configured_generation_count` → no work; the job should already be marked complete.
3. If any `task_claims` row for this job has `state = 'claimed'` → a generation is already in progress; return "no work" (only one task in-flight at a time).
4. Determine `next_generation = completed_generations + 1` and find `previous_artifact_key` from the most recent `leave_records` row (NULL for generation 1 — worker uses the lexicon's default leaves).
5. `INSERT INTO tasks (job_id, seed=NULL, state='claimed', active_claim_count=1) RETURNING id`.
6. `INSERT INTO leave_requests (task_id, lexicon, variant, iteration=$next_generation)`.
7. `INSERT INTO task_claims (...)`.
8. Return the request, including `previous_artifact_key` so the worker knows which leave file to seed from.

**Worker behaviour**: The worker downloads the previous generation's leave file from S3 (or uses the built-in default for generation 1), invokes MAGPIE's `autoplay` command with leave-generation parameters for the configured number of games, then uploads the resulting leave file to S3 and submits the key as the result.

**Dashboard progress**: The rack-with-fewest-occurrences display requires the worker to report intermediate progress. Workers include a `progress` payload with their heartbeat (`POST /api/worker/heartbeat`) containing the current minimum rack occurrence count and the rack string. The server stores this on the in-progress `task_claims` row for display.

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

### Skeleton Implementation

Single file (`worker/worker.py`). Business logic (MAGPIE invocation, result parsing) is omitted; structure and all integration points are shown.

```python
#!/usr/bin/env python3
"""birdtest worker client — claims tasks, invokes MAGPIE, submits results."""

import argparse
import logging
import os
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import requests
import tomllib

__version__ = "1.0.0"

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

@dataclass
class Config:
    server_url: str
    magpie_dir: Path
    api_key: Optional[str]        # None → anonymous worker (X-Worker-UUID header)
    worker_uuid: str              # persistent across runs; generated on first run
    heartbeat_interval: int = 30  # seconds between heartbeats
    retry_delay_seconds: int = 5  # seconds to wait when the server has no work

def _load_or_generate_uuid(state_dir: Path) -> str:
    """Read persistent UUID from disk; generate and save one if absent."""
    path = state_dir / "worker_uuid"
    if path.exists():
        return path.read_text().strip()
    worker_uuid = str(uuid.uuid4())
    state_dir.mkdir(parents=True, exist_ok=True)
    path.write_text(worker_uuid)
    return worker_uuid

def _load_config(args: argparse.Namespace) -> Config:
    """Merge TOML config file with CLI flags; flags take precedence."""
    ...

# ---------------------------------------------------------------------------
# HTTP helpers
# ---------------------------------------------------------------------------

def _auth_headers(cfg: Config) -> dict:
    if cfg.api_key:
        return {"Authorization": f"Bearer {cfg.api_key}"}
    return {"X-Worker-UUID": cfg.worker_uuid}

def _check_for_self_update(cfg: Config) -> None:
    """
    GET /api/worker/client-version. If the server reports a version different from
    __version__, download the new script to a temp file and re-exec this process with
    it, passing along all original argv. Does not return if an update is applied.
    """
    info = requests.get(f"{cfg.server_url}/api/worker/client-version").json()
    if info["version"] == __version__:
        return
    logger.info("Updating worker %s → %s", __version__, info["version"])
    with tempfile.NamedTemporaryFile(suffix=".py", delete=False) as tmp:
        tmp.write(requests.get(info["download_url"]).content)
        tmp_path = tmp.name
    os.chmod(tmp_path, 0o755)
    os.execv(sys.executable, [sys.executable, tmp_path] + sys.argv[1:])

def _get_magpie_version(cfg: Config) -> str:
    """Run `magpie version` once and return the version string. Cached by the caller."""
    result = subprocess.run(
        [cfg.magpie_dir / "bin" / "magpie", "version"],
        capture_output=True, text=True, check=True,
    )
    return result.stdout.strip()

def _claim_task(cfg: Config) -> Optional[dict]:
    """POST /api/worker/task. Returns parsed body, or None on 204 (no work available)."""
    ...

def _send_heartbeat(cfg: Config, claim_token: str, progress: Optional[dict] = None) -> None:
    """POST /api/worker/heartbeat. `progress` carries leave-gen rack data when set."""
    ...

def _submit_result(cfg: Config, claim_token: str, result: dict) -> None:
    """POST /api/worker/result."""
    ...

# ---------------------------------------------------------------------------
# Task handlers — one function per job type
# ---------------------------------------------------------------------------

def _handle_opening_rack(request: dict, cfg: Config) -> dict:
    """Invoke MAGPIE to analyze a single opening rack position."""
    ...

def _handle_game(request: dict, cfg: Config) -> dict:
    """Invoke MAGPIE autoplay to run a batch of games."""
    ...

def _handle_game_pair(request: dict, cfg: Config) -> dict:
    """Invoke MAGPIE autoplay with -gp true for a batch of game pairs."""
    ...

def _handle_leave_gen(request: dict, cfg: Config) -> dict:
    """Download previous-gen leaves from S3, run autoplay, upload result, return S3 key."""
    ...

_HANDLERS = {
    "opening_rack_analysis": _handle_opening_rack,
    "games":                 _handle_game,
    "game_pairs":            _handle_game_pair,
    "leave_generation":      _handle_leave_gen,
}

# ---------------------------------------------------------------------------
# Heartbeat thread
# ---------------------------------------------------------------------------

def _heartbeat_loop(
    cfg: Config,
    claim_token: str,
    stop: threading.Event,
    progress_fn,  # () -> Optional[dict]; leave-gen supplies live rack data, others return None
) -> None:
    while not stop.wait(timeout=cfg.heartbeat_interval):
        try:
            _send_heartbeat(cfg, claim_token, progress=progress_fn())
        except Exception:
            logger.warning("Heartbeat failed", exc_info=True)

# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def _worker_loop(cfg: Config, magpie_version: str) -> None:
    while True:
        # 1. Claim
        response = _claim_task(cfg)
        if response is None:
            time.sleep(cfg.retry_delay_seconds)
            continue

        claim_token = response["claim_token"]
        task_request = response["task_request"]
        min_ver = response.get("min_magpie_version")

        # 2. Version gate — skip task if MAGPIE is too old; claim expires server-side
        if min_ver and magpie_version < min_ver:
            logger.error(
                "MAGPIE %s < required %s for this job; skipping task", magpie_version, min_ver
            )
            continue

        handler = _HANDLERS[task_request["job_type"]]

        # 3. Heartbeat
        stop = threading.Event()
        hb = threading.Thread(
            target=_heartbeat_loop,
            args=(cfg, claim_token, stop, lambda: None),
            daemon=True,
        )
        hb.start()

        result = None
        try:
            # 4. Execute
            result = handler(task_request, cfg)
        except Exception:
            logger.exception("Task execution failed; claim will expire server-side")
        finally:
            stop.set()
            hb.join()

        # 5. Submit
        if result is not None:
            _submit_result(cfg, claim_token, result)

def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="birdtest worker client")
    p.add_argument("--config", type=Path, default=Path("~/.birdtest/config.toml").expanduser())
    p.add_argument("--magpie-dir", type=Path)
    p.add_argument("--api-key")
    p.add_argument("--server-url")
    return p.parse_args()

def main() -> None:
    args = _parse_args()
    cfg = _load_config(args)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    # 1. Self-update check — re-execs this process if a newer script is available
    _check_for_self_update(cfg)

    # 2. Cache MAGPIE version once at startup
    magpie_version = _get_magpie_version(cfg)
    logger.info(
        "Worker started (uuid=%s, authenticated=%s, magpie=%s, client=%s)",
        cfg.worker_uuid, cfg.api_key is not None, magpie_version, __version__,
    )

    _worker_loop(cfg, magpie_version)

if __name__ == "__main__":
    main()
```

---

## API

All endpoints return JSON. State-mutating endpoints require a valid CSRF token. Worker endpoints accept either an `Authorization: Bearer <api-key>` header (authenticated workers) or no auth header plus an `X-Worker-UUID` header (anonymous workers).

### Worker API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/worker/client-version` | Returns the current worker script version and S3 download URL. Workers call this on startup to self-update. |
| `POST` | `/api/worker/task` | Send a task claim. Returns a job-type-specific task request, claim token, and `min_magpie_version` for the assigned job. |
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
| `GET` | `/api/me/api-keys` | List API keys (label, active status, created/last-used timestamps; hashes are never returned). |
| `POST` | `/api/me/api-keys` | Generate a new API key. Returns the raw key exactly once. Rejected if the user already has 100 keys. |
| `PATCH` | `/api/me/api-keys/:id` | Set a key's active status (`{ "is_active": bool }`). |
| `DELETE` | `/api/me/api-keys/:id` | Permanently revoke an API key. |

### Admin API

All Admin API endpoints require the requesting user to have `is_admin = TRUE`. Requests from non-admin authenticated users or anonymous workers are rejected with `403 Forbidden`.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/admin/player-configs` | List all player configurations. |
| `POST` | `/api/admin/player-configs` | Create a new player configuration. |
| `GET` | `/api/admin/player-configs/:id` | Get a single player configuration. |
| `DELETE` | `/api/admin/player-configs/:id` | Delete a player configuration. Rejected if any job references it. |
| `POST` | `/api/admin/jobs` | Create a new job. Becomes active immediately. |
| `POST` | `/api/admin/jobs/:id/deactivate` | Set a job to inactive. Workers will no longer be assigned tasks from it. |
| `POST` | `/api/admin/jobs/:id/activate` | Activate an inactive job. Body: `{ "allocation": int }`. Sets allocation and transitions status to active. |
| `POST` | `/api/admin/jobs/:id/complete` | Force-complete a job immediately, regardless of task progress. |
| `POST` | `/api/admin/jobs/:id/purge` | Purge all results for a job and return tasks to unclaimed. |
| `DELETE` | `/api/admin/jobs/:id` | Delete a job and all its tasks. |
| `DELETE` | `/api/admin/users/:id` | Delete a user account and all their task claims and records. |
| `POST` | `/api/admin/workers/ban` | Ban a worker by user ID or anonymous UUID. |
| `DELETE` | `/api/admin/workers/ban/:id` | Remove a ban. |
| `GET` | `/api/admin/audit-log` | Query the audit log with filtering and pagination. |

### Public API

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/jobs` | List jobs with status and summary stats. Paginated. |
| `GET` | `/api/jobs/:id` | Job detail, configuration, and aggregate statistics. |
| `GET` | `/api/jobs/:id/results` | Paginated task records for a job. Filterable by worker. |
| `GET` | `/api/jobs/:id/stream` | SSE stream of live stat updates for a job. Pushes an event after each accepted result. |
| `GET` | `/api/jobs/:id/results/stream` | Streaming download of all task records for offline analysis. |
| `GET` | `/api/users` | List all registered user accounts with contribution stats. Paginated. |
| `GET` | `/api/workers` | Contributor stats for all workers (anonymous and authenticated), paginated. |

---

## Frontend Routes

SvelteKit uses file-based routing under `frontend/src/routes/`. Each directory with a `+page.svelte` is a page. Layout files (`+layout.svelte`) apply to all routes nested beneath them.

### Public Routes

| Route | Page |
|---|---|
| `/` | Landing page — brief description of birdtest, links to the job list and the worker setup guide. |
| `/jobs` | Job list — all jobs with type, status, priority, and completion counter. Live-updated via SSE. |
| `/jobs/[id]` | Job detail — job-type-specific stats and per-worker contribution table. Live-updated via SSE. |
| `/users` | Registered user list — all user accounts with contribution stats. |
| `/workers` | Contributor leaderboard — all workers (anonymous and authenticated) ranked by tasks completed. |

### Auth Routes

| Route | Page |
|---|---|
| `/register` | Registration form — username, email, password with client-side strength feedback. |
| `/register/check-email` | Static holding page shown after successful registration — instructs the user to check their inbox. |
| `/confirm-email` | Email confirmation landing — reads `?code=` from the URL, auto-submits to the API, shows success or error. On success redirects to `/login`. |
| `/login` | Login form. On success redirects to `/account` or the originally requested page. |
| `/reset-password` | Password reset request form — enter email address. |
| `/reset-password/confirm` | Password reset apply form — reads token from URL, shows new password field. |

### Authenticated Routes

Protected by a layout guard (`/account/+layout.svelte`) that redirects unauthenticated users to `/login`.

| Route | Page |
|---|---|
| `/account` | Account overview — username, email, confirmation status, API key list (labels only), generate/revoke API keys. |

### Admin Routes

Protected by a layout guard (`/admin/+layout.svelte`) that requires `is_admin = true`; redirects non-admins to `/`.

| Route | Page |
|---|---|
| `/admin` | Admin overview — redirects to `/admin/jobs`. |
| `/admin/jobs/new` | Create job form — job type selector, then type-specific config fields. |
| `/admin/jobs/[id]` | Admin job view — same stats as the public detail page plus controls: deactivate, activate, purge, delete. |
| `/admin/player-configs` | Player config list — name, recorder type, sort strategy, sim parameters. |
| `/admin/player-configs/new` | Create player config form. |
| `/admin/users` | User account list — delete accounts. (Contribution stats are shown publicly at `/users`.) |
| `/admin/workers` | Worker ban management — ban / unban workers by user ID or anonymous UUID. |
| `/admin/audit-log` | Audit log viewer — filterable by action type, actor, and target; paginated. |

---

## Directory Structure

```
birdtest/
├── backend/                        # Axum web server (Rust)
│   ├── Cargo.toml
│   ├── migrations/                 # sqlx migration files
│   │   └── 0001_initial.sql
│   └── src/
│       ├── main.rs                 # server startup, router assembly
│       ├── config.rs               # config loading from SSM / env
│       ├── db.rs                   # PgPool initialization
│       ├── error.rs                # AppError type, IntoResponse impl
│       ├── auth/
│       │   ├── mod.rs
│       │   ├── session.rs          # Paseto token creation / validation
│       │   ├── api_key.rs          # API key hashing / verification
│       │   └── csrf.rs             # CSRF token middleware
│       ├── email.rs                # SES email sending
│       ├── jobs/                   # job type system
│       │   ├── mod.rs
│       │   ├── handler.rs          # JobHandler trait definition
│       │   ├── registry.rs         # JobType enum and dispatch
│       │   ├── opening_rack.rs
│       │   ├── game.rs
│       │   ├── game_pair.rs
│       │   └── leave_gen.rs
│       ├── models/                 # SQLx row types (one file per table group)
│       │   ├── mod.rs
│       │   ├── job.rs
│       │   ├── task.rs
│       │   ├── claim.rs
│       │   ├── user.rs
│       │   └── worker.rs
│       ├── routes/                 # Axum handlers (one file per API section)
│       │   ├── mod.rs
│       │   ├── worker.rs           # /api/worker/*
│       │   ├── auth.rs             # /api/auth/*
│       │   ├── account.rs          # /api/me/*
│       │   ├── admin.rs            # /api/admin/*
│       │   └── public.rs           # /api/jobs/*, /api/users, /api/workers
│       └── sse.rs                  # SSE broadcaster (job result push)
│
├── frontend/                       # SvelteKit app
│   ├── package.json
│   ├── svelte.config.js
│   ├── vite.config.ts
│   └── src/
│       ├── app.html
│       ├── app.css
│       ├── lib/
│       │   ├── api.ts              # typed fetch wrappers for every API endpoint
│       │   ├── auth.ts             # session store (current user, is_admin)
│       │   ├── sse.ts              # SSE subscription helper
│       │   └── components/         # shared UI components
│       │       ├── JobStatusBadge.svelte
│       │       ├── WorkerTable.svelte
│       │       └── ...
│       └── routes/
│           ├── +layout.svelte      # global layout (nav bar, footer)
│           ├── +page.svelte                        # /
│           ├── jobs/
│           │   ├── +page.svelte                    # /jobs
│           │   └── [id]/
│           │       └── +page.svelte                # /jobs/[id]
│           ├── users/
│           │   └── +page.svelte                    # /users
│           ├── workers/
│           │   └── +page.svelte                    # /workers
│           ├── register/
│           │   ├── +page.svelte                    # /register
│           │   └── check-email/
│           │       └── +page.svelte                # /register/check-email
│           ├── confirm-email/
│           │   └── +page.svelte                    # /confirm-email
│           ├── login/
│           │   └── +page.svelte                    # /login
│           ├── reset-password/
│           │   ├── +page.svelte                    # /reset-password
│           │   └── confirm/
│           │       └── +page.svelte                # /reset-password/confirm
│           ├── account/
│           │   ├── +layout.svelte                  # auth guard: redirect to /login if no session
│           │   └── +page.svelte                    # /account
│           └── admin/
│               ├── +layout.svelte                  # auth guard: redirect to / if not is_admin
│               ├── +page.svelte                    # /admin (redirects to /admin/jobs)
│               ├── jobs/
│               │   ├── new/
│               │   │   └── +page.svelte            # /admin/jobs/new
│               │   └── [id]/
│               │       └── +page.svelte            # /admin/jobs/[id]
│               ├── player-configs/
│               │   ├── +page.svelte                # /admin/player-configs
│               │   └── new/
│               │       └── +page.svelte            # /admin/player-configs/new
│               ├── users/
│               │   └── +page.svelte                # /admin/users
│               ├── workers/
│               │   └── +page.svelte                # /admin/workers
│               └── audit-log/
│                   └── +page.svelte                # /admin/audit-log
│
├── worker/                         # Python worker client
│   ├── pyproject.toml
│   └── worker.py                   # single-file implementation (see Worker Client section)
│
├── data/
│   └── letterdistributions/        # letter distribution files, mirroring MAGPIE-DATA/data/letterdistributions/
│                                   # used by the server to enumerate racks for opening rack analysis jobs
│
└── infra/                          # Terraform
    ├── main.tf
    ├── variables.tf
    ├── outputs.tf
    ├── ecs.tf                      # ECS cluster, task definition, service
    ├── rds.tf                      # RDS Postgres instance and security group
    ├── s3.tf                       # S3 bucket for artifacts
    ├── ses.tf                      # SES domain and sending identity
    └── ssm.tf                      # SSM Parameter Store entries (names only; values set manually)
```

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
    num_iterations INT NOT NULL
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
--   2. Delete all task records (game_records, etc.) linked to those claims.
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
    -- For leave generation: the worker reports the bottleneck rack and its occurrence count
    -- with each heartbeat so the dashboard can show generation progress.
    progress_rack        TEXT,
    progress_occurrences INT,
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
```

---

## Possible Future Improvements (from fishtest)

### Worker Client Features

- **Fleet mode** — a flag that makes the worker exit cleanly on error or empty queue, enabling orchestrators (systemd, Docker, CI) to manage its lifecycle.
- **Global artifact cache** — multiple workers on the same machine or network share downloaded dictionaries and bot binaries rather than each fetching independently.
- **Hardware-aware binary selection** — workers report CPU capabilities and download or compile the appropriate binary variant for their architecture.

### Configuration

- **Per-job-type (or per-job) heartbeat timeout** — the heartbeat timeout window is a single global constant for v1. A future improvement could make it configurable per job type or per individual job.

### Scaling

- **Primary/secondary server split** — one instance owns task scheduling and mutations; read-only instances serve the dashboard. Eliminates concurrent scheduling conflicts under high worker load.

---

## Required MAGPIE Changes

MAGPIE requires the following additions to be compatible with birdtest. These changes should be made in the [MAGPIE repository](https://github.com/jvc56/MAGPIE) and versioned with semver so birdtest can enforce `min_magpie_version` per job.

### `version` command

MAGPIE currently has no way to report its version. Add a `version` command (invocable as `magpie version`) that prints a single semver string to stdout and exits with code 0:

```
1.4.0
```

The birdtest worker calls this once at startup (`_get_magpie_version`) to cache the version for per-task compatibility checks.

### Additional changes (to be identified during implementation)

Further integration requirements will be discovered as the worker handlers are implemented. Known areas likely to require changes:

- **Structured / machine-readable output** — the worker needs to reliably parse autoplay and analysis results. If MAGPIE's current output format is human-readable text, a JSON or CSV output mode may be needed.
- **Rack occurrence reporting for leave generation** — the worker needs to extract per-rack occurrence counts from an autoplay run to report progress via heartbeat. MAGPIE may need a flag to emit this data.
- **Non-interactive (scripted) execution mode** — `autoplay` and position analysis must run to completion without requiring interactive input. Confirm that the existing `-mo` / `mode` option covers this, or add a dedicated batch mode.
