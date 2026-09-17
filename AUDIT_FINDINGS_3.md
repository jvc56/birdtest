# birdtest audit — findings, version 3

Branch: `audit/birdtest-2026-09-16`, off `audit/birdtest-2026-09-15` (which is
`main` at `6a72333` plus the sixth audit's three commits, unmerged when this
audit began — see "Where this branch sits").
MAGPIE: `birdtest-contribute` at `6308b63c`, **no changes made** (section 8).
Date: 2026-09-16.

**This is the seventh audit.** It builds on [AUDIT_FINDINGS_2.md](AUDIT_FINDINGS_2.md)
(the sixth audit, branch `audit/birdtest-2026-09-15`), on
[AUDIT_FINDINGS_1.md](AUDIT_FINDINGS_1.md) (the fifth, PR #6) and, through
them, on the four unnumbered records before those (branches
`audit/birdtest-2026-09-11`, `-09-13`, `-09-13-pass2` and `-09-14`, whose
shared `AUDIT_FINDINGS.md` on `main` is the fourth audit's record). The highest
numbered file anywhere in the history is `AUDIT_FINDINGS_2.md`, so this is
`AUDIT_FINDINGS_3.md`; the earlier files are untouched. Where an entry revisits
a prior item it names it ("prior U1", "prior B1").

This file is the authoritative record of every code-versus-`PLAN.md` decision
made in this audit, and of the bugs, MAGPIE argument gaps, critical-path
analysis, performance and storage findings behind them.

**Counts: 4 code-wins (PLAN.md updated to match the code), 7 plan-wins (code
changed), 1 item left for human input (U1).**

The default bias is that the code wins and `PLAN.md` is brought level with it.
The code was changed only where it was wrong, or where the plan described the
behaviour the rest of the system needs.

---

## Where this branch sits

The sixth audit's branch (`audit/birdtest-2026-09-15`, three commits ending at
`8a5d6c3`, with `AUDIT_FINDINGS_2.md`) had not been merged to `main` and had no
pull request when this audit began. Branching from `main` would have meant
re-finding what that audit fixed and producing a branch that conflicts with it,
so this branch is cut from the sixth audit's head instead: it is `main` plus
the sixth audit plus this one, and merging it merges both. Nothing in the sixth
audit's record was re-litigated; section 0 checks its open items.

---

## 0. What the prior audit left open

| Prior item | Still true? | How it was checked |
|---|---|---|
| U1 publish `65a246d5`, move the pin, raise the floor to `0.5.1` — "**the push is still to do**" | **Resolved since.** `origin/birdtest-contribute` is `6308b63c`, one commit past `65a246d5` (the fixture bump the prior record said was uncommitted), and the MAGPIE checkout is clean | `git fetch origin birdtest-contribute`; `git status`; the backend image built from `docker/Dockerfile`'s pin `65a246d5` in this audit, which does a shallow fetch of that commit from GitHub |
| U2 thin old rating runs | Built (`ratings::thin_old_runs`, hourly) | Read; its test passes |
| U3 leave the CGP as `TEXT` | Nothing to build | — |
| U4 wait for the slow-stats log line | Nothing to build | — |
| Prior B1 (moves cascade) | Holds — and had a sibling it did not cover: the *records* cascade from `task_claims` has the same shape (B3 below) | `EXPLAIN` against the migration |

Nothing else was pending. The prior record's section 9 (the Python worker is
never described as a production client) was re-checked and still holds
(section 9 below).

---

## 1. How this audit was run

