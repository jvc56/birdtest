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
