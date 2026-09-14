# birdtest audit — findings

Branch: `audit/birdtest-2026-09-13-pass2`, off `main` at `40d2218`.
MAGPIE changes: `birdtest-contribute` only, at `cac07a8a`.
Date: 2026-09-13.

This is the authoritative record of every code-versus-`PLAN.md` decision made
during the audit, plus the bugs, races, critical-path changes and performance
findings behind them. Each entry states what the code does, what the plan says,
what was decided, and why — enough to second-guess the decision without reading
the diff.

**Counts: 14 code-wins (PLAN.md updated), 6 plan-wins (code changed), 6 left
unresolved pending human input.**

The default bias is that the code wins and `PLAN.md` is brought level with it,
because the plan is a summary. The code was changed only where it was plainly
wrong, or where the plan described the behaviour the rest of the system needs.

---

## How this audit was run

- Read `PLAN.md` in full, then the whole backend, the migration, the frontend,
  `infra/`, both CI workflows, `scripts/`, `worker/fake_worker.py`, and MAGPIE's
  `birdtest-contribute` branch (`src/impl/contribute.c`, the
  `config_contribute_*` executors, `src/ent/autoplay_results.c`).
- `PLAN.md`'s reproduced schema block was diffed against
  `backend/migrations/0001_initial.sql` mechanically; they were in sync before
  the audit and are in sync after it.
- `cargo test` (117 tests before, 126 after) and `cargo clippy --all-targets -D
  warnings` on every change, against a real Postgres.
- MAGPIE's `magpie_test contribute` after the MAGPIE change.
- `scripts/e2e_magpie.py` — one real `magpie contribute` task per job type
  against the real stack — run twice: once to establish a baseline, once after
  every change on both sides. Both passed. The second run is what proved
  finding **B1** was real (see below).

---

## 1. Bugs

### B1 — an opening-rack job with a `best` recorder silently analyses one move per rack

*Code wins in principle, but the code was wrong: **code changed**, both sides.*

- **What the code did.** `player_configs.recorder_type` is validated against
  `best | equity | all`, `num_plays_recorded` is `NOT NULL CHECK >= 1`, and job
  creation accepted any combination. MAGPIE's opening-rack executor calls
  `impl_move_gen` and then, for a simmer, `impl_sim` over the resulting move
  list.
- **What actually happens.** `-r best` is `MOVE_RECORD_BEST`: move generation
  keeps the single top play and discards the rest. Confirmed directly against
  MAGPIE — `generate` on `AEINRST` reports `Showing 1 of 1 plays` under
  `-r1 best` and `Showing 15 of 100 plays` under `-r1 all`. So an opening-rack
  job with a `best` player stores exactly one move per rack whatever
  `num_plays_recorded` says, and a *simming* player configured that way has
  nothing to choose between, which makes `num_plies`, `num_plays`,
  `max_iterations` and `stopping_pct` inert as well.
- **Why nothing caught it.** Every downstream signal is green: the racks are
  analysed, the submission is accepted, `racks_analyzed` climbs, `?rack=`
  returns a "full ranked move list" of one entry. Before this audit, the
  end-to-end suite's own simming opening-rack job stored 1 move per rack with
  `num_moves = 1` and passed, because it only asserted that win% and per-ply
  statistics were *present*.
- **What PLAN.md said.** That an opening-rack job returns "the full ranked move
  list (all N plays that were evaluated)", that "a simmer may need to rank
  hundreds of candidates to order the top few correctly", and — in the
  migration's own comment — "For autoplay in birdtest, always use 'best'". The
  last is right for autoplay and wrong for opening racks, and nothing said so.
- **Decision: code changed on both sides, PLAN.md updated to match.**
  - birdtest refuses `recorder_type = 'best'` with `num_plays_recorded > 1` on
    an opening-rack job, naming the remedy
    (`routes::admin::validate_opening_rack_player`). `best` with
    `num_plays_recorded = 1` stays legal, because "the best opening play for
    every rack" is a real job. The rule is scoped to opening racks: a `games`
    job's players are applied through autoplay, where a simmer's candidate list
    is sized by `num_plays` rather than by the move recorder, and `best` is
    correct there.
  - `scripts/e2e_magpie.py` now uses `recorder_type: 'all'` for its simming
    player and a separate `num_plays_recorded: 1` static player, and asserts
    `num_moves > 1`.
- **Evidence it was real.** After the change, the same end-to-end job stores
  **72–100 ranked candidates per rack with the top 5 kept**, each with win
  percentage, blended utility and per-ply statistics — against 1 ranked and 1
  stored before. The job also went from 5 s to 342 s, which is the cost of
  actually simulating rather than "simulating" a single forced move.
- **Alternative considered and left unresolved — see U6.**

### B2 — `num_moves` for an opening rack was the reported count, not the ranked count

*Plan wins: **code changed**, in MAGPIE and birdtest.*

- **What the code did.** `RackAnalysis` carried only `rack` and `moves`, and
  `PositionAnalysis::opening_rack` set `num_moves = moves.len()`. MAGPIE's
  opening-rack writer already computed the ranked count and threw it away
  (`autoplay_results_write_ranked_plays_json` returns it; the caller ignored the
  return).
- **What PLAN.md said.** `position_analysis_records.num_moves` "records how many
  were ranked, so the discarded tail is still visible as a count" — and the
  column's own migration comment says the same.
- **Decision: code changed.** MAGPIE now writes `num_moves` on each rack
  analysis; birdtest reads it as an optional field and falls back to the list
  length when absent (builds that predate it reported everything they ranked,
  so the length *was* the honest answer for them). The plausibility rule "a
  worker cannot report more moves than it says it generated" now applies to
  opening racks as it already did to captured positions.

### B3 — an opening-rack submission was unbounded by anything the job controls

*Neither: **code changed** in MAGPIE (a gap, not a disagreement).*

- **What the code did.** MAGPIE reported *every* ranked play per rack
  (`play_cap = 0`), and the server truncated to `num_plays_recorded` on receipt.
- **Why that is a problem.** A task is a batch of up to 10,000 racks, and how
  many plays each ranks is decided by the recorder type and `num_plays`, neither
  of which relates to how many the server keeps. With a recorder that keeps
  candidates, an ordinary job can put a submission past `MAX_RESULT_BYTES`
  (64 MiB), which comes back `413`, counts as a failed task, and ends the
  contributor's run after five of them. Everything past `num_plays_recorded` was
  bytes nobody stores.
- **Decision: code changed.** MAGPIE caps the reported list at the player's
  `num_plays_recorded` and states `num_moves` alongside it, so nothing is lost.
  A request that omits `num_plays_recorded` keeps the old behaviour (the writer
  treats a cap of 0 as no cap). PLAN.md's "How much of an analysis is kept"
  rewritten to describe the client-side cap.

---

## 2. Race conditions

### R1 — rating fits are not serialized, so the newest run can be the stalest

*Plan wins on intent: **code changed**.*

- **What the code did.** `ratings::recompute` opened a transaction, read
  membership and evidence, fitted, and wrote a `rating_runs` row stamped
  `now()` — with no lock. The two-minute sweep and an admin's
  `add_member` / `remove_member` / `recompute` can run concurrently.
- **The race.** The sweep reads membership at T. An admin removes a config at
  T+1 and its refit commits. The sweep commits at T+2 with a *later*
  `computed_at` but the *older* membership. `/api/rating-pools/:id` reads
  `ORDER BY computed_at DESC LIMIT 1`, so the page shows the removed config
  still rated, and the residuals still built from its games. It self-corrects at
  the next sweep (the stored `pairs_used` no longer matches), so the window is
  up to two minutes of a visibly wrong answer to an action the admin just took.
- **What PLAN.md said.** "either change refits the whole pool", and a fit is "a
  pure function of (pool membership, matching evidence)" — which it is not if
  the two are read across an interleaving.
