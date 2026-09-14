# birdtest audit — findings, version 1

Branch: `audit/birdtest-2026-09-14-pass2`, off `main` at `ba139de`.
MAGPIE changes: `birdtest-contribute` only, commit `2a68705a` on top of
`cb390035`. **Committed locally, not pushed** (see U2).
Date: 2026-09-14.

**This is the fifth audit.** The four before it all recorded their findings in a
single unnumbered `AUDIT_FINDINGS.md`, each replacing the last (branches
`audit/birdtest-2026-09-11`, `audit/birdtest-2026-09-13`,
`audit/birdtest-2026-09-13-pass2` and `audit/birdtest-2026-09-14`, all merged into
`main`; the file's history is in `git log -- AUDIT_FINDINGS.md`). No numbered
findings file existed anywhere in the history, so this record is
`AUDIT_FINDINGS_1.md`. The unnumbered file on `main` is left untouched as the
record of the fourth audit, and is called "the prior record" below. Where an
entry revisits one of its items it names it (for example "prior U2").

This file is the authoritative record of every code-versus-`PLAN.md` decision
made in this audit, and of the bugs, races, MAGPIE argument gaps, critical-path
analysis and performance findings behind them.

**Counts: 8 code-wins (PLAN.md updated to match the code), 8 plan-wins (code
changed), and 6 items left for human input (section 9).**

The default bias is that the code wins and `PLAN.md` is brought level with it.
The code was changed only where it was wrong, or where the plan described the
behaviour the rest of the system needs.

---

## 0. What the prior audits left open

The prior record's section 9a lists nine decisions (U1–U9), all marked
implemented. Each was checked against the code rather than taken on trust:

| Prior item | Still true? | How it was checked |
|---|---|---|
| U1 no audit rows for claims or submissions | Yes | `issue_claim` and `submit_result` write none; `admin_api` census test asserts it |
| U2 capture jobs refuse simmers capture would raise | Yes, and its premise holds | `validate_capture_play_cap` present; MAGPIE's raise (`autoplay.c`, `position_play_cap`) is applied only when `captures_positions` |
| U3 simmers: iteration budget, `time_limit_secs` 0 | Yes | `create_player_config` |
| U4 leave generation at redundancy 1 | Yes | `validate_job_body` |
| U5 `task_failed` decline | Yes, both sides | `decline_task`; MAGPIE `decline_failed_task` |
| U6, U7 wait for evidence / no retention | Nothing to build | — |
| U8 push `birdtest-contribute` | Yes | `origin/birdtest-contribute` was `cb390035` when this audit started, equal to the local branch |
| U9 generation 1 seeded lazily | Yes | `create_job` and `purge_job` write no universe; but PLAN.md still said otherwise in three places (K6–K8) |

Nothing was left pending. **One prior finding is revisited and reversed:** the
prior record's MAGPIE argument table classed the rack info table (`-rit`) as "No:
exact precomputed leave values". It is not exact with respect to the leaves a job
pins; see M2.

---

## 1. How this audit was run

- Read `PLAN.md` in full (the schema block diffed mechanically against
  `backend/migrations/0001_initial.sql`: identical), the prior record in full,
  then the scheduler, worker routes, registry, every job type, plausibility,
  jobstats, SSE, admin routes, public routes, ratings, exports, auth, config,
  `main.rs`, the test harness, CI, Docker, compose, `infra/ecs.tf`, the frontend
  player-config form and the Python fake worker.
- On MAGPIE's `birdtest-contribute`: `src/impl/contribute.c` in full; in
  `src/impl/config.c` every contribute executor, both resets, the lexical loader,
  `config_load_lexicon_dependent_data`, `config_fill_game_args`,
  `config_fill_sim_args` and `config_fill_autoplay_args`; `players_data.c` and
  `player.c` for how wordmaps and rack info tables reach a player; the rack info
  table's layout, builder and its use in `move_gen.c`; `autoplay.c` for game pairs
  and the capture cap; `sim_results.c` for display sorting.
- Backend: `cargo clippy --locked --all-targets -- -D warnings` and
  `cargo test --locked` against Postgres 16 (the compose `postgres` service).
  **141 tests before, 147 after** (88 unit and contract, 59 integration), all
  passing, clippy clean.
  `svelte-check`: 0 errors, 0 warnings.
- MAGPIE: `make magpie_test` (`-Werror`, address/undefined/leak sanitizers), then
  `./bin/magpie_test contribute` and `./bin/magpie_test config`, both passing.
  clang-format reports nothing on the lines this audit wrote (the file's other
  warnings predate it).
- End to end: `scripts/e2e_magpie.py` against an isolated compose project
  (`birdtest-e2e`, its own volumes and ports, backend image built from this
  branch) with the release `magpie` from `2a68705a` reporting `0.3.0`. The
  seeding went through the real API, including a MAGPIE-DATA `data-20251004`
  import. **Every job type passed:** games, game pairs, opening racks static and
  simming (2 s of MAGPIE each, 2 accepted claims each), and leave generation
  (120 s including the lazy universe seeding, 2 accepted claims). The backend log
  held no error, warning or deadlock line. The stack was torn down afterwards.

---

## 2. MAGPIE arguments that change a task's outcome

The prior record's enumeration (its section 1) was re-derived from the arg
builders rather than reused. Every field `config_fill_game_args`,
`config_fill_sim_args` and `config_fill_autoplay_args` read was traced to where
the contribute path sets or resets it. The prior table holds, with two
corrections (M1, M2) and the additions below.

| Setting | Changes results? | How it is set now | Change |
|---|---|---|---|
| Wordmap use (`-w1/-w2`, `-wmp`) | Only if the `.wmp` does not match the `.kwg` — which is exactly the case a leftover flag reaches | Stated per player **before** the lexicon loads | **M1** |
| Rack info table use (`-rit*`) | **Yes**: entries carry leave values used instead of the KLV's | Always off in contribute; refused by birdtest | **M2** (reverses the prior table) |
| `use_game_pairs` | Not for leave generation: `autoplay_worker` builds a second game runner only for `AUTOPLAY_TYPE_DEFAULT`, and the recorder check passes for `games` | Set by the games executor; left for leave generation | Checked, no change |
| `max_num_display_plays`, `shplies` | Not for stored results: `sim_results_lock_and_sort_display_simmed_plays` sorts every simmed play; the opening-rack writer caps from the request. `position_play_cap` raises simmers only when capturing (prior U2) | Set by the games executor from player 1 | Checked, no change |
| `print_boards`, `game_string_options`, `human_readable` | No: output only | — | — |
| Overtime penalty and period | Only with the play chooser, which is reset off | — | — |
| `leavegen` `games_before_force_draw_start` | Yes | Constant `0` in the executor, the same on every build | Checked |
| Every setting whose null means "MAGPIE's default" | Yes, if a release changes the default | Compile-time default of the worker's build | **Flagged, U3** |

### M1 — a task's wordmap and rack-info-table flags applied to the *next* task

*Plan wins: **code changed** (MAGPIE).*

- **How MAGPIE decides.** `config_load_lexicon_dependent_data` loads a player's
  wordmap and rack info table as it loads the lexicon, naming the file only if
  that player's `use_when_available` flag is set *at that moment*, and freeing
  it otherwise. `player_update` then copies whatever pointer is left
  (`players_data_get_wmp`, `players_data_get_rack_info_table`); nothing later
  consults the flag.
- **What the code did.** All three executors called
  `config_contribute_load_lexicon_and_variant` first and set the flags afterwards
  (`config_contribute_apply_wmp_rit`, called from
  `config_contribute_apply_player_settings`, and a loop in the leave-generation
  executor). Each task's flags therefore governed the next task's load.
- **Why it matters.**
  1. A task that did not ask for a wordmap, claimed after one that did, loaded
     the `.wmp` for *its own* lexicon. `config_contribute_ensure_wordmap` — the
     sidecar check that rebuilds a wordmap built from a different `.kwg` — runs
     only for tasks that ask, so a stale wordmap could generate a different move
     set with nothing to flag it. Whether it happens depends on the order a
     worker happened to claim tasks in.
  2. A rack info table switched on by the contributor's `settings.txt` stayed on
     for every task (see M2).
  3. A task that did ask for a wordmap ran without one if the previous task had
     not asked: correct, but it forfeited the speedup the job asked for.
- **What PLAN.md said.** "A job that omits it runs without a wordmap … a wordmap
  already sitting in `./data` from an earlier job is not switched on by its mere
  presence."
- **Fix.** `config_contribute_load_lexicon_and_variant` takes each player's
  wordmap flag, sets both players' wordmap flags and switches the rack info table
  off **before** it loads. `config_contribute_apply_wmp_rit` and the leave-gen
  loop are gone. Exposed in `config.h` for testing.
- **Test.** `test_lexical_flags_are_set_before_the_load`: a config loaded with
  `-wmp true` and both rack-info-table flags forced on is loaded for a task that
  asks for neither — no wordmap and no table; then for a task asking player 1 for
  one — player 1 has it, player 2 does not.

### M2 — a rack info table replaces the leaves a job pins

*Plan wins: **code changed** (both repositories). Reverses a row of the prior record's table.*

- **What a table is.** `RackInfoTableEntry` stores, per full rack, the leave value
  of every subset (`leaves_packed`) and the best leave per size. `move_gen.c`
  unpacks those into the leave map in place of KLV lookups whenever the player has
  a full rack (`rack_info_table_entry_unpack_leaves`). A table is built by
  `convert klvwmp2rit` from one KLV, is **named after the lexicon**, records nothing
  about which KLV it came from, and no `expected_data` digest covers it.
- **Consequences.**
  - Leave generation plays with a KLV fetched for its generation
    (`<lexicon>_birdtest_previous`). A table built from the lexicon's shipped
    leaves would replace the very values being generated, and bad generation-N
    values propagate into every later generation.
  - Any player whose `leaves` are not the KLV the table was built from — or a
    contributor whose table is older than their `.klv2` — ranks moves on the wrong
    values.
  - Combined with M1, the flag could be on for a job that never asked.
- **What the code did.** birdtest accepted `use_rit` on player configs and sent it;
  MAGPIE applied it (late, M1). The prior record called the table "exact".
- **What PLAN.md said.** The executor table mapped `use_rit` to `-ritN` with no
  caveat.
- **Fix.** MAGPIE: rack info tables are always off in contribute (inside the
  loader, M1). birdtest: `validate_player_config_body` refuses `use_rit = true`,
  naming why; the form no longer offers it. The wire field stays, so no contract
  change.
- **Why refuse rather than verify.** Verifying a table would need either pinning
  `.rit` files as `input_data` or a sidecar recording the KLV digest, and a way to
  build tables for fetched KLVs. That is a feature; refusing removes the
  corruption now. Recorded as a possible future improvement, not a decision.
- **Test.** `admin_api::a_player_config_cannot_ask_for_a_rack_info_table`, and the
  MAGPIE test under M1.

### M3 — version

*Code changed (MAGPIE).* `MAGPIE_VERSION` is `0.3.0`, so a floor can tell builds
with M1 and M2 fixed from builds without. **birdtest's default floor stays
`0.2.0`**: raising it before `birdtest-contribute` is pushed would make CI's
MAGPIE jobs and every existing contributor get `magpie_too_old`. A job's own
`min_magpie_version` can be set to `0.3.0` today. See U2.

---

## 3. Bugs

### B1 — games and game-pairs jobs dispatched past their hard cap

*Code changed; PLAN.md made explicit (K3).*

- **What the code did.** `game::next_request` and `game_pair::next_request`
  generated the next batch from `MAX(seed)` with no upper bound. A job stopped
  only when a finish check saw `units_completed >= max_units`. That check is
  debounced (every eighth submission, or when nothing is in flight), so every
  worker asking in between got a new batch past the cap.
- **Why it matters.** Nothing those batches play can change the verdict:
  `terminated_at_max` is decided by units the cap already covers. With W workers
  on a job that reaches its cap, up to roughly W further batches are generated
  and played — minutes of donated compute each — and then their results are
  stored against a finished job.
- **What PLAN.md said.** "the job auto-completes when `max_games` (or
  `max_pairs`) is reached" — silent on dispatch.
