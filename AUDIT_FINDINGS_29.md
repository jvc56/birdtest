# AUDIT_FINDINGS_29 — the thirty-third audit (2026-10-05), pass 1 (iteration 1)

Branch `audit/birdtest-2026-10-05`, cut from `main` at `0149541` (2026-10-05). This
one branch carries the whole loop run, and every iteration commits to it.

MAGPIE: `~/MAGPIE` is on `birdtest-contribute`. The run started at `dbf8df3b`, which
`docker/Dockerfile` pinned (`ARG MAGPIE_COMMIT=dbf8df3b…`, `MAGPIE_VERSION` 0.1.1 =
`MIN_MAGPIE_VERSION` 0.1.1). This pass's MAGPIE changes are committed on
`birdtest-contribute` as `c3c6a875`, and the Dockerfile pin moves to that commit.

> **NOT PUSHED.** The owner has not asked for pushes, so neither `birdtest-contribute`
> nor the birdtest branch is on a remote. Until `birdtest-contribute` (with
> `c3c6a875`) is pushed to `origin`, three things fail at the step that fetches
> the pin:
> - CI's `magpie-contract` job;
> - the e2e image builds (tier 5) in CI;
> - the nightly job.
>
> All three read `ARG MAGPIE_COMMIT=` from `docker/Dockerfile` and fetch it from
> `origin/birdtest-contribute`. This pass also changes the claim body on both sides
> (C-1). So the new birdtest server refuses every claim from the old pin `dbf8df3b`
> with a 400, and the pin cannot simply be reverted to the pushed commit either.

**Builds on** AUDIT_FINDINGS_7 to _28 and PLAN.md's Known Limits KL-1 to KL-92. This
pass adds KL-93 and KL-94, so the set is now KL-1 to KL-94. A documented KL is not
re-flagged here unless the code under it changed. The KLs this pass revisits are named
in each entry: KL-10, KL-14, KL-32, KL-43, KL-62, KL-75, KL-82 and KL-87.

**Process.** The run follows `audit_loop.md`. Each iteration is a full audit, and the
loop continues until a pass finds no new fixable issue, up to 10 iterations. This file
records iteration 1, which ran in two stages:
1. Eight read-only reviewers:
   - A: races and locking;
   - B: backend bugs;
   - C: MAGPIE arguments and `birdtest-contribute`;
   - D: critical path, performance and storage, with an index-to-query map;
   - E: docs and prompt drift, with two helpers: E3 for TESTING.md's *(Covered: …)*
     notes and E4 for RUNBOOK/README procedures;
   - F: dead and out-of-date code;
   - G: deployment;
   - H: frontend, e2e and the fake worker.
2. Seven fixers, working in one tree:
   - W1: opening-rack consensus and the consensus edit;
   - W2a: claim-slot schema, plausibility and the fake worker;
   - W2b: other backend fixes, dead code and dependencies;
   - W3: MAGPIE and the contract;
   - W4: frontend;
   - W5: infra, deploy and CI;
   - W6: docs.

The coordinator decided the flagged items listed under "Coordinator decisions" below.

**Findings this pass, after de-duplication:** 4 high, 8 medium, about 85 low.
- **High:**
  - D-1: the opening-rack reissue walked every unsettled rack under the dispatch lock.
  - A-2/D-2: the consensus edit held the dispatch lock and every open claim without a
    `DispatchHold`.
  - H-1: a failed word-info-table build could never be retried.
  - H-10: the fake worker reported a simulation for static players, so E-12 and E-13
    proved nothing about real output.
- **Medium:** A-1, B-1, C-1, D-3, E-11, F-1/A-5, H-2 (a false positive, hardened
  anyway) and H-11.

All highs and every medium except B-1 and D-3 are fixed. B-1 and D-3 are design
trade-offs left to the owner (KL-75 revisited, and the new KL-93).

**Loop decision:** pass 1 found and fixed issues, so iteration 2 runs a full audit.

---

## Pass summary

### (a) Branch, prior findings and versions

- **Branch:** `audit/birdtest-2026-10-05`, off `main` `0149541`.
- **Remote audit branches** (`git ls-remote --heads origin 'audit/*'`), 12 in all:
  - `audit/birdtest-2026-09-11`;
  - `-09-13` and `-09-13-pass2`;
  - `-09-14` and `-09-14-pass2`;
  - `-09-15`;
  - `-09-16` and `-09-16-pass2`;
  - `-09-17` and `-09-17-pass2`;
  - `-09-24-pass21`;
  - `-09-25-2`.
