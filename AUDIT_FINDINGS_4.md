# birdtest audit — findings, version 4

Branch: `audit/birdtest-2026-09-16-pass2`, off `audit/birdtest-2026-09-16` at
`2a6fe2c` (which is `main` at `6a72333` plus the sixth and seventh audits and the
commit that implemented the seventh's decisions, none of it merged — see "Where
this branch sits").
MAGPIE: `birdtest-contribute`, **one commit added** (`d93dacaf`, section 9). The
branch is now two commits ahead of `origin`, and **neither is pushed** — which is
this audit's one deployment blocker (D1, section 10).
Date: 2026-09-16/17.

**This is the eighth audit.** It builds on [AUDIT_FINDINGS_3.md](AUDIT_FINDINGS_3.md)
(the seventh, branch `audit/birdtest-2026-09-16`), [AUDIT_FINDINGS_2.md](AUDIT_FINDINGS_2.md)
(the sixth, `audit/birdtest-2026-09-15`), [AUDIT_FINDINGS_1.md](AUDIT_FINDINGS_1.md)
(the fifth, PR #6) and, through them, the four unnumbered records before those
(branches `audit/birdtest-2026-09-11`, `-09-13`, `-09-13-pass2` and `-09-14`, whose
shared `AUDIT_FINDINGS.md` on `main` is the fourth audit's record). The highest
numbered file anywhere in the history was `AUDIT_FINDINGS_3.md`, so this is
`AUDIT_FINDINGS_4.md`; the earlier files are untouched. Where an entry revisits a
prior item it names it ("prior U1", "prior U4").

This file is the authoritative record of every code-versus-`PLAN.md` decision made
in this audit, and of the bugs, races, MAGPIE argument gaps, critical-path
analysis, performance and storage findings behind them.

**Counts: 5 code-wins (PLAN.md updated to match the code), 8 plan-wins (code
changed), 3 items left for human input (U1–U3), plus one thing only a human can
do (D1: push MAGPIE).**

The default bias is that the code wins and `PLAN.md` is brought level with it. The
code was changed only where it was wrong, or where the plan described the
behaviour the rest of the system needs.

---

## Where this branch sits

`audit/birdtest-2026-09-16` had not been merged to `main`, had no pull request, and
was one commit ahead of its own remote (`2a6fe2c`, "Drop job priority, seed every
task, …", the human's implementation of the seventh audit's decisions plus the
removal of job priority). Branching from `main` would have meant auditing code
three commits of fixes behind and producing a branch that conflicts with all of
them, so — as the seventh audit did — this branch is cut from that head. A branch
named for today already existed, so this one is `…-pass2`. Merging it merges
everything since `main`. Nothing in the sixth or seventh records was re-litigated;
section 0 checks their open items.

`2a6fe2c` is the newest code and the only code no audit had read, so it got the
closest reading. Three of this audit's findings (B1, K9, D1) come from it.

---

## 0. What the prior audits left open

| Prior item | Still true? | How it was checked |
|---|---|---|
| Seventh audit, U1 — lapsed claims of jobs nobody claims from; recommendation (a), leave it | Stands, and has a third case: `2a6fe2c` made **0% allocation** a way to park an active job, and a job at 0% is never a candidate either, so its lapsed claims also wait for activation above 0% or an export. PLAN.md's Task States sentence now names it | Read `scheduler::candidate_jobs`; no code change |
| Seventh audit — "dispatch the nightly workflow once after merge" | **Not yet possible.** The branch is unmerged, so the nightly still runs `main`'s workflow and has failed every night (`gh run list`: 09-14, 09-15, 09-16, all `failure`, 30–100 s — the `BUILD=release` step the seventh audit fixed on this branch). It will keep failing until this branch merges **and** MAGPIE is pushed (D1) | `gh run list --workflow nightly.yml` |
| Sixth audit, U1 — publish the MAGPIE pin | **Recurred.** `2a6fe2c` moved `docker/Dockerfile`'s pin to `0f6a4cb1`, which exists only in the local checkout: `origin/birdtest-contribute` is `6308b63c`. See D1 | `git ls-remote origin birdtest-contribute`; and the end-to-end run hit it (section 11) |
| Sixth audit — `tasks_claimed_idx` unused, left alone as tiny | Left alone again; decided | — |
| Fifth audit, U4 — leave-generation throughput; decided (a), leave it, with batch size as the knob | The *contention* decision stands. This audit measured something that decision did not have in front of it — the write volume of a fold — and flags it separately as U2 | Section 7 |
| Every K-item of the prior three records | Hold | Spot-checked against the code while reading it; the schema block in PLAN.md is byte-identical to the migration (`diff`) |

---

## 1. How this audit was run

- Read `PLAN.md` in full (6,535 lines), diffed its schema block mechanically against
  `backend/migrations/0001_initial.sql` (identical before and after this audit — no
  migration edit was needed), read the seventh audit's record in full and the open
  items of the two before it, then the diff of `2a6fe2c` line by line.
- Backend, read in full: `scheduler.rs`, `routes/worker.rs`, `jobs/registry.rs`,
  `jobs/mod.rs`, `jobs/dispatch.rs`, `jobs/handler.rs`, `jobs/game.rs`,
  `jobs/opening_rack.rs`, `jobs/leave_gen.rs`, `derived.rs`, `ratings.rs`,
  `jobstats.rs`, `exports.rs`, `routes/public.rs`, `auth/mod.rs`, `state.rs`,
  `sse.rs`, `db.rs`, `lib.rs`, `error.rs`, `main.rs`, `stats/sprt.rs`,
  `bin/build-derived.rs`, the test harness; and the parts of `routes/admin.rs`
  (activate, deactivate, complete, purge, delete, export, import), `account.rs` and
  `ratelimit.rs` where state is read and then written.
- MAGPIE `birdtest-contribute` at `0f6a4cb1`: every contribute executor in
  `src/impl/config.c` (games, opening racks, leave generation), both resets, the
  lexical load, `config_fill_sim_args`, `config_fill_autoplay_args`,
  `impl_move_gen`, `impl_sim`, the `Config` struct, `config_load_parsed_args`, the
  PRNG seeding, and `test/contribute_test.c`.
- Docker, compose, both workflows, the Terraform's ECS and RDS settings, the
  scripts, the frontend's API client and admin job page, and the four operational
  documents.
- **Measured, not estimated**, against a throwaway Postgres 16: a synthetic
  opening-rack job of 1,000,000 records and 5,000,000 moves over 2,000 claims and
  200 workers, and a leave generation's 3,199,724 progress rows (sections 6, 7).
- Backend: `cargo clippy --locked --all-targets -- -D warnings` and `cargo test
  --locked`. **165 tests before, 172 after**, all passing, clippy clean. Frontend:
  `npm run check` (0 errors) and `npm run build`.
- MAGPIE: `make magpie_test` (`-Werror`, address/undefined/leak sanitizers) and
  `./bin/magpie_test contribute`, passing; `clang-format --dry-run -Werror` clean.
- End to end: `scripts/e2e_magpie.py` with the checkout's `portable_release` MAGPIE
  at `d93dacaf` against an isolated compose project built from this branch
  (section 11).

---

## 2. MAGPIE arguments that change a task's outcome

The prior tables were not reused. The trace was re-run from both ends against
`0f6a4cb1`: every field the executors read, and every `Config` field asked whether
autoplay, move generation or a simulation reads it. Everything the prior records
list still holds. Checked this time and not named before:

| Setting | Changes results? | How it is covered | Change |
|---|---|---|---|
| **`letter_distribution`, `board_layout`** on the request | **Yes** — the bag and the board | birdtest states both on every request of every job type (`NOT NULL` on all three request tables). **MAGPIE treated them as optional**: absent meant the distribution inferred from the lexicon's *name* and the layout named for the compile-time `BOARD_DIM` — this build's defaults. They were the last two settings on the contribute path a request could leave to the build that ran it, against the rule the same file states for every other key ("contribute takes no setting from this build's defaults") | **M1, fixed in MAGPIE** (`d93dacaf`): both are required; a request without one is refused like a player object missing a setting |
| The opening-rack `seed` (new in `2a6fe2c`/`0f6a4cb1`) | Yes, for a simmer | `seed + i` is written to `config->seed` before each rack; `config_fill_sim_args` passes `config->seed` to `sim_args_fill`; each simmed play's PRNG is seeded from it. Seed `0` (the first task's) is safe: `prng_seed` runs it through splitmix64, so an all-zero state cannot result | None |
| `sim`'s known-opponent-rack argument (`impl_sim(config, ARG_TOKEN_SIM, 0, …)` reads the parsed value of the last `sim` command) | Would, if it survived | It cannot: `config_load_parsed_args` zeroes every argument's `num_set_values` when the `contribute` command is parsed, so the value reads `NULL`, and the fallback is the opponent's rack in the game, which `game_reset` has just emptied | None |
| `config->game_history` under `sim_with_inference` | Would, for an opening rack | `config_contribute_use_player_settings_for_analysis` forces `sim_with_inference` off for analysis (prior fix); autoplay manages its own history | None |
| Thread count for a simmer | Yes, inherently | Decided by the fourth audit (its U3): documented, excluded from equality cross-checks | None — not re-litigated |

**One gap found (M1), closed on the MAGPIE side.** The server half was already
correct, so no dispatched task was ever affected; what changed is that MAGPIE can
no longer be the reason one is.

---

## 3. Bugs and footguns

### B1 — a job parked at 0% could still tell workers to shut down

*Plan wins: **code changed** (K1). Introduced by `2a6fe2c`.*

- **What the code did.** `2a6fe2c` removed priority and made 0% allocation mean
  "offered to nobody, exactly as inactive is". `candidate_jobs` got `allocation >
  0`. `shutdown_or_idle` — reached when the candidate list is empty — did not: its
  three queries counted every `status = 'active'` job. So a parked job whose floor
  was above a worker's MAGPIE answered `magpie_too_old` ("Every active job requires
  MAGPIE 2.0.0"), and a parked job in a worker's unsupported set answered
  `data_out_of_date`, over a job handing out nothing to anyone. The same job
  switched to `inactive` answered `204`.
- **Why it matters.** A shutdown makes `magpie contribute` exit. A contributor who
  exits because of a parked job is not polling when the admin raises a job they
  could have run all along. The code comment beside the query ("or every one of
  them is at 0%" → `Idle`) shows the intent was the other way.
- **Fix.** All three queries in `shutdown_or_idle` carry `AND allocation > 0`. With
  every active job parked the answer is `NoWorkExists` (`204`).
- **Test.** `worker_api::a_parked_job_shuts_nobody_down`: two parked jobs, one too
  new and one unsupported → `204`; raised to 50% → `shutdown`/`both`.

### B2 — the opening-rack export and stream contained no moves

*Plan wins: **code changed** (K2). A half-finished feature.*

- **What the code did.** `exports::export_query` and the admin stream both ran
  `SELECT to_jsonb(r) FROM position_analysis_records r WHERE job_id = $1`. A record
  row is a header: rack, `num_moves`, timestamps. The analysis — every ranked move,
  score, equity, win%, per-ply statistics — is in `position_analysis_moves` and
  `_plies`, and was not in the export at all.
- **What PLAN.md says.** Removing the dashboard's move aggregates, it promises
  "every ranked move is still there … and an admin export is the path for analysing
  the corpus properly." Nothing else reads moves in bulk: the public feed returns
  the best move only and `?rack=` one rack at a time.
- **Fix.** One shared query (`exports::OPENING_RACK_CORPUS`): each record with
  `moves: [{rank, move, score, equity, win_percentage, blended_utility, plies:
  [...]}]` nested. One line per record still, so `row_count` still counts records.
  The stream calls `exports::export_query` instead of carrying its own copy.
- **Cost, measured.** 72 s per million records at five moves each (2 s for the
  header-only query it replaces), an index probe per record through
  `position_analysis_moves_record_idx`. A full English job is a few minutes, once,
  on a background task.
- **Test.** `worker_api::an_opening_rack_corpus_carries_each_racks_ranked_moves`.
- **What remains:** positions captured *during games* have no export path at all
  (U3).

### B3 — `?worker=` read the whole job, on a public route

*Plan wins: **code changed** (K5). Also performance item 1.* See section 6.

### B4 — a driver-level database error reached the response body

*Plan wins: **code changed** (K4).* `From<sqlx::Error>` scrubbed the message only
for errors carrying a SQLSTATE. Anything else — `PoolTimedOut`, a protocol error, a
decode mismatch — became `500 {"message": "database error: <sqlx's text>"}`. PLAN.md:
"never includes a database error". Now logged in full and answered generically;
`PoolTimedOut` and a cancelled display read are `503 unavailable` with `Retry-After`
(load, not a fault — and MAGPIE's client already retries a `5xx` with backoff).
Tests: `error::tests::*`.

### B5 — a duplicate rack in an opening-rack batch was a `409`

*Plan wins, minor: **code changed** (K13).* `check_batch_against_task` compared
sizes and containment and left duplicates to the unique index, which refused them
as `409 that already exists` after the batch had been sent to the database. Every
other malformed submission is a `400` that says what is wrong. Checked explicitly
now. Test: extended `an_opening_rack_result_must_answer_the_racks_it_was_given`.

### B6 — purge and force-complete were one unconfirmed click

*Code changed; PLAN.md's route table updated (K11).* On the admin job page both sat
beside Activate with no confirmation, while Delete asked. A purge deletes every
task, claim and result; a completed job can never be reactivated. Both ask now. The
purge notice also said "tasks returned to available", which stopped being true when
purge began deleting tasks.

### B7 — the export feature had no user interface

*Code changed (K11). A half-finished feature / deployment gap.* `POST` and `GET
/api/admin/jobs/:id/export` existed, tested, with no page: an export could only be
started with curl and a hand-copied CSRF cookie pair. The admin job page has an
export panel for a completed job — start, poll while running, row count, size,
download link.

### B8 — checked and found sound (no change)

- Every claim-path race the prior audits closed was re-read and holds (section 4).
- `2a6fe2c`'s seed changes: `tasks.seed NOT NULL`; a leave task's random `u64`
  reinterpreted as `i64` on both `tasks` and `leave_requests`; a collision is a
  unique violation → `JobClaimError::Retry`. The games/pairs/opening-rack cursor
  (`MAX(seed)`) is never read for a leave job, so random seeds cannot disturb it.
- `activate_job` with the single advisory lock: row lock on the job, then the
  lock, then an unlocked `SUM` over the others — no cycle with another activation.
- `thin_old_runs`, `recompute_if_stale`, `close_generation`, `seed_leave_universe`:
  as the prior records describe.
- Rolling deploys cannot overlap two instances (`deployment_maximum_percent =
  100`), which is what the single-instance assumptions (import/export reaping,
  in-memory caches, SSE) need. Graceful shutdown waits on SSE streams that never
  end, so a stop runs to ECS's 30 s `stopTimeout`; in-flight submissions finish
  well inside it.

---

## 4. Race conditions

### R1 — the API key limit was a count and then an insert

*A bug under objective 2: **fixed**, plan wins (K6).*

- `POST /api/me/api-keys` ran `SELECT COUNT(*)` and then `INSERT` as two statements
  on the pool. Requests arriving together each read the same count and each
  inserted. The limit is enforced only in the application (the migration says so),
  so this check is all there is.
- **Demonstrated**, not inferred: with the account two keys short of the limit,
  twelve concurrent requests created **eleven** keys against the old code (expected
  two). The test was run against both versions.
- **Fix.** Count and insert in one transaction under `SELECT … FROM users … FOR
  UPDATE`. The lock is the account's own row and is held for two statements.
- **Test.** `auth_api::concurrent_key_requests_cannot_exceed_the_key_limit`.

### Examined and found sound

- **Claim vs reclaim vs decline vs heartbeat vs submit.** All five touch a claim
  row. Submit and decline take it `FOR UPDATE` and re-check `state = 'claimed'`;
  reclaim's `UPDATE` re-evaluates its `WHERE` on the row version it waited for
  (both the state and the heartbeat time), so a heartbeat that lands first keeps
  the claim and a submission that lands first completes it; a claim is never both.
  Lock order is claim → task → job everywhere, purge and delete included.
- **Redundant submissions of one task** serialize on the task row before anything
  is stored; "first accepted" is `accepted_count = 0` under that lock.
- **The new display pool (C1)** introduces no shared mutable state: it is a second
  set of connections to the same database. A dashboard payload may now be built
  from a snapshot a few milliseconds older than the submission that triggered it,
  which the coalescing already allowed.
- **`resolve_worker` then the feed query** (B3) are two statements; a contributor
  deleted or a purge in between yields an empty or shorter page, which is what a
  concurrent reader would have seen anyway.
- **The export panel** polls a row the background task updates with `WHERE state =
  'running'`; unchanged.
- **Template and derived caches**: as the seventh audit argued; `2a6fe2c` changed
  nothing they depend on.

---

## 5. Critical path

Traced statement by statement again. **Claim:** identity (1 statement) →
`candidate_jobs` → one reclaim statement → per candidate, cache hits for the
derived gate and the template → `BEGIN`, `SET LOCAL lock_timeout`, the advisory
lock, `SET LOCAL … DEFAULT`, the available-task probe, the seed cursor, task
insert, request insert, claim insert, task update, guarded job update, `COMMIT`.
**Submit:** identity → claim `FOR UPDATE` → task `FOR UPDATE` → job read →
validation → record inserts → claim, task, job and identity counters → `COMMIT` →
debounced finish check → spawned SSE push. After the seventh audit's template
work, **every statement left on both paths is one the decision needs.** What was
still wrong was not *what* ran on the path but *what the path shared a resource
with*.

### What moved off it

| Change | Path | Why it was safe |
|---|---|---|
| **C1 — display reads have their own pool.** `AppState.read_pool` (`db::connect_read`): 8 connections, `statement_timeout = 15s`, 5 s acquire timeout. The public pages (`/api/jobs*`, `/api/users`, `/api/workers`, `/api/rating-pools*`), the SSE stream's first payload and **every live stats push** use it | Claim and submit no longer compete for connections with anything display-only. Before, all of it shared one pool of 20: the live push a submission spawns took a connection from the pool the *next* claim needed, and twenty slow public reads — dashboards on a busy job, or one caller in a loop — held every connection while claims and submissions queued for sqlx's 30 s acquire timeout and then failed | Nothing on the display pool decides anything: no statistic is read while dispatching or accepting, the finish check stays on the main pool, and so do the admin stream and exports (they hold a cursor for minutes, which the statement timeout exists to forbid; the two-stream cap bounds them). 28 connections total against an RDS `db.t4g.micro`'s ~85 usable |
| **C2 — a saturated or slow display read fails fast.** `PoolTimedOut` and SQLSTATE `57014` map to `503` + `Retry-After` | A page view under load costs five seconds and a retry, not a parked request | Display-only |

### Examined and left on the path

| Kept | Why |
|---|---|
| The reclaim statement on every claim | ~1–2 ms at thousands of open claims (one pass of the partial open-claims index); throttling it would need a clock in `AppState` and would make tests time-dependent, for milliseconds |
| The three-statement lock dance (`SET LOCAL`, lock, reset) | Two round trips could be folded into one simple-protocol batch; milliseconds, and the reset is the only one under the lock |
| The finish check's config-row read (`jobstats::game_stats`) | The template has it, but the function is shared with the display path, which has no `AppState`; one read on one submission in eight |
| Everything the prior records list (derived-gate miss, identity counters in the submit transaction, the debounced finish check) | Decided |

---

## 6. Performance — most severe first

1. **`GET /api/jobs/:id/results?worker=` read the whole job.** *Fixed (B3, K5).*
   The filter was `u.username = $2 OR left(encode(sha256(convert_to(
   c.claimed_by_anon_uuid::text …))), 16) = $2`, evaluated per record behind two
   joins; no index can serve it. For a contributor with few results — or a name
   nobody has — the scan never fills its `LIMIT` and reads the entire job.
   **Measured: 2.2–2.6 s per request at 1,000,000 records** (a third of a full
   English job; linear, so ~8 s at full size and worse on RDS with a cold cache),
   on a **public, unauthenticated, unmetered** route, each request holding a pool
   connection for all of it. Twenty in flight emptied the pool and no worker could
   claim or submit. **After: 1–3 ms** for the same requests. The name is resolved
   first (`users.username` is unique; a pseudonym is matched among contributors
   through `anonymous_workers_contribution_idx` — the hash itself cannot be
   indexed, `convert_to` is only `STABLE`); a name that is nobody's is an empty
   page without reading the job; the filter is a plain equality on
   `task_claims_user_idx` / `task_claims_anon_idx`. The filtered statement is sent
   unprepared (`persistent(false)`): the right plan differs by two orders of
   magnitude with who is asked about (a heavy contributor is found at the head of
   the feed index, a rare one through their own claims), and the cached generic
   plan — confirmed with `plan_cache_mode = force_generic_plan` — picks the feed
   scan for everyone.
2. **Any slow public read could stall the fleet.** *Fixed structurally (C1).* The
   general form of item 1. Expected impact before: under a burst of dashboard
   traffic on a large job, claims and submissions waiting up to 30 s and then
   failing with a `500`; after: they are unaffected, and the page views queue
   among themselves behind a 15 s bound.
3. **A leave-generation fold is seconds of random-access writes inside the submit
   transaction, and hundreds of megabytes of WAL.** *Flagged, U2 — see section 7
   for the numbers.* Expected impact: 2.5 s (40,000 racks) to 5.5 s (200,000
   racks) per submission inside the transaction the worker waits on, during which
   overlapping submissions of the same generation queue behind its row locks.
4. **The opening-rack export is now minutes instead of seconds** (B2). *Accepted:*
   72 s per million records on a background task, once per completed job, against
   an export that previously contained no analysis.
5. **`GET /api/admin/fleet` scans `task_claims` with no time index; the evidence
   sweep rebuilds every pool's matrix every two minutes; `worker_contributions`
   and task counts per push.** *Unchanged; decided by prior audits.* All three are
   on the display pool or a background sweep, and the display ones are now bounded
   at 15 s.

---

## 7. Storage

### Fixed

Nothing needed a schema change. No index was added: B3's fix uses indexes that
exist (`task_claims_user_idx`, `task_claims_anon_idx`,
`position_analysis_records_claim_idx`, `anonymous_workers_contribution_idx`). The
migration and PLAN.md's schema block are untouched and identical.

### Flagged — U2, the write volume of a leave-generation fold

Every accepted leave task runs `UPDATE leave_rack_progress … FROM UNNEST(…)` over
every rack that occurred in its games. **Measured** on a seeded generation
(3,199,724 rows: 258 MB heap, 172 MB indexes):

| One fold of | Time | WAL written | HOT updates |
|---|---|---|---|
| 40,000 racks, first touch after a checkpoint | 2.7 s | **409 MB** | 0 of 40,000 |
| 40,000 racks, same checkpoint cycle | 2.5 s | 147 MB | 0 |
| 200,000 racks (a 10,000-game task), pages already imaged | 5.5 s | 69 MB | 0 |

Why: the racks a task draws are scattered uniformly over the table, so a fold
touches a large fraction of its 33,000 heap pages, and the first touch of each page
after a checkpoint writes a full 8 kB page image; and `occurrence_count` is an
indexed column (`leave_rack_progress_pick_idx`, which claim-time selection needs),
so **no update can ever be HOT** — every one writes a new heap tuple and a new
entry in both indexes, and leaves a dead tuple for autovacuum. In steady state
that is roughly the whole table and its indexes re-imaged once per five-minute
checkpoint (~430 MB) plus 70–150 MB per fold. At one fold a minute that is on the
order of **10 GB of WAL an hour per active leave job** — archived for the whole PITR
window, replayed by any replica, and written through a `db.t4g.micro`'s burst I/O
budget — and a table that needs vacuuming every few folds.

This is not the fifth audit's U4 (contention, decided: leave it). It is the same
statement seen from the disk. Options:

- **(a)** Leave it and size for it: a larger instance class and a shorter PITR
  window while a leave job runs. Nothing to build.
- **(b)** Fold into an append-only staging table (`task_id, rack, count,
  equity_sum`) in the submit transaction, and merge into `leave_rack_progress` in
  one sequential pass every N minutes on a background task; claim-time selection
  and the transition read the merged table, and a transition first drains the
  staging table. Sequential appends instead of random updates — an order of
  magnitude less WAL — and the submit transaction drops from seconds to
  milliseconds. Costs: selection lags by up to N minutes (so a little duplicated
  coverage near a generation's end), and a drain step the transition must not
  skip, which is a new correctness obligation on a path that has had races before.
- **(c)** Fewer, larger tasks (`num_iterations`): fewer folds, each touching more
  of the table. Reduces the per-fold full-page cost, not the row volume.
- **(d)** Drop `leave_rack_progress.updated_at` (8 bytes a row, rewritten on every
  fold, read only by the public leave feed). Small, and does not make updates HOT.

**Recommendation: (b) before the first full-size leave job, (c) meanwhile.** Not
done here: (b) restructures the one path in the system that has needed the most
race fixes, and deserves a decision rather than a guess.

### Flagged, unchanged from prior records

`leave_rack_progress` kept for the life of the job; captured CGPs as `TEXT`;
`audit_log` and `worker_data_gaps` growing with declines; `tasks_claimed_idx`
unused; no index on `audit_log`'s filters or `task_claims.claimed_at`.

---

## 8. PLAN.md reconciliation

"Code wins" means PLAN.md was updated to match the code. "Plan wins" means the code
was changed (and PLAN.md updated wherever its wording also needed it).

| # | Subject | Code | PLAN.md said | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | Shutdown and parked jobs | `shutdown_or_idle` counted every active job, at 0% included | "A job at 0% is offered to nobody, which is exactly what inactive means" | **Plan wins, code changed** | B1. PLAN.md's "Deciding between shutdown and idle" now defines "offering work" and says why |
| K2 | What an export line holds | Opening racks: the record row only | The export "is the path for analysing the corpus properly"; "every ranked move is still there" | **Plan wins, code changed** | B2. PLAN.md's Exports section and the admin route table now state the line's shape and its measured cost |
| K3 | Connection pools | One pool of 20 for everything | "nothing in the claim path reads a statistic", and an SSE push built "off the submitting request" — but on the submitting request's pool | **Code changed (objective 4); PLAN.md gained "Two connection pools"** | C1. The plan's separation of display from decision was true of statements and false of connections |
| K4 | Error bodies | A driver-level error's text reached the client; a pool timeout was a `500` | "never includes a database error"; seven codes | **Plan wins, code changed**; `unavailable` (`503`) added to the list | B4 |
| K5 | `?worker=` | Applied per row of the whole job | "With the feed indexes and cursor pagination, a page costs a page" | **Plan wins, code changed** | B3. "What these reads cost" has the measurement |
| K6 | The 100-key limit | Count, then insert | "a request to create the 101st is refused" | **Plan wins, code changed** | R1 |
| K7 | `letter_distribution` / `board_layout` on a request | MAGPIE: optional, absent = build defaults | Both: "absent means MAGPIE's defaults" *and* "No setting that can change a result is left to the worker's build" | **Plan wins on the rule, MAGPIE changed, PLAN.md's exception removed** | M1. The two sentences contradicted each other; the rule is the one the rest of the design depends on |
| K8 | Directory structure: `docker/Dockerfile` | Builds a pinned MAGPIE into the backend image; three targets | "backend + fake-worker targets; neither needs MAGPIE" | **Code wins** | Stale since MAGPIE_DEPENDENCY.md was implemented |
| K9 | "Tier" wording | No tiers since `2a6fe2c` | Task States: "activation puts it back in a tier"; Lazy reclamation: "the whole candidate tier" | **Code wins** | Two leftovers of the priority removal; also a comment in `derived.rs` and one in a test |
| K10 | "Pre-populated" wording | Every job type is on-demand | Workflow step 4, the Four Components table and Task Request Types still offered pre-populated as a live option | **Code wins** | PLAN.md says elsewhere, correctly, that the strategy is gone |
| K11 | `/admin/jobs/[id]` | Activate, deactivate, force-complete, purge, delete, check artifacts — and now export | "controls: deactivate, activate, purge, delete" | **Code wins** (after B6, B7) | The table now lists what the page has |
| K12 | Startup | Reaps imports **and exports** left `running`; now also connects the display pool | Imports only | **Code wins** | |
| K13 | Duplicate rack in a batch | `409` from the unique index | Malformed submissions are `400` naming what was wrong | **Plan wins, code changed** | B5 |

Counted: K8–K12 are code-wins (5). K1–K7 and K13 changed code (8). U1–U3 are
unresolved (3).

**Also updated in PLAN.md, not discrepancies:** Task States names 0% as a third
way a job stops being reclaimed from (prior U1); two bullets under "What these
reads cost" (C1, B3); TESTING.md's `I-SCHED-3` names the parked-job case; comments
in `jobs/mod.rs` (`expected_data` no longer "runs on every claim") and `derived.rs`.

**Checked and found in agreement:** the schema block (byte-identical); the Worker,
Auth, Account, Admin and Public API tables against the routers; the configuration
table against `config.rs`; the rate-limit table; the claim loop and submission
steps against `scheduler.rs` and `routes/worker.rs`; the Ratings section's trigger
table against `main.rs` and `ratings.rs`; the contract fixtures in both
repositories (identical); `0.1.0` everywhere a floor is written.

---

## 9. MAGPIE `birdtest-contribute`

Checked directly rather than assumed: the three executors read every key birdtest
sends, and birdtest sends every key they require (the contract test pins both
directions); `magpie builders` prints what `magpie.rs` parses; `help convert` lists
`dawg2wordmap`, `klvwmp2rit` and `rackequity2klv`; `createdata klv` exists; the
opening-rack executor reads the new `seed`; the version is `0.1.0`.

**One change, committed on `birdtest-contribute` as `d93dacaf`** (not on `main` or
any other branch; **not pushed**):

- *What was missing:* `letter_distribution` and `board_layout` were read with
  `json_get_string_or_null`, and a request without them played on defaults chosen
  by the build (M1, K7).
- *Why it was needed:* every other result-changing setting is already refused when
  absent, because a version floor is a minimum and cannot exclude a release that
  changed a default. These two were the exception.
- *What changed:* `contribute_validate_common` became
  `config_contribute_validate_common` (exposed for testing, like the file's other
  contribute helpers) and requires both; the lexical load still accepts `NULL`s,
  which MAGPIE's own unit tests use.
  `test_a_request_must_state_its_distribution_and_layout` covers a complete
  request, each key missing, a `null`, and a name that would leave the data
  directory. No version bump: nothing a well-formed task computes is different, so
  the floor and the fixtures stay at `0.1.0`.

---

## 10. Deployment blockers

### D1 — the backend image cannot be built: its MAGPIE pin is not published

**Not resolvable from here; needs a push.** `docker/Dockerfile` pins
`MAGPIE_COMMIT=0f6a4cb1…` and fetches it from `github.com/jvc56/MAGPIE`.
`origin/birdtest-contribute` is `6308b63c`; `0f6a4cb1` (and now `d93dacaf`) exist
only in the local checkout. Every build of the `backend` or `derived-builder`
target fails at `git fetch`: `fatal: remote error: upload-pack: not our ref
0f6a4cb1…` — **reproduced** in this audit's end-to-end run, where
`docker compose run --build derived-builder` died on exactly that line. Until it is
pushed: CI's `images` job fails on this branch, `magpie-contract` tests the wrong
MAGPIE, the nightly cannot pass, and nothing can be deployed.

```
cd ~/MAGPIE && git push origin birdtest-contribute
```

Pushing is outward-facing and was not part of this audit's instructions, so it was
not done. The pin can stay at `0f6a4cb1` (it becomes fetchable once it is an
ancestor of a published ref) or move to `d93dacaf`; the builders are identical.

### Resolved in this audit

| # | Blocker | Resolution |
|---|---|---|
| D2 | Exports usable only with curl and a hand-built CSRF pair | Export panel on the admin job page (B7) |
| D3 | The export artifact of an opening-rack job held no analysis | B2 |
| D4 | One public URL could take the worker API down | B3, C1 |

### Checked, no change needed

Migrations run before bind; `SESSION_SIGNING_KEY` and the database URL fail startup
when absent; secrets come from SSM through the task definition; CSRF is enforced on
every cookie-backed mutation (each admin handler calls `csrf::verify`, the worker
routes are exempt by design); the server refuses to start without a working MAGPIE
at or above its own floor; the ECS service cannot run two instances at once; the
ALB idle timeout is 300 s; `desired_count` and the display pool fit the instance
class's connection limit.

---

## 11. Left for human input

### U1 — a newly activated job takes every claim until it has "caught up"

The scheduler orders candidates by `claims_issued / allocation`, and
`claims_issued` is the job's **lifetime** count. Two jobs at 50% each, one of
which has been running for a month (2.6 million claims at one a second) and one
activated today: the new job's ratio is 0, so it is first in every candidate list
until it, too, has issued 2.6 million claims. The old job gets **nothing for a
month**. The same happens after a purge (which zeroes `claims_issued`), after a
job has waited days on a derived-file build or a leave transition, after a
deactivate/reactivate, and — in the other direction — when an allocation is
raised (a job at 10% with a ratio of 100,000 drops to 20,000 at 50%). PLAN.md
claims "no starvation of any job above 0%" and "most behind its configured
allocation share"; the code implements the formula PLAN.md gives, and the formula
does not have the property PLAN.md claims for it. With priority gone there is no
second axis to work around it with.

Left alone because what "share" means over time is a product decision:

- **(a) Virtual start time** (start-time fair queuing). Add
  `jobs.claims_baseline BIGINT`; order by `(claims_issued - claims_baseline) /
  allocation`; on activation, on an allocation change and on a purge, set the
  baseline so the job's ratio equals the **lowest ratio among the other jobs
  offering work** (or 0 if there are none). A job joins at parity and gets its
  share from then on; nobody is starved; selection stays deterministic and one
  statement. One column, ~30 lines, and the existing scheduler tests keep their
  meaning.
- **(b)** Keep catch-up but cap it: clamp a job's deficit to at most N claims
  behind the next job. Simple; still a burst.
- **(c)** Windowed deficit (claims in the last hour): needs a time index on
  `task_claims` and a count per claim request, which is what the counter exists
  to avoid.
- **(d)** Leave it, and say so: PLAN.md would have to drop "no starvation" and
  tell admins to expect a newly activated job to monopolise the fleet.

**Recommendation: (a).**

### U2 — the write volume of a leave-generation fold

Section 7. **Recommendation: (b), a staging table with a periodic merge, before the
first full-size leave job; larger tasks meanwhile.**

### U3 — positions captured during games cannot be exported

`capture_positions` exists to build "a corpus being built for later use" — 1.8
million positions for a 40,000-pair job — and a games job's export and stream are
its `game_results` rows only. The positions are reachable one page at a time
through nothing at all (the public feed for a games job lists results, not
positions). Options: **(a)** a second artifact per export
(`…/positions.ndjson.gz`) using B2's query filtered to `game_index IS NOT NULL`;
**(b)** tagged lines (`"kind": "result" | "position"`) in the one file, which
changes what existing consumers of a games export read; **(c)** leave it until
there is a consumer. **Recommendation: (a)**, since (b) changes an existing format
and (c) means the first person to want the corpus needs database access.

### Smaller, noted rather than asked

- **Identity before rate limit.** `WorkerIdentity` resolves the credential with a
  database statement *before* the handler's `check_rate_limit`, and a request with
  an unknown UUID or key is answered `401` without ever being counted. An
  unauthenticated caller can therefore make the server run one indexed lookup per
  request at any rate. Each is a primary-key probe, so this is cheap to absorb; a
  per-IP bucket for failed worker authentication would close it.
- **`?rack=` canonicalisation** sorts the query's characters by Unicode code point.
  That equals machine-letter order for English (`?` before `A`–`Z`), and would not
  for a distribution whose letters are outside ASCII or longer than one character;
  such a lookup would miss. No such opening-rack job exists yet.
- **Concurrent imports** each hold a ~94 MB tarball in memory; two admins
  importing at once on a 2 GB task is survivable, five is not. Admin-only.

### To do after merge (not decisions)

1. **Push MAGPIE `birdtest-contribute`** (D1).
2. Dispatch the nightly workflow once, as the seventh audit asked; it has failed
   on `main` every night this week.

---

## 12. Verification

- Backend: `cargo clippy --locked --all-targets -- -D warnings` clean; `cargo test
  --locked` against Postgres 16: **172 tests** (92 unit and contract, 80
  integration), all passing (165 before this audit). The five `magpie_smoke` tests
  are `#[ignore]` by design.
- Frontend: `npm run check` — 0 errors, 0 warnings; `npm run build` succeeds.
- MAGPIE at `d93dacaf`: `make magpie_test` (sanitizers, `-Werror`) and
  `./bin/magpie_test contribute` pass; `clang-format --dry-run -Werror` clean on
  the three touched files; `magpie builders` reports `0.1.0`/`nehalem`.
- R1's test fails against the old code (eleven keys created) and passes against
  the new; B3's numbers are `psql \timing` on a 1,000,000-record synthetic job,
  before and after, with the generic plan forced to check the unprepared
  statement is needed.
- End to end: see section 13.

---

## 13. End-to-end run

`scripts/e2e_magpie.py` against an isolated compose project (`birdtest-e2e8`, its
own volumes and ports), backend and builder images built from this branch's
working tree, the checkout's `portable_release` MAGPIE at `d93dacaf` reporting
`0.1.0`, and a `data-20251004` import through the real API.

The first attempt reproduced D1: the script runs `docker compose run --build
derived-builder`, whose image build fetched the unpublished pin and failed. The
run recorded below supplied the two images' `MAGPIE_REPO`/`MAGPIE_COMMIT` build
arguments from a temporary compose override pointing at the local checkout
(nothing in the repository was changed for it).

**Every job type passed:**

| Job | Result |
|---|---|
| `games` | 2 accepted claims |
| `game_pairs` | 2 accepted claims, pentanomial stored |
| `opening_rack`, static, with a wordmap | derived files built by the builder task in 4 s; the worker built its own wordmap and the bytes agreed; 2 accepted claims |
| `opening_rack`, simming | 2 accepted claims, simulated statistics stored |
| `leave_generation` | 128 s, 2 accepted claims, occurrences folded, nothing written into MAGPIE's data directory |

The backend log held **no** `ERROR` or `WARN` line. M1 is exercised by every one of
these: each request went through the stricter `config_contribute_validate_common`.

Then, against the same live stack, the things this audit changed:

- **B2.** The static opening-rack job was force-completed and exported through the
  real API: `running` → `ready`, 40 rows. The object was read back out of MinIO and
  its first line carries `moves: [{rank: 1, move: "(exch VWWY)", score: 0, equity:
  9.482, …, plies: []}]` beside the record's own columns — a real MAGPIE's moves,
  in the artifact.
- **B3.** `?worker=<the job's contributor's pseudonym>` returned that worker's rows
  and a cursor; `?worker=nobody-here` and a well-formed pseudonym nobody has
  returned empty pages in ~1 ms.
- **C1.** Job list, job detail and rating pools answered `200` through the display
  pool while MAGPIE claimed and submitted through the main one.

The stack was torn down afterwards, volumes and images included.

---

## 14. Tests added

| Test | What it pins |
|---|---|
| `worker_api::a_parked_job_shuts_nobody_down` | B1 |
| `worker_api::an_opening_rack_corpus_carries_each_racks_ranked_moves` | B2: the admin stream's lines carry moves and plies |
| `worker_api::the_results_feed_filters_by_who_a_name_is` | B3: a username, a pseudonym, a name that is nobody's, and a well-formed pseudonym nobody has |
| `worker_api::the_display_pool_bounds_its_reads` | C1/C2: the display pool's statement timeout, the main pool's absence of one, and a cancelled read as `503` |
| `auth_api::concurrent_key_requests_cannot_exceed_the_key_limit` | R1; fails against the old code |
| `error::tests::a_pool_timeout_is_a_503_with_retry_after`, `…::a_driver_error_is_not_shown_to_the_client` | B4 |
| `worker_api::an_opening_rack_result_must_answer_the_racks_it_was_given` (extended) | B5: a rack listed twice is a `400` |
| MAGPIE `test_a_request_must_state_its_distribution_and_layout` | M1 |