- **Fix.** `game::past_the_cap(next_seed, max_units)`: seeds start at 1 and tile
  the space a batch at a time, so `next_seed - 1` units are out; once that reaches
  the cap, no new task is generated (`Acquired::NoWork`). Tasks whose claims lapse
  are still re-dispatched. Opening racks already stopped at `total_racks`.
- **Trade-off, recorded as U6.** A task no worker can ever complete (a seed that
  crashes MAGPIE every time) now blocks completion, where before, generation past
  it eventually pushed the job over the cap. Opening-rack jobs already behave this
  way.
- **Tests.** `worker_api::sprt_jobs_hand_out_nothing_past_their_cap` (games at a
  cap of 3 with batch 2; pairs at a cap of 1), and the unit test
  `game::tests::dispatch_stops_once_every_unit_up_to_the_cap_is_out`.

---

## 4. Race conditions

### R1 — concurrent leave-generation submissions deadlocked

*Code changed; PLAN.md updated (K9).*

- **What the code did.** `fold_into_generation` added a submission's occurrences
  with one `UPDATE leave_rack_progress … FROM UNNEST(...)`. Submissions for
  different tasks of one generation hold different claim and task rows and take no
  job-row lock (leave generation does not maintain `jobs` counters), so nothing
  serialized them — and they overlap on every commonly drawn rack, whatever each
  task forced. An `UPDATE` locks rows in the order its plan visits them, which
  follows the submitted list for a nested loop over `UNNEST`.