- Read `PLAN.md` in full; diffed its schema block mechanically against
  `backend/migrations/0001_initial.sql` (identical at the start; identical
  again after this audit's edits); read the sixth audit's record in full and
  the fifth's open items; then every backend module — scheduler, worker
  routes, registry, all four job types, `derived.rs`, `magpie.rs`,
  `inputdata.rs`, `exports.rs`, `artifacts.rs`, plausibility, jobstats, SSE,
  admin, public, account, auth and rating routes, `ratings.rs`, the two stats
  modules, config, `main.rs`, `build-derived`, the test harness — plus CI, the
  nightly workflow, Docker, compose, the Terraform, the scripts, the fake
  worker, the frontend's API client, and the four operational documents.
- On MAGPIE's `birdtest-contribute` at `6308b63c`: `src/impl/contribute.c` in
  full; in `src/impl/config.c` every contribute executor, both resets, the
  lexical load, `config_load_lexicon_dependent_data`'s flag handling, and the
  `Config` struct field by field against what the resets cover; `autoplay.c`
  for the play chooser, overtime and endgame paths; `client_state.c` for the
  settings keys; the Makefile's build targets; the contract fixtures against
  birdtest's (identical).
- Backend: `cargo clippy --locked --all-targets -- -D warnings` and
  `cargo test --locked` against Postgres 16 (the compose `postgres` service).
  **159 tests before, 163 after** (90 unit and contract, 73 integration), all
  passing, clippy clean. The frontend is untouched.
- `EXPLAIN` with `enable_seqscan = off` against a database built from the
  migration, for every foreign-key cascade a purge or a delete fires (B3).
- MAGPIE: `make magpie_test` (`-Werror`, address/undefined/leak sanitizers)
  and `./bin/magpie_test contribute`, passing; `magpie builders` reports
  `0.5.1`/`nehalem`; `magpie help convert` lists the three conversions the
  server runs.
- End to end: `scripts/e2e_magpie.py` against an isolated compose project
  (`birdtest-e2e`, its own volumes and ports, backend image built from this
  branch's working tree with the new migration and the dispatch template) with
  the checkout's `portable_release` MAGPIE `0.5.1` and a MAGPIE-DATA
  `data-20251004` import through the real API. **Every job type passed**
  (section 11).

---

## 2. MAGPIE arguments that change a task's outcome

The prior records' table was re-derived against `6308b63c` rather than reused:
every field the three executors read, traced to where the contribute path
sets, resets or verifies it, and then the other direction — every `Config`
field, asked whether autoplay, move generation or a simulation reads it and,
if so, whether a request states it or a reset covers it. Everything in the
prior tables still holds; `0.5.1`'s word info table reset (prior M1) is on the
branch. Two fields were checked this time that no prior record names:

| Setting | Changes results? | How it is covered | Change |
|---|---|---|---|
| `overtime_penalty_points` / `overtime_period_ms` (`-overtime*`), not reset by `config_contribute_reset_shared_settings` | Only when a player uses the **play chooser**: `game_runner_assess_overtime` skips a player with `use_play_chooser` off, and the reset sets `p*_play_chooser_time_ms` to `-1`, which is off. A contributor's leftover overtime settings therefore reach no contributed game | None |
| `challenge_bonus` (`-cb`) | No: read only when a GCG's challenge events are replayed (`game_history`); autoplay never challenges | None |
| `use_mmap_for_rit` | No: how a table is read into memory, not what it holds | None |
| Endgame and pre-endgame settings (`endgame_plies`, `tt_fraction_of_mem`, `peg_*`) | No: autoplay's players never call the endgame or pre-endgame solvers; both are separate commands | None |

**No gap found; no MAGPIE change made.** birdtest's `PlayerSpec` and the
run-wide fields on every request cover every setting that reaches a result,
and MAGPIE refuses a request that leaves one out (`contribute_require_keys`).

---

## 3. Bugs and footguns

### B1 — an export could be blocked for good by a claim whose worker vanished

*Plan wins: **code changed** (K1).*

- **What the code did.** `exports::start` refused (`409`) while any claim of
  the job was `claimed`, and its message promised the wait was "at most the
  heartbeat timeout". But a claim only becomes `abandoned` through
  `scheduler::reclaim_expired_for`, which runs when a worker asks for work
  from the job's priority tier — and a completed job is never in any
  worker's candidate list. So a claim whose worker died (or was killed by a
  deploy) after the job completed stayed `claimed` forever, and every export
  of that job was refused forever. The job list's `tasks_claimed` figure and
  `GET /api/jobs/:id`'s `tasks_claimed` showed it too.
- **Fix.** `exports::start` runs `scheduler::reclaim_expired` for the job
  before its in-flight check, through the statement dispatch uses, so "open"
  means live.
- **Test.** `admin_api::an_export_is_not_blocked_by_a_claim_whose_worker_vanished`:
  a completed job with a claim past the timeout exports, and the claim reads
  `abandoned` with its task's counter at zero afterwards.
- **What remains** is the general form of the same fact — lapsed claims of
  *any* job nobody claims from stay on its books — which is a design question
  rather than a bug (U1). PLAN.md's Task States paragraph now says where
  reclamation runs and does not (K10).

### B2 — the nightly end-to-end workflow could never build MAGPIE

*Plan wins: **workflow changed** (K2). A CI blocker.*

- **What the code did.** `.github/workflows/nightly.yml` built MAGPIE with
  `make magpie BUILD=release`. MAGPIE's Makefile has refused that spelling
  since `fefc3466` (2026-07-22, "Make static PGO the release build"):
  `ifeq ($(BUILD),release) $(error release is a target; run 'make release', or
  use BUILD=no_pgo_release)`. The workflow was written after that (`c81e14a`),
  so the nightly job has failed at its build step on every run and has never
  executed a task on GitHub. The sixth audit's end-to-end runs were local,
  with a `portable_release` binary, which is why they passed.
- **Fix.** `BUILD=portable_release` — the build the backend image
  (`docker/Dockerfile`) and the MAGPIE release use, with a fixed instruction-set
  target, so the wordmap the worker builds hashes the same as the server's.
  Verified by running the script locally against this branch (section 11).
  The workflow itself cannot be exercised from here; dispatching it once after
  merge is the check (section 10).

### B3 — purging or deleting a job scanned the position table once per claim

*Code changed; PLAN.md schema block regenerated (K3). Revisits prior B1.*

- **What the code did.** `position_analysis_records.task_claim_id` is `NOT
  NULL REFERENCES task_claims(id) ON DELETE CASCADE`. Its only index was the
  partial unique one on `(task_claim_id, rack) WHERE game_index IS NULL`, and
  Postgres uses a partial index only when the query provably implies its
  predicate — a plain `task_claim_id = $1` does not imply `game_index IS
  NULL`, so the cascade's lookup could not use it. `purge_job` begins with
  `DELETE FROM task_claims … WHERE t.job_id = $1` and `delete_job` reaches the
  same rows through `jobs → tasks → task_claims`; Postgres runs the cascade
  once per deleted row, so each claim cost a **sequential scan of the whole
  records table** — including every other job's rows, which is why deleting
  the job's own records first would not have helped. `worker_data_gaps.claim_id`
  had the same shape with no index at all.
- **Why it matters.** A full English opening-rack job is ~6,400 claims over
  ~3.2 million records (9 million for a large capture job): thousands of
  sequential scans of the table, inside one transaction holding the job's
  dispatch lock and every open claim's row — the same hours-long, ALB-timed-out,
  rolled-back purge the sixth audit fixed for the moves table (prior B1). That
  fix moved the cost one table up rather than removing it.
- **Confirmed** with `EXPLAIN` and `enable_seqscan = off` against the
  migration: `DELETE FROM position_analysis_records WHERE task_claim_id = …`
  and `DELETE FROM worker_data_gaps WHERE claim_id = …` still planned as
  sequential scans; `… WHERE task_id = …`, `game_results` and `leave_records`
  by claim used their indexes.
- **Fix.** `position_analysis_records_claim_idx ON (task_claim_id)` and
  `worker_data_gaps_claim_idx ON (claim_id)`, with comments saying why the
  partial index could not serve. After: both plan as index (bitmap) scans.
  Every other cascade a purge or a delete fires was checked and has an index:
  `tasks` by job, claims by task, records and results by task and by job,
  leave records by task and by claim, requests by task (primary keys), moves
  by record, plies by move, exports and gaps by job. The migration is edited
  in place, as the convention is before release; existing development
  databases must be reset (PLAN.md, "Resetting the database after a schema
  change"). One more index on the records table costs an entry per record —
  a few tens of megabytes per full job — on inserts that already maintain
  four.

### B4 — the account page counted the account's claims instead of reading its counter

*Plan wins on intent: **code changed** (K8).*

- `GET /api/me` ran `COUNT(*) FROM task_claims WHERE claimed_by_user_id = $1
  AND state = 'completed'`: a walk of every claim the account ever made, and
  a number that disagreed with `/api/users` and `/api/workers` (which read
  `users.tasks_completed`) whenever a purge had given some of them back. It
  reads the counter now.
- **Test.** `auth_api::the_account_page_reads_the_contribution_counter`.

### B5 — the rack lookup interleaved redundant analyses

*Plan wins on intent: **code changed** (K9).*

- `?rack=` ordered a rack's moves by `rank` alone. Under redundancy above 1 a
  rack has one record per accepted claim, so the page showed two rank-1
  rows, then two rank-2 rows, as if one analysis had ranked every move twice.
  Ordered by record first now; PLAN.md says so.

### B6 — checked and found sound (no change)

- Every claim-path race the prior audits closed (prior R1–R4 and their
  predecessors) was re-read and still holds; section 4.
- `after_submission` loaded the job row a second time after the commit. It
  now reuses the row the submission's transaction read (C3), which is also the
  ordering `complete_unless_purged`'s witness needs (job before results).
- `thin_old_runs`' window functions: the pool's first run is the first among
  the runs older than the window, which is the pool's first run, since
  anything older is older; the newest run is the last of its day and survives.
- `expire_unconfirmed_imports` against `confirm_import`, `close_generation`
  against a purge, `seed_leave_universe` against a claim: all as the prior
  records describe.
- `recompute_if_stale` compares `pairs_used` only: a purge that removes N
  pairs followed by exactly N new ones is not noticed until the count moves
  again. Noted, not changed — it corrects itself on the next result, and a
  membership change refits unconditionally.

---

## 4. Race conditions

No new race was found. What was examined, including everything this audit
added:

- **The job template cache (C1).** `JobTemplates::get_or_load` is read-then-
  insert under a mutex released between the two; two callers loading the same
  job at once both read identical rows (a job's config rows have no update
  path, player configs are immutable, `input_data` rows cannot be deleted
  while pinned), so whichever inserts last changes nothing. A purge changes
  no template input; `delete_job` forgets the entry after its commit, and a
  template for a deleted job is never asked for. The submit path loads on a
  miss inside its transaction, holding the claim and task locks for the extra
  reads once per job per process.
- **`after_submission` with the pre-store job row (C3).** `job.status` is as
  of the submission's read. A job completed by another submission's check in
  between costs one redundant finish check whose `UPDATE … WHERE status =
  'active'` does nothing; a job deactivated in between is guarded the same
  way; a purge in between is caught by the `claims_issued` witness exactly as
  before, since the row was read before any result. Nothing can complete a
  job that should not be completed.
- **Export reclaim (B1).** `reclaim_expired` re-checks `state = 'claimed'`
  after waiting on a claim row lock, so a submission racing it is either
  completed there or abandoned here, never both — the same argument as on the
  claim path.
- **The new indexes** change no locking; they are read plans for cascades.
- **Batch size from the template rather than the request row.** The row's
  `num_games` is written from the job's batch size and nothing updates it, so
  the two cannot disagree; a purge regenerates rows from the same config.

---

## 5. Critical path

The critical path is a worker getting its next task and getting its result
accepted. Traced statement by statement, as the prior records did.

**Claim** (`POST /api/worker/task`), before this audit: identity lookup with
throttled touch and ban check (one statement); `candidate_jobs`; one reclaim
statement for the tier; per candidate: the derived gate (cached after the
first dispatch); then, inside the transaction and the job's dispatch lock:
`SET lock_timeout`, the advisory lock, `SET lock_timeout DEFAULT`, the
available-task probe, **the config row, the job's distribution (parsed each
time), one three-way join per player**, the seed cursor, the task insert, the
request insert, the claim insert, the task update, the guarded job update,
**the six-table `expected_data` union**, commit. A re-dispatched task instead
read **the request row, both players, the task's job, the distribution** and
`expected_data`.

**Submit** (`POST /api/worker/result`), before: identity; claim `FOR UPDATE`;
task `FOR UPDATE`; job read; validation and plausibility; **the request row
for the batch size** (games and pairs) or **the request row joined to the
config for the range plus the distribution** (opening racks); **the player
config for the move cap** (opening racks, and games with capture); record
insert; claim, task and job counters; identity counter; commit. Then inline:
**a second read of the job row** and the debounced finish check. Then spawned:
the SSE payload.

### What moved off it, or shrank, in this audit

| Change | Path | Why it was safe |
|---|---|---|
| **C1** A job's immutable inputs — config row, parsed distribution, player specs, `expected_data`, and for opening racks the rack-space table — are a `JobTemplate` read once per job per process (`jobs/dispatch.rs`, `AppState.templates`) | Claim, inside the dispatch lock: five reads gone for a generated games task, six for a re-dispatched one, four for an opening-rack task | Fixed at job creation and unchangeable after it (section 4); a purge touches none of it; `delete_job` forgets it. A miss takes one pool connection before the transaction, like the derived gate |
| **C2** The submit path reads the batch size and the per-position move cap from the template | Submit, inside the task's row lock: one read gone for games and pairs, two for opening racks, one for games with capture | The request rows denormalize the same job settings and nothing updates them |
| **C3** The finish check uses the job row the submission's transaction read | Submit, after commit: one read gone per submission | `complete_unless_purged` guards on `status` and `claims_issued`, so a stale status costs a check and never a write (section 4) |

For a generated games claim that is 13 statements inside the lock down to 8;
for a re-dispatched task 11 down to 5; for a games submission 11 statements in
the transaction down to 10 plus one fewer after it. Each round trip is about a
millisecond on RDS, and every one under the dispatch lock is time no other
worker can be claiming from that job, so this raises the per-job claim ceiling
by roughly the same fraction.

### Kept on the path, and why

| Kept | Why |
|---|---|
| The derived gate (a miss) | The hashes travel with the claim; decided by the prior audits |
| The available-task probe, the seed cursor, the request row of a reissued task | They are what changes from claim to claim |
| Finish check, debounced, and its in-flight `EXISTS` on seven submissions in eight | Gates dispatch; decided by the prior audits |
| `users`/`anonymous_workers.tasks_completed` in the submit transaction | Decided (prior audits): a single-row update against counter accuracy |
| Re-expanding an opening-rack range at submit | Needed for the exact-racks check; now a handful of additions against the template's table, with one read (the range) instead of two |
| The SSE payload | Already spawned and coalesced |

---

## 6. Performance — most severe first

1. **Purging or deleting a job scanned the position table once per claim.**
   *Fixed (B3).* Impact: on a full English opening-rack job, ~6,400 sequential
   scans over ~3.2 million rows (9 million for a large capture job) inside one
   transaction holding the job's dispatch lock and every open claim row —
   hours, past the ALB's 300-second idle timeout, so the purge rolled back and
   never completed while the job's claims timed out. The sixth audit removed
   the same shape for the moves table; this was the next table up.
2. **The nightly end-to-end suite never ran on GitHub.** *Fixed (B2).* Not a
   performance cost but the one check that runs a real MAGPIE against the real
   server, silently failing at its first step since it was written.
3. **Five to six reads of immutable rows under the dispatch lock on every
   claim.** *Fixed (C1).* Impact: two to six milliseconds of lock hold time
   per claim on RDS, serializing every worker on the same job; with a fleet
   claiming from one job at tens of claims a second, a proportional share of
   the job's dispatch ceiling.
4. **Two to three reads of the same rows on every submission.** *Fixed (C2,
   C3).* Impact: two to three milliseconds per submission, part of it inside
   the task's row lock.
5. **`GET /api/me` counted the account's claims.** *Fixed (B4).* Impact:
   linear in an account's history, on a page every signed-in contributor
   loads; a heavy contributor's page cost a scan of hundreds of thousands of
   index entries.
6. **`GET /api/admin/fleet` scans every claim of the last week with no time
   index.** *Flagged, not changed.* `task_claims` has no index on
   `claimed_at`, so the query is a sequential scan of the whole table filtered
   to seven days — hundreds of milliseconds to seconds at millions of claims,
   on an admin page. An index on `claimed_at` would cost an entry per claim on
   the hottest write table for one admin view; not worth it until the page is
   slow.
7. **`worker_contributions` per push** and **`jobstats::compute`'s task
   counts per push.** *Unchanged; prior U4/U6 decided to wait for the
   slow-stats log line.*
8. **Leave-generation throughput per job** and **`rebuild-artifacts`
   inline.** *Unchanged; prior U4, U5.*

---

## 7. Storage

### Fixed

| # | What | Change |
|---|---|---|
| S1 | `position_analysis_records.task_claim_id` had no index the cascade could use | `position_analysis_records_claim_idx (task_claim_id)`: one entry per record, tens of megabytes per full job, against purges and deletes that finish (B3) |
| S1 | `worker_data_gaps.claim_id` had no index | `worker_data_gaps_claim_idx (claim_id)` |

### Flagged, not changed

- Everything the sixth audit flagged stands: `leave_rack_progress` kept for
  the life of the job (prior U2, decided), `rating_run_residuals` (thinned
  since), `worker_data_gaps` and `audit_log` growing with declines, captured
  CGPs as `TEXT` (prior U3, decided), `tasks_claimed_idx` unused.
- **`audit_log`'s filters** (`action`, `actor_user_id`, `target_type`) have
  no index; a filtered admin query scans the log by `created_at`. Admin-only
  and bounded by the log's size, which the prior audits already cut by
  removing per-claim and per-submission rows.
- **`task_claims.claimed_at`** (see performance item 6).

---

## 8. PLAN.md reconciliation

"Code wins" means PLAN.md was updated to match the code. "Plan wins" means the
code was changed (and PLAN.md updated wherever its wording also needed it).

| # | Subject | Code | PLAN.md said | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | Exporting a completed job with a lapsed claim | Refused forever: nothing reclaims a completed job's claims | "refused (`409`) while any claim … is still open … at most the heartbeat timeout" | **Plan wins, code changed** | B1. The plan's rule is the right one and the code could not deliver it |
| K2 | The nightly workflow's MAGPIE build | `BUILD=release`, which the Makefile refuses | The nightly run "compiles MAGPIE `birdtest-contribute` … and runs one real task per job type" | **Plan wins, workflow changed** | B2. PLAN.md now names the build (`portable_release`) and why |
| K3 | Schema: the claim cascades | No usable index on `position_analysis_records.task_claim_id` or `worker_data_gaps.claim_id` | The block matched the migration, and "What these reads cost" said the moves cascade was fixed | **Code changed; PLAN.md's block regenerated, a bullet added** | B3. Identical by `diff` after the edit |
| K4 | `expected_data` per claim | Built inside the dispatch lock and the job's row lock on every claim | "It runs inside the job's dispatch lock, so what it costs is time no other worker can be claiming from that job" | **Code changed (objective 4); PLAN.md updated** | C1. The plan described the cost accurately; the cost was avoidable |
| K5 | The `JobHandler` trait | Takes the job's template | Snippet took ids | **Code wins** | Consequence of C1; the snippet and the paragraph after it updated |
| K6 | Auth API table | `POST /api/auth/sign-out-everywhere` exists (described in the reset-flow text and the audit table) | Not in the table | **Code wins** | Added |
| K7 | Directory structure | `jobs/dispatch.rs` | Not in the tree | **Code wins** | Added |
| K8 | `GET /api/me`'s `tasks_completed` | A `COUNT` over the account's claims | Identity totals are the counters on `users` / `anonymous_workers`, decremented by a purge | **Plan wins on intent, code changed** | B4 |
| K9 | `?rack=` under redundancy | Lists interleaved by rank | "the full ranked move list … for that rack" | **Plan wins on intent, code changed** | B5; PLAN.md says what comes back under redundancy |
| K10 | Where lazy reclamation runs | Only for jobs in a worker's candidate tier; never for inactive or completed jobs | "Reclamation is lazy — it runs at the moment the next task is requested" | **Code wins**: PLAN.md's Task States paragraph states the scope | The general case is a design question (U1); the one place it broke something is fixed (B1) |
| K11 | The finish check's job read | Re-read the row after the commit | Silent | **Code changed; PLAN.md's submission step 6 says which row the check uses** | C3 |

Counted: K5, K6, K7, K10 are code-wins (4). K1, K2, K3, K4, K8, K9, K11
changed code (7).

**Also updated in PLAN.md, not discrepancies:** a paragraph under "The claim
loop in full" describing the template and what is read under the lock; two
bullets under "What these reads cost" (C1, B3).

**Checked and found in agreement:** every prior K-item still holds; the Admin,
Public and Worker API tables against the routers; the configuration table
against `config.rs`; the rate-limit table; the audit-actions table; the
contribute settings keys against `client_state.c`; the heartbeat interval and
the consecutive-failure guard against `contribute.c`; the wire contract
against the fixtures (identical in both repositories); the schema block.

---

## 9. Python worker

Searched README, RUNBOOK, TESTING, PLAN, MAGPIE_DEPENDENCY, compose, the env
examples, the Dockerfile, the Terraform, both workflows, the scripts, the
frontend and the backend for any description of `worker/fake_worker.py` as a
production client. **None found; nothing to correct.** Every mention says the
opposite: its docstring ("Test tooling only … Never run it against anything
but a disposable test stack"), the compose profile comment ("end-to-end suite
only"), README's table and its "no Python, no Docker" paragraph,
`scripts/dev.py`'s refusal to run it ("Contributors are always real MAGPIE"),
RUNBOOK's "Never use `worker/fake_worker.py` for this", TESTING.md's tier
table, and PLAN.md's Worker Client and Development sections. The only backend
reference is the fixture test pinning its opening-rack submission shape.

---

## 10. Left for human input

### U1 — lapsed claims of jobs nobody claims from

Reclamation runs when a worker asks for work from a job's priority tier
(`scheduler::claim`), which is the right place for an active job — every
claim request in the tier sweeps it — and never happens for an inactive or
completed job. Consequences: a claim whose worker died stays `claimed` on such
a job until it is activated again (or, now, exported); its late submission is
**accepted** if the worker comes back hours later, where PLAN.md's Workflow
step 7 says a timed-out claim's result is refused; `tasks_claimed` on the job
page counts it meanwhile. B1 fixes the one place this blocked an operation.
Options: **(a)** leave it — a late result for a finished job is real work,
harmless to SPRT (already decided) and to ratings (which refit on it), and the
counter is a display figure; **(b)** make the rule deterministic by refusing a
submission whose claim is past the heartbeat timeout even if nothing has
reclaimed it yet (one predicate on the submit path's claim lookup), which
throws away a real result in the rare case the design already says it would;
**(c)** a periodic reclaim sweep over every job, which reintroduces the
background process the lazy design exists to avoid. **Recommendation: (a)**,
recorded in PLAN.md (K10). It is the only decision in this audit with a
reasonable case on more than one side, and (a) changes nothing.

### Not a decision, but to do after merge

Dispatch the nightly workflow once (`workflow_dispatch` on "Nightly MAGPIE
end-to-end") to see it run on GitHub for the first time. It passes locally
against this branch (section 11); the workflow file itself cannot be executed
from here.

---

## 11. Verification

- Backend: `cargo clippy --locked --all-targets -- -D warnings` clean;
  `cargo test --locked` against Postgres 16: **163 tests** (90 unit and
  contract, 73 integration), all passing (159 before this audit). The five
  `magpie_smoke` tests are `#[ignore]` by design. The frontend is untouched.
- MAGPIE: `make magpie_test` (sanitizers) and `./bin/magpie_test contribute`
  pass at `6308b63c`; no MAGPIE change was needed, so nothing to commit or
  push there.
- `EXPLAIN` (`enable_seqscan = off`) before and after B3: the two claim
  cascades went from `Seq Scan` to `Bitmap Heap Scan` on the new indexes;
  every other cascade a purge or a delete fires was already indexed.
- End to end: `scripts/e2e_magpie.py` against an isolated compose project
  (`birdtest-e2e`) whose backend image was built from this branch's working
  tree, with the checkout's `portable_release` MAGPIE reporting `0.5.1` and
  a `data-20251004` import through the real API. **Every job type passed**:
  games and game pairs (2 accepted claims each), opening racks static (with
  the wordmap built by the server and matched by the worker) and simming (2
  each), and leave generation (105 s, 2 accepted claims, the wordmap already
  built and matching). The backend log held no error or warning line. The
  stack was torn down afterwards, volumes included.

---

## 12. Tests added

| Test | What it pins |
|---|---|
| `admin_api::an_export_is_not_blocked_by_a_claim_whose_worker_vanished` | B1: a completed job with a claim past the timeout exports; the claim is reclaimed through the dispatch path |
| `worker_api::a_jobs_template_is_read_once_survives_a_purge_and_goes_with_the_job` | C1: the first claim reads the template, a purge leaves it and the next claim carries the same players, files and settings from seed 1, and a delete forgets it |
| `auth_api::the_account_page_reads_the_contribution_counter` | B4 |
| `jobs::dispatch::tests::a_forgotten_job_is_read_again` | C1: the cache's forget path |
| `leave_gen::overlapping_leave_submissions_wait_instead_of_deadlocking` (updated) | Calls the handler through the template, as the submit path does |