- **Decision: code changed.** `fit_and_store` takes
  `pg_advisory_xact_lock(2, hashtext(pool_id))` before reading anything, so the
  read and the write are one atomic decision. Per pool, so pools never wait on
  each other. PLAN.md updated to state the lock and the symptom it prevents.
- **Bonus:** folding the staleness check into the same transaction removed a
  second `build_matrix` per stale pool per tick (see P1).

### R2 — a purge and an in-flight claim can both win

*Plan wins on intent: **code changed**.*

- **What the code did.** `purge_job` took the job's row lock (`load_job_for_update`)
  and then deleted claims, progress, artifacts, transitions and tasks. It did
  **not** take the job's dispatch advisory lock.
- **The race.** A claim holds the dispatch lock, has read the seed cursor, and
  has inserted its task and request but not yet reached the
  `UPDATE jobs SET claims_issued = ...` that blocks on the purge's row lock. The
  purge's `DELETE FROM tasks` cannot see that uncommitted row, so it commits a
  clean job; the claim then commits a task into it. The purged job restarts with
  its seed cursor already past zero — so the slice that task covers is never
  regenerated — and with `claims_issued = 1` on a counter the purge just reset.
  For a leave-generation job the claim can also land a task in a generation
  whose universe the purge just reseeded.
- **What PLAN.md said.** It acknowledged this obliquely: "a purge running
  between the cursor read and the insert can still produce [a lost race]".
- **Decision: code changed.** `purge_job` calls `jobs::lock_job_dispatch` first,
  before the census, so the numbers audit-logged are the ones actually
  destroyed. PLAN.md updated in two places (the purge bullet and the lost-race
  note).

### R3 — a rolling deployment fails the outgoing instance's live work

*Neither: **code changed** (deployment configuration and two guards).*

- **What the code did.** Startup marks every `input_data_imports` and
  `job_exports` row still `running` as `failed`, on the documented assumption
  that birdtest is a single instance. `infra/variables.tf` enforces
  `desired_count <= 1` — but the ECS *service* used the default rolling deploy
  (minimum healthy 100%, maximum 200%), which starts the new task before
  stopping the old one.
- **The race.** Every deployment briefly runs two instances. The new one reaps
  the old one's in-flight import or export as failed; the old one then writes
  `staged` / `ready` over that `failed` row, leaving the reaper's error text
  attached to a row that claims to have succeeded. An admin confirms a diff, or
  downloads an export, that nobody was sure had finished.
- **What PLAN.md said.** "birdtest runs as a single instance, so the spawned
  task needs no lease — and, for the same reason, startup marks any row still
  `running` as `failed`". True of steady state, not of deployments.
- **Decision: code changed.** `deployment_minimum_healthy_percent = 0` and
  `deployment_maximum_percent = 100` on `aws_ecs_service.main`, so ECS stops the
  old task before starting the new one; and both terminal writes are guarded on
  `state = 'running'`, so a reaped row stays reaped. PLAN.md's backup section
  item 4 updated.

### R4 — checked and found sound (no change)

Recorded because "we looked and it holds" is worth as much as a fix:

- **Lock ordering is consistent everywhere.** Submit takes claim → task → job.
  `release_claim` and `reclaim_expired` take claims → tasks. `issue_claim` takes
  task → claim-insert → job. No cycle exists, and the one shape that could close
  one — a claim waiting on an existing claim row's unique index while holding a
  task lock — cannot arise, because `next_available` excludes tasks the identity
  already holds a non-abandoned claim on.
- **`count_first_result` is sound.** It decides "first accepted result for this
  task" from a `COUNT`, which is only correct because `submit_result` takes
  `SELECT 1 FROM tasks WHERE id = $1 FOR UPDATE` before storing anything.
  Verified the lock is still there and still precedes `store_result`.