- **Race.** Submission A holds rack X and needs Y; submission B holds Y and needs
  X. Postgres detects the cycle after `deadlock_timeout` (1 s) and fails one
  transaction: that worker's result is a 500, MAGPIE retries with backoff, and the
  whole fold runs again. The more workers on a leave job, the more often.
- **Fix.** Before the update, `SELECT 1 … WHERE rack = ANY($3) ORDER BY rack FOR
  UPDATE`. Postgres locks the rows as they leave the sort, so every submission
  takes them in rack order: the second waits holding nothing, and the update
  itself acquires no new locks.
- **Cost.** One extra indexed statement per leave submission. Overlapping
  submissions now wait for each other instead of one failing (see P4).
- **How likely.** Plan-dependent. On the small test universe Postgres plans the
  unordered `UPDATE` from the primary key `(job_id, generation, rack)`, which
  happens to lock in rack order; a heap scan or a loop over the submitted list —
  both valid plans, chosen by table statistics — locks in another order, and then
  every overlapping pair of submissions is a candidate cycle. Nothing in the old
  code prevented that plan.
- **Test.** `leave_gen::overlapping_leave_submissions_wait_instead_of_deadlocking`:
  the lower rack's row is moved to a later heap position (asserted), B's
  transaction reads without indexes (`SET LOCAL enable_indexscan/bitmapscan =
  off`, the plan the deadlock needs), A folds the lower rack and stays open, B
  folds both listing the higher first, and once B is blocked A touches the higher
  rack. Both commit and both racks read 2. **Checked against the old code** (the
  ordered lock removed): Postgres reports `deadlock detected` (40P01) and the test
  fails. An earlier version of the test, without the heap move and the planner
  settings, passed against the old code — the index plan hid the race — and was
  corrected before this record was written.