- **Highest findings file anywhere:** `AUDIT_FINDINGS_28.md`, from the thirty-second
  audit (branch `audit/birdtest-2026-09-25-2`). It is in `main`'s history, together with
  `_27.md` (commit `7ca481e`), until commit `7e74935` ("Remove the audit records and
  prompts").
- `AUDIT_FINDINGS_7.md` to `_26.md` are on `audit/birdtest-2026-09-24-pass21`.
- So this file is number 29, and this is the thirty-third audit.
- **What the prior file left open.** `_28` ended with its open items recorded as KLs:
  KL-74 closed, KL-78 to KL-92 added. None is re-flagged unless listed under "KLs
  revisited" below.
- **KLs added this pass:**
  - KL-93: a small batch caps a fast job at a claim a second per worker (D-3).
  - KL-94: an export's snapshot holds back vacuum while it runs (D-8).
- **KLs revisited or updated:**
  - KL-75: its premise no longer holds (B-1).
  - KL-62: closed, because `terraform test` now exercises the validations (G-2).
  - KL-82: list of in-process state completed (A-4).
  - KL-14: solves added to the nondeterministic list (C-4).
  - KL-10: the sort it describes no longer exists (D-5).
  - KL-32: the fleet scan read all history, not a week (D-7).
  - KL-43: "not a limit" bullet added (H-2).
  - KL-87: schema default and CHECKs (B-5).
  - KL-48: build check added to the claim (C-1).
  - KL-7: now points to KL-94.
  - No KL was renumbered.

### Coordinator decisions on flagged items

- **C-1, how to close the build gap.** Chosen: the claim body carries
  `board_dim`/`rack_size`, and the server answers a non-15/7 build with a new
  `unsupported_build` shutdown.
  - The alternative was MAGPIE refusing locally when `BOARD_DIM != 15 || RACK_SIZE != 7`.
  - It was rejected for two reasons. MAGPIE's own `BOARD_DIM=21` test build must keep
    working. And the server should stay the authority on what it can use, which also
    leaves room for 21×21 jobs later.
- **F-1/A-5 (one-slot schema).** Done in this pass rather than left to a human. The
  schema is pre-release and has a single migration, and the per-identity indexes were
  verified unreachable.
- **D-6 (`position_analysis_records_rack_idx`).** Kept as defence in depth against a
  decoder regression, and recorded in PLAN's schema narration.
- **F-13 (`/api/worker/client-version`).** Not dead: the admin new-job form reads it.
  Docs and comments fixed.
- **H-2 (rack-lookup ordering).** A false positive, because the distribution's tiles
  are already sorted by character. The coupling was made explicit anyway.
- **Coordinator's own fixes:**
  - the E-8 spec comment (`e2e/tests/e8-pentanomial.spec.ts`: "five pair outcomes as
    three labelled rows", and no Elo);
  - the admin export link text, now "(links valid for up to an hour)" (G-6).
- **Left pending a human decision.** Each is listed under "Issues and Recommended
  Solutions":
  - B-1: rating pools mix bingo bonuses and sim cutoffs (KL-75 revisited);
  - B-2: a consensus edit reopens a force-completed job;
  - D-3: default batch sizing (KL-93);
  - D-4: `task_claims.job_id` is unindexed and unread (a schema rework);
  - D-5: the FK drops and the KL-10 re-measurement;
  - D-8: an export's snapshot versus vacuum (KL-94);
  - F-4/A-RATE-6: keep or remove `/api/rating-pools/:id/history`;
  - F-12: `/api/admin/jobs/:id/results/stream` has no consumer;
  - F-15 and F-16: the root-level plan docs, and `SETTINGS_COMPARISON.md`'s stale line
    anchors;
  - G-3: `wait_for_steady_state`;
  - W2a's two open calls: an even-batch CHECK, and checking positions against the
    exact mover.

### (b) Bugs and fixes made in birdtest

#### Race conditions

Reviewer A traced the documented lock order on every path: merge lock, then dispatch
lock, then claim, then task, then job, then contributor rows. That covers claim,
submit, decline, heartbeat, reclaim, purge, delete, activate, set_allocations,
deactivate, force-complete, the finish checks, leave closes and merges, ratings versus
admin, exports, backups, the derived builder and input-data import. **No lock-order
deadlock was found.** The races below are not ordering bugs.

| ID | Sev | Race | Fix | Test |
|---|---|---|---|---|
| A-1 | medium | A finish check that read an opening-rack job as settled completed it after a concurrent consensus edit had unsettled racks: `complete_unless_purged`'s `UPDATE` re-checked only `status`/`claims_issued`. | The `UPDATE` also requires `job_type <> 'opening_rack' OR racks_settled >= total_racks`, which Postgres re-evaluates after waiting on the edit's row lock (`jobs/mod.rs`). | `I-OR-EDIT-3` (fails with the predicate removed) |
| A-2 / D-2 | high | The consensus edit held the job's dispatch lock and `FOR UPDATE` on every open claim through `restate_racks` (one UPDATE over up to 3.2 M rows) without a `DispatchHold`. Claims waited 2 s each and submissions 5 s each, all on main-pool connections: the pool-exhaustion pattern `DispatchHolds` exists to stop. It also ran inline in the request. | New `DispatchHolds::try_hold_claims_uncounted`: a claims hold that does not bump the purge witness `claims_holds_taken`. `update_consensus` takes it first, answers 409 to a concurrent edit or lifecycle action, runs its body on `run_to_completion`, and drops the hold before the post-commit `finish_idle_job`. | `I-OR-EDIT-4` (claim 204 and submission 503 in under 1.5 s; second edit 409) |
| A-3 | low | An export's `mark_ready` could store `is_final = true` in the milliseconds between the edit's `unfinalize` and its commit. | `mark_ready`'s job-status subquery reads `FOR SHARE` (`exports.rs`). | `I-EXPORT-15` (fails without `FOR SHARE`) |
| A-5 / F-1 | medium | Nothing in the schema enforced the one-slot invariant the counters depend on ("at most one claimed-or-completed claim per task"). The two per-identity unique indexes, and the claim path's unique-violation retry, were redundancy-era and unreachable. | Replaced by `task_claims_one_slot_idx ON task_claims (task_id) WHERE state IN ('claimed','completed')`. Added `CHECK (accepted_count IN (0,1))` and `CHECK (active_claim_count IN (0,1))`. The dead retry and the dead `next_available` arm are removed, and an `issue_claim` error is now `Fatal`. | `I-SCHED-15`/`-16` updated, new `I-SCHED-22` |

The other paths reviewer A checked and found correct are listed under "Checked and
found OK" in the Issues section (A-OK).

#### Other bugs fixed

| ID | Sev | Bug | Fix | Test |
|---|---|---|---|---|
| H-1 | high | The admin derived-data list and Retry used `CASE role WHEN 'wmp' THEN $1 ELSE $2`, so every word-info-table (`wit`) row looked like another builder's. A WIT build that failed three times showed no Retry and could not be retried, and its job dispatched nothing. RUNBOOK's "press Retry" step could not work. | Three-arm CASE in `list_derived_data` and `retry_derived_data` (`routes/admin.rs`), binding `builders.wit()`. | `A-ADMIN-24` extended with a `wit` row |
| D-1 | high | `next_reissue` walked every unsettled rack (with a join per rack) before falling back, while holding the dispatch lock. An identity that had analysed the whole first pass paid a full walk on every claim. `<> ALL($2)` went linear under a generic plan. | Bounded window: 4 × batch candidates, ordered by "this identity analysed it", then results, then rack. The in-flight exclusion is now `NOT IN (SELECT unnest($2))`, a hashed subplan. Measured at 1 M racks: old 5.7 s with a custom plan, and more than 120 s with a generic plan; new 12–50 ms. | `I-OR-REISSUE-1`; PLAN "What these reads cost" gains two rows |
| H-3 | low | The job list showed an opening-rack job's racks *analysed*, and the job page racks *settled*, so a consensus job read as finished in the list. | `list_jobs` uses `racks_settled`, and the unit reads "racks settled". | `A-PUBLIC-1f` |
| B-3 | low | `divergent_games` was not cross-checked against the full tally and the pentanomial. | `game_pair::check_divergent_against_the_whole`: subset within the whole; outside it wins = losses and draws even; non-split pairs ≤ divergent/2. | `U-PLAUS-8` |
| B-4 | low | A result's analysis kind was never checked against the dispatched players: a static player could report a sim, a non-solver a PEG or endgame, and so on. | `plausibility::check_analyses_against_players`, run from `decode_result`, against either player. `check_analysis` also refuses `iterations>0` or plies on a non-sim move. | `U-PLAUS-6`, `U-PLAUS-7`, `A-WORKER-12`; real MAGPIE fixtures pass (`C-3`/`-4`/`-5`) |
| B-5 | low | The match-test settings had no schema CHECKs, so a direct insert could make a job that never completes. The schema default batch of 1 was odd (KL-87). | `job_game_config_counts` / `job_game_pair_config_counts` CHECKs; `games_per_batch DEFAULT 2`. | `I-JOB-1f` |
| B-8 | low | Rating-pool and player-config names had no length or control-character rule, and pool names were stored untrimmed. | Shared `name_problem` / `MAX_NAME_CHARS` (100); pool names trimmed; `CHECK (char_length(name) <= 100)` on both tables. | `A-ADMIN-2c` |
| B-9 | low | The public `?rack=` lookup returned every analysis's full move list: up to about 3.3 M rows, unauthenticated. | It counts the analyses, then keeps `rank <= RACK_LOOKUP_MOVES / analyses`, with `RACK_LOOKUP_MOVES = i16::MAX`. One analysis is listed whole, as before. | `I-SCHED-21` extended; `A-PUBLIC-4` |
| D-7 | low | The admin fleet view scanned all of `task_claims`, not "a week" (KL-32). Admin reads also ran on the main pool with no statement timeout. | Fleet counts claims completed in the week (`task_claims_completed_idx`) plus open claims claimed in the week (`task_claims_open_idx`). Fleet and audit-log reads moved to `read_pool`. Measured at 1.5 M claims: 209 ms (48 k buffers) before, 88 ms (2.1 k) after. | `A-ADMIN-9` extended |
| E4-R2 | low | RUNBOOK says a job's `.census` row holds the counts about to be lost, but `job_census` did not count `opening_rack_progress`. | The census adds `rack_standings=…`. RUNBOOK §0 lists exactly what the census counts. | `I-OR-EDIT-1` assertion; `tests/audit.rs`, `tests/jobs.rs` |
| G-6 | low | Export download links were signed with the task role's cached temporary credentials, so they could expire well before the stated hour. | `presigned_get` fetches fresh credentials per link and caps the TTL at the credentials' remaining life less 60 s (`presign_ttl`). The UI reads "up to an hour". | `U-ART-1` (two tests) |
| H-4 | low | The consensus form and editor disagreed with the server: "Min 3, Max 1" was sent as 1/1; a stale share was sent at max ≤ 1; `min="51"` blocked 50.5. | `consensusFields` sends the share only when max > 1, and `consensusProblem` checks min/max always. `min="51"` removed. | `F-CONS-3` rewritten, `F-CONS-4` |
| H-5 | low | The first saved-positions search sent `?cursor=undefined`. | `api.ts` `query()` drops `undefined` entries. | `F-API-7` |
| H-6 | low | Any `/api/me` failure (502, 503, network) signed the user out on the client. | Only 401/403 set `null`. Other failures keep the state and retry: 2 s doubling to 30 s, honouring `Retry-After`. | `F-AUTH-1`/`-2` rewritten |
| G-4 | low | Nginx's keep-alive (65 s) was shorter than the ALB's idle timeout (300 s), which gives occasional 502s. | `keepalive_timeout 310s`. | `F-NGINX-2` (parses both files) |

### (c) MAGPIE argument coverage (objective 3)

Reviewer C traced every outcome-affecting setting from `contribute.c` through
`config_create_for_contribute` to the four executors. The task config is fresh per
task. `settings.txt` reaches only data paths. In `contribute.txt`, only `threads`
affects outcomes (KL-14, accepted). Every per-player and run-wide key is required and
digest-verified, or reset. The gaps found:

| ID | Sev | Gap | Fixed |
|---|---|---|---|
| C-1 | medium | `RACK_SIZE` and `BOARD_DIM` are compile-time, and were neither stated, reported nor checked. A `RACK_SIZE=8` build plays 8-tile racks and awards the bingo bonus only for 8-tile plays, passing every digest and plausibility check unless the job pins a rack info table: silently wrong rows in match tests, ratings and consensus. A 21 build failed loudly but as `task_failed` ×5. | Yes, on both sides. MAGPIE's claim body carries `board_dim`/`rack_size`. The server's `ClaimBody` requires them and answers a non-15/7 build with `shutdown {reason: "unsupported_build"}`, handing out no task and minting no identity. `A-WORKER-22`; the contract tests; new fixture `shutdown-unsupported-build.json`; MAGPIE claim-body and shutdown tests. |
| C-3 | low | The sim margin forecast (`-sm1/-sm2`, `-smargin`) changes sim equity and utility, but was in neither contribute reset, so it was left to `config_create`'s built-in default. | Yes (MAGPIE): both resets set it `false`, with new getters and tests. |
| C-2 | low (doc) | PLAN's "Every setting that can change a result" table was stale for solvers and omitted the margin forecast. | Yes (doc): solver and PEG rows, `-ttfraction`, the margin forecast, and the PlayChooser-only row; solver keys added to the JSON field table. |
| C-4 | low (doc) | PLAN said "games are deterministic". Only static, non-solving players are; sims and endgame/PEG solves are multithreaded and nondeterministic. | Yes (doc): PLAN 166, 250 and 2370, and KL-14. |
| C-5 | low | MAGPIE kept fail-open fallbacks for an "older server" that birdtest never is: no `expected_data` or a non-sha256 algorithm ran unverified; an unpinned wordmap was built from a `.wmp.src` sidecar; NULL distribution or layout defaults; a claim without `job_id`. | Yes (MAGPIE): each branch was confirmed unreachable from the current server, then removed. A missing or unknown verification is now a server error, and an unpinned wordmap is a `derived_mismatch` decline. `test_an_unverifiable_assignment_is_refused`. |

The determinism claims in the docs now state what the owner rule says: only static
players are deterministic, and seeds help consistency only. Thread count still changes
what a simming or solving player can reproduce (KL-14).

### (d) Critical-path and async changes (objective 4)

Reviewer D traced claim, heartbeat, decline, submit and completion. **What is on the
path now is correctness- or dispatch-gating, with documented exceptions:**
- the decline's audit and gap rows (KL-28);
- the contributor counters (KL-5);
- the display columns of the job `UPDATE`, which ride locks already held.

Nothing new needed moving. Ratings are still not read or written on the claim, submit
or completion path (grep of `worker.rs`, `scheduler.rs` and `jobs/*`). The
finish-check debounce is safe: every 8th submission checks, so does any submission
that finds no open claim, the idle check covers the rest, and the purge witnesses
guard both.

Changes this pass that touch the path:
- **Consensus edit (A-2).** Claims now skip a job under an edit, and submissions are
  answered 503 at once, instead of each holding a pool connection for 2–5 s.
- **Reissue (D-1).** The opening-rack reissue under the dispatch lock is bounded:
  milliseconds, not seconds or minutes.
- **Completion (A-1).** The completion `UPDATE` gains a predicate on the row it
  already updates, at no cost.
- **Decode (B-3, B-4).** New plausibility checks run in `decode_result` on the
  blocking pool. They cost O(positions) and are cheap.
- **Export (A-3).** `mark_ready` waits on the job row, at most milliseconds, once per
  export, off the worker path.
- **Admin reads (D-7).** Fleet and audit-log reads moved to the display pool, off the
  claim pool.

### (e) Performance, most to least severe (objective 5)

| # | ID | Issue and expected impact | Status |
|---|---|---|---|
| 1 | D-1 | Opening-rack reissue: a full walk of up to 3.2 M unsettled racks per claim under the dispatch lock. With custom plans that is about 6 s per claim at 1 M racks. Under a generic plan, more than 120 s with 50,000 racks in flight. A consensus job's reissue phase stalls, and its other claims wait 2 s each on the pool. | **Fixed**, measured at 12–50 ms |
| 2 | A-2/D-2 | Consensus edit on a full English job: minutes of `restate_racks` holding the dispatch lock and every open claim, with no hold. About 10 claims a second, or a few hundred finishing workers, exhaust the 20-connection pool, which stalls every job. | **Fixed** (hold plus `run_to_completion`) |
| 3 | D-3 | Default batch of one pair or two games, under the one-claim-a-second limit: a fast (static) job's worker plays about one pair a second whatever its cores, and pays a claim and a submit per pair. Row counts and the sweep's and finish check's reads scale with tasks. | **Flagged** as KL-93 (owner) |
| 4 | B-9 | Public rack lookup building up to about 3.3 M rows for one unauthenticated request. | **Fixed** |
| 5 | D-7 | Admin fleet sequential scan of all `task_claims` history on the main pool: minutes at about 100 M rows after a year. | **Fixed** (index-bounded, display pool) |
| 6 | D-5 | KL-10's cost figures describe a per-task sort that no longer exists. The current finish-check read is a plain sum, unmeasured and expected well under the documented 340–620 ms. | **Doc updated**; re-measurement **flagged** |
| 7 | D-4 | Job-scoped claim reads (reclaim, `should_check_finish`, leave in-flight count, ETA, KL-58) go through fleet-wide partial indexes and `tasks`, because `task_claims.job_id` is unindexed. | **Flagged** (schema rework) |
| 8 | W1 note | With `jit=on` (Postgres's default) the new reissue query's generic plan crosses `jit_above_cost` and spends about 75 ms compiling. | **Flagged**; recorded in PLAN's cost row. Production `jit` is the owner's call. |

### (f) Storage (objective 6)

**Fixed:**
- **`task_claims`, the hottest write table.** Two partial unique indexes were replaced
  by one (`task_claims_one_slot_idx`); the table now has eight indexes. One fewer
  index entry per claim insert and per state change.
- **`game_results_task_idx`.** Shrunk from `(task_id, submitted_at)` to `(task_id)`.
  No reader orders by `submitted_at` per task; the index stays for the FK cascade.
- **Captured positions (C-7a).** Three fields nobody reads (`total_iterations`,
  `time_elapsed`, `status`) are no longer sent. That is about 60 bytes per position, or
  about 100 MB of upload and parsing for a 40,000-pair capture job.
- **`opening_rack_requests.previous_play`.** The column, always NULL, is dropped, with
  its wire field on both sides (C-7b).
- **Schema CHECKs:** the counters (A-5), the match-test counts (B-5) and the name
  lengths (B-8).

**Flagged:**
- **D-4:** add a `job_id` index on `task_claims` and re-key the completed index to
  `(job_id, completed_at)`.
- **D-5:** drop the redundant `game_results.task_id` and `leave_records.task_id` FK
  cascades.
- **D-6:** `position_analysis_records_rack_idx` is kept as defence in depth. It costs
  about 3.2 M entries (100+ MB) per full English job, and an index write per record.
- **D-8 / KL-94:** an export's REPEATABLE READ snapshot pins the vacuum horizon for up
  to six hours.
- **D-3 / KL-93:** row counts scale with tasks at batch 1.

**Verified:**
- Rating-run thinning exists, runs hourly, deletes in batches of 1,000, and keeps each
  pool's first run and each day's last.
- Every `ON DELETE CASCADE` FK has a supporting index.
- The S3 lifecycles hold.
- `backup.sh`, `restore-drill.sh` and `restore-roundtrip.sh` have no new growth hazard
  beyond KL-31 and KL-46.

**Observed, not acted on.** These came from reviewer D's index map and were not raised
as findings; all are harmless:
- `input_data_role_name_idx` has no reader;
- the `UNIQUE (variant, letterdist_id, layout_id, name)` on `rating_pools` is implied
  by `name UNIQUE`;
- `audit_log_job_idx` could be partial (`WHERE job_id IS NOT NULL`);
- `derived_data_queue_idx` is probably never chosen on a table of tens of rows.

### (g) Dead or out-of-date code removed, and items flagged as uncertain

**Removed (each verified against its callers, then by compiler and tests):**
- **Redundancy-era claim machinery (F-1):**
  - the two per-identity unique indexes;
  - the claim path's unique-violation `Retry`;
  - `next_available`'s `state NOT IN ('abandoned','declined')` arm.
- **`insert_position_analyses` (F-2):**
  - the `on_conflict_ignore` parameter and its `ON CONFLICT DO NOTHING` re-matching;
  - the "skip records written by another claim" path;
  - the unreachable `ON CONFLICT (move_id, ply)` on plies.
- **`JobStats.results_accepted` (F-11).** It equalled `tasks_completed` and was never
  displayed. Removed from the backend, the frontend type and PLAN; e2e and two tests
  now use `tasks_completed`.
- **The opening-rack `num_moves` shim (F-9).** It was optional "for builds that
  predate it", but every build the 0.1.1 floor admits sends it. It is now required, and
  the fallback and its test are gone.
- **`magpie_defaults::ENDGAME_PLIES` and `PEG_MAX_BAG` (F-8).** Never read: a depth of
  0 is off, so no "default when on" exists.
- **Backend dependencies (F-6).** `thiserror` (unused) and `password-hash` (argon2's
  default features cover it) removed; `tower` moved out of `[dependencies]` (test-only);
  `tower-http`'s `cors` and `fs` features dropped. `Cargo.lock` only loses packages.
- **Frontend devDependency `@types/d3-scale` (F-7)**, orphaned when d3 was removed.
- **The fake worker's dead modes and flags (F-14).** The decline modes, `decline()`,
  `--magpie-version`, `--delay`, `--work-seconds`, `--stop-when-idle`, `--timeout`,
  `--verbose` and `--api-key`. Nothing in compose, e2e, scripts, CI or docs invoked
  them.
- **Contract fields one side never reads (C-7):**
  - MAGPIE's three captured-position fields;
  - `previous_play` on both sides;
  - `GameRequest.game_pairs`, which duplicated the `job_type` tag;
  - MAGPIE's `CONTRIBUTE_KEY_GENERATION`.
- **MAGPIE's "older server" fallbacks (C-5)** and its dead `fatal` parameter, with
  `ERROR_STATUS_CONTRIBUTE_UNKNOWN_JOB_TYPE`, `…_MAGPIE_TOO_OLD` and
  `…_DATA_NOT_WRITABLE` (C-6).

**Stale comments and text corrected:**
- **Redundancy-era wording (F-3, B-7, E-2):**
  - the migration's `task_claims` and users-totals comments, and their PLAN copies;
  - `jobs/mod.rs` `submitted_at` and the `ON CONFLICT` comments;
  - `models/job.rs` `games_completed`;
  - `routes/worker.rs` (two places);
  - `scheduler.rs`: both `accepted_count > 0` arms, now commented as drift defence;
  - `ratings.rs:296`.
- **The removed rating-history chart (F-4/H-7):**
  - the ratings page's help text;
  - `routes/ratings.rs`, which cited a deleted `ratingHistory.ts`;
  - the thinning justification in `ratings.rs`, `main.rs` and the migration (and its
    PLAN copy);
  - PLAN's "Four panels".
- **The SPRT (E-6/F-5/G-7/H-9):**
  - `docker-compose.e2e.yml`'s "drive … to an SPRT verdict";
  - the nightly step name "M-1 to M-11", now M-1 to M-16;
  - `e2e/run.sh --help` truncation;
  - the e8 spec's "Elo" and "five buckets" comment.
- **Other comments:**
  - `DerivedCache`'s soundness comment (A-6);
  - `ResultsQuery.worker` (E-16);
  - `client-version`'s self-update comparison (F-13);
  - five MAGPIE `contribute.h`/`contribute_defs.h`/test comments (C-6);
  - the testdata README's "no game_pairs fixture" (H-11).

**Flagged as uncertain or for the owner:**
- F-4: keep or remove `GET /api/rating-pools/:id/history`. It is now API-only and its
  text is fixed.
- F-12: keep or remove `GET /api/admin/jobs/:id/results/stream`. It has no in-repo
  consumer.
- F-15: `SETTINGS_COMPARISON.md`. 59 of its 62 migration line anchors are stale, and
  `F-SET-1` cites it.
- F-16: the root plan docs. `FEATURE_BATCH_PLAN.md` has no "implemented" banner, and
  batches 2 and 3 and `ENDGAME_PEG_PLAN.md` are superseded. JOURNEYS.md's
  `E-1..E-15` was fixed to `E-18`.

**Not dead:**
- F-13: `/api/worker/client-version` is read by the admin job form.
- Write-only provenance columns are forensic fields, kept.
- `DB_*` env vars are a documented extension point.

**Still stale and fixable, carried to iteration 2:**
- `e2e/tests/e12-saved-positions.spec.ts:8` still names the checkbox
  "Save the positions played"; it is "Position Recorder".
- `routes/admin.rs:1660` writes the opening-rack `rack_size` bound as a literal
  `1..=7`, not `leave_gen::RACK_SIZE`. Cosmetic.

### (h) Python-worker-as-production-client corrections

No code, config, CI step or doc treats `worker/fake_worker.py` as a production or
dev-environment client. Reviewers F and H checked README, TESTING, PLAN, RUNBOOK, both
compose files, the Dockerfile, `scripts/dev.py`, e2e and CI. **No framing correction
was needed.**

The instrument was brought in line with MAGPIE so that tier 5 proves what it claims:
- **Analysis shape (H-10).** Positions are shaped by the mover's player config, as
  MAGPIE shapes them: static, sim or inference, with plays and plies capped at the
  recorded counts. E-12 now uses simming configs for the sim columns it asserts, and
  E-13 asserts the static case.
- **Fixtures (H-11).** The stale fixtures were regenerated, and a game-pairs divergence
  fixture was added. `scripts/fake-worker-fixtures.sh --check` diffs them, and runs in
  CI's `scripts` job.
- **Transport (H-12, H-13):**
  - 5xx and 429 responses are retried after `Retry-After`, as MAGPIE does;
  - `--server-url` is required, so there is no dev-port default;
  - `e2e/lib/worker.ts` `firstClaim` pins the job it claims from.
- **Dead modes (F-14)** removed, as listed in (g).

**Prompt drift (PD-5):** the prompt says tier 6 and `dev.py` "refuse" the fake. Neither
has an option to refuse; neither can run it.

### (i) `birdtest-contribute` changes, pin and version floor

All of these are committed on `birdtest-contribute` as `c3c6a875`, which is
not pushed.

| Change | Why | Files |
|---|---|---|
| Claim body states `board_dim`/`rack_size`; `print_shutdown` handles `unsupported_build`, which is always obeyed | C-1: a non-default build could contribute silently wrong results | `contribute.c`, `contribute.h`, `contribute_test.c` |
| Contribute resets turn the sim margin forecast off, per player and run-wide; new getters | C-3 | `config.c`, `config.h`, `contribute_test.c` |
| Missing `expected_data` or an unknown algorithm is a server error; an unpinned wordmap is refused (sidecar path deleted); NULL distribution/layout defaults removed; `job_id` required | C-5: unreachable from birdtest, fail-open, untested | `contribute.c`, `config.c`, `contribute.h`, `io_util.h` |
| Stale comments fixed; dead `fatal` parameter and two dead error statuses removed | C-6 | `contribute.h`, `contribute_defs.h`, `contribute.c`, `config.c`, `io_util.h`, `contribute_test.c` |
| Captured positions drop `total_iterations`, `time_elapsed`, `status`; `previous_play` refusal removed; `CONTRIBUTE_KEY_GENERATION` deleted | C-7 / F-10: bytes nobody reads | `autoplay_results.c`, `config.c`, `contribute_defs.h` |
| Contract fixtures: 18 files (new `shutdown-unsupported-build.json`), byte-identical to birdtest's `contract-fixtures/` | Protocol change | `test/birdtest_contract/*` |

- **Pin.** `docker/Dockerfile` `ARG MAGPIE_COMMIT=` moves from `dbf8df3b` to
  `c3c6a875`. CI (`ci.yml`, two jobs) and the nightly read it from that line.
- **Version and floor.** `MAGPIE_VERSION` stays 0.1.1, and `MIN_MAGPIE_VERSION` stays
  0.1.1 in all eight copies, which the new `U-CFG-5` now checks. There was no bump,
  under the owner rule that nothing is deployed and both sides change together.
  - Consequence: a pre-change 0.1.1 build's claim, without `board_dim`/`rack_size`, is
    a 400 naming those fields rather than a version shutdown.
  - That is acceptable only because nothing is deployed. Before a first deployment, any
    later result-changing release must bump both.
- **Derived data.** No builder or conversion code changed, so builder versions and
  derived-data hashes are unaffected.
- **Fixtures.** Recaptured with `scripts/e2e_magpie_native.sh --cases capture` against
  the new build: 8 captured, the hand-written ones edited. The fake-worker fixtures
  were regenerated from the new assignments.
- **MAGPIE checks:**
  - **Run:**
    - `python3 format.py --write` (clang-format-20), twice; the second run changed
      nothing;
    - `make magpie_test BUILD=dev -j3`, clean with `-Werror`;
    - `./bin/magpie_test contribute`, passed twice: before and after the recapture;
    - `make magpie BUILD=portable_release`.
  - **Not run:**
    - cppcheck and clang-tidy (standing owner instruction: they froze the machine);
    - the `BOARD_DIM=21` build and tests;
    - the wasm tests.

    The changes are written to be BOARD_DIM-agnostic. The claim test compares against
    this build's `BOARD_DIM`/`RACK_SIZE`, and the layout test uses
    `board_layout_get_default_name()`. CI's 21 shard is the first real check.

### (j) Findings file and count

`AUDIT_FINDINGS_29.md` — objective-7 discrepancies: **code-wins 47 / doc-wins 9 / unresolved 3**.
One more discrepancy is open, fixable and carried to iteration 2 (O7-60); it is not counted.

### (k) Deployment blockers

No high deployment blocker was found. The deploy path, the first apply, SSM, IAM, the
ALB, RDS, backups, the ops and drill tasks, fresh-account and fresh-region restores, and
RUNBOOK §1–§6 were all checked against the Terraform names and outputs (reviewer G,
"Checked and found OK").

| ID | Issue | Resolution |
|---|---|---|
| H-1 | A failed WIT build could not be retried, so its job never dispatched; RUNBOOK's step could not be done. | Fixed (b). |
| C-1 | A contributor build with non-default dimensions was accepted. | Fixed (c). |
| G-1 | The version floor had eight hand-kept copies, with nothing checking they agree. Raising Terraform's copy above the pin takes production down at deploy. | Fixed: `DEFAULT_MIN_MAGPIE_VERSION` const plus `U-CFG-5`, which reads every copy. |
| G-2 | KL-62's three cross-variable rules were not validated, and no validation was ever exercised in CI. | Fixed: `infra/tests/variables.tftest.hcl` (50 runs, mock provider); three new validations (builder image tag = backend tag, ACM ARN in region, sender within `ses_domain`); `terraform test` in CI. KL-62 closed. |
| G-3 | No ECS deployment circuit breaker: a release that cannot start stays down until a manual rollback. | Fixed: `deployment_circuit_breaker {enable, rollback}`, with a RUNBOOK paragraph. `wait_for_steady_state` is flagged. |
| G-4 | Nginx keep-alive shorter than the ALB idle timeout. | Fixed (b). |
| G-5 | README's first deployment stopped at the first admin, but a new stack needs input data before any job. | Fixed: README step 8. |
| G-6 | Export links could die early. | Fixed (b). |
| A-4 | The single-instance descriptions omitted `DispatchHolds` and the purge witness. | Fixed (doc): `desired_count` description, `ecs.tf` comment, PLAN KL-82 and the future-improvements bullet. |
| W5 | `outputs.tf` `ses_dkim_records` indexed `[0]`, which errors on a mock plan. | Fixed: `flatten(...[*].tokens)`, same value on a real apply. |
| — | The MAGPIE pin is not pushed. | **Open**: push `birdtest-contribute` before CI, e2e images and the nightly can pass (header). |

**Terraform variables with no default:** eight (PD-1). `backend_image`,
`frontend_image`, `acm_certificate_arn`, `alert_email`, `mail_from_address`,
`ses_domain` and `public_url` are in `variables.tf`; `derived_builder_image` is in
`derived.tf`. README lists all eight. Three of them now carry cross-variable
validations (G-2).

### (l) Prompt drift

| # | `audit_loop.md` says | The repository says | Checked in |
|---|---|---|---|
| PD-1 | Terraform variables with no default: `backend_image`, `frontend_image`, `acm_certificate_arn`, `alert_email`, `derived_builder_image` | Eight: the five plus `mail_from_address`, `ses_domain` and `public_url` (`infra/variables.tf:430-480`; `derived_builder_image` in `infra/derived.tf:26`) | script over `variable` blocks; README.md:722-724 lists all eight |
| PD-2 | The derived-data builder "builds reference wordmaps, rack-info tables and leave-generation KLVs" | `derived.rs` builds wordmaps, rack info tables **and word info tables** (`wmp`/`rit`/`wit`). Leave-generation KLVs are built by the backend server process (`jobs/leave_gen.rs` `zero_klv`/`generation_klv` through `magpie.rs`), not by the builder task | `derived.rs` header, migration CHECKs |
| PD-3 | Plausibility in `plausibility.rs` "including the pentanomial cross-check and per-job-type structural checks" | `plausibility.rs` holds the shared checks. The pentanomial cross-check is in `jobs/game_pair.rs` and the per-type checks are in each handler. PLAN now says so (E-7) | grep `pentanomial` |
| PD-4 | CI runs `cargo test --lib --bins` and the DB tiers | CI builds a `cargo nextest archive` and runs `cargo nextest run` in 4 partitions with Postgres and MinIO. The MAGPIE job also runs `./bin/magpie_test builderhash` and a conversions smoke test. The nightly runs `--run-ignored ignored-only`, `restore-roundtrip.sh`, `restore-job-check.sh` and `reapply-check.sh`. This pass adds `terraform test` and `fake-worker-fixtures.sh --check` to CI | `ci.yml`, `nightly.yml` |
| PD-5 | "Tier 6 and `scripts/dev.py` refuse" the fake worker | Neither has a fake-worker option to refuse: `dev.py` always uses real MAGPIE, and `e2e_magpie*.{py,sh}` never reference the fake. The substance holds | grep in `scripts/` |
| PD-6 | `audit/birdtest-2026-09-24-pass21` held `AUDIT_FINDINGS_7.md`–`_26.md` | True but incomplete. `_27.md` (`7ca481e`) and `_28.md` were in `main`'s history and removed by `7e74935`. `audit/birdtest-2026-09-25-2` contains those commits but no findings file at its tip. So the next number is 29, and there were 32 prior audits | `git log --all --diff-filter=A`, `git ls-tree` |
| PD-7 | `accepted_count`, `active_claim_count` "each 0 or 1" | It held by code and comment, but no CHECK enforced it. **Now enforced** by CHECKs and the one-slot index (A-5) | migration |
| PD-8 | "The server refuses to start otherwise" (floor above the pinned `MAGPIE_VERSION`) | True for the web process (`main.rs` runs `magpie builders` before connecting). The derived-file builder (`bin/build-derived.rs`) does not check, as KL-82 records. A refinement, not an error | `main.rs:58-82`, KL-82 |
| PD-9 | The floor `MIN_MAGPIE_VERSION` 0.1.1 admits exactly the builds the server accepts | The `played_move` and `mean_spread` keys were added to MAGPIE after the commit that set 0.1.1 (`dd8b0299`, `79100bf1`). And after this pass the claim needs `board_dim`/`rack_size`. So the floor admits 0.1.1 builds the server would reject. Moot while nothing is deployed (owner rule), but the floor and `MAGPIE_VERSION` must be bumped together before a first deployment | `git merge-base --is-ancestor` in `~/MAGPIE` |

All other Snapshot claims were verified true: `main` at `0149541`, MAGPIE at
`dbf8df3b`, the Dockerfile pin, `MAGPIE_VERSION` 0.1.1, the floor copies, the
allocation-only scheduler, `DispatchHolds`, consensus, the match test (`test_enabled`
off by default, debounce 8), ratings siloed from job control, `worker_bans`, one
migration, the documented scripts, PLAN at about 10.4 k lines, TESTING with seven
tiers, and the root plan docs. MAGPIE has no drift (reviewer C).

### (m) Checks run and not run

See **Checks** below for the end-of-pass results.
- **Baseline before any change:** `cargo clippy --locked --all-targets -- -D warnings`
  clean; `cargo nextest run --run-ignored all` **685/685**.
- **During the pass,** each fixer ran clippy, targeted or full nextest, the frontend
  checks, the MAGPIE contribute tests, Terraform (in Docker: `fmt -check`, `validate`,
  `test` 50/50), `nginx -t`, `runbook-check.sh` and the fixture capture. Details are in
  the Issues entries.
- **Not run in pass 1:**
  - **Tier 5 (Docker/Playwright).** Image builds need owner approval under a standing
    instruction, so it is deferred to the final green run (loop step 3), where tiers 5
    and 6 run natively. The e2e specs edited this pass type-check under `tsc --strict`
    but have not been executed.
  - **Tier 6** (`scripts/e2e_magpie_native.sh`, full case list). Deferred to the final
    run; only its `capture` case ran this pass, which passed.
  - **The opt-in `magpie_leave` Rust tests.**
  - **MAGPIE cppcheck and clang-tidy.** Never run, under a standing owner instruction
    (they froze the machine).
  - **MAGPIE `BOARD_DIM=21` unit tests and the wasm tests.**

---

## Checks run at the end of pass 1

Run on the combined tree after all seven fixers finished, with every heavy
command serialized behind one build lock (16 GB machine):

| Check | Result |
|---|---|
| `cargo clippy --locked --all-targets -- -D warnings` | clean |
| `cargo test --locked --doc` | ok (0 doctests) |
| `cargo nextest run --locked --run-ignored all` (Postgres 16 + MinIO; opt-in MAGPIE smoke/route tests included) | **701 / 701 passed** (baseline before the pass: 685 / 685) |
| `npm run check` / `npm test` / `npm run build` (frontend) | 0 errors / **211 / 211** / built |
| `scripts/runbook-check.sh RUNBOOK.md README.md` | 32 and 19 bash blocks parse |
| `scripts/dev-restore-check.sh` | passed |
| `scripts/fake-worker-fixtures.sh --check` (new) | every fixture is what the fake emits |
| Terraform 1.9.8 (Docker): `fmt -check -recursive`, `init -backend=false`, `validate`, `test` | clean / ok / **50 / 50** |
| `nginx -t` (nginx:1.30-alpine) | ok |
| MAGPIE `python3 format.py` (clang-format 20.1) | no differences |
| MAGPIE `find_circ_deps.py` (clean copy) | no cycles |
| MAGPIE `make magpie_test BUILD=dev` + `./bin/magpie_test contribute` | clean build, passed |
| Contract fixtures recaptured (`e2e_magpie_native.sh --cases capture`) and copied to MAGPIE `test/birdtest_contract/` | 18 byte-identical |

**Not run in pass 1, and why:** tier 5 (Playwright) and the full tier 6
(`scripts/e2e_magpie_native.sh`) are deferred to the final green run (loop
step 3), run natively, because the Docker images need the owner's approval
to build on this machine; cppcheck and clang-tidy are never run here (the
owner's standing instruction after they froze the machine), so CI's two
MAGPIE lint jobs are unverified; MAGPIE's BOARD_DIM=21 unit tests and the
wasm tests were not run in pass 1. MAGPIE `c3c6a875` is committed on
`birdtest-contribute` but **not pushed**, so CI's magpie-contract job, the
e2e image builds and the nightly cannot fetch the new pin until it is.

---

## Objective 7 — every code-versus-doc discrepancy found in pass 1

Decision key:
- **code wins:** behaviour kept, and the doc, comment or test note was updated to match
  it.
- **doc wins:** behaviour changed to what the doc says or needs.
- **unresolved:** neither changed beyond describing the situation; it awaits the owner.

| # | ID | Code does | Doc said | Decision | Reasoning |
|---|---|---|---|---|---|
| O7-1 | E-1 | The match-score card is a per-player table (W/L/D, average score, average spread, colour-coded), plus a "Games that diverged" table for pairs, over every result | PLAN:967 described a W/L/D chart, score/win %, over "the first accepted result of each task" | code wins | Redesign in 42226dd; doc lagged |
| O7-2 | E-2 | One result per task; `games_completed` is a running total | PLAN:770, KL-10 options and the measured-cost preamble spoke of "first result per task" and redundancy 2 | code wins | Redundancy removed in October 2026 |
| O7-3 | F-3/B-7 | One slot per task | Migration comments (and PLAN copies), `jobs/mod.rs`, `models/job.rs`, `worker.rs`, `scheduler.rs` and `ratings.rs` comments described redundant claims and first results | code wins | Same; the defensive SQL arms are kept, commented as drift defence |
| O7-4 | E-3 | Seven tiers. Compose sets `MAGPIE_BIN` inline, and `MAGPIE_ROOT` is the knob. A MAGPIE build is required | PLAN Development said six tiers, said to set `MAGPIE_BIN`, and listed "Docker and nothing else". The compose header said the same | code wins | Docs and compose header corrected |
| O7-5 | E-4 | — | PLAN:548 linked `#rating-pools`, which does not exist. Two `[Schema](#schema)` links hit the wrong one of two "Schema" headings | code wins | Links fixed (`#schema-1`) |
| O7-6 | E-5 | `format.test.ts` has 45 cases; 1F totals as now counted | TESTING status table: 46 / 203 | code wins | Counts updated (1F 211 after this pass) |
| O7-7 | E-6/G-7/F-5/H-9 | No SPRT; the match test runs only with `test_enabled`; tier 6 runs M-1..M-16 (no M-8); `run.sh` header runs to line 20 | Compose comment "to an SPRT verdict"; nightly step "M-1 to M-11"; `--help` printed lines 2–19; the e8 spec mentioned Elo | code wins | Comments and step name fixed. The e12 part is O7-60 |
| O7-8 | E-7 | The pentanomial and per-type checks live in each handler | PLAN:100 said all of them are in `plausibility.rs` | code wins | PLAN names both places |
| O7-9 | E-8/C-4 | Only static, non-solving players are deterministic | PLAN 166, 250 and 2370 ("games are deterministic, which the scheduler already assumes"); KL-14 omitted solves | code wins | Owner determinism rule; KL-14 extended |
| O7-10 | E-9 | `GET /api/admin/jobs/:id/derived-data` exists, and the admin page uses it | Missing from PLAN's Admin API table | code wins | Row added |
| O7-11 | E-10 | — | A blank line split PLAN's public API table | code wins | Fixed |
| O7-12 | E-11 | `bingo_bonus` and `sim_cutoff` are optional body fields, validated, and sent by the form | PLAN:5321 said they "are not fields of the body" | code wins | Doc describes them, their defaults and validation. The rating-pool consequence is O7-57 |
| O7-13 | E-12 | Writes `job.allocation_changed`, `rating_pool.anchor_changed`, `rating_pool.deleted` and `.deleted.census` | PLAN's "complete" audit-action table lacked them; RUNBOOK §0's prose omitted pool deletion | code wins | Rows and prose added |
| O7-14 | E-13 | Migration has `position_analysis_moves.iterations` | PLAN's "reproduced in full" schema lacked it | code wins | Inserted; block diffed identical |
| O7-15 | E-14 | Many files and directories exist | PLAN's directory tree omitted them and had three stale comments | code wins | Tree completed, including every `lib/` module (W6's call; the owner may prefer less churn) |
| O7-16 | E-15 | `MAGPIE_DOWNLOAD_URL` is also returned by `client-version`; `DB_PORT` defaults to 5432 and `DB_HOST`/`NAME`/`USER`/`PASSWORD` are required without `DATABASE_URL`; `MAIL_MAX_PER_SECOND` and `DEV_LOGIN` fail fast | PLAN config table said "unset" and named only `SECURE_COOKIES` as fail-fast | code wins | One phrase each |
| O7-17 | E-16 | `?worker=` resolves a username or the public pseudonym `anon_id` | Comment on `ResultsQuery.worker` said "an anonymous worker UUID" (PLAN was right) | code wins | Comment fixed; exposing a UUID would leak a credential |
| O7-18 | A-4 | Single instance depends on `DispatchHolds`, the purge witness, reapers and in-process state | The `desired_count` description, the `ecs.tf` comment, PLAN future improvements and KL-82 named only imports, rate limits and SSE | code wins | Lists completed; enforcement was already correct |
| O7-19 | A-6 | `delete_input_data` deletes `derived_data` rows (only for unpinned inputs) | `DerivedCache` comment: "nothing deletes one" | code wins | Comment gives the true reason the cache is sound |
| O7-20 | C-2 | Autoplay reads per-player solver settings and `-ttfraction`; the margin forecast changes results | PLAN's settings table said solvers were unreachable, and omitted solver keys and the forecast | code wins | Rows replaced and added |
| O7-21 | D-5 | `game_stats` is a plain sum through `game_results_feed_idx` | KL-10 and the measured table described a per-task sort that spills `work_mem` | code wins | Text marks the old figures as history; the new sum is "not re-measured" (re-measurement flagged) |
| O7-22 | F-13 | `client-version` is read by the admin form only, and returns `download_url` | PLAN said it means the floor "rather than … download URL"; the handler comment compared it with the Python self-update | code wins | PLAN (three places), handler and config comments fixed |
| O7-23 | F-14 | Tier 5 never sent declines; declines are covered by Rust tests | PLAN:3517 said the fake worker covers "a decline for each reason" | code wins | Sentence removed with the dead modes |
| O7-24 | F-4/H-7 | No rating-history chart (removed in 52c7d1a); the endpoint is API-only | Ratings page help text, `routes/ratings.rs` (citing a deleted file), the thinning comments, PLAN "Four panels", the route row "history", and KL-75's first bullet | code wins | Text fixed. Keep or remove the endpoint is a separate owner question (F-4) |
| O7-25 | H-8 | E-1 shows status on its own row and three cards; E-17 shows one move at a time with a toggle; E-8 shows three rows with a column per player | TESTING E-1, E-17 and E-8, line 155 ("eleven journeys"), 3379-3382; the e8 spec comment | code wins | Entries reworded; `/admin/allocation` listed as unvisited |
| O7-26 | B-6 | — | Match-test unit tests carried `I-STATS-1/2/9` in their doc comments; TESTING lists them as `U-STATS-1/2/3/3b` | code wins | Doc-comment ids fixed |
| O7-27 | E3-1 | — | Duplicate ids `U-ERR-6`, `I-STATS-9f`, `A-PUBLIC-1c` | code wins | Renamed to `U-ERR-7`, `I-STATS-9i`, `A-PUBLIC-1e`, in TESTING and the tests |
| O7-28 | E3-2 | — | Ids cited in code but undefined: `A-ADMIN-PC-2b`, `I-STATS-9h`, `U-PAIRS-DIV-1`, `F-FMT-13`; wrong `I-STATS-9b` | code wins | Test comments repointed (`I-JOB-14b`, `-9g`, `A-PUBLIC-4f`, `-9e`); `F-FMT-13` entry added |
| O7-29 | E3-3 | `describe('F-FMT-5c scores')` has `scorePct` only | TESTING `F-FMT-5c` claimed a `signedElo` | code wins | Entry fixed |
| O7-30 | E3-3 | `generation_klv` | TESTING `I-LEAVE-8` named `generation_means` | code wins | Fixed |
| O7-31 | E3-3 | History endpoint has no page consumer | TESTING `A-RATE-6`/`6b` described it as feeding the chart | code wins | API-only wording |
| O7-32 | E3-3 | 18 journeys; `/admin/allocation` unvisited | TESTING coverage map "eleven"; tier-5 preamble counts | code wins | Fixed |
| O7-33 | E3-3 | — | `I-SCHED-20` and `A-PUBLIC-6a` lacked *(Covered: …)* markers; `I-STATS-7b` wording | code wins | Fixed |
| O7-34 | E4-R1 | The state repair is §2.3's second `UPDATE tasks` | RUNBOOK:1022 "§2.4's state repair" | code wins | Fixed |
| O7-35 | E4-R3 | `rating_pool.deleted(.census)` is written | RUNBOOK had no procedure to restore a deleted pool | code wins | New RUNBOOK §2.6, run locally (copy restores the pool and members; a rerun fails atomically) |
| O7-36 | E4-R4 | The ops shell has no `python3` or `aws` | RUNBOOK §4 step 1 used both, and only printed the manifest | code wins | Pure-psql count comparison, tested locally |
| O7-37 | E4-R5 | `psql` is available only in the ops task; `aws` only on the operator's machine | RUNBOOK §0's second block mixed them | code wins | Split |
| O7-38 | E4-R6 | Key alias `alias/birdtest-backups` in the replica region | RUNBOOK:1527 "backups-dr key" | code wins | Fixed |
| O7-39 | E4-R7 | — | `$REGION`, `$CLUSTER` and `$BUCKET` were never assigned | code wins | Guarded assignment block from `terraform output` |
| O7-40 | E4-D1 | MAGPIE prints four builder versions, `magpie_version` and the build target | README listed three | code wins | Fixed |
| O7-41 | E4-D2 | `dev.py` has `--sim-games-job`, `--sim-pairs-job` and `-ab` variants | README flag row listed four | code wins | Fixed |
| O7-42 | E4-D3 | `--lexicon` applies to non-`-ab` jobs; `--variant` defaults to none (seed default classic) | README defaults row | code wins | Fixed |
| O7-43 | E4-D4 | The backend loads `backend/.env` | README "without Docker" pointed at the root `.env.example`, which lacks the backend's settings | code wins | Points at `backend/.env.example` and lists needs |
| O7-44 | E4-D5 | — | README's successive `cd backend && …` lines failed when pasted as one block | code wins | Subshells |
| O7-45 | E4-D6 | `e2e/`, `docker/`, `fixtures/`, `contract-fixtures/` and several scripts exist | README layout table omitted them | code wins | Added |
| O7-46 | G-5 | A new stack needs `input_data` before any job | README "Deploying" ended at the first admin | code wins | Step 8 added (import `data-20260925`, the GitHub limit, the derived builds) |
| O7-47 | F-16 | e2e has E-1..E-18 | JOURNEYS.md:11 "E-1..E-15" | code wins | Fixed |
| O7-48 | C-1 | Nothing checked a worker's `BOARD_DIM`/`RACK_SIZE` | PLAN:5219, 5346: "every MAGPIE build the fleet runs has `BOARD_DIM` 15" | doc wins | The server now enforces it (`unsupported_build`) |
| O7-49 | H-1 | Derived-data Retry could not act on `wit` rows | RUNBOOK:1234: "press **Retry** on it" | doc wins | Code fixed so the step works |
| O7-50 | E4-R2 | `job_census` did not count `opening_rack_progress` | RUNBOOK §0: the census holds the counts about to be lost | doc wins | Census adds `rack_standings`; RUNBOOK lists each count's table |
| O7-51 | D-7 | Fleet query scanned every claim ever made | KL-32 / PLAN: "a week of claims" | doc wins | Query bounded to the week through indexes (semantic note: lapsed and declined claims no longer counted); KL-32 revised |
| O7-52 | H-3 | Job list counted racks analysed | PLAN: an opening-rack job is done when every rack is *settled*; the job page shows settled | doc wins | List uses `racks_settled` |
| O7-53 | G-6 | Links could expire with the signing credentials | UI "(links valid for an hour)", PLAN's export paragraph | doc wins | Code refreshes credentials and caps the TTL; text softened to "up to an hour" |
| O7-54 | H-10 | Fake worker reported sim analyses for static players | TESTING E-12/E-13 claimed the journeys prove what pages show for MAGPIE results | doc wins | Fake and specs changed so the guarantee holds |
| O7-55 | H-11 | Committed fake fixtures predated a12fdd7 | testdata README: fixtures are regenerated from the fake; it also said there is no game_pairs fixture (false) | doc wins | Fixtures regenerated with a CI `--check`; README corrected |
| O7-56 | E3-3 (F-CONS-3) | Client skipped the share check at max ≤ 1 but sent the share | TESTING claimed client/server parity | doc wins | The client no longer sends the share at max ≤ 1, so parity holds; entry rewritten |
| O7-57 | B-1 | Rating pools fit pairs results across different `bingo_bonus`/`sim_cutoff` | KL-75: "None changes a rating today; the defaults have not moved" | **unresolved** | The premise is false since 12c11f4. KL-75 now states that. Scope pools by the two settings, or pin them at job creation? Owner |
| O7-58 | B-2 | A consensus edit with unsettled racks reopens a force-completed job and resumes its uncovered first pass | `complete_job` "Completion is final"; PLAN: the edit hands out only unsettled racks | **unresolved** | Comments and PLAN now describe the actual behaviour, marked pending. Should an edit ever undo a force-complete? Owner |
| O7-59 | F-15 | — | `SETTINGS_COMPARISON.md`: 59 of 62 migration line anchors stale; `F-SET-1` cites its display order | **unresolved** | It may be an intentional record: freeze it with a commit stamp, or move the live part into PLAN. Owner |
| O7-60 | H-9 (rest) | The checkbox is "Position Recorder" | `e12-saved-positions.spec.ts:8` comment: "Save the positions played" | open, fixable | Missed by the fixers; carried to iteration 2. Not counted |

**Count: code-wins 47 / doc-wins 9 / unresolved 3** (plus 1 open, fixable, not counted).

---

## Issues and Recommended Solutions

Each entry gives:
- **Context:** where, and how it was found;
- **Problem;**
- **Options** considered;
- **Recommendation / outcome:** what was done, or why it was flagged.

Reviewer ids: A races, B backend bugs, C MAGPIE, D performance and storage, E docs
(E3 TESTING notes, E4 RUNBOOK/README), F dead code, G deployment, H frontend, e2e and
the fake worker.

### Races and locking

**A-1 (medium). A finish check completed an opening-rack job a consensus edit had just unsettled.**
- **Context:** `jobs/mod.rs` `complete_unless_purged`; `routes/worker.rs`
  `after_submission` and `finish_idle_job`; `routes/admin.rs` `update_consensus`.
  Found by reviewer A tracing the edit against the finish checks.
- **Problem:** the opening-rack finish read (`racks_settled >= total_racks`) was
  unlocked. The completion `UPDATE` guarded only `status = 'active'`,
  `claims_issued >= observed` and the purge witness. An edit that committed between
  the read and the update left the job `active` with unsettled racks, and the update
  then completed it anyway. Those racks were never reissued, and the next export was
  marked final. Repeating the edit is a no-op (`new == old`).
- **Options:**
  - re-check the condition inside the `UPDATE`, where Postgres re-evaluates it on the
    committed row;
  - take a lock on the read;
  - have the edit bump a witness.
- **Outcome:** the first option, which is free on the updated row. `I-OR-EDIT-3`
  drives a pre-edit observation through `complete_unless_purged`: it passes with the
  predicate and fails without it.

**A-2 / D-2 (high). The consensus edit stalled the pool.**
- **Context:** `update_consensus` and `restate_racks`. Found independently by reviewers
  A and D.
- **Problem:** the edit holds the dispatch lock and `FOR UPDATE` on every open claim
  for one `UPDATE` over every progress row: up to 3.2 M rows, and minutes on the
  default instance. It held no `DispatchHold`, so every claim waited 2 s and every
  submission 5 s on a connection: the pool exhaustion the eleventh audit fixed for
  purge. It also ran inline, so a dropped request rolled back mid-statement.
- **Options:**
  - reuse `try_hold_claims`. Rejected: it bumps `claims_holds_taken`, the purge
    witness, so an edit would look like a purge to `complete_unless_purged`;
  - add a new `HoldKind`;
  - add an uncounted claims hold;
  - batch the restate.
- **Outcome:** the uncounted hold (`try_hold_claims_uncounted`, which shares
  `try_claims`). The edit:
  - answers 409 when any claims hold is present;
  - runs on `run_to_completion`;
  - checks the purge count only under the row lock, since `refuse_if_purged_since`
    would see its own hold;
  - drops the hold before the post-commit `finish_idle_job`, whose `purged_since`
    closure would otherwise treat the job as purged;
  - never calls `committed()`, so the reclaim grace always applies.

  PLAN "What the edit holds, and for how long" was added. `I-OR-EDIT-4` passes, and
  fails with the hold pointed at another job.

**A-3 (low). Export `mark_ready` versus the edit's `unfinalize`.**
- **Context:** `exports.rs` `mark_ready` and `unfinalize`. Reviewer A.
- **Problem:** in the milliseconds between `unfinalize` and the edit's commit, a
  finishing export read the old `completed` status and stored `is_final = true`.
- **Options:**
  - `FOR SHARE` in the status subquery;
  - have the edit lock `job_exports` rows.
- **Outcome:** `FOR SHARE`. `I-EXPORT-15` passes, and fails without it.

**A-4 (low, doc). The single-instance dependencies were under-documented.**
- **Context:** `infra/variables.tf` `desired_count`, `infra/ecs.tf`, PLAN:9578, KL-82.
  Reviewer A.
- **Problem:** enforcement was correct (0..1, min 0% / max 100%). But the text an
  operator reads before raising the count named only imports, rate limits and SSE.
  Missing were `DispatchHolds`, the purge witness, the startup reapers,
  `RECENTLY_BUSY`, `BUILDING`, `MERGE_TURNS` and the debounce.
- **Options:** doc only, or add enforcement. Enforcement already exists.
- **Outcome:** descriptions rewritten; KL-82's consequence list completed (W5).

**A-5 / F-1 (medium). The one-slot invariant was not in the schema; the redundancy-era indexes were unreachable.**
- **Context:** migration `task_claims` indexes; `scheduler.rs` `issue_claim`;
  `registry.rs` `next_available`. Found by reviewers A, D (S4) and F.
- **Problem:** the counters' correctness rests on at most one claimed-or-completed
  claim per task, which only the claim path's locks guaranteed, and
  `GREATEST(x - 1, 0)` hid any drift. The two per-identity unique indexes cost a write
  per claim and per state change, but could never fire. The `Retry` on their violation
  and the `NOT IN ('abandoned','declined')` arm were dead.
- **Options:**
  - make `task_claims_open_idx` unique;
  - add one index over `state IN ('claimed','completed')`;
  - just drop the per-identity indexes.
- **Outcome:** the one-slot index, which also covers completed claims, plus the counter
  CHECKs. The open index stays non-unique for the reclaim scan. Every writer was
  traced first. An `issue_claim` error is now `Fatal`, since a retry would re-select
  the same task. The `acquire` retry is kept for `(job_id, seed)` races. PLAN updated
  ("Two things restart an attempt" is now one). `I-SCHED-15` and `-16` updated;
  `I-SCHED-22` added.

**A-6 (low, comment). `DerivedCache` soundness comment.**
- **Context:** `derived.rs:386`. Reviewer A.
- **Problem:** the comment says "nothing deletes" a `derived_data` row, but
  `delete_input_data` does.
- **Options:** reword the comment.
- **Outcome:** reworded, after checking the pin count in `delete_input_data` (W2b).

**A-OK (checked and found correct).**
- **Claim path:** dispatch lock with a 2 s bound, then task `SKIP LOCKED`, then claim
  insert, then task, then a guarded job `UPDATE`.
- **No double dispatch:** `available` only, under the lock; the `(job_id, seed)`
  backstop; leave reissue only in the current generation; opening-rack reissues exclude
  in-flight racks.
- **Submission:** claim (5 s, 55P03 → 503), then task, then store, then claim, then
  counters, then contributor, then job.
- **Decline, heartbeat and reclaim:** `SKIP LOCKED` throughout; post-restart and
  post-hold graces.
- **Purge and delete:** in-process hold, then merge, then dispatch, then claims, then
  job, then pool fit locks in id order, then contributor give-back.
- **Lifecycle actions:** the activation lock is taken only after job rows.
- **Leave merges and transitions.**
- **Ratings versus admin:** the fit lock is taken first.
- **Exports:** `FOR SHARE` start and a single snapshot.
- **`backup.sh`:** one exported snapshot.
- **Derived builder:** lease `SKIP LOCKED`.
- **Input-data import:** insert-only, FK `NO ACTION` against new pins.
- **Denormalized counters:** all single-statement.

### Backend bugs and validation

**B-1 (medium; KL-75 revisited). Rating pools mix different bingo bonuses and sim cutoffs. Pending the owner.**
- **Context:** `ratings.rs` `build_matrix` and `evidence_games`; `routes/admin.rs`
  `CreateJobBody`; the new-job form. Reviewer B, via `git log -S` to 12c11f4
  (2026-10-01).
- **Problem:** pools are scoped by `(variant, letterdist_id, layout_id)`. Since
  12c11f4, every job takes `bingo_bonus` and `sim_cutoff` from the body, and the form
  edits both. The fit pools head-to-heads played under different rules as if they were
  one question. KL-75 accepted this because "the defaults have not moved", which is no
  longer true.
- **Options:**
  - add `bingo_bonus` and `sim_cutoff` to `rating_pools` and filter on them in both
    queries;
  - refuse non-default values for pairs jobs at creation (weaker);
  - leave it and label pools.
- **Recommendation:** the first, which keeps admins free to run variants.
- **Outcome:** behaviour unchanged. KL-75 rewritten to state the current situation and
  the options. PLAN:5321 corrected (E-11).

**B-2 (low). A consensus edit reopens a force-completed job. Pending the owner.**
- **Context:** `update_consensus` reopen logic; `complete_job`; `opening_rack.rs`
  `next_request`. Reviewer B.
- **Problem:** `reopened = completed && unsettled > 0` counts only racks that already
  have progress rows. A job the admin force-completed mid-first-pass is reopened, and
  `next_request` resumes the uncovered first pass: possibly millions of racks the admin
  deliberately stopped. A force-completed one-analysis job is never reopened. The
  behaviour hinges on an incidental count.
- **Options:**
  - reopen only server-completed jobs (`actor_user_id IS NULL` on the newest
    `job.completed` audit row);
  - reopen only when `racks_analyzed >= total_racks`;
  - reopen a force-completed job `inactive`.
- **Recommendation:** reopen only server-completed jobs, and report the restated counts
  otherwise.
- **Outcome:** behaviour unchanged. The stale "Completion is final" comments were fixed
  (`complete_job`, `deactivate_job`, `complete_finished`, the reopen comment). PLAN
  describes the actual behaviour and marks it pending (W1).

**B-3 (low). Divergent subset not cross-checked.**
- **Context:** `game_pair.rs:91-103`; MAGPIE `play_autoplay_game_or_game_pair`.
  Reviewer B.
- **Problem:** impossible divergent and whole combinations were accepted, and fed the
  divergent-games table.
- **Options:** add the identities, or leave it.
- **Outcome:** `check_divergent_against_the_whole`. Real `result-game-pairs.json`, the
  test bodies and the fake all pass. `U-PLAUS-8` added; PLAN validation list updated.

**B-4 (low). Analysis kind not checked against the dispatched players.**
- **Context:** `plausibility.rs` `check_analysis`; `registry.rs` `decode_result`.
  Reviewer B. Also surfaced by H-10.
- **Problem:** a static player could report sim iterations and win %, a non-solver
  PEG or endgame positions, and so on: impossible for an honest MAGPIE.
- **Options:**
  1. attribute each position to its exact mover (game index, turn, first-mover rule);
  2. check against either player;
  3. also check iterations against `max_iterations` and PEG fidelity against
     `endgame_plies`.
- **Outcome:** option 2, which has no false positives, plus "a non-sim move has no
  iterations or plies".
  - The iterations bound was rejected: an empty-bag sim's limit is its move count and
    `sample_minimum` can exceed it.
  - The PEG-fidelity bound was rejected: the PEG schedule takes no `endgame_plies`.
  - The fixer judged exact-mover attribution too fragile to promise no false positives.
    It is an **open call for the owner** (stronger but riskier).
  - Real MAGPIE fixtures pass. Eight tests that submitted sim data for static players
    were fixed.
  - `U-PLAUS-6` and `-7` added; `A-WORKER-12` extended.

**B-5 (low; builds on KL-87). Match-test counts had no schema CHECKs.**
- **Context:** the `job_game_config` and `job_game_pair_config` DDL;
  `jobstats.rs:803-811` (`as u64`). Reviewer B.
- **Problem:** a direct insert with a negative `max_games` casts to about 1.8e19, so
  the job never completes. A batch of 0 regenerates the same seed. The schema default
  batch was 1 (odd, KL-87).
- **Options:**
  - add CHECKs;
  - clamp in Rust;
  - add an evenness CHECK too.
- **Outcome:** CHECKs added and the default set to 2. No evenness CHECK: 25 tests
  deliberately use batch 1, so evenness stays an API rule. That is an **open call for
  the owner**, recorded in KL-87. `I-JOB-1f` added.

**B-6 (low, doc).** Covered as O7-26.

**B-7 (low, comment).** Covered as O7-3.

**B-8 (low). Names without a length or control-character rule.**
- **Context:** `routes/ratings.rs` pool create; `routes/admin.rs` player-config
  create. Reviewer B.
- **Problem:** a newline, a bidi override or kilobytes in a name breaks the public
  layouts. Untrimmed pool names were treated as distinct.
- **Options:** reuse the job-name rule, or add a separate rule.
- **Outcome:** shared `name_problem` and `MAX_NAME_CHARS` (100); pool names trimmed;
  `CHECK (char_length(name) <= 100)` on both tables (PLAN copy identical).
  `A-ADMIN-2c` added.

**B-9 (low). Public rack lookup unbounded.**
- **Context:** `routes/public.rs` `rack_lookup`. Reviewer B.
- **Problem:** with consensus, up to 100 analyses × 32,767 moves are returned in one
  unauthenticated response.
- **Options:**
  - cap moves per analysis;
  - page by analysis;
  - require sign-in.
- **Outcome:** proportional cap `rank <= i16::MAX / analyses`. One analysis is listed
  whole, every analysis keeps its top moves, and no frontend change was needed.
  `I-SCHED-21` and `A-PUBLIC-4` updated.

**B-OK (checked and found correct).**
- The match-test maths (two-sided Robbins mixture CS, the tuning, pre-specified
  boundaries).
- `outcomes.rs` moments; the verdict stored with completion; no SPRT logic left.
- `validate_job_body`; the pentanomial cross-check is complete.
- Bradley–Terry gradient and Hessian, damping and standard errors.
- `ratings.rs` locking and thinning.
- CSRF on every mutating route; sessions and API keys; `extract.rs` and `error.rs`.
- Rate limits and client IP; SSE caps.
- `config.rs` validation and the startup floor check; `racks.rs` unranking.
- Registry decode off the executor; exports; artifacts; `inputdata::classify`.

### MAGPIE arguments and `birdtest-contribute`

**C-1 (medium). `RACK_SIZE` and `BOARD_DIM` not negotiated.**
- **Context:** MAGPIE `rack_defs.h`, `Makefile`, `static_eval.h:193`,
  `contribute.c:909-940`; birdtest `ClaimBody`, `leave_gen::RACK_SIZE`, PLAN:5219.
  Reviewer C.
- **Problem:** a contributor's non-default build passes every digest and plausibility
  check and corrupts results silently (`RACK_SIZE`), or fails five times opaquely
  (`BOARD_DIM`).
- **Options:**
  - the server negotiates through the claim body;
  - MAGPIE refuses to start when the dimensions are non-default.
- **Outcome:** claim-body negotiation (coordinator), with the reasoning under
  "Coordinator decisions".
  - Server: `routes/worker.rs::unsupported_build` runs right after the body parses, and
    the 400 message names the new fields.
  - MAGPIE: the claim body carries both fields, and `print_shutdown` gives rebuild
    advice.
  - Every claim sender was updated: test helpers, e2e, `e2e_magpie.py` and the fake.
  - Fixtures: `claim-request.json` and the new `shutdown-unsupported-build.json`.
  - PLAN (several sections, plus KL-48) and TESTING updated.
  - Tests: `A-WORKER-22`, the contract tests, and the MAGPIE tests. They pass at
    `BOARD_DIM` 15; 21 was not run.

**C-2 (low, doc).** Covered as O7-20.

**C-3 (low). Sim margin forecast not reset.**
- **Context:** MAGPIE `config.c` contribute resets. Reviewer C.
- **Problem:** the one outcome-affecting setting left to a built-in default.
- **Options:** reset it to off, or add a per-player request key.
- **Outcome:** reset to off, per player and run-wide, with getters; leak tests
  extended. A key can be added if birdtest ever wants the forecast.

**C-4 (low, doc).** Covered as O7-9.

**C-5 (low). Fail-open "older server" fallbacks in MAGPIE.**
- **Context:** `contribute.c:337-358`; `config.c` unpinned wordmap and NULL defaults;
  claims without `job_id`. Reviewer C.
- **Problem:** unreachable against birdtest and untested, and one branch ran tasks
  unverified. The owner rule says no compat shims.
- **Options:** remove them, or keep them and test them.
- **Outcome:** removed, each confirmed unreachable first. Failures are now
  server-error or `derived_mismatch` paths. PLAN and README sidecar mentions removed.
  `test_an_unverifiable_assignment_is_refused` added.

**C-6 (low). Stale MAGPIE comments and a dead parameter.**
- **Context:** `contribute.h`, `contribute_defs.h`, `contribute_test.c`;
  `contribute_submit_result`'s `fatal`. Reviewer C.
- **Outcome:** five comments corrected. `fatal`,
  `ERROR_STATUS_CONTRIBUTE_UNKNOWN_JOB_TYPE` and the never-referenced
  `…_MAGPIE_TOO_OLD` removed.

**C-7 / F-10 (low). Contract fields one side never reads.**
- **Context:** MAGPIE `autoplay_results.c` captured positions; birdtest
  `OpeningRackRequest.previous_play` and its column; `GameRequest.game_pairs`;
  `CONTRIBUTE_KEY_GENERATION`. Reviewers C and F (F asked whether per-position effort
  is wanted data).
- **Options:** drop them, or store them (per-position effort).
- **Outcome:** all dropped on both sides, with fixtures recaptured and edited.
  Per-position sim effort is not consumed anywhere. `time_elapsed` is
  hardware-dependent. The owner can ask to store them later.

**C-OK (checked and found correct).**
- The full settings table in (c).
- The protocol routes and decline reasons match both sides.
- The four executors and every dispatched feature.
- The server-side MAGPIE commands (`builders`, `convert …`, `createdata klv`) match.
- The contract fixtures were identical across repos before the change and are again
  after it.
- No SPRT, chi-square or self-update remnants in MAGPIE.

### Performance and storage

**D-1 (high). Reissue walk under the dispatch lock.**
- **Context:** `opening_rack.rs` `next_reissue`. Reviewer D.
- **Problem:** see (e) #1.
- **Options:**
  - (a) a bounded candidate window;
  - (b) store the identities that analysed each rack on its progress row;
  - (c) a hashed in-flight exclusion.
- **Outcome:** (a) and (c). Option (b) does not bound the walk: an identity that has
  seen every rack still steps over all of them. Measured on PG16 with 1 M racks (W1):
  - old query, custom plan: 5.7–6.0 s;
  - old query, generic plan: more than 120 s (25 s for the second query alone at
    50,000 in flight);
  - new query: 12–13 ms with nothing in flight, 17–18 ms at 5,000, 42–50 ms at 50,000.

  Two rows added to PLAN's measured table. `I-OR-REISSUE-1` added.

**D-2 (high).** Covered under A-2.

**D-3 (medium). Default batch sizing versus the claim rate limit. Pending the owner (KL-93).**
- **Context:** form defaults (`batchSize = 1`, 2 for games); API and schema defaults;
  `ratelimit.rs` (1 claim/s, burst 5); MAGPIE's 429 handling. Reviewer D.
- **Problem:** see (e) #3.
- **Options:**
  - size defaults from the player configs (for example 100 pairs static versus static,
    1 for a sim);
  - add a note at the field;
  - keep per-job pentanomial running totals for the rating sweep only (display, so
    KL-10's objection does not apply).
- **Recommendation:** duration-sized defaults, plus the form note.
- **Outcome:** recorded as KL-93 and cross-referenced from the rate-limit table. The
  trade-off (overshoot past a test's stopping point and the cost of a lapsed claim,
  against per-task overhead) is the owner's.

**D-4 (low). `task_claims.job_id` unindexed and unread. Pending the owner (schema rework).**
- **Context:** the migration; the readers listed in reviewer D's index map (reclaim,
  `should_check_finish`, `claims_in_flight`, ETA, KL-58, `lock_open_claims`).
- **Problem:** job-scoped claim reads go through `tasks` and fleet-wide partial
  indexes.
- **Options:**
  - (a) one-slot unique: done as A-5;
  - (b) `task_claims_job_open_idx (job_id) WHERE claimed`, and rewrite the reads;
  - (c) re-key the completed index to `(job_id, completed_at)`.
- **Recommendation:** (b) and (c), measured before and after on the claim path; update
  KL-4 and KL-58.
- **Outcome:** flagged. It is a hot-table schema trade-off that needs measurement.

**D-5 (low). Leftovers of "first accepted result per task".**
- **Context:** `game_results_task_idx` and FK; `leave_records.task_id` FK and index;
  comments; KL-10 and the measured table. Reviewer D (index map S1 and S3).
- **Problem:**
  - a two-column index serves only a cascade;
  - the cascades from `tasks` are redundant (results also cascade from their claim and
    job);
  - the cost text describes a sort that no longer exists.
- **Options:**
  - shrink the index;
  - drop the FK and the index;
  - re-measure.
- **Outcome:** index shrunk to `(task_id)`; comments fixed; KL-10 and the measured
  table marked as history with the plain sum "not re-measured".
- **Flagged:**
  - dropping the `game_results.task_id` and `leave_records.task_id` FKs (subtractive,
    pre-release, but outside the fixer's brief);
  - re-measuring `game_stats`/`game_pair_stats` at 400,000 rows, at batch 1 and batch
    100. That needs a synthetic full-volume corpus.

**D-6 (low). `position_analysis_records_rack_idx` guards nothing the code does not check first.**
- **Context:** migration:1295; `opening_rack.rs` `check_batch_against_task`;
  `registry.rs:398`. Reviewer D (index map S2).
- **Problem:** about 3.2 M entries (100+ MB) per full English job, plus an index write
  per record inside the task lock.
- **Options:** drop it, or keep it as defence in depth.
- **Outcome:** kept (coordinator), as a backstop against a decoder regression. Recorded
  in PLAN's schema narration.

**D-7 (low; KL-32 revisited). Admin reads on the main pool; fleet scanned all history.**
- **Context:** `routes/admin.rs` `fleet`, `audit_log`, `delete_player_config`;
  `db.rs`. Reviewer D.
- **Options:**
  - move display reads to `read_pool`;
  - bound the fleet query by an index;
  - accept a statement timeout.
- **Outcome:** both of the first two for fleet; audit log moved to `read_pool`.
  - Data gaps, derived-data and backups views were left on the main pool: small tables,
    a judgment call.
  - `delete_player_config`'s full scans are documented in its comment as accepted
    (rare, admin-only).
  - **Note for the owner:** the fleet page no longer counts claims that lapsed or were
    declined in the week. Revert to a `claimed_at` index if those should count. KL-32
    records this.

**D-8 (low). An export's snapshot pins the vacuum horizon. Pending the owner (KL-94).**
- **Context:** `exports.rs:551-600`; `EXPORT_BUILD_LIMIT` (6 h). Reviewer D.
- **Problem:** while an export runs, heartbeats go non-HOT, merge dead tuples are not
  vacuumed, and dead rows accumulate.
- **Options:**
  - document it;
  - keyset-paginate in short transactions with a finality witness.
- **Outcome:** KL-94 added, and KL-7 points to it. Not changed: the single snapshot is
  what makes "final" sound.

**W1 side observation (low). JIT on the reissue query's generic plan.**
- **Context:** W1's measurement on PG16 defaults.
- **Problem:** about 75 ms spent on JIT compilation per reissue claim under a generic
  plan.
- **Options:**
  - `jit = off`, or a higher `jit_above_cost`, in the RDS parameter group;
  - per-session settings;
  - leave it.
- **Outcome:** recorded in the PLAN cost row and flagged. It is a production database
  parameter.

**Index-map observations (S5–S8, low, not acted on).**
- `input_data_role_name_idx` has no reader.
- The `rating_pools` 4-column unique is implied by `name UNIQUE`.
- `audit_log_job_idx` could be partial.
- `derived_data_queue_idx` sits on a tiny table.
- All are harmless and on small or low-rate tables. Recommendation: drop the first two
  in a later pass (pre-release, subtractive), and leave the others.

**D-OK (checked and found correct).**
- The critical-path trace in (d).
- Thinning exists and works.
- Leave merges and selection are indexed.
- `generation_klv` streams.
- No blocking file I/O on request paths.
- The long main-pool holders are bounded.
- Opening-rack submits use indexed statements.
- `estimate_eta`, the feeds and the rack lookup are indexed.
- Backups and drills hold, and so do the S3 lifecycles.
- MAGPIE caches data and digests per file identity.
- Every CASCADE FK is indexed.

### Docs (objective 7)

Every E, E3 and E4 item, H-8, and the doc halves of C-2, C-4, D-5, D-7, F-4, F-13,
F-14, F-16 and G-5 are recorded one per row in the Objective 7 table above (O7-1 to
O7-60). Each row gives the context (file and line, from reviewers E, E3, E4, B, C, D, F,
G or H), what the code does, what the doc said, the decision and the reasoning. They
are summarised here:
- **Doc-only fixes (code wins):** 47. They were made by W6, plus W4 (E-5, the 1F
  counts, F-FMT items), W5 (E-6, the compose header) and W2b (test-id comments, B-6).
  W6 verified them:
  - `runbook-check.sh`: 32 and 19 blocks pass;
  - PLAN's schema copy is byte-identical to the migration;
  - there are no duplicate KL ids;
  - the new RUNBOOK §2.6 and §4 blocks were run against a throwaway Postgres.
- **Behaviour fixes (doc wins):** 9, each covered under its own id in this section.
- **Unresolved:** 3 (B-1, B-2, F-15).
- **Open and fixable:** 1 (O7-60).

The options for every doc row were the same: update the doc, or change the code. The
default "code wins" applied wherever the code's behaviour was correct.

### Dead and out-of-date code

**F-1.** Covered under A-5.

**F-2 (low). Unreachable `ON CONFLICT` re-matching in `insert_position_analyses`.**
- **Context:** `jobs/mod.rs:508-605`. Reviewer F.
- **Options:** remove it, or keep it as a silent backstop.
- **Outcome:** removed, so a duplicate now fails loudly on the unique index. The plies
  `ON CONFLICT` was also removed. PLAN updated in three passages. Covered by the
  existing submit and capture tests.

**F-3.** Covered as O7-3.

**F-4 (low). Rating-history endpoint has no consumer. Keep or remove is pending the owner.**
- **Context:** `routes/ratings.rs` `pool_history`, its caps and comments. Reviewers F
  and H, and E3 (`A-RATE-6`).
- **Problem:** the chart that fed the endpoint was removed (52c7d1a). Text everywhere
  still described the chart, and the caps cited a deleted file.
- **Options:**
  - (a) delete the endpoint, its tests and its authz row;
  - (b) keep it as a documented API with caps justified as payload bounds.
- **Outcome:** all the text was fixed (W2b, W4, W6). PLAN:580 already records the
  endpoint as intentionally kept for API callers. **Owner:** keep it or delete it.

**F-5.** Covered as O7-7.

**F-6 (low). Unused backend dependencies.**
- **Context:** `backend/Cargo.toml`. Reviewer F, by grep; confirmed by building.
- **Outcome:** removed and trimmed as listed in (g). `Cargo.lock` regenerated with
  `cargo metadata`; it only loses packages, and no version changed. `clippy --locked`
  is clean.

**F-7 (low). `@types/d3-scale` orphaned.**
- **Outcome:** `npm uninstall`; the lockfile loses two packages; check, test and build
  pass.

**F-8 (low). `ENDGAME_PLIES` never read.**
- **Options:**
  - delete it;
  - have the frontend mirror it.
- **Outcome:** deleted, with `PEG_MAX_BAG`, which is unused for the same reason. The
  form's `endgamePlies = 6` gained a comment naming MAGPIE's `eplies` default.

**F-9 (low). `num_moves` compatibility shim.**
- **Context:** `handler.rs:356-361, 571-596`. Reviewer F, with
  `git merge-base --is-ancestor` in MAGPIE showing every 0.1.1 build sends it.
- **Outcome:** required. The fallback and its test were replaced by "an analysis
  without its ranked count is malformed". Six test bodies updated; `U-WIRE-3` adjusted;
  PLAN updated, including an API example that was itself implausible (plies without a
  win %). `MoveEntry.iterations` is left optional (tidying, not needed).

**F-10.** Covered under C-7.

**F-11 (low). `results_accepted` duplicated `tasks_completed`.**
- **Outcome:** removed from `JobStats`, its `SUM`, the frontend type and PLAN. e2e
  `waitForResults` and two backend tests switched to `tasks_completed`.

**F-12 (low). `/api/admin/jobs/:id/results/stream` has no consumer. Pending the owner.**
- **Context:** `routes/admin.rs:47` → `public.rs` `job_results_stream`, documented at
  PLAN:5238. Reviewer F, route-consumer sweep.
- **Options:**
  - keep it as a scripted bulk-read admin API and name it in RUNBOOK/README;
  - remove the route and handler, keeping the shared export queries.
- **Recommendation:** keep it only if operators will script against it; otherwise
  remove it.
- **Outcome:** unchanged.

**F-13 (low). `client-version`.** Not dead. Covered as O7-22.

**F-14 (low). Fake-worker dead modes and flags.**
- **Options:**
  - remove them;
  - keep them, documented as ad-hoc debugging aids.
- **Outcome:** removed (W2a). The reviewer's "human" flag was low-stakes, nothing
  invoked them, and the Rust tests cover declines. Covered as O7-23 for the PLAN
  sentence.

**F-15 (low). `SETTINGS_COMPARISON.md` line anchors stale. Pending the owner.**
- **Context:** the whole file; 59 of its 62 anchors are wrong (scripted check).
  `F-SET-1` cites its display order.
- **Options:**
  - freeze it as a record, with commit stamps and the anchors stripped;
  - move the display-order tables into PLAN and archive the rest.
- **Recommendation:** move the live part into PLAN.
- **Outcome:** unchanged.

**F-16 (low). Root-level plan documents. Banners pending the owner.**
- **Context:** `FEATURE_BATCH_PLAN.md` (implemented, superseded, no status banner, 36
  SPRT mentions); batches 2 and 3 ("Status: implemented"); `ENDGAME_PEG_PLAN.md`
  (implemented, framed around redundant claims); `JOURNEYS.md` (live).
- **Options:**
  - add an "implemented; historical" banner;
  - move them to `docs/history/`;
  - leave them as they are.
- **Recommendation:** a banner on `FEATURE_BATCH_PLAN.md` at least.
- **Outcome:** only JOURNEYS.md:11 fixed (O7-47).

**F-OK (checked and found correct).**
- **Repo-wide greps:**
  - no SPRT, α/β or Elo-hypothesis remnants outside deliberate history;
  - no priority tiers or weighted random;
  - no chi-square except a why-not comment;
  - no self-update;
  - no TODO, FIXME or HACK markers.
- **Rust items:** every one used except F-8's.
- **Ignored tests:** documented, and run nightly.
- **Routes:** every other route has a consumer.
- **Frontend and e2e:** every component and export used.
- **Schema, env vars, Terraform and scripts:** every table, env var, variable, local
  and script has a reader.
- **Fixtures and CI paths:** every fixture referenced; every CI path exists.
- **Docs:** every coverage name resolves.
- **Python:** no unused imports.

### Deployment

**G-1 (low). Version floor copies unchecked.**
- **Context:** eight copies (`config.rs`, `variables.tf`, both compose files,
  `e2e_magpie_native.sh`, the migration's defaults, `dev.py`). Reviewer G.
- **Problem:** a raise that misses Terraform passes CI and admits old builds in
  production. A Terraform copy raised above the pin takes production down at deploy
  (`minimum_healthy_percent = 0`).
- **Options:**
  - a test reading every copy;
  - an extra CI step comparing Terraform's default with MAGPIE's `config.c`.
- **Outcome:** `DEFAULT_MIN_MAGPIE_VERSION` const plus `U-CFG-5`, which reads all eight
  copies and the `ecs.tf`/`derived.tf` wiring (`U-CFG-4` was taken). The CI step was
  not added: tier 5's startup check already compares compose against the pin, and
  `U-CFG-5` ties Terraform to compose.

**G-2 (low; KL-62 revisited). Validations never exercised in CI.**
- **Context:** `infra/`, CI `terraform` job. Reviewer G. KL-62's reasoning predates
  mock providers.
- **Outcome:** `infra/tests/variables.tftest.hcl` with 50 runs: 6 plans and 44
  `expect_failures`, using a mock provider with an alias and mock data. It adds the
  three KL-62 cross-variable validations and `terraform test` in CI.
  - Verified with `hashicorp/terraform:1.9.8` in Docker: `fmt`, `init`, `validate`,
    and `test` 50/50.
  - Mutation checks: two.
  - `outputs.tf` `ses_dkim_records` made mock-safe.
  - Terraform 1.9 skips a validation that reads a refused variable; the test header
    documents this.
  - KL-62 closed. `S-TF-1` and `-2` added to TESTING.

**G-3 (low). No ECS circuit breaker.**
- **Context:** `infra/ecs.tf` `aws_ecs_service.main`. Reviewer G.
- **Options:**
  - a circuit breaker with rollback;
  - also `wait_for_steady_state`.
- **Outcome:** the breaker was added. Migrations are additive, so the previous revision
  can run. The RUNBOOK "Rolling back a deploy" paragraph and a README note cover how to
  tell it happened and that Terraform state still names the bad revision.
  `wait_for_steady_state` (making apply block and fail on rollback) is **pending the
  owner**: it changes the deploy workflow.

**G-4 (low).** Covered in (b).

**G-5 (low, doc).** Covered as O7-46.

**G-6 (low). Presigned links and temporary credentials.**
- **Context:** `artifacts.rs` `presigned_get`; `exports.rs` `DOWNLOAD_URL_TTL`.
  Reviewer G.
- **Options:**
  - document it;
  - refresh the credentials before signing;
  - shorten the TTL.
- **Outcome:**
  - The credentials are refreshed per link (`IdentityCache::no_cache`) and the TTL is
    capped (`presign_ttl`).
  - A deprecated SDK accessor that always returned `None` was replaced with the stored
    provider; the new test caught it.
  - UI text "up to an hour" (coordinator); PLAN states the rule.
  - `U-ART-1` added.
  - Residual: ECS's rotation timing cannot be verified offline.

**G-7.** Covered as O7-7.

**G-OK (checked and found correct).**
- The startup floor check.
- ECS env versus `config.rs`.
- IAM against every AWS call.
- SSM secrets, kept out of state.
- ALB health checks and rules.
- RDS settings and TLS.
- The backup, drill and ops tasks.
- Fresh account and region: names, AZs, partition, ACM, SES.
- RUNBOOK §1, §2 and §5 identifiers.
- Rollback on an additive schema.
- Images: pin fetch, build target, smoke check.
- CI and nightly read the pin with a guard.
- Auth cookies and CSRF.
- No half-finished features.

### Frontend, e2e and the fake worker

**H-1 (high).** Covered in (b).

**H-2 (medium, false positive). Rack lookup "Unicode sort versus distribution order".**
- **Context:** `public.rs` `rack_lookup`; `racks.rs`. Reviewer H.
- **Problem as reported:** German and Polish racks never found.
- **Investigation:** `LetterDistribution::parse` sorts tiles by character, so stored
  racks are already in code-point order, blank first, and the old sort found them.
- **Outcome:** made the coupling explicit with `RackIndex::spelling`. `canonical_rack`
  was not used, because it puts the blank last. `U-RACK-12` covers a German-like
  distribution. KL-43 gains a "not a limit" bullet.

**H-3 (low).** Covered in (b) and O7-52.

**H-4, H-5, H-6 (low).** Covered in (b).

**H-7.** Covered as O7-24.

**H-8.** Covered as O7-25.

**H-9 (low). Stale e2e comments and `run.sh --help`.**
- **Outcome:** the compose comment and `run.sh` were fixed (W5), and the e8 comment
  (coordinator).
- **Still open:** the e12 comment (O7-60). It is fixable, and is carried to iteration 2.

**H-10 (high). The fake reported sims for static players.**
- **Context:** `worker/fake_worker.py` position and move builders; the E-12 and E-13
  specs; MAGPIE `autoplay_results.c`. Reviewer H, by emitting a result from the static
  game-pairs assignment.
- **Problem:** tier 5 asserted sim columns on data MAGPIE cannot produce for that
  config, and never proved the static rendering.
- **Options:**
  - shape results by the mover's config;
  - change only the specs.
- **Outcome:** both (W2a). Results are shaped as in (h). E-12 uses simmers; E-13
  asserts the static case. TESTING updated. The plausibility check (B-4) would now
  refuse the old shape.

**H-11 (medium). Stale fake fixtures.**
- **Context:** `backend/src/jobs/testdata/`, its README; a12fdd7. Reviewer H.
- **Problem:** `U-FAKE-1` and `-3` tested a shape the fake no longer sends, so
  `check_inference` never saw the fake's inferences.
- **Options:**
  - regenerate the fixtures;
  - regenerate and add a guard.
- **Outcome:** regenerated through the new `scripts/fake-worker-fixtures.sh`. Added a
  divergence fixture and a `--check` step in CI. `U-FAKE-1`, `-2b`, `-3` and `-6`. The
  README was rewritten. The fake imports `requests` lazily so `--emit-fixture` runs on
  a bare Python.

**H-12 (low). Fake transport differs from MAGPIE.**
- **Outcome:**
  - 5xx and 429 are retried after `Retry-After`;
  - a claim 429 waits;
  - new `rate_limited` and `unavailable` stats.
  - Heartbeats are moot, since the fake now submits at once and never holds a claim
    (noted in its code).

**H-13 (low). Smaller fake and e2e gaps.**
- **Outcome:** `--server-url` required; `firstClaim(api, jobId)` asserts the job.
- **Not done:**
  - the leave-generation KLV fetch and verify in the fake. Not cheap, and there is no
    leave journey. Low; recommended only if a leave journey is added;
  - `syntheticResult` for capture and opening racks, which is not needed now that
    `firstClaim` is pinned to a games job.

**H-OK (checked and found correct).**
- Every `api.*` wrapper's route and method.
- Response shapes against the serde structs.
- Request bodies against the `Deserialize` structs.
- No SPRT, redundancy or priority leftovers in the frontend.
- CSRF; error handling; SSE resubscribe.
- The admin guard and login `next`; the allocation page.
- `F-*` ids present.
- The fake's wire protocol; the e2e specs; `MIN_MAGPIE_VERSION` consistent.

### Open items carried forward

**Pending the owner** (genuine trade-offs; they do not count as fixable for the loop):
- **B-1:** pool scoping (KL-75).
- **B-2:** force-complete versus consensus edit.
- **D-3:** batch defaults (KL-93).
- **D-4:** `task_claims` job-scoped indexes.
- **D-5:** the FK drops and the KL-10 re-measurement.
- **D-8:** export snapshot versus vacuum (KL-94).
- **F-4 / A-RATE-6:** the history endpoint.
- **F-12:** the results stream.
- **F-15 and F-16:** the root plan docs.
- **G-3:** `wait_for_steady_state`.
- **W2a:** the even-batch CHECK (B-5) and exact-mover attribution (B-4).
- **Also noted for the owner, not blocking:**
  - the D-7 fleet semantic change;
  - E-14's tree completeness;
  - production `jit` (W1);
  - bumping the version and floor together before a first deployment (PD-9).

**Fixable, carried to iteration 2:**
- the e12 spec comment (O7-60);
- the `routes/admin.rs:1660` `1..=7` literal;
- the index-map S5 and S6 drops.

**Blocking CI until done (owner action):** push `birdtest-contribute` with
`c3c6a875`.