- **Leave-generation transition ownership holds.** The deciding claim commits
  the `leave_generation_transitions` row (it is the transaction's only write),
  which both publishes ownership and releases the advisory lock before the
  upload. A generation closes only when no claim for it is still `claimed`, so a
  submission cannot arrive mid-transition through the normal flow; the
  `leave_generation_artifacts` check in `insert_record` remains correct as
  defence against restored or hand-edited state.
- **`reclaim_expired` versus a concurrent submission.** The reclaim statement
  re-checks `state = 'claimed'` after waiting on the submit path's row lock, so
  a claim is abandoned *or* completed, never both, and the task's
  `active_claim_count` is decremented once.
- **Two concurrent activations in one tier** are serialized on
  `pg_advisory_xact_lock(hashtext('birdtest.activate_tier'), priority)`; the
  job row lock is taken first and consistently.

---

## 3. Critical path — work moved off it

The priority is completing tasks and getting a worker its next one. Everything
here is either removed, batched, or moved off the request; nothing that decides
anything was deferred.

| # | What moved | Was | Now | Why safe |
|---|---|---|---|---|
| C1 | The live stats payload | Built synchronously after commit, before the worker was answered | Spawned, and coalesced per job | Display-only; nothing in the claim path reads a statistic. Every event carries the whole payload, not a delta, so a merged one says everything the ones it replaced would have |
| C2 | `game_stats` | Computed twice per submission when a job had a subscriber (finish check, then the payload) | Once, inline, for the finish check | The second was only for the payload, which now runs elsewhere |
| C3 | The leave-generation transition | Spawned but **awaited**, so the claiming worker's request hung for tens of seconds | Spawned and not awaited; the claim moves to the next candidate | Nothing in the request needs the answer: the generation it opens has no tasks until the transition commits. Ownership is committed *before* the spawn, so no second transition can start; failure hands ownership back and is logged |
| C4 | `expected_data` | Three queries (player ids, player configs, input data), inside the job's dispatch lock | One query, a union over the per-type config tables | Same rows, asserted by a new test; a type with no row in a table contributes nothing |
| C5 | `insert_game_request` | Re-resolved both player ids from their *names* | Ids passed in from the config the caller already read | The caller read them a moment earlier from the same row |
| C6 | Worker identity | Three statements per worker request: lookup, `last_used_at`/`last_seen_at` touch, ban check | One | Same answers; the ban check is now in the statement that resolves the identity, covered by a new test for both credential kinds |
| C7 | `reclaim_expired` | One statement per candidate job | One statement per claim attempt, over the whole tier | `EXPLAIN` confirms the planner reaches expired claims through the partial index on *open claims* and filters by job afterwards, so the scan never depended on the job in the first place |
| C8 | Position / move / ply inserts | One statement per position, plus its moves | Multi-row statements | An opening-rack task is up to 10,000 positions, so this was thousands of round trips inside the submit transaction holding the task's row lock |

**Deliberately left inline:** the SPRT and finish-condition evaluation. It gates
whether a job keeps dispatching, `PLAN.md` chose it over a counter on measured
evidence, and a counter that drifted low would leave a job running forever.

---

## 4. Performance — most severe first

Impact figures are derived from `PLAN.md`'s own measured table where one exists,
and stated as reasoning where one does not.

1. **A slow claim on one job could stall the whole server.** *Fixed.*
   `pg_advisory_xact_lock` on a job's dispatch was an unbounded wait held for
   the rest of the claim transaction — and one claim transaction is not short:
   seeding a leave generation's rack universe is 3.2 M rows and **37–56 s**,
   inside the claim, under that lock. Every other claim for that job blocks for
   the duration *while holding one of the pool's twenty connections*, so twenty
   waiting workers starve submissions, the dashboard and every other job. Now
   bounded at two seconds (`lock_timeout`); a claim that gives up treats the job
   as having nothing right now and tries the next candidate. Ordinary contention
   is milliseconds, so this is never reached in normal operation.
   *Expected impact avoided: a full-server stall of up to a minute, per
   generation transition, on any leave-generation job.*
2. **The rating matrix scanned the whole of `game_results`.** *Fixed.*
   `build_matrix` selected one result per task across the entire table and
   filtered to the pool's jobs afterwards, so it sorted every paired result ever
   recorded. Measured at **452 ms for 600,000 results**; it grows with the
   database, not with the pool. It runs every two minutes per pool *and* on
   every public read of `/api/rating-pools/:id` (residuals rebuild the same
   matrix). The job filter now comes first, in a CTE, so it is an index walk of
   those jobs' tasks. The sweep also builds it once per tick instead of twice
   (staleness check, then fit).
   *Expected impact avoided: seconds per pool per sweep at ten million results,
   and the same on an unauthenticated page view.*
3. **Opening-rack submissions cost a round trip per rack.** *Fixed.*
   `racks_per_batch` defaults to 500 and is capped at 10,000; each position was
   its own `INSERT ... RETURNING`, followed by its own moves insert. At 1 ms to
   RDS that is **1–20 s per submission**, all inside the submit transaction
   holding the task's row lock, with the worker waiting. Now a handful of
   multi-row statements.
4. **A generation transition blocked the worker that triggered it.** *Fixed
   (C3).* Tens of seconds of one worker doing nothing, once per generation.
5. **The live stats payload was on the submission path.** *Fixed (C1).*
   Measured components: `game_pair_stats` 54 ms, `worker_contributions` 136 ms,
   `leave_gen_stats` 210 ms — all per submission, for every job with a
   dashboard open, and `game_stats` twice over. Now one build at a time per job,
   off the request.
6. **Claim-time task selection sorted every available task of a job.** *Fixed.*
   `next_available` orders by `created_at`, and the queue index was
   `(job_id, state) WHERE state = 'available'` — so the sort column was not in
   the index and the planner read and sorted the job's whole available set. At
   `redundancy > 1` tasks stay available until their slots fill, so that set is
   not small. Index changed to `(job_id, created_at) WHERE state = 'available'`.
7. **Per-claim round trips.** *Fixed (C4–C7).* A `games` claim went from roughly
   eighteen statements inside the dispatch lock to about twelve, and a worker
   request from three statements of identity resolution to one. This is the
   number that bounds a single job's dispatch throughput, because the lock
   serializes claims per job.
8. **`GET /api/jobs/:id/results` is an unbounded, unindexed scan.** *Flagged —
   see U1.* For an opening-rack job it joins every `position_analysis_records`
   row of the job and sorts by `submitted_at`; a full English job is 3.2 M rows
   and a capture-on games job is millions more. Public and unauthenticated.
   *Expected impact: seconds to minutes per request at full job size.*
9. **The job list and the contributor lists aggregate over whole tables.**
   *Partly fixed, rest flagged — see U2.* `GET /api/jobs` runs two `COUNT(*)`s
   over `tasks` per job per page view (measured at **2,188 ms** before the
   `games_completed` counter, which fixed only the game totals); `GET /api/users`
   and `GET /api/workers` group over all of `task_claims` (measured 93 ms at
   44,000 claims, linear from there). `tasks_job_idx` widened to
   `(job_id, state)` so both task counts are index-only; the rest needs running
   totals or a cache.
10. **SPRT reads `game_results` on every submission.** *Left as designed — see
    U3.* Measured at ~50 ms per 400,000 units. `PLAN.md` chose this
    deliberately over a counter so a drifted counter cannot stop a job early.
11. **A simming opening-rack job is genuinely expensive.** *Inherent, noted.*
    With the recorder fixed (B1), the end-to-end job ranked ~100 candidates and
    simulated 5 of them at 60 iterations over 2 plies: **~57 s per rack**. That
    is the work the config asks for, not a defect, but it makes
    `racks_per_batch` the lever that keeps a task inside the heartbeat timeout,
    and an admin sizing one should know the number.

---

## 5. PLAN.md reconciliation

Every place the code and `PLAN.md` disagreed. "Code wins" means `PLAN.md` was
updated; "plan wins" means the code was changed.

| # | Subject | Code | PLAN.md | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | Captured positions' key | `(task_id, game_index, turn_number)`, with MAGPIE flattening a pair as `game_number * 2 + (pair_game_number - 1)` | "the server keys on `(game_number, pair_game_number, turn_number)`" | **Code wins** | The plan contradicts itself: its own wire format shows only `game_index`. The flattening is what makes the schema's two-column partial index and the batch-size check work, and it is verified in MAGPIE's `write_captured_position`. Plan updated to describe the flattening |
| K2 | Opening-rack move reporting | Was: report everything, truncate on receipt | "A worker reports every move it ranked" | **Plan wins (code changed)** | See B2/B3. The plan's own `num_moves` semantics were unachievable without the client stating the count, and reporting everything is an unbounded submission. Both sides changed; plan rewritten to describe the cap |
| K3 | `recorder_type` on an opening-rack job | Any of `best`/`equity`/`all` accepted | "always use 'best'" (migration comment), free choice (admin semantics) | **Plan wins (code changed)** | See B1. `best` makes the job store a fraction of what it claims. Validation added; plan updated with the rule and the reasoning |
| K4 | The generation transition and the claim | Spawned and awaited | "roll back, run the transition, and restart the whole attempt" | **Code wins, then improved** | The plan's "roll back" was already wrong (the ownership row must commit, which the code does and the plan elsewhere explains). The await was a real cost, so the code changed too — the claim now returns to the next candidate. Plan updated to both |
| K5 | Lazy reclamation scope | Per selected job, one statement each | "Expired claims **for the selected job**" | **Code wins, then improved** | `EXPLAIN` shows the scan is job-independent, so per-job was N scans of the same rows. Now one statement per attempt over the tier; plan updated |
| K6 | The dispatch lock's wait | Unbounded | "it turns a lost race into a short wait" | **Code wins, then improved** | "Short" was true of ordinary contention and false of the universe-seeding case. Bounded at two seconds; plan updated with the pool-exhaustion reasoning |
| K7 | Purge versus dispatch | No dispatch lock | "a purge running between the cursor read and the insert can still produce [a lost race]" | **Plan wins (code changed)** | The plan named the hole rather than accepting it. Closed; plan updated |
| K8 | The SSE push | Built inline, once per submission | "build and push the full stats payload only if the job has an SSE subscriber"; "Debouncing the SSE push per job is the cheaper next move ... not needed yet" | **Code wins, then improved** | The plan already identified the remedy and deferred it. The audit's brief is explicitly to move observational work off the critical path, so it was done. Plan updated: a new step 7, and the measurement note rewritten |
| K9 | Rating fits | Unserialized | "either change refits the whole pool"; "a pure function of (pool membership, matching evidence)" | **Plan wins (code changed)** | See R1. Plan updated with the lock and the symptom |
| K10 | Rating evidence query | Whole-table `DISTINCT ON`, filtered after | "the query above is one grouped scan" | **Code wins, then improved** | Scoped to the pool's jobs first; plan's measured-costs table annotated (it also now records that the query runs on public reads, not only the sweep) |
| K11 | `expected_data` | Three queries, matching on `job_type` | "The `expected_data` builder is a query, not an inference engine" | **Code wins, then improved** | It was three queries and a match. Collapsed to one union query; plan updated |
| K12 | Worker identity resolution | Three statements | "It verifies the worker is not banned" (silent on cost) | **Code wins, then improved** | Collapsed to one; plan's request-handling step 1 updated |
| K13 | Leave generation and the filesystem | Writes the fetched previous-generation KLV to `lexica/<lexicon>_birdtest_previous.klv2` | "Neither the forced racks nor the results touch the filesystem"; `./data` writable only for wordmaps | **Code wins** | The statement is true of the racks and the results, which is what it was about, but the fetched KLV *must* be on disk for MAGPIE to load it as leaves, and the prefixed name is load-bearing (`lexicons_and_leaves_compat` infers a distribution from the name). Plan updated to say so, including that `./data` must be writable for leave generation |
| K14 | ECS deployment | Default rolling deploy | "birdtest runs as a single instance" | **Plan wins (code changed)** | See R3. Service configured to stop-then-start; plan updated |
| K15 | Position/move/ply writes | One statement per position | Silent | **Code wins, then improved** | No disagreement, but the batching has two correctness properties worth stating (insertion-order `RETURNING`, and matching the conflict-ignoring subset back on its index columns). Plan updated |
| K16 | Task indexes | `tasks_queue_idx (job_id, state)`, `tasks_job_idx (job_id)` | Reproduced verbatim | **Code changed, plan tracks it** | See P6/P9. Both indexes changed in the migration and in the plan's schema block, which was re-diffed and is in sync |
| K17 | `desired_count` | Constrained to 1 in `variables.tf` | "`desired_count` stays 1" | **Agree, extended** | No disagreement; the plan now also records what the service does during a deploy |
| K18 | Import/export terminal writes | Unconditional | Silent | **Code changed** | Guarded on `state = 'running'` so a reaped row stays reaped; plan's item 4 updated |
| K19 | Python worker | Test-only everywhere: README, RUNBOOK, TESTING, compose profile, Dockerfile target, docstring | "That client is retired; `worker/fake_worker.py` remains as a MAGPIE-free way to test the server itself" | **Agree — nothing to correct** | Searched the whole tree for any treatment of the Python worker as a production client and found none. The compose service is behind a `fake-worker` profile documented as end-to-end-only, the Dockerfile target says the same, and `RUNBOOK.md` explicitly says "**Never use `worker/fake_worker.py` for this.**" One change made for a different reason: it now sends `num_moves`, to keep matching what MAGPIE sends |

---

## 6. MAGPIE `birdtest-contribute`

Checked out and verified directly; **all changes were made on
`birdtest-contribute`, none on `main` or any other branch.**

### What was missing, and was fixed

**M1 — the opening-rack executor neither capped its play list nor reported the
ranked count.** `config_contribute_analyze_rack` passed `play_cap = 0` to
`autoplay_results_write_ranked_plays_json` and discarded the function's return
value, which is exactly the ranked count. Fixed: the cap is the player's
`num_plays_recorded` and the count is written as `num_moves`. This is the MAGPIE
half of B2 and B3, and it is what makes B1's fix observable —
`num_moves` is now the number that says whether the recorder actually ranked
anything. Verified end to end: **72–100 ranked, 5 stored, per rack**.

### What was checked and found already present

- **All six worker endpoints**, with the right methods, bodies and headers
  (`Authorization: Bearer` / `X-Worker-UUID`, never both).
- **The claim body is sent** with `magpie_version` and `unsupported_jobs`, and
  the unsupported set is kept in memory only, as specified.
- **Version negotiation**: `MAGPIE_VERSION` is `0.1.0`, which meets the
  server-wide floor; `contribute_compare_versions` compares numerically
  (`1.10.0 > 1.9.0` is unit-tested); a job above the build declines rather than
  exiting.
- **Decline reasons** are exactly the three the server accepts
  (`missing_data`, `magpie_version`, `unknown_job_type`), and an unrecognised
  `job_type` declines rather than ending the run.
- **Data verification** resolves through `data_filepaths_get_readable_filename`,
  caches digests on `(path, size, mtime, inode, ctime)` with nanoseconds — with
  a test that a same-size replacement inside one mtime tick invalidates the key
  — and prints the resolved absolute path.
- **Retry policy**: 429 honours `Retry-After` (5 attempts), 5xx and transport
  errors back off 1/2/4/8/16 s, other 4xx are returned to the caller.
- **`seed` is read with `strtoull` from a decimal string**, unit-tested against
  `UINT64_MAX`.
- **The pentanomial** is accumulated at the one point both games of a pair are
  final, consolidated across threads, and satisfies both of birdtest's
  cross-checks by construction (`Σ i·bucket[i] = 2·wins + ties` because a
  bucket is the sum of the two games' half-points; `Σ buckets · 2 = games`
  because both games and the bucket are recorded together).
- **A pair's two games flatten to one `game_index`** over `[0, 2N)`, which is
  what birdtest's `game_index < games_in_batch` check and its
  `(task_id, game_index, turn_number)` index need. This was the audit's main
  suspicion about the capture path and it is correct.
- **`stat_get_variance` returns 0 for ≤ 1 samples**, so a one-game batch cannot
  send `NaN`/`Inf` into a plausibility check that would reject it; an empty
  `divergent_games` reports zeros rather than `0/0`.
- **`win_percentage` is on 0–100 and `blended_utility` on 0–1**, matching
  birdtest's bounds exactly (`sim_args.h` documents the utility as in `[0, 1]`).
- **`leavegen_max_games`** is what ends a leave-generation task, with an
  unreachable rack target, and the generation's target is deliberately absent
  from the request.
- **Wordmap staleness** is handled by the `.wmp.src` sidecar, written after the
  rename.
- **The settings snapshot/restore round trip works on this branch.** A stale
  `settings.txt` in the MAGPIE checkout, written by an *older* build, fails
  every start with `unrecognized command or argument 'wit1'` — `-wit1` and
  `-writerackequitycsv` no longer exist. Confirmed this is a local artefact and
  not a branch bug: with the stale file removed, the branch writes a settings
  file and reads it back cleanly. Worth knowing because `impl_contribute`
  performs exactly that round trip on every run, so writer and parser must never
  drift apart.
- **Contract fixtures** are byte-identical between `contract-fixtures/` and
  MAGPIE's `test/birdtest_contract/`, and `magpie_test contribute` passes.
- **`magpie` and `magpie_test` both build** on the branch with
  `-Werror -fsanitize=address,undefined,leak`.

---

## 7. Deployment and implementation blockers

| Finding | Status |
|---|---|
| A rolling deployment runs two instances and the new one fails the old one's imports/exports (R3) | **Fixed**: stop-then-start on the service, plus `state = 'running'` guards |
| A single slow claim can exhaust the connection pool and stall the server (P1) | **Fixed**: bounded dispatch-lock wait |
| Migrations, graceful shutdown, health checks | Verified: migrations run before `bind`; `SIGTERM`/`SIGINT` drain in-flight requests (which matters precisely because a dropped submission costs the worker a whole batch); `/health` backs both the container healthcheck and the ALB |
| ALB `idle_timeout` versus long requests | Verified 300 s, above MAGPIE's own 120 s request timeout. With C3 the longest claim is now short anyway; job creation for an English leave-generation job (38 s measured) is the remaining long request, and is an admin action |
| Nginx body limit and SSE | Verified `client_max_body_size 64m` matches `MAX_RESULT_BYTES`, and `proxy_buffering off` on `/api/` |
| Secrets | Verified `DATABASE_URL` and `SESSION_SIGNING_KEY` come from SSM at task start, never appearing in the task definition or state; `GITHUB_TOKEN` optional |
| Config validation | Verified startup fails on a missing or wrong-length signing key, an unknown `MAIL_BACKEND`, a non-boolean `SECURE_COOKIES`, an unparseable number, or a `MIN_MAGPIE_VERSION` that is not a version |
| CI | Verified it covers clippy with `-D warnings`, the backend tests against a real Postgres, the frontend check and build, both images, `terraform validate`, and MAGPIE's half of the contract against this branch's fixtures. Nightly runs the real-MAGPIE end-to-end suite |
| MAGPIE binary availability | Verified the backend has no MAGPIE dependency at all — leave-generation KLVs are built in `jobs::klv` — so neither image builds or ships one |

No blocker was left unresolved.

---

## 8. Left unresolved — needs human input

These are genuine trade-offs. **Neither the code nor `PLAN.md` was changed for
any of them.** Each carries a recommendation, which is a suggestion for whoever
picks it up rather than a decision taken here — the point of the section is that
the evidence does not settle these on its own.

Each entry opens with what the thing under discussion is *for*, so it can be
read without the rest of this document or any prior knowledge of birdtest: what
the mechanism does, who depends on it, and which higher-level goal it serves.
The trade-offs only make sense against that goal — "slow" and "wrong" cost
different amounts depending on whether something is on a contributor's path, on
a visitor's page, or standing between a job and its conclusion.

### U1 — `GET /api/jobs/:id/results` has no bounded plan

**What this is.** birdtest crowdsources word-game computation. An admin defines
a *job* — a long-running research goal such as "analyse every possible opening
rack" or "play 400,000 games between these two bot configurations" — the server
breaks it into *tasks*, and volunteers running the MAGPIE engine claim tasks,
compute them, and submit results. Those results are the product: the entire
reason for asking strangers to donate CPU time.

`GET /api/jobs/:id/results` is how that product is read back. It is public,
paginated, and returns one row per stored result — for an opening-rack job, a
rack and the best play found for it; for a games job, one batch's win/loss/draw
counts and score statistics. Its being public is a deliberate decision made
earlier: the bulk NDJSON download was moved behind admin authentication because
each caller holds a database connection for as long as they keep reading, and
this paginated read is what the public was left with in its place. So it is the
only route by which anyone but an admin can see what a job actually produced.

**What question is it answering, though?** This turns out to be the crux, so it
is worth being exact. The endpoint is ordered
`ORDER BY r.submitted_at DESC, r.rack ASC` and takes an optional `?worker=`
filter, which together make it a **recency feed**: *what has this job — or this
one contributor — produced lately?* The `DESC` is what makes the first page the
newest arrivals, which is the only sensible thing for a live dashboard and the
only sensible thing for "show me what this volunteer just submitted". The
`rack ASC` after it is not decoration: `submitted_at` defaults to `now()`, which
in Postgres is transaction time, so all 500 racks of one batch share one
timestamp exactly. Without a tiebreaker the order within a batch would be
arbitrary and, worse, free to differ between two requests — so page 2 could
repeat or skip rows from page 1. Some total order is required for pagination to
mean anything; recency-then-rack is the one chosen.

But a recency feed is only one of two questions a caller might have, and the
other is *enumerate this job's corpus* — read every rack and its best play, in
some order that lets me get through all of them. The endpoint answers that one
badly and expensively:

- **Recency is the wrong order for a corpus.** For a *completed* opening-rack
  job, every rack has been analysed, so "most recently submitted" is an
  arbitrary (if stable) shuffle of the rack space, correlated with dispatch
  order and jittered by which volunteers finished when. Nobody wants racks in
  that order; they want them in rack order, or they want a specific rack.
- **Only the first page of a feed is a feed.** Recency ordering earns its cost
  on page 0 and stops earning anything by page 500.
- **The two questions want opposite plans.** A feed wants the newest rows and
  can stop early. An enumeration wants every row exactly once and never needs
  a global sort at all.

So the expensive case — deep paging — is precisely the case where the ordering
that makes it expensive delivers no value.

**The problem.** For an opening-rack job the query joins every
`position_analysis_records` row of the job (through `tasks`), left-joins its
rank-1 move, and sorts by `submitted_at DESC` before `LIMIT/OFFSET`. A full
English job is 3.2 M records; a games job with `capture_positions` on is
millions more (PLAN.md's own figure: 9 M positions for `max_games = 400,000`).
No index can serve it, because the record tables carry `task_id` but not
`job_id`, so the job filter sits on the other side of a join from the sort
column — the whole job has to be gathered and sorted before the first 50 rows
can be returned. `OFFSET` makes deep pages worse rather than better: the rows
are produced and then discarded server-side, and `page` is only clamped at zero,
so `?page=1000000` is a legal request. The endpoint is unauthenticated, so no
credential is needed to start one of these.

Two things bound how bad this is today. The frontend only ever calls this
endpoint with `?rack=`, which takes a *different* branch — a single-rack lookup
that goes `job → tasks → (task_id, rack)`, so it is bounded by the job's task
count (thousands) rather than its record count (millions). The listing branch is
reached by following the "paginated JSON" link on the job page, or by anyone
calling the API directly. And `total` is already `-1` on this endpoint, so the
unboundedness of the *count* was already acknowledged; the scan was not.

Scope: opening-rack and games/pairs jobs only. The leave-generation branch reads
`leave_rack_progress`, which is keyed on `(job_id, generation, rack)` and needs
nothing.

**Options.**

- **(a) Denormalise `job_id` onto `position_analysis_records` and
  `game_results`, and index `(job_id, submitted_at DESC)`.** The direct fix:
  the filter and the sort end up in one index, and the plan becomes an ordered
  index scan that stops at `LIMIT`. It is also the only option that helps the
  admin NDJSON stream and the export, which run the same join. Costs: a column
  on the two largest tables in the schema (tens of millions of rows each); a
  third cascade path to `jobs` on tables that already cascade through `tasks`;
  and index maintenance on every insert, on the same path B3/C8 just made
  cheaper. `position_analysis_moves` already carries a denormalised `task_id`
  for exactly this kind of reason, so the pattern is not new here. While there
  is one migration, this is a free edit; after release it is a backfill over
  those tables.
- **(b) Keyset pagination on `(submitted_at, id)`.** Replaces `page`/`offset`
  with an opaque cursor, so page *N* costs what page 1 costs. This is the
  textbook fix for unbounded offsets and it needs no schema change — but on its
  own it does not help, because without (a) the job filter still forces the
  join, and the first page still scans the job. It is the right complement to
  (a), not a substitute. It also changes the public response shape (`page` and
  `per_page` are part of the documented pagination contract in PLAN.md's API
  conventions), and the frontend's `Pagination.svelte` assumes page numbers.
- **(c) Bound the endpoint to a recent window** — the last *N* tasks, or
  results from the last *N* days. Cheapest to implement and it keeps the API
  shape, but it silently changes what the endpoint means: "the job's results"
  becomes "some of the job's results", and a caller paging to the end has no way
  to tell the difference between "that is all of them" and "that is where we cut
  it off".
- **(e) Page by *task*, in the job's own tiling order.** This is the option the
  question above surfaces, and it needs no schema change at all. An
  opening-rack job's tasks each cover a contiguous, disjoint slice of the
  enumerated rack space, with `tasks.seed` holding the slice's start; a games
  job's tasks tile the seed space the same way. `tasks (job_id, seed)` is
  already a unique index. So "the job's results in rack order" is an indexed
  walk of that index, plus `(task_id, rack)` per task — both already exist,
  both bounded, and the result is stable and complete by construction.
  It answers the *enumeration* question properly, and it answers it cheaply.
  What it gives up is the feed: ordered by the job's own tiling, the first page
  is the beginning of the rack space, not the newest arrivals. Keeping both
  would mean saying which question is being asked — a `?order=` parameter, or
  splitting the feed onto its own route.
- **(d) Make the whole route admin-only, as the NDJSON stream already is.**
  Consistent with the reasoning that already moved bulk reads behind admin, and
  a one-line change. Two objections. This is a *paginated* read of at most 500
  rows — the thing the public was explicitly left with when the stream was taken
  away — so it removes the public's only path to a job's results rather than
  fixing the cost of serving them. And it would take the `?rack=` lookup with
  it, since both branches live on the same route: the job detail page's "look up
  a rack" box is the one part of this endpoint the site actually uses, and it is
  not the expensive part.

**Recommendation: decide which question the endpoint answers, then (e) if it is
enumeration and (a) if it has to be both.**

The cheapest honest outcome is to accept that this endpoint is a **feed**, cap
it, and let (e) or the export serve enumeration:

- Keep `submitted_at DESC` and the `?worker=` filter, which is what a feed
  needs.
- Bound `page` so the feed cannot be paged into a scan — a feed has no
  hundredth page. That converts the unbounded cost into a fixed one without a
  schema change, a new column, or an API shape change.
- Serve enumeration by (e), in the job's own tiling order, which is both the
  order a corpus actually wants and the one the existing indexes already
  provide.

If instead both questions must be answerable on one route with one order, **(a)
is the fix**: it is the only option that removes the cost rather than moving it,
it fixes three call sites at once (this read, the admin stream, and the export),
and it is nearly free while `0001_initial.sql` is still edited in place — a
window that closes at the first deployment. **(b)** is worth taking alongside
(a) if the pagination contract is still negotiable pre-release, since it makes
page *N* cost what page 1 costs.

I would not take **(c)**: an endpoint that quietly truncates is worse than a
slow one, because nothing tells the caller which it did. I would not take **(d)**
alone: it answers "who is allowed to pay this" rather than "why does this cost
so much", the query would still be slow for the admin, and it would take the
`?rack=` lookup with it.

**What would change the answer.** Whether anyone actually needs to page through
a finished job's corpus over the API at all. The export exists for exactly that
— built once, reused by every download — so if the answer is no, the feed-plus-
cap outcome is simply correct and (a)'s column on two enormous tables is not
worth buying. That is a product question about who reads
`/api/jobs/:id/results` and why, which the code cannot answer. It is also the
question worth asking *first*, because it is the one that decides between a
three-line change and a migration.

### U2 — the job list and contributor lists still aggregate over whole tables

**What this is.** Three public pages, and the first three things a visitor sees.
`/jobs` is the site's index of what is being worked on — every job with its
type, status, priority and how far along it is. `/users` and `/workers` are the
contributor leaderboards, ranking accounts and anonymous workers by tasks
completed. Volunteers are donating compute for nothing; the leaderboard is the
recognition, which makes it load-bearing rather than decorative.

None of the three is on a worker's path. They are page views, so being slow
costs a visitor's patience rather than a job's throughput — which is why this
was not fixed outright along with the dispatch and submission paths.

**The problem.**

- `GET /api/jobs` runs `COUNT(*)` and `COUNT(*) FILTER (state = 'completed')`
  over `tasks` for every job on the page — up to 500 jobs per request, each
  count linear in that job's task history. PLAN.md measured this page at
  **2,188 ms** before `games_completed` became a counter; that change fixed the
  game totals and left the task counts.
- `GET /api/users` computes a completed-claim count per user as a correlated
  subquery and then orders by it, so it must evaluate the count for *every*
  user before `LIMIT` can apply.
- `GET /api/workers` groups over all of `task_claims` twice — once for the rows,
  once for the total. Measured at 93 ms for 44,000 claims and linear from there.

**What was already done.** `tasks_job_idx` widened from `(job_id)` to
`(job_id, state)`, so both task counts are index-only rather than a heap visit
per task. That is a real improvement and not a fix: the work is still
proportional to the job's history, just with a much smaller constant.

**Options.**

- **(a) Running totals on `jobs`** (`tasks_total`, `tasks_completed`),
  maintained in the claim and submit transactions. Exactly the pattern
  `games_completed` and `racks_analyzed` already use, with the same properties:
  constant-time reads, and a counter that can drift under a partial restore.
  Drift is survivable here for the same reason PLAN.md gives for the existing
  two — nothing decides anything from these numbers, they are a progress display
  — and RUNBOOK §2.3 already documents recomputing the existing counters, so
  this extends a procedure rather than inventing one. Cost: two more writes on
  the claim path, which is the path this audit spent its effort emptying.
- **(b) A short-lived in-process cache** (say 5–10 s) in front of the whole job
  list. No schema change, no writes on the claim path, and the staleness is
  bounded and obvious. Fits a page a human refreshes. But it is per-process
  state — fine today, since `desired_count` is pinned at 1 and rate limits and
  SSE subscribers already work this way, and one more thing to reconsider if
  that ever changes. Does nothing for `/api/users` or `/api/workers` unless
  applied there too.
- **(c) A materialized view refreshed on a sweep.** Moves the whole cost off
  every request path to a timer, and covers all three endpoints with one
  mechanism. Heaviest option: a new object in the schema, a refresh that itself
  scans the tables, and a second source of truth to reason about during a
  restore.
- **(d) Leave it, and revisit on measurement.** The current numbers are a page
  view, not a worker-facing path, and the index change bought real headroom.
  PLAN.md's `SLOW_STATS_THRESHOLD` log line exists precisely to make this
  decision on evidence, and nothing equivalent logs for these three endpoints.

**Recommendation: (d) for now, with (a) as the intended fix — and add the
logging that would trigger it.** PLAN.md took an explicit position ("No
pre-aggregation for dashboard v1, except two measured exceptions"), and both
exceptions were taken *on measurements* of the real query. There is no such
measurement for these at production volume, and the index change moved them
materially. The concrete next step is small and uncontroversial: log these three
endpoints when they cross a threshold, the way `jobstats::compute` already does,
so the third exception gets made on the same evidence the first two did. When it
is time, (a) is the right shape — it matches the established pattern, it has a
documented recovery path, and it is the only option that makes the job list
genuinely constant-time.

**What would change the answer.** A single job past a few hundred thousand
tasks, or a job list page that starts appearing in the slow-query log. Either
makes (a) worth its two extra writes immediately.

### U3 — SPRT reads `game_results` on every submission

**What this is.** A `games` or `game_pairs` job exists to answer one question:
is bot configuration A stronger than configuration B, and by enough to matter?
Rather than playing a fixed number of games and looking at the result, the job
runs a **sequential** test — SPRT, the same procedure chess engine testing uses.
After each result it asks whether the evidence has crossed a significance
boundary, and stops the moment it has.

Stopping early is the point. Volunteer compute is finite and donated, so a
comparison that can be settled in 20,000 games should not play 400,000. A
minimum-games gate stops an early lucky streak from ending the job on noise, and
a maximum-games gate bounds the cost when the answer never becomes clear. Being
sequential is precisely why the test is evaluated on **every** submitted result:
the job cannot stop at the right moment if it only checks occasionally.

**The problem.** The evaluation reads the job's results to compute the test
statistic — one row per task, summed — on every submission of every games or
game-pairs job. Measured at ~50 ms per 400,000 units, and linear in the job's
history, so it grows for the life of the job. It is the largest remaining cost
on the path a worker waits on before it can ask for its next task, and the only
one this audit deliberately left there.

**Options.**

- **(a) Leave it exactly as PLAN.md has it.** The stopping rule reads the rows
  it is a statement about, so it cannot be wrong because a counter drifted.
  PLAN.md's measurement note settles this in as many words: "About 50 ms at
  400,000 units, on every submission, is within budget for the result rate a job
  actually sees."
- **(b) Gate the read behind the existing `jobs.games_completed` counter** — if
  the counter says the job cannot have reached `min_units`, skip the aggregate
  entirely. Cheap, and it only ever *delays* a decision, never brings one
  forward. But a counter that drifts low would stop the job from ever
  evaluating its stopping rule, which is the failure PLAN.md's design rules out
  by construction: "a drifted counter is a wrong number on a page and cannot
  stop a job early" would no longer be true.
- **(c) Maintain the tally and the pentanomial as counters** and drop the
  aggregate. Constant time, and the largest win available on the submit path.
  It also makes a drifted counter able to stop a job early or late — the exact
  thing the design is written against — and SPRT's conclusion is the job's
  entire output.
- **(d) Debounce the finish check** — evaluate it on some submissions rather
  than all of them. No new source of truth: the check still reads the rows, it
  just runs less often, so the only thing traded away is *when* the job notices
  it is finished. Four shapes, below.

**The four debounce shapes.** What is being traded is always the same pair:
how much of the read cost is saved, against how much compute is played past the
boundary before anyone notices. Note what "overshoot" costs — extra tasks
dispatched and completed after the job should have stopped, which is donated
volunteer time spent on a question already answered.

- **(d1) Count-based** — keep a per-job submission counter and evaluate on every
  *k*th. Overshoot is bounded at *k*−1 tasks, full stop, with no dependence on
  anything else. Cost per submission is 1/*k* of the read regardless of how fast
  results arrive, so a busy job — the one where the read hurts most — pays
  proportionally least. The counter can live in memory beside the SSE
  coalescing map; losing it on restart costs one extra check.
- **(d2) Time-based** — keep a per-job "last evaluated at" and evaluate if it is
  older than *T*. Bounds the cost in *server* terms: at most one read per *T*
  per job, whatever the fleet does. But it does not bound overshoot in the unit
  that matters — overshoot becomes *submission rate × T* tasks, so it grows
  with the number of contributors. That is backwards: the more people donating
  compute, the more of it is wasted, and a job at 10 submissions a second with
  *T* = 5 s plays 50 tasks past the boundary where a quiet job plays none.
- **(d3) Hybrid** — evaluate when *k* submissions **or** *T* seconds have passed,
  whichever comes first. Bounds both, which sounds strictly better and is: the
  count bounds the waste, the timer bounds the cost during a burst. It costs a
  second piece of per-job state and a second constant to justify.
- **(d4) Distance-aware** — use the last computed LLR and unit count to estimate
  how far the job is from its boundary, and skip in proportion: check rarely
  when far away, every time when close. The best overshoot-per-read ratio
  available, and the only one that spends reads where they decide something.
  Also the only one that can be *wrong*: an estimate assumes the trend
  continues, and a run that reverses sharply gets checked late precisely when it
  mattered. Needs the most care and the most explaining.

**Recommendation: (a), unchanged — and when it stops being affordable, (d1),
count-based.**

On the main question: this is the one place in the system where being wrong is
expensive and being slow is not. A job that stops early on a drifted counter
publishes a wrong SPRT verdict, and that verdict is the job's whole product.
Debouncing is the right escape hatch because it trades *latency of the decision*
for cost, and the decision has no deadline — where (b) and (c) trade *correctness
of the decision* for cost. `jobs.games_completed` already exists if someone
decides otherwise, which is what makes (b) tempting and worth naming explicitly
as the thing not to do first.

**On count versus time, count wins, and the reason is unit-matching.** The harm
being avoided is wasted volunteer compute, and waste is denominated in *tasks*.
A count-based bound is stated in tasks directly: at most *k*−1, always,
regardless of fleet size, batch size or submission rate. A time-based bound is
stated in seconds and only converts into tasks by multiplying by the submission
rate — which is the one quantity that varies most and that nobody controls. It
therefore lets waste scale with the number of contributors, which is exactly
backwards: the healthier the project gets, the more donated compute it throws
away. Count-based also has the better cost profile, for the same reason in
reverse: amortised at 1/*k* of a read per submission, a fast job pays
proportionally less, while a slow job checks nearly every time and pays nearly
full price — which is fine, because a slow job's read is cheap and its check is
the one most likely to matter.

**How to pick *k*, and why it can be generous.** Overshoot is already non-zero
and always was. When the LLR crosses, the job flips to `completed`, but every
task already claimed across the fleet is still played and still accepted — the
submit path validates the claim, not the job's status. So the floor on wasted
compute is "however many tasks are in flight right now", which is roughly the
number of active contributors. **A debounce of *k* below that number is free**:
it wastes nothing that was not already going to be wasted. That makes *k*
defensible on evidence rather than taste — set it at or below the typical
in-flight task count, and the change costs nothing at all while cutting the read
rate by a factor of *k*. A refinement, if a single constant feels too blunt:
derive it per job as a small fraction of the job's own budget, e.g.
`k = max(1, max_units / (100 × units_per_task))`, capping overshoot at 1% of
what the job was authorised to spend.

**Take (d3) only if a burst turns out to hurt the server**, which per-worker
rate limiting (1/s, burst 5) and a twenty-connection pool already bound from the
other side. **Avoid (d4) for now:** it optimises the axis that is not the
problem, and it introduces the one property this path has been carefully kept
free of — a decision that depends on an estimate rather than on the rows.

**One failure mode either shape shares, worth writing down.** The check is
triggered *by* submissions, so a job whose fleet stops between checks is not
evaluated again until work resumes. That is benign while a job is live — the
next submission carries the check, and it self-heals — but it means a job could
sit `active` past its stopping point if every contributor left at the wrong
moment. Debouncing widens that window from one submission to *k*. If that
matters, the cheap cover is to evaluate unconditionally when a job's last
in-flight claim is released, which is a moment the scheduler already knows
about.

**What would change the answer.** A job reaching several million units, at which
point the read is hundreds of milliseconds on every submission and this stops
being theoretical. The trigger to watch is the same one PLAN.md already set up
for the stats payload: log the check when it crosses a threshold, and let the
number decide.

### U4 — duplicate `worker_bans` rows

**What this is.** A worker is either an account authenticating with an API key
or an anonymous UUID the server mints on that worker's first task. birdtest
deliberately has **no automatic banning** — the reasoning, set out at length in
PLAN.md, is that no statistical test can separate "this worker is broken" from
"these seeds favoured player 2", so automatic flagging would punish honest
contributors while missing a real attacker. What is left is a table an admin
writes to by hand: an identity in `worker_bans` cannot claim or submit.

That makes it the only lever there is against a contributor sending garbage or
abusing the API, and it is pulled by a human who then has to trust it. Ban and
unban have to do exactly what they say.

**The problem.** Nothing stops two `worker_bans` rows naming the same identity.
Enforcement is unaffected — the check is an `EXISTS`, so any row bans — but
`DELETE /api/admin/workers/ban/:id` removes one row by its id, and the identity
stays banned with nothing in the response to say so. An admin who lifts a ban
and watches the worker stay locked out has no signal beyond re-reading the
table, and the natural conclusion — that banning is broken — is wrong in a way
that is hard to check.

**Options.**

- **(a) Partial unique indexes** on `user_id` and on `anon_uuid`, each
  `WHERE ... IS NOT NULL`. One line of migration; a second ban becomes a `409`
  through the existing unique-violation mapping, and unban means what it says.
  Changes an admin-facing status code for a request that succeeds today, and
  forecloses "ban again with a different reason", which is arguably a useful
  thing to be able to do.
- **(b) Delete by identity rather than by row id** —
  `DELETE /api/admin/workers/ban` taking `user_id`/`anon_uuid`, mirroring how
  `POST .../ban` already addresses the target. Makes unban idempotent and
  complete without constraining what may exist, and makes the two halves of the
  API symmetrical. Changes a public admin route's shape and the frontend's
  `/admin/workers` page.
- **(c) Have unban delete every row for the identity it resolves**, keeping the
  `:id` route. Smallest change that fixes the actual symptom, no new constraint,
  no API shape change. Slightly surprising semantics for a route addressed by
  row id.
- **(d) Leave it.** Nobody has hit it; the data is not corrupted, only
  confusing.

**Recommendation: (a).** The duplicate row carries no information — the second
ban's reason is never read by anything, since enforcement is an `EXISTS` — so
the constraint removes a state that only ever misleads, and it does it in the
database rather than in a handler that a future route could forget. "Ban again
with a different reason" survives as unban-then-ban, and the audit log records
both halves (`worker.unbanned`, then `worker.banned` with the new reason), which
is a better history than two rows nobody reads. Note that under (a) a refused
duplicate writes *no* audit row, since `ban_worker` logs inside the transaction
the insert would abort — acceptable, because the `409` tells the caller the ban
is already in place, but worth knowing. (c) is the reasonable fallback if
someone wants no new constraint; (b) is the tidiest API but costs a change to
`api.unbanWorker` and the `/admin/workers` page for a cosmetic gain.

**What would change the answer.** If ban *reasons* ever need to accumulate per
identity — a history rather than a flag — then (b) or (c) are right and (a) is
wrong, because the duplicates stop being noise.

### U5 — registration's taken-email check is outside its transaction

**What this is.** An account exists for one reason in v1: to hold an API key, so
a volunteer's contributions are credited to them rather than to an anonymous
UUID. Anyone can contribute without one.

Registration is careful about a single property — it must not reveal whether an
email address already has an account. Login and password reset both go out of
their way not to reveal it (one identical "incorrect username or password" for
both a wrong password and an unknown user; always `200` for a reset request,
registered or not), and registration answering "that address is taken" would
undo all of it, turning a public endpoint into an oracle for "does this person
have an account here?". So a taken address receives byte-for-byte what a new
registration receives, the notice goes to the address's owner instead of the
caller, and the password is hashed *before* that branch so both paths pay the
same Argon2 cost and the response time does not give the answer away either.
Several deliberate decisions are stacked up to protect this one property.

**The problem.** `register` evaluates `EXISTS` for the username and the email,
then inserts in a separate transaction. Two concurrent registrations for the
same address both pass the check; one insert wins and the other hits the unique
index, which the error mapping renders as `409 conflict`. So a caller who can
arrange that race gets a different answer for a taken address than for a free
one — a narrow account-enumeration oracle in exactly the place all that care was
taken to close one.

**Options.**

- **(a) Catch the unique violation and replay the taken-email path** — send the
  notice to the address's owner and return the same `201` body. Restores the
  invariant exactly, with the distinction that the caller's timing now includes
  a failed insert. Needs care to tell a username collision (which *should* be a
  `409`, by design) from an email collision, which means reading the constraint
  name from the error rather than just its SQLSTATE.
- **(b) Do the check and the insert in one transaction** with the row locked, or
  as an `INSERT ... ON CONFLICT DO NOTHING` whose zero-row result drives the
  taken-email path. Structurally cleaner than catching an error, and it removes
  the read entirely. Reshapes a handler that is currently written to be read
  top-to-bottom in the order of its reasoning, which is most of why it is easy
  to audit.
- **(c) Leave it.** The window is a few milliseconds wide and requires the
  attacker to be racing a *real* registration of the address they are probing —
  which means already knowing the address is being registered, a strictly
  stronger position than the one the oracle would grant.

**Recommendation: (c), with (b) if this handler is touched for any other
reason.** The attacker model that makes this exploitable already assumes the
answer, so closing it buys close to nothing — and the handler's current shape,
where each branch sits next to the comment explaining why it exists, is worth
more to future audits than the few milliseconds it leaves open. (b) is the
better of the two fixes if it is being rewritten anyway: `ON CONFLICT DO
NOTHING` makes the race structurally impossible rather than caught, which is
always preferable. I would avoid (a): distinguishing collisions by constraint
name puts a security-relevant decision on a string the database chooses.

**What would change the answer.** Any evidence of registration being probed in
the wild, or a change that widens the window — an expensive validation step
added between the check and the insert, for instance.

### U6 — the alternative fix for B1: relax `MOVE_RECORD_BEST` in MAGPIE

**What this is.** An opening-rack job analyses openings exhaustively: for each
of the 3,199,724 distinct 7-tile racks drawable from the English bag, a worker
generates the legal plays, ranks them, and reports the best few. The stored
ranking is the corpus the job exists to build — "what should you play with this
opening rack, and what were the alternatives".

What engine settings a worker uses come from a *player config*: a frozen set of
MAGPIE command-line flags, stored in birdtest and pinned by every job that
references it, so that results from different contributors are comparable. Two
of those settings matter here. `recorder_type` decides what move generation
keeps — `best` retains only the single top-ranked play and throws the rest away
(fast, and exactly right for autoplay, where only the move actually played
matters), while `all` and `equity` retain candidates. `num_plays_recorded`
separately says how many ranked plays to store per rack.

**The problem.** The two settings can contradict each other, and nothing said
so: with `recorder_type = 'best'`, a job asking for ten ranked plays per rack
gets one, because the other nine were discarded before anything could rank them.
Finding B1 covers the defect and how it was caught; this entry is about the
choice of remedy.

It shipped as a birdtest-side refusal — an opening-rack job may not pair
`recorder_type = 'best'` with `num_plays_recorded > 1`. The other remedy is on
the MAGPIE side: have the opening-rack executor record candidates regardless of
the player's recorder type, on the grounds that an opening-rack analysis *is* a
ranked list and the recorder is an implementation detail the job should not have
to know about. This mirrors the note in PLAN.md's capture section about relaxing
the same override for static players, which is listed there as "the one phase
not yet done".

**Options.**

- **(a) The validation that shipped.** Refuses the contradictory configuration
  at the point it is introduced, names the remedy in the error, and never
  changes what a running job computes. The admin has to understand that
  `recorder_type` matters for opening racks, which is one more thing to know —
  but it is a thing that is true, and the error says it.
- **(b) Override the recorder in MAGPIE's opening-rack executor**, forcing a
  candidate-keeping type when `num_plays_recorded > 1`. The job does what the
  admin asked instead of being refused, and every existing config keeps working.
  Against it: birdtest's `player_configs` row is meant to be the exhaustive
  record of what a job asked MAGPIE for — the migration says so in as many words
  — and this makes it lie in the other direction, with the stored config saying
  `best` and the worker doing something else. It also has to be conditioned
  carefully so a deliberate `best` + `num_plays_recorded = 1` job (the "best
  opening play for every rack" case) is left alone, and move recording is on the
  hot path: PLAN.md calls relaxing this override "a real slowdown on the job's
  primary purpose" in the capture case, and the end-to-end measurement here
  agrees — the same job went from 5 s to 342 s once it actually ranked.
- **(c) Both: validate in birdtest *and* have the UI steer the choice.** Refuse
  the contradiction, and have `/admin/player-configs/new` and
  `/admin/jobs/new` explain which recorder an opening-rack job wants, so the
  refusal is something an admin rarely meets.
- **(d) Widen it to a general rule**: derive `recorder_type` from the job type
  rather than storing it per player at all, since `best` is right for autoplay
  and wrong for opening-rack analysis. The cleanest model, and much the largest
  change — it moves a column off `player_configs`, which is immutable and
  referenced by existing jobs.

**Recommendation: (c) — keep the validation, add the guidance.** The validation
is the half that must exist either way: whatever the client does, birdtest
should not be able to store a job config that contradicts itself, and (a) is the
only option that holds without trusting the worker to compensate. (b) is
genuinely attractive for the admin experience, but it breaks the property that
makes this system auditable at all — that the stored config is what ran — and it
would have hidden B1 rather than surfacing it. The UI half of (c) is cheap and
turns a refusal into a choice made correctly the first time. (d) is the right
long-term model and the wrong thing to do in an audit: it changes an immutable
table that live jobs reference.

**What would change the answer.** If contributors ever run opening-rack jobs
configured by people who do not know MAGPIE's flags — a "submit a rack space to
analyse" feature, say — then (b) or (d) become the right answer, because at that
point the recorder genuinely is an implementation detail and asking the user
about it is the bug.

---

## 9. Tests added

| Test | What it pins |
|---|---|
| `worker_api::a_banned_identity_is_refused_however_it_authenticates` | The ban check, now inside the identity-resolution statement, still refuses both an API key and an anonymous UUID, on claim and on heartbeat |
| `worker_api::a_batched_opening_rack_submission_keeps_each_racks_own_moves` | Batched inserts still attach each rack's own moves at the right rank — the failure mode of getting `RETURNING` order wrong is silent and plausible-looking |
| `worker_api::redundant_captured_positions_are_recorded_once` | The conflict-ignoring path matches its returned *subset* back correctly: two claims of one task produce two `game_results` rows but two position records and two moves, not four |
| `worker_api::an_assignment_names_every_file_the_task_loads_and_no_others` | The single-query `expected_data` returns the same union: deduplicated across two players sharing a config, and no `winpct` entry for a static player |
| `admin_api::an_opening_rack_job_cannot_rank_moves_with_a_best_recorder` | B1's rule, and both configurations that stay legal |
| `jobs::opening_rack::tests::*` (3) | `num_moves` kept when larger than the reported list, defaulted when absent, refused when smaller |
| `sse::tests::pushes_coalesce_into_one_in_flight_and_one_pending` | The coalescing contract: one owner, one pending, back to idle |
| `leave_gen::the_transition_owner_is_committed_before_the_transition_runs` (updated) | Ownership commits with the claim (unchanged), and ownership comes back *after* the now-detached transition fails (new) |

`scripts/e2e_magpie.py` also gained a real assertion where it had a vacuous one:
the simming opening-rack job must rank more than one move per rack.