### R2 — a claim could hand out a task of a job completed or deactivated underneath it

*Plan wins on intent: **code changed** (K10).*

- **What the code did.** `candidate_jobs` selects active jobs with no lock. The
  claim then takes the dispatch lock, generates or reissues a task, and last runs
  `UPDATE jobs SET claims_issued = …` — which waits on the job's row if an admin
  (`complete_job`, `deactivate_job`) or the finish check is mid-update, and then
  updated it regardless.
- **Race.** The job is marked completed (or inactive) and committed while a claim
  that already selected it is between selection and that update. The claim
  commits a task of a completed job.
- **Why it matters.** Wasted work on a finished or switched-off job; and it
  reopened the export guarantee prior B3 established — an export checks that no
  claim of a completed job is open, which is sound only if no claim can appear
  afterwards.
- **What PLAN.md said.** Inactive jobs are ones "workers are not assigned tasks
  from"; a completed job's results are "immutable".
- **Fix.** The update is `WHERE id = $1 AND status = 'active'`. Under Read
  Committed, Postgres re-evaluates the condition against the row version it waited
  for, so a claim that loses the race updates nothing; `issue_claim` returns
  `None` and the claim rolls back, task and all.
- **Test.** `worker_api::a_claim_racing_a_jobs_completion_hands_nothing_out`:
  the job is completed in an open transaction, a claim runs until it blocks on the
  job row, the completion commits; the claim answers `204` and no task or claim
  exists. **Checked against the old code** (guard removed): the test fails, the
  claim answered `200` with a task.

### R3 — a finish check overtaken by a purge completed the purged job, for good

*Plan wins on intent: **code changed** (K11).*

- **What the code did.** `after_submission` loaded the job, read its results
  (`game_stats`, tens of milliseconds and more as a job grows), then ran
  `UPDATE jobs SET status = 'completed' WHERE id = $1 AND status = 'active'`.
  `purge_job` deletes results and zeroes counters but leaves `status` active.
