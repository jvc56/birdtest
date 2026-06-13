# birdtest — Project Plan

## Overview

birdtest is a crowdsourced word game analysis platform, modeled after Fishnet (which crowdsources chess game analysis for Lichess). Users contribute compute by running tasks locally and submitting results back to the site. Admins define jobs and allocate work; the site aggregates results and presents them on a polished dashboard.

---

## Jobs

Jobs are long-running research goals defined and managed by admins. The following job types are supported:

- Analyze all possible opening racks
- Run simulated games
- Run simulated game pairs
- Run static games
- Run static game pairs
- Analyze a single preendgame

Each job has a percentage allocation that determines how frequently workers are assigned tasks from that job. Allocations across all active jobs sum to 100%. Jobs and their allocations are managed by admins only.

---

## Tasks

Tasks are the atomic units of work that workers execute. The following task types are supported:

- Analyze a single position
- Play a game with a given seed
- Play a game pair with a given seed
- Play a batch of games or game pairs with a given seed

Each task has a **natural key** (a seed or position ID) that guarantees global uniqueness. Duplicate tasks are prevented at the database level.

### Task States

Tasks move through an explicit state machine:

```
available → claimed → completed
```

- **available**: Ready to be assigned to a worker.
- **claimed**: Assigned to a worker, with a claim timestamp and a claim token (UUID) recorded.
- **completed**: Results have been submitted and accepted.

If a claimed task is not completed within the timeout window, it is returned to **available** lazily — expiry is checked at the moment a new task is requested, not by a background process.

### Task Generation

Task generation is job-type-dependent:

- Most jobs pre-populate their tasks at creation time.
- Preendgame jobs divide into tasks via an algorithm to be defined later.

---

## Workflow

1. A worker requests a task (anonymously or with an API key).
2. The system selects a job by weighted random selection based on percentage allocations, falling through to the next job if no tasks are available.
3. At this point, expired claimed tasks are lazily reclaimed and returned to available.
4. The system assigns the next available task for the selected job, recording a claim timestamp and issuing a claim token (UUID) to the worker.
5. The worker performs the task and submits results along with the claim token.
6. If the claim token matches the current record, the result is accepted and the task is marked completed. If the token is stale (the task timed out and was reclaimed), the result is silently ignored.

---

## Workers

Workers are the clients that perform tasks and submit results. Two types are supported:

- **Anonymous workers**: No authentication header. All anonymous contributions are attributed to a single synthetic "Anonymous" account.
- **Authenticated workers**: Identified by an API key tied to a user account. Contributions are tracked per user.

---

## User Accounts

Users can create an account to track their contributions. Account creation requires:

- Username
- Password
- Email address (used for account confirmation and password reset)

A confirmation code is sent to the email address on registration. Users can generate one or more API keys from their account, which are used to authenticate task submissions.

API keys are stored as hashes (never raw values) in the database. The raw key is shown to the user exactly once at generation time.

---

## Dashboard

The site hosts a polished dashboard where users can view job status and results. The dashboard shows:

- Current status of all active jobs
- Simple per-job statistics queried directly from raw result data (no pre-aggregation for v1)

Raw result data is also queryable via a public API.

---

## Tech Stack

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

### Key Technology Notes

**Postgres task queue**: Tasks are claimed using `SELECT ... FOR UPDATE SKIP LOCKED`, which allows concurrent workers to claim tasks without lock contention. Timeout reclamation is lazy and runs at claim time.

**Claim tokens**: Each claim issues a UUID token. Workers must submit this token with their results. Stale tokens (from timed-out claims) are silently rejected.

**ECS with Fargate**: The Axum backend and SvelteKit frontend are served from the same ECS service initially. The frontend will be split out to S3 + CloudFront in a later phase.

**Auth**: Sessions use Paseto tokens stored in httpOnly cookies. API keys are random strings hashed with Argon2 before storage.

**Secrets**: Database credentials, signing keys, and SES credentials are stored in AWS SSM Parameter Store and injected into the ECS task at runtime.

---

## Design Decisions & Rationale

