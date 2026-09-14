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

These are genuine trade-offs. Neither the code nor `PLAN.md` was changed for any
of them.

### U1 — `GET /api/jobs/:id/results` has no bounded plan

**The problem.** For an opening-rack job the query joins every
`position_analysis_records` row of the job (through `tasks`), left-joins its
rank-1 move, and sorts by `submitted_at DESC` before `LIMIT/OFFSET`. A full
English job is 3.2 M records; a capture-on games job is millions more. No index
can serve it, because the records tables carry `task_id` but not `job_id`, so
the job filter lives on the other side of a join from the sort. The endpoint is
public and unauthenticated. `total` is already `-1`, so the *count* is
acknowledged as unbounded; the scan is not.

**Options.** (a) Denormalise `job_id` onto `position_analysis_records` and
`game_results` and index `(job_id, submitted_at)` — the straightforward fix, at
the cost of a column on the two largest tables and a third cascade path.
(b) Switch to keyset pagination on `(submitted_at, id)`, which changes the
public API's shape. (c) Bound the endpoint to recent tasks only, which changes
what it means.

**Why not decided here.** All three change either the schema of the largest
tables or a public API contract, and the right answer depends on whether deep
paging into a finished job's corpus is a use case at all — the export exists for
exactly that. Flagged rather than guessed.

### U2 — the job list and contributor lists still aggregate over whole tables

**The problem.** `GET /api/jobs` runs `COUNT(*)` and
`COUNT(*) FILTER (state = 'completed')` over `tasks` for every job on the page;
`GET /api/users` computes a completed-claim count per user and then sorts by it,
so it reads every user's claims before applying `LIMIT`; `GET /api/workers`
groups over all of `task_claims` twice (rows and total).

**What was done.** `tasks_job_idx` widened to `(job_id, state)`, which makes the
two task counts index-only rather than a heap visit per task. That is a real
improvement and not a fix.

**Options.** Running totals on `jobs` (the pattern `games_completed` and
`racks_analyzed` already use, with the same drift caveat), a materialized view
refreshed on a sweep, or a short-lived in-process cache.

**Why not decided here.** `PLAN.md` took an explicit position — "No
pre-aggregation for dashboard v1, except two measured exceptions" — and adding a
third counter is a decision against that position, which should be made on
measurement of the real volume rather than in an audit.

### U3 — SPRT reads `game_results` on every submission

Measured at ~50 ms per 400,000 units, linear in the job's history, on every
submission of every games job. `PLAN.md` chose this deliberately: "the stopping
rule keeps reading `game_results` rather than a counter, and cannot be wrong
because a counter drifted." A counter would make it constant time, at the cost
that a counter which drifts *low* leaves a job running past its stopping rule
forever. Left exactly as the plan has it. Worth revisiting if a job ever reaches
several million units; `jobs.games_completed` already exists as a conservative
pre-filter if someone decides that trade is acceptable.

### U4 — duplicate `worker_bans` rows

Nothing stops two ban rows for the same identity. `DELETE /api/admin/workers/ban/:id`
removes one, and the identity stays banned with no indication why. A partial
unique index on `user_id` and on `anon_uuid` would make a second ban a `409` and
make unban mean what it says. Not done because it changes an admin-facing status
code for a case nobody has hit, and because "ban again with a different reason"
is arguably a feature. One line of migration whenever someone decides.

### U5 — registration's taken-email check is outside its transaction

`register` checks `EXISTS` for the username and the email, then inserts in a
separate transaction. Two concurrent registrations for the same address leave
one with a `409` rather than the byte-identical body the endpoint goes to
considerable trouble to return, so the race is a narrow account-enumeration
oracle. Closing it means either an upsert-shaped insert or catching the unique
violation and replaying the notification path. Left alone: the window is a few
milliseconds and requires the attacker to already be racing a real registration
of the address they are probing, which is a strictly harder position than the
one the design defends against.

### U6 — the alternative fix for B1: relax `MOVE_RECORD_BEST` in MAGPIE

Instead of birdtest refusing `best` + `num_plays_recorded > 1`, MAGPIE's
opening-rack executor could force a recorder that keeps candidates, on the
grounds that an opening-rack analysis *is* a ranked list and the recorder type
is not something the job should have to think about. That would make the job do
what the admin asked rather than refusing it, and it mirrors the note in
`PLAN.md`'s capture section about relaxing the same override for static players.

Against it: it makes birdtest's job config lie in the other direction (the
config says `best`, the worker does not do `best`), it changes what a
*deliberate* `best` + `num_plays_recorded = 1` job does unless carefully
conditioned, and move recording is on the hot path — `PLAN.md` calls relaxing it
"a real slowdown on the job's primary purpose" in the capture case.

The validation shipped because it never changes what a running job computes; it
only refuses a configuration whose result would be a quiet fraction of what was
asked for. Which of the two is right is a design call.

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