- **Race.** A purge commits between the read and the update. The job the admin
  just restarted is marked completed, and `activate_job` refuses a completed job,
  so the purge's intent cannot be recovered through the API.
- **Why not a lock.** Holding the job row or the dispatch lock across the results
  read would stall that job's submissions or claims for the length of an
  aggregate over its history, every eighth submission.
- **Fix.** `jobs::complete_unless_purged(pool, job_id, observed_claims_issued)`
  adds `AND claims_issued >= $2`, with `claims_issued` read by the `load_job` that
  precedes the results read. It only ever grows, except that a purge zeroes it.
- **Test.** `admin_api::a_finish_check_overtaken_by_a_purge_does_not_complete_the_job`.

### R4 — the same race on leave generation's final generation

*Code changed (K12).*

- **What the code did.** On `Acquired::JobFinished` the claim transaction was
  rolled back — releasing the dispatch lock — and the job then marked completed
  in a separate statement. A purge (which takes that lock) could land in between
  with the same permanent result as R3.
- **Fix.** The update runs inside the claim transaction, under the dispatch lock,
  and commits. Lock order is dispatch lock then job row, the order purge uses.
- **Test.** Structural; exercised by the existing leave-generation tests. A
  deterministic test would need a hook between two statements that no longer
  exists.

### R5 — checked and found sound

- **Claim, reissue, reclaim, decline and submit** on one claim or task: each
  re-checks `state = 'claimed'` under the claim lock; lock order is claim, task,
  job everywhere (re-verified, including purge and delete after prior R1).
- **Two reclamation statements at once** (`reclaim_expired_for` from two claim
  requests): each locks expired claims as it finds them and re-checks the state
  after waiting. With identical plans they lock in the same order; with different
  tiers a deadlock is possible in principle, and it is logged and non-fatal (the
  claim goes on without reclaiming). Not changed.
- **Leave-generation claims** vs submissions vs transitions: all claim decisions
  are under the dispatch lock; a generation closes only with no claim open, so no
  fold reaches a closed generation outside a restore (the guard remains).
- **Opening-rack submissions of one task:** serialized on the task row; redundant
  copies of in-game positions de-duplicated under that lock.
- **Activation budget:** tier advisory lock after the job row; no cycle with
  claims (which never take the tier lock).
- **Export start:** sound once R2 holds.
- **SSE push loop:** coalescing state is under a mutex; pushes stay ordered.

---

## 5. Critical path

The critical path is a worker getting its next task and getting its result
accepted. It was traced statement by statement.

**Claim** (`POST /api/worker/task`): identity lookup with throttled touch and ban
check (one statement); `candidate_jobs`; one reclaim statement for the tier; per
candidate: dispatch lock, reissue or generate, `anonymous_workers` insert (new
workers only), claim insert, task counter update, guarded job update (R2),
`expected_data`, commit. Every statement decides or records the dispatch.

**Submit** (`POST /api/worker/result`): identity; claim `FOR UPDATE`; task `FOR
UPDATE` (`accepted_count`); job read; validation and plausibility; batch checks
against the request; record insert; claim, task and job counters; identity
counter; commit. Then inline: `load_job`, and the debounced finish check. Then
spawned: the SSE payload.

**What moved in this audit: nothing.** The observational work the prior audits
found on these paths is already off them (audit rows removed, SSE spawned and
coalesced, rating fits on a sweep). What remains was examined and kept:

| Kept on the path | Why |
|---|---|
| Finish check (SPRT / opening-rack exhaustion), debounced | Gates whether the job keeps dispatching; the brief says not to defer SPRT that gates completion |
| `users`/`anonymous_workers.tasks_completed` in the submit transaction | Observational, but a decided design (prior U2 of the second audit): moving it trades counter accuracy for a single-row update |
| `expected_data` per claim | The worker needs it to verify data |
| Re-expanding an opening-rack range at submit | Needed for the exact-racks check |
| R1's ordered lock | Correctness |

This audit's changes shorten the path in effect rather than in statements: B1
stops generating work nobody needs, and R1 turns a 1-second deadlock plus a failed
request and a retried fold into an ordinary wait.

---

## 6. Performance — most severe first

1. **SPRT jobs generated work past their cap.** *Fixed (B1).* A job finishing on
   `max_games`/`max_pairs` handed out roughly one extra batch per worker asking
   before the debounced check ran. Impact: on a 200-worker fleet with batches of
   10 games, on the order of 2,000 wasted games per job end, each batch minutes of
   contributor CPU.