| Decision | Rationale |
|---|---|
| Percentage-based job allocation | Simple, predictable, easy to tune later |
| Admin-only job management | Avoids abuse prevention and quota complexity in v1 |
| Natural key uniqueness for tasks | Prevents duplicate work at the DB level, not just app logic |
| Lazy timeout reclamation | No background process needed; simpler to operate |
| Claim token for stale result rejection | Race-condition-free; no timestamp comparison needed |
| Anonymous → single proxy account | Simple attribution; no need to handle anonymous-to-user claim migration |
| No pre-aggregation for dashboard v1 | Simple stats don't require it; avoids premature optimization |
| AWS throughout | Learning goals; avoids future migration pain; production-grade from day one |

---

## Open Items

- Preendgame task subdivision algorithm (to be defined later)
- Schema design (next step)
- API design
- Dashboard scope per job type

---

## Possible Future Improvements (from fishtest)

The following features are drawn from fishtest, the distributed testing framework for Stockfish. They are candidates for inclusion in a later phase.

### Worker Integrity and Anomaly Detection

- **Chi-square testing per worker** — statistical test that flags workers submitting results that deviate significantly from the population. Detects buggy or malicious clients without needing to trust any individual submission.
- **Worker ban list** — a persistent table of banned worker identities; banned workers cannot claim or submit tasks.
- **Redundant task execution** — send the same task to multiple workers and cross-validate results. Could be implemented as an explicit "send to N workers, require M matching" model.

### Heartbeats and Active Lease Tracking

- **Heartbeat endpoint** — workers ping the server periodically to prove they are still alive on a claimed task. Distinguishes "worker died" from "worker is slow," which lazy timeout reclamation alone cannot do.

### Statistical Result Evaluation

- **SPRT (Sequential Probability Ratio Test)** — for game-pair jobs, stop as soon as statistical significance is reached rather than running a fixed number of pairs. Saves compute by terminating early when the outcome is clear.
- **ELO / rating accumulation** — aggregate game results across many runs into a persistent strength rating for each bot or configuration being tested.
- **Completion time forecasting** — estimate hours remaining until a job reaches statistical significance based on current throughput and SPRT progress.

### Worker Client Features

- **Self-updating worker binary** — worker checks a version endpoint on startup, downloads a newer version if available (with hash verification), and restarts itself. Reduces manual update burden on contributors.
- **Fleet mode** — a flag that makes the worker exit cleanly on error or empty queue, enabling orchestrators (systemd, Docker, CI) to manage its lifecycle.
- **Global artifact cache** — multiple workers on the same machine or network share downloaded dictionaries and bot binaries rather than each fetching independently.
- **Hardware-aware binary selection** — workers report CPU capabilities and download or compile the appropriate binary variant for their architecture.

### Contributor Tracking and Transparency

- **Per-worker contribution stats** — games completed, uptime, error rate, etc., surfaced on a machines dashboard page.
- **Audit log** — every significant action (task claimed, result submitted, job created, user banned) written to an append-only log table for debugging and accountability.

### Job Lifecycle Controls

- **Approval workflow** — users submit job requests that admins must approve before workers are assigned tasks. Adds a pending-requests queue layer on top of the current admin-only model.
- **Stop / pause / purge controls** — admins can stop an active job mid-run, purge its accumulated results and restart, or delete it entirely.
- **Job priority field** — a priority field separate from allocation weight; high-priority jobs jump the queue even if their allocation percentage is modest.

### Security

- **CSRF protection** — CSRF tokens tied to the session on all state-mutating endpoints.
- **Password strength enforcement** — reject weak passwords at account creation time (e.g., via zxcvbn) before hashing.

### Data Access

- **Streaming result download** — an endpoint that streams raw result data for a completed job for offline analysis.
- **Paginated public API for finished jobs** — the existing public API plan should include pagination with filtering by username, success status, and time control.

### Scaling

- **Primary/secondary server split** — one instance owns task scheduling and mutations; read-only instances serve the dashboard. Eliminates concurrent scheduling conflicts under high worker load.