2. **Leave-generation submissions deadlocked under load.** *Fixed (R1).* Each
   occurrence cost `deadlock_timeout` (1 s), a failed request, MAGPIE's retry
   backoff (1 s, 2 s, 4 s …) and a full re-fold of thousands of rows. Frequency grows
   with the number of workers on one leave job, since every pair of concurrent
   submissions overlaps.
3. **A public rating-pool page rebuilds the evidence matrix on every view.**
   *Flagged, U1.* `GET /api/rating-pools/:id` runs `evidence_matrix` for residuals:
   452 ms at 600,000 paired results (PLAN.md's measured table), on one of twenty
   pool connections, unauthenticated and not rate limited. Twenty concurrent
   viewers — or one client looping — hold the whole pool, and claims and
   submissions stall behind them.
4. **Leave-generation throughput per job is serialized twice.** *Flagged, U4.*
   Claims serialize on the dispatch lock around `next_step` (47 ms measured), a
   ceiling of roughly 20 claims a second per leave job. Submissions now serialize
   on overlapping progress rows (R1) for the length of each fold. Neither is a
   regression — claims already serialized, and submissions already contended —
   but a large fleet on one leave job will see it.
5. **`rebuild-artifacts` runs every generation inline in the admin request.**
   *Flagged, U5.* About 13 s per generation in a release build; past roughly 20
   generations the request outlasts the ALB's 300 s idle timeout, axum drops the
   future, and the rebuild stops part-way (each upload is idempotent, so a re-run
   repairs it).
6. **Unchanged from the prior record:** display reads that grow with a job's
   history (prior U6, decided: wait for the slow-stats log line) and unbounded
   `audit_log`/`rating_runs` (prior U7, decided: no retention yet).

---

## 7. PLAN.md reconciliation

"Code wins" means PLAN.md was updated to match the code. "Plan wins" means the
code was changed (and PLAN.md updated wherever its wording also needed it).

| # | Subject | Code | PLAN.md said | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | Deduplicating non-seed tasks | Opening racks use `seed` as range start; leave tasks have a null seed and no content dedupe | "deduplicated by their request content via the typed request tables" | **Code wins** | No such dedupe exists or is needed: opening racks tile by index, leave claims never share forced racks |
| K2 | SPRT evaluation cadence | Debounced | "evaluated on every submitted result" (Statistical Result Evaluation, first list) | **Code wins** | Stale since the debounce; the same section already describes it two paragraphs later |
| K3 | Dispatch at the hard cap | Generated past it | Silent; "auto-completes when `max_games` is reached" | **Plan wins on intent, code changed** | B1. Work past the cap cannot affect the verdict |
| K4 | SSE cadence (Dashboard intro) | Coalesced, ≤ 1/s | "pushes a new event whenever a task result is accepted" | **Code wins** | Stale; Live updates section already correct |
| K5 | Stats payload | `opening_racks { racks_analyzed, racks_total }`; `other_workers` | `average_best_equity`, `best_move_types`; no `other_workers` | **Code wins** | The aggregates were deliberately removed by an earlier audit; `other_workers` exists in code and frontend |
| K6 | Long operations paragraph | Creation writes no universe (prior U9) | "Creating an English leave-generation job writes those 3.2 million rows inside the creating request" | **Code wins** | Stale since prior U9 |
| K7 | Transition paragraph | `seed_generation` for every generation from the seeding task | "the same `seed_generation` that writes generation 1 at job creation" | **Code wins** | Stale since prior U9 |
| K8 | Transition ownership paragraph | Purge deletes the universe and does not reseed | "A purge … reseeds generation 1 … copy a freshly zeroed universe into generation 2" | **Code wins** | Stale since prior U9; the "copy" was removed even earlier |
| K9 | Leave fold | Unordered row locks | Silent on locking | **Code changed, PLAN updated** | R1 |
| K10 | Claim against a job no longer active | Issued | Inactive jobs get no work; completed results are immutable | **Plan wins, code changed** | R2 |
| K11 | Finish check vs purge | Could complete a purged job | Purge restarts the job from the start of its space | **Plan wins, code changed** | R3 |
| K12 | Final leave generation vs purge | Completed after releasing the lock | Same | **Plan wins, code changed** | R4 |
| K13 | `use_rit` on the wire | Honoured (late) | "`-ritN`, same" | **Code changed (both repos), PLAN updated** | M2 |
| K14 | Wordmap flag timing | Applied to the next task | "a wordmap already sitting in `./data` … is not switched on by its mere presence" | **Plan wins, code changed (MAGPIE)** | M1 |
| K15 | Player-config validation | Accepted `use_rit = true` | Listed no such rule | **Code changed, PLAN updated** | M2; admin semantics now state the refusal |
| K16 | Worker rate-limit table | Artifact is `GET` | `POST /api/worker/{…,artifact}` | **Code wins** | Typo-level; the route is and always was `GET` |

Counted: K1, K2, K4–K8, K16 are code-wins (8). K3, K9–K15 changed code (8).

**Also updated in PLAN.md with the changes above:** the version section notes
MAGPIE `0.3.0` and why the default floor stays `0.2.0` (M3); the wordmap
provisioning section states when the flag is set (M1).

**Stale comments fixed in code (not PLAN.md discrepancies):** `leave_gen.rs`
(five comments still describing generation 1 as seeded at creation or purge, and
`run_transition` as seeding the next universe), `jobs/mod.rs`
(`DISPATCH_LOCK_WAIT_MS` said seeding runs inside a claim), `routes/worker.rs`
(`DeclineBody` omitted `task_failed`), `infra/ecs.tf` (the ALB timeout's
justification named seeding in the creating request and transitions inside
claims).

**Checked and found in agreement:** the schema block (mechanical diff); version
negotiation and the shutdown decision table; capability filtering before
`MIN(priority)`; redundancy "first accepted result"; exports' settle check;
purge and delete lock order; ban uniqueness; U1–U9 as listed in section 0; the
Python worker's status (section 10).

---

## 8. MAGPIE `birdtest-contribute`

**All MAGPIE changes are on `birdtest-contribute`, in commit `2a68705a`,
on top of `cb390035`; none on `main` or any other branch. Not pushed** (U2).

### Does the branch support birdtest?

Checked directly, not assumed:

| Interface birdtest relies on | Present |
|---|---|
| The six worker endpoints, identity headers, required claim body | Yes (`contribute.c`) |
| Four job types, internally tagged `job_type` | Yes |
| `expected_data` verification through `data_filepaths`, digest cache keyed on inode and ctime | Yes |
| Declines: `missing_data`, `magpie_version`, `unknown_job_type`, `task_failed` | Yes |
| Shutdown handling and `required_*` messages | Yes |
| Seeds as decimal strings (games, leave generation) | Yes |
| Pentanomial, divergent aggregate, positions with `game_index` flattening | Yes |
| Opening-rack play cap and `num_moves` | Yes |
| Leave generation: fetched KLV, in-memory forced racks, `leavegen_max_games`, no files written | Yes |
| Contract fixtures: MAGPIE's three assignment copies identical to birdtest's | Yes (`diff`) |
| Per-player and shared resets, analysis settings copy, per-rack seed | Yes (prior M1–M6) |
| **Wordmap/table flags applied to the task that states them** | **No — fixed (M1)** |
| **No unverified leave values** | **No — fixed (M2)** |

### What changed

| # | What | Where |
|---|---|---|
| M1 | Wordmap flags set, and rack info tables switched off, before the lexical load; `config_contribute_apply_wmp_rit` and the leave-gen post-load loop removed | `config_contribute_load_lexicon_and_variant` (now exported), its three callers |
| M2 | Rack info tables never used in contribute | same function |
| M3 | `MAGPIE_VERSION` `0.3.0` | `config.c` |
| — | Test `test_lexical_flags_are_set_before_the_load` | `test/contribute_test.c` |

---

## 9. Left for human input

### U1 — residuals recomputed on every public pool view

- **(a)** Snapshot residuals with each fit (a `rating_run_residuals` table, or a
  column): page views cost a read; residuals describe the evidence the fit used,
  which is arguably more coherent than today's "stored ratings against live
  evidence". Needs a schema change.
- **(b)** In-process cache keyed by run id, filled on first view, with a per-pool
  single-flight so a burst after a refit computes once. No schema change; the
  first view after each refit still pays; relies on the single instance.
- **(c)** A semaphore on residual computation (like the result streams), answering
  `429` or waiting past the cap. Protects the pool; degrades the page under load.
- **Recommendation: (a),** with (c) as a stopgap if (a) waits.

### U2 — publish `birdtest-contribute` and raise the floor

The MAGPIE commit is local. CI's `magpie-contract` and nightly e2e jobs check the
branch out from GitHub, so until it is pushed they test the build without M1/M2.
Nothing breaks, because the floor stays `0.2.0`. After pushing, raise
`MIN_MAGPIE_VERSION`, the `min_magpie_*` column defaults, the Terraform variable,
compose, both env examples, `scripts/dev.py`, the contract fixtures, the test
harness, README, TESTING.md and PLAN.md to `0.3.0`, as the prior audit did for
`0.2.0`. **Recommendation: push, then raise.** Both are outward-facing actions.

### U3 — "null means MAGPIE's default" pins results to a build, not to the task

Every per-player setting a player config leaves null (stopping condition, BAI
threshold and sampling rule, inference margin, utility weights, play count …) and
every run-wide one no request states (bingo bonus, cutoff, movegen margin,
multi-threading mode) is reset to the worker's **compile-time** default. Two
workers on different MAGPIE releases agree only if no release changed a default.
The floor is a minimum, not a pin, so a job cannot exclude newer builds.

- **(a)** Materialize defaults server-side: player-config creation fills every
  null with MAGPIE's value (as `MAGPIE_DEFAULT_NUM_PLAYS` already does for one
  check), and the request gains fields for the run-wide settings. Every result is
  then a function of the task alone. Wire change on both sides, plus a list of
  defaults to keep in step.
- **(b)** A `max_magpie_version` ceiling per job. Cheap, but it strands jobs when
  contributors upgrade.
- **(c)** Release discipline: a MAGPIE release that changes a default bumps the
  minor version, and admins raise floors deliberately. Documented, not enforced.
- **Recommendation: (a)** before launch, while the wire is still pre-release.

### U4 — leave-generation throughput on one job

Claims serialize under the dispatch lock (≈ 47 ms each), and submissions now wait
on overlapping rows (R1). Options: **(a)** leave it until a leave job's fleet is
large enough to show it; **(b)** fold into a per-task staging table and apply to
`leave_rack_progress` in a periodic batched merge (removes submit contention, adds
a lag the transition must drain first); **(c)** larger `num_iterations` per task
so fewer, bigger submissions. **Recommendation: (a) with (c) as the tuning knob.**

### U5 — `rebuild-artifacts` inline

**(a)** Run it on a spawned task with a status row, like exports. **(b)** Leave it:
it is an admin repair path and idempotent. **Recommendation: (a)** only if a job
passes ~20 generations.

### U6 — a poison task now blocks an SPRT job at its cap (B1's trade-off)

**(a)** Accept: opening-rack jobs already need every task completed, and a task
every worker fails is visible as repeated `task_failed` declines. **(b)** Let a
job past its cap complete when its only unfinished tasks have failed more than N
times. **Recommendation: (a),** revisiting if a job is ever seen stuck on one task.

---

## 10. Python worker

Searched README, RUNBOOK, TESTING, PLAN, compose, `.env.example`, Dockerfile,
Terraform, CI, scripts and the backend for any description of
`worker/fake_worker.py` as a production client. **None found; nothing to correct.**
Its own docstring says "Test tooling only. The one production client is MAGPIE
itself"; the compose service is behind the `fake-worker` profile marked
end-to-end-only; `RUNBOOK.md` forbids it for restore verification; README,
TESTING.md, `scripts/dev.py` and PLAN.md's Worker Client and Development sections
say MAGPIE is the only contributor. The only backend reference is a fixture test
pinning its opening-rack submission shape. This matches prior K19 and the prior
record's section 10.

---

## 11. Tests added

| Test | What it pins |
|---|---|
| `leave_gen::overlapping_leave_submissions_wait_instead_of_deadlocking` | R1 |
| `worker_api::a_claim_racing_a_jobs_completion_hands_nothing_out` | R2 |
| `admin_api::a_finish_check_overtaken_by_a_purge_does_not_complete_the_job` | R3 |
| `worker_api::sprt_jobs_hand_out_nothing_past_their_cap` | B1, games and pairs |
| `game::tests::dispatch_stops_once_every_unit_up_to_the_cap_is_out` | B1 arithmetic |
| `admin_api::a_player_config_cannot_ask_for_a_rack_info_table` | M2 (server) |
| MAGPIE `test_lexical_flags_are_set_before_the_load` | M1, M2 (worker) |
