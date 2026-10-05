# AUDIT_FINDINGS_30 — the thirty-third audit (2026-10-05), pass 2 (iteration 2)

Branch `audit/birdtest-2026-10-05`, the same branch as iteration 1. Iteration 1's
work is committed on it as `fe701c8`, `deaacd8` and `e529018`; this iteration
audited `e529018` and commits on top of it.

MAGPIE: `~/MAGPIE` is on `birdtest-contribute`. The iteration started at
`c3c6a875` (iteration 1's commit), which `docker/Dockerfile` pinned. This
iteration's MAGPIE changes are committed on `birdtest-contribute` as
`1a0ae932`, and the Dockerfile pin moves to that commit.
`MAGPIE_VERSION` stays 0.1.1, and so does `MIN_MAGPIE_VERSION` in every copy.

> **STILL NOT PUSHED.** Neither `birdtest-contribute` nor the birdtest branch
> is on a remote; `origin/birdtest-contribute` is still `dbf8df3b`. Until
> `birdtest-contribute` (now with both `c3c6a875` and `1a0ae932`) is
> pushed, three things fail at the step that fetches the pin:
> - CI's `magpie-contract` job;
> - the e2e image builds (tier 5) in CI;
> - the nightly job.
>
> As in iteration 1, the pin cannot be reverted to the pushed commit either:
> the server refuses a pre-`c3c6a875` claim (no `board_dim`/`rack_size`) with
> a 400.

**Builds on** AUDIT_FINDINGS_29 (iteration 1 of this audit, same branch), and
through it AUDIT_FINDINGS_7 to _28, and PLAN.md's Known Limits KL-1 to KL-94.
**No KL is added this iteration** (no fix report added one), so the set is
still KL-1 to KL-94. KLs revisited (text only, none renumbered): KL-48
(narrowed, C2-2), KL-82 (consensus edit named, A2-4), KL-32 (the fleet's
"open" claims, E2-10) and KL-62 (closure wording, ET2-9).

**Process.** The run follows `audit_loop.md`. This file records iteration 2,
which ran in two stages:
1. Nine read-only reviewers, each re-auditing its whole scope and reading
   iteration 1's diff (`git diff 0149541..e529018`) in it:
   - A: races and locking;
   - B: backend bugs;
   - C: MAGPIE arguments and `birdtest-contribute`;
   - D: critical path, performance and storage (with measurements);
   - E: PLAN against the code, and prompt drift;
   - E2: TESTING, RUNBOOK and README;
   - F: dead and out-of-date code;
   - G: deployment;
   - H: frontend, e2e and the fake worker.

   An API spend limit interrupted the reviewers once; they resumed with their
   context intact, so no scope was lost.
2. Five fixers, working in one tree:
   - X1: consensus edit, reissue query and exports (A2-1..A2-4, D2-1, D2-2,
     D2-4, ET2-3);
   - X2: MAGPIE and plausibility (B2-1, B2-2, B2-3, C2-1, C2-3, F2-10);
   - X3: backend cleanup (B2-4/F2-3, C2-4, C2-6, D2-3/F2-4, F2-5, F2-6, F2-8,
     F2-9, ET2-2, ET2-4, ET2-6);
   - X4: frontend and infra (H2-1..H2-4, F2-7, G2-1..G2-4);
   - X5: docs (E2-1..E2-10, ET2-5, ET2-7, ET2-9, C2-2, C2-5, F2-1).

**Findings this iteration, after de-duplication:** 0 high, 5 medium, 46 low.
- **Medium:**
  - B2-1: an exhaustive PEG schedule was accepted at creation, and every
    position it produced was then refused as implausible;
  - A2-1: an export spanning a consensus edit's reopen and the job's
    re-completion was stored final with the pre-edit corpus;
  - G2-1: a circuit-breaker rollback was silent;
  - D2-1: the reissue's in-flight read walked the job's whole reissue history
    under the dispatch lock;
  - D2-2: the reissue's "unseen first" sort could be planned as a hash over
    the identity's whole history.
- **Duplicates merged:** B2-4 = F2-3 (the `1..=7` literal); D2-3 = F2-4 (the
  S5/S6 index drops); E2-9 = ET2-8 = F2-2 = H2-3 (iteration 1's O7-60, the
  e12 comment).

Every medium is fixed. Every low is fixed, ET2-1 (TESTING's status table
counts) by the coordinator after the fixers, except E2-1's optional MAGPIE half (moving a platform `#if` into `src/compat/`).
Three of iteration 1's carried fixable items are closed: the e12 comment
(O7-60), the `1..=7` literal and the S5/S6 index drops.

**Loop decision:** iteration 2 found and fixed issues, so iteration 3 runs a
full audit.

---

## Pass summary

### (a) Branch, prior findings and versions

- **Branch:** `audit/birdtest-2026-10-05` (unchanged; one branch per loop run).
- **Prior findings:** as listed in AUDIT_FINDINGS_29 (a): `_7`–`_26` on
  `audit/birdtest-2026-09-24-pass21`, `_27`/`_28` in `main`'s history until
  `7e74935`, and `_29` on this branch (local, unpushed). So this file is
  number 30.
- **What iteration 1 left open, and what happened to it:**
  - Fixable, carried (all three closed this iteration):
    - the e12 spec comment (O7-60): fixed by X4 (O7i2-20 below);
    - `routes/admin.rs`'s `1..=7` literal: fixed by X3 (B2-4/F2-3);
    - the index-map S5 and S6 drops: done by X3 (D2-3/F2-4).
  - Pending the owner: unchanged and not re-flagged (see "Open items carried
    forward").
- **KLs added:** none. **KLs revisited:** KL-48, KL-82, KL-32, KL-62 (text
  only).

### Coordinator decisions on flagged items

- **B2-1, exhaustive PEG.** Chosen: plausibility accepts what MAGPIE
  legitimately produces, rather than refusing exhaustive PEG at job creation.
  A PEG move's depth is bounded at 40 (`PEG_EXHAUSTIVE_PLIES`) for the
  exhaustive schedule `[INT_MAX]`, and at the schedule's own reach otherwise
  (stages + 1); an endgame move stays at 25. Rejected: capping each stage at
  `PEG_CAND_LIST_CAP`, which would have made a MAGPIE mode unreachable from
  birdtest for no correctness reason.
- **B2-2, the bingo bonus ceiling.** `0..=500`, at creation and as a schema
  CHECK. Real variants use 0 to 50; the plausibility constants assume an
  ordinary bonus. Scaling the plausibility bounds by the job's bonus was
  rejected: `process_response` deliberately has no job context.
- **B2-3, a simmer's forced turn.** Fixed in MAGPIE: a turn with one legal
  play is recorded as a static analysis of that play. **No version bump**,
  under the owner rule that nothing is deployed and both sides change
  together. The wire shape is unchanged (`analysis: "static"` already
  existed), and birdtest's plausibility already accepts a static position from
  any player.
- **C2-1, `movegen_margin` and `recorder_type`.** The docs are corrected, and
  the margin half of the I-JOB-2 refusal is dropped: MAGPIE's autoplay never
  reads either setting, so refusing a games job over a margin difference
  refused something that changes nothing. The win% half of the refusal stays
  (a real shared setting).

### Coordinator post-fix integration

The full suite on the combined tree surfaced two problems, both fixed by the
coordinator:
- **X2's PEG schedule bound refused a test fixture.** The new rule (U-PLAUS-9:
  a schedule of L halving stages ranks at most L+1 plies deep) refused
  `backend/tests/submissions.rs`'s `solved_positions_keep_their_analysis_spread_and_depth`,
  whose PEG player had the schedule `{8,4}` but whose fixture reports a
  depth-4 PEG move. MAGPIE cannot produce that. The bound was verified in
  MAGPIE's `peg.c`: stages run at `stage_idx` 1..`num_stages`, each at
  `stage_idx + 1` plies, and the exhaustive `[INT_MAX]` schedule runs at
  `PEG_EXHAUSTIVE_PLIES` (40). So the rule is right and the fixture was
  impossible; its schedule became `{8,4,2}` (three stages, reaching 4 plies),
  with a comment citing U-PLAUS-9.
- **A new README bash block lacked `export AWS_PAGER=""`.** X4's alarm check
  (`aws events test-event-pattern`) calls `aws`, and `runbook-check.sh`
  requires every block that calls `aws` to disable the pager. Added.

### (b) Bugs and fixes made in birdtest

#### Race conditions

Reviewer A re-traced the documented lock order on every path, including
iteration 1's consensus-edit hold, the one-slot index and the export
`FOR SHARE`. **No lock-order deadlock was found**, and iteration 1's A-1, A-2,
A-3 and A-5 fixes hold. The races below are not ordering bugs.

| ID | Sev | Race | Fix | Test |
|---|---|---|---|---|
| A2-1 | medium | An export whose REPEATABLE READ snapshot was taken while its job was completed (`is_final = true`), then a consensus edit reopened the job, its unsettled racks were reissued and analysed, and the job completed again before the upload finished: `mark_ready` read the new `completed` status and stored the pre-edit corpus as final. A second export cannot be requested while one runs, so the stale artifact was what the results stream served. Iteration 1's A-3 (`FOR SHARE`) covered only the edit-in-flight window. | `exports::unfinalize` (called by the edit) also fails the job's running exports (`state = 'failed'`, "the job reopened while this export was building: export it again"). `mark_ready`'s `WHERE state = 'running'` then matches nothing, and its existing branch removes the objects. Lock order checked: `mark_ready` waits on the job row in its SET subquery, then re-checks the updated export row (EvalPlanQual). | `I-EXPORT-15` changed (now expects `failed`); new `I-EXPORT-16` (fails without the fix) |
| A2-2 | low | The consensus edit took its `DispatchHold`, the dispatch lock and every open claim before validating the request, so a 400 (games job, bad share) or a no-op 200 still made claims skip the job, answered submissions 503, and left the reclaim grace behind: lapsed claims of that job, a games job included, were not reclaimed for five minutes. | `update_consensus` loads the job and config without locks and refuses a wrong job type, an invalid merged body, or answers a no-op, all before taking the hold. `consensus_body` re-validates under the locks through the same helper (`consensus_request`). | New `I-OR-EDIT-5` (fails without the fix) |
| A2-3 | low | After the edit's commit, `request_derived_data(...)?` and the final `load_job(...)?` could answer 5xx for a committed edit, skipping the post-edit finish check and push. | Post-commit failures are logged; the finish check and push still run; a failed reload answers from the committed row, as `purge_body` does. | None (a post-commit pool failure is not injectable without test hooks) |
| A2-4 | low (doc) | KL-82, PLAN's primary/secondary note and `desired_count`'s description named only purge, delete and seeding as `DispatchHolds` users; iteration 1 made the consensus edit a third. | All three name the consensus edit. | — |

The paths A checked and found correct are under A2-OK in the Issues section.

#### Other bugs fixed

| ID | Sev | Bug | Fix | Test |
|---|---|---|---|---|
| B2-1 | medium | `peg_stage_top_k: [2147483647]` (MAGPIE's exhaustive mode) was accepted at creation, but MAGPIE then tags every graded play at fidelity 40 and `check_analysis` refused anything over 25. A capturing job with such a player had every task near the end refused on every retry: a false positive, which the plausibility design rules out. | `check_analysis` bounds PEG at 0..=40 (`MAX_PEG_FIDELITY_PLIES`) and endgame at 0..=25 (`MAX_ENDGAME_FIDELITY_PLIES`). `check_analyses_against_players` adds a per-job bound: a PEG position no deeper than the deepest PEG player's schedule reaches (`deepest_peg_fidelity`: 40 for `[i32::MAX]`, else stages + 1; an unknown schedule gets the widest). Creation is unchanged (it matches MAGPIE: 1–16 stages, 2..`INT_MAX`, non-increasing), with a comment. | New `U-PLAUS-9`; `U-PLAUS-5` extended (PEG 41, endgame 26 refused) |
| B2-2 | low | `bingo_bonus` had no upper bound, while plausibility's bounds are absolute: from about 1,700 a batch's mean score passes `MAX_SCORE_MEAN`, and higher values trip the rack-mean and move-score bounds, then overflow MAGPIE's `int32` equity. A typo (5000 for 50) wedged the job on false positives. | `validate_job_body` refuses outside 0..=500 (`MAX_BINGO_BONUS`), by field; schema `CHECK (bingo_bonus BETWEEN 0 AND 500)`; the form's input has `max="500"`; PLAN states the bound and why. | New `I-JOB-1g` (API and CHECK) |
| B2-4 / F2-3 | low | The rack size was written three times: `leave_gen::RACK_SIZE`, `plausibility::MAX_RACK_TILES` and the literal `1..=7` in the opening-rack validation. | `leave_gen::RACK_SIZE` is the one copy; the validation and its message use it; `MAX_RACK_TILES` removed. | New unit test `routes::admin::tests::an_opening_rack_is_one_tile_to_a_full_rack` |
| H2-1 | low | After sign-in, a 502/503 from `/api/me` (exactly what a deploy produces) left the store at `null` (already resolved by the login page's earlier 401) with no retry scheduled, so the guard silently sent the now-signed-in user back to `/login`. The import watcher's `signedOut` had the same shape. | New `resetSession()` (store to `undefined`, pending retry reset, then `refreshSession()`), called by the login page and the import watcher. | New `F-AUTH-3` (3 cases) |
| H2-4 | low | The player-config and rating-pool name inputs had no `maxlength`, unlike the job name, though iteration 1's B-8 holds all three to 100 characters. | `maxlength="100"` on both. | — (copy-level; the server's refusal is `A-ADMIN-2c`) |
| G2-1 | medium | See (k). | | `S-TF-3` |
| G2-3 | low | See (k). | | `S-TF-1` (3 new runs) |

### (c) MAGPIE argument coverage (objective 3)

Reviewer C re-traced every outcome-affecting setting from the server's rows to
the executors' reads at `c3c6a875` (the full table is in C's report and
matches iteration 1's (c) except as below). The task config is fresh per task,
`settings.txt` reaches only data paths, and in `contribute.txt` only `threads`
affects outcomes (KL-14, accepted). Iteration 1's C-1 (build dimensions), C-3
(margin forecast reset) and C-5 (no fail-open fallbacks) hold.

| ID | Sev | Gap | Fixed |
|---|---|---|---|
| C2-1 | low | A games, pairs or leave player's `recorder_type` and `movegen_margin` are never read by MAGPIE's autoplay: static players generate with margin 0 and `BEST` (or `ALL` when capturing), simmers with `ALL`, solvers with margin 0. Only an opening-rack static analysis reads them. PLAN called both result-changing for games, and job creation refused a games job whose players disagreed on the margin. No result was wrong; the refusal was needless and the docs misleading. | Yes, both sides. birdtest: `validate_shared_player_options` compares only the win% model; `I-JOB-2` now asserts two margins are accepted; PLAN's key table, the "shared setting" paragraph and the "Every setting that can change a result" rows corrected; the player-config form's margin hint corrected. MAGPIE: the games executor no longer applies the margin (the key is still required, so the protocol is unchanged). |
| C2-3 | low | Three C-5 remnants in MAGPIE: (1) a simmer with no string `win_pct_model` silently loaded the build's default table, which is not in `expected_data` and so unverified (reachable with a non-string value); (2) dead NULL guards and a contradictory comment in `validate_common`, and a test passing NULL for the distribution and layout; (3) `expected_data_matches` skipped a malformed or unknown-role entry instead of refusing it. None reachable from the current server. | Yes (MAGPIE): a NULL win% name is a `CONTRIBUTE_SERVER_ERROR`; the dead guards and comment fixed and the test passes real names; a claim whose `expected_data.files` is not an array, or has an entry missing a field or with an unknown role, is refused like an unknown algorithm. `test_an_unverifiable_assignment_is_refused` gains four refused cases and checks all four assignment fixtures pass. |
| B2-3 | low | A simmer's turn with one legal play (typically a forced pass) runs no simulation, but `simmed_this_turn` was still true, so the captured position carried the thread's previous simulation: another position's plays, iterations, plies and inference, stored as `analysis: "sim"`. Every birdtest check passed it. | Yes (MAGPIE): `get_top_simming_move` reports whether it simulated; `AutoplayWorker.turn_was_simmed` drives `simmed_this_turn`, `inferred_this_turn`, the print-boards output and the sim-iteration counter. A forced turn is recorded static, one play, no inference. New `test_a_forced_turn_is_not_simulated`; `test_inferring_players_report_their_inference` tightened. |

The determinism claims in the docs remain accurate: only static, non-solving
players are deterministic; sims, solves and multithreaded leave runs are not.

### (d) Critical-path and async changes (objective 4)

Reviewer D re-traced claim, heartbeat, decline, submit and completion. Nothing
purely observational runs inline beyond the documented exceptions (KL-5,
KL-28, the display columns of the job `UPDATE`); nothing needed moving.
Ratings are still neither read nor written on the claim, submit or completion
path. The match-test debounce is safe (every 8th submission, any submission
that finds no open claim, the 10 s idle check, guarded by the purge witness).
Iteration 1's new plausibility checks cost O(positions) with tiny constants:
low single-digit milliseconds at the 64 MiB ceiling.

Changes this iteration that touch the path:
- **Reissue (D2-1, D2-2).** The two reissue reads under the dispatch lock no
  longer depend on the job's history or on planner statistics: 2–5 ms and
  about 25 ms where they could reach hundreds of milliseconds or seconds.
- **Consensus edit (A2-2).** A refused or no-op edit takes no hold and no lock,
  so it no longer makes claims skip the job, answers submissions 503, or parks
  lapsed claims behind the reclaim grace.
- **Decode (B2-1).** One more per-position comparison (PEG depth against the
  schedule) in `check_analyses_against_players`: negligible.
- **Export (A2-1).** One extra `UPDATE job_exports` inside the edit's
  transaction, admin-side only.

### (e) Performance, most to least severe (objective 5)

Measured by reviewer D and re-measured by X1 on PG16.15 at default settings
(`jit` on): 1 M unsettled racks, 3 M analysis records over 10 identities,
200,000 completed reissue tasks and 100 open, plus another job's 2 M tasks and
1,000 open claims.

| # | ID | Issue and expected impact | Status |
|---|---|---|---|
| 1 | D2-2 | The reissue's `ORDER BY EXISTS (…)` ("unseen first") is an AlternativeSubPlan. With fresh statistics Postgres picks the bounded per-row probe; with understated statistics (a database just restored by `pg_restore`, which in PG16 carries no statistics; autovacuum behind; fast growth) it picks a hashed subplan over the identity's whole history: **0.9–2.0 s per claim**, up to **5.3 s** with the table never analysed, all under the dispatch lock, with other claims waiting up to 2 s and going `Busy`. | **Fixed**: a scalar `(SELECT 1 … LIMIT 1) IS NOT NULL`, which Postgres never hashes: 25–27 ms under the same bad statistics, 22–23 ms with fresh ones. RUNBOOK §5 step 3 gains a `vacuumdb --analyze-only` before the service starts. |
| 2 | D2-1 | The reissue's in-flight read scanned `tasks_seed_unique_idx` over every reissue task the job ever made and filtered on `state` in the heap: 27–51 ms at 200,000 completed reissues, linear, so about 150–300 ms per claim at a million (a consensus job at batch 10 makes 0.6–1.3 M reissues), under the dispatch lock, on every claim of the reissue phase including `NoWork` ones. | **Fixed**: read from the open tasks only (`tasks_queue_idx` for `available` reissues, `task_claims_open_idx` for claimed ones): 2–5 ms, independent of history. Same 5,000 racks returned. |
| 3 | A2-2 | A refused or no-op consensus edit parked a job's lapsed claims for a heartbeat timeout (5 min) and answered its submissions 503 while it held the claim locks. | **Fixed** (validated before the hold) |
| 4 | W1 note (AF29) | JIT on the reissue's generic plan. D2-2's measurement narrows it: under `plan_cache_mode = auto` sqlx's prepared statements stayed on custom plans (0 generic of 8–9 executions), so the JIT cost needs a forced generic plan. | **Narrowed**; production `jit` remains the owner's call |
| 5 | D2-3 | Two indexes with no reader: an index write per insert on two tiny tables. | **Fixed** (dropped); negligible |

PLAN's measured-cost table gains "as it was" / "as it is now" rows for both
reissue reads, the reissue row says "with current statistics", and the stale
"Public worker list, all claims | 93 ms" row is labelled "as it was" (D2-4).

### (f) Storage (objective 6)

**Fixed:**
- `input_data_role_name_idx` dropped (no reader) and the four-column
  `UNIQUE (variant, letterdist_id, layout_id, name)` on `rating_pools` dropped
  (implied by `name UNIQUE`). Iteration 1's S5 and S6 (D2-3/F2-4).
- `jobs.deactivated_at` dropped: written twice, read only by one test, and
  duplicated by the `job.deactivated` audit row (F2-5).
- `CHECK (bingo_bonus BETWEEN 0 AND 500)` (B2-2).

**Flagged:** nothing new. D-4, D-5, D-8 (KL-94) and D-3 (KL-93) remain with
the owner.

**Verified:** no new unbounded growth from iteration 1; every
`ON DELETE CASCADE` FK still has an index leading with its column; the
backup, drill and round-trip scripts name no dropped index or column, and the
round-trip seed satisfies the one-slot index; thinning unchanged.

### (g) Dead or out-of-date code removed, and items flagged as uncertain

**Removed (each verified against its callers, then by compiler and tests):**
- `input_data_role_name_idx` and the `rating_pools` four-column unique
  (D2-3/F2-4).
- `jobs.deactivated_at`, its two writes and the `Job` field (F2-5). X3 found
  the one reader the reviewer missed, a test, and changed it to assert the
  audit row.
- axum's unused `macros` feature; `Cargo.lock` drops `axum-macros` (F2-6).
- The frontend devDependency `tslib`, which nothing required (F2-7).
- `plausibility::MAX_RACK_TILES`, a copy of `leave_gen::RACK_SIZE` (F2-3).
- The inert `mock_resource "aws_sesv2_email_identity"` block in
  `infra/tests/variables.tftest.hcl`, whose comment was false (G2-2).
- MAGPIE: the games executor's movegen-margin application (C2-1), and the C-5
  remnants (C2-3).

**Made live instead of removed:** MAGPIE's `chttp_is_available()` was defined
three times and never called (F2-10). `impl_contribute` now calls it first and
fails at once with `ERROR_STATUS_HTTP_UNAVAILABLE` ("libcurl was not found")
instead of spending the first claim's retry budget (about 15 minutes) on a
transport error.

**Stale comments and text corrected:**
- `scheduler.rs` and `derived.rs`: a claim without a derived-file pin is a
  `derived_mismatch` decline on every worker, not an unchecked fallback (C2-4).
- `leave_gen.rs`: `klv::FullRackLeaves` no longer exists; MAGPIE's
  `convert rackequity2klv` builds the KLV (C2-6).
- The outer claim-retry comment, `JobClaimError::Retry`'s doc, `error.rs`'s
  `db_code` doc and PLAN's "One thing restarts an attempt": the retry now
  covers only leave generation's random seeds and `LostRace`; the dispatch
  lock rules out cursor-seed races, and the generation transition no longer
  restarts an attempt (F2-8).
- `scheduler.rs` reclaim and release: "at capacity" and the unannotated
  `'claimed'` arm are now described as drift defence under one slot (F2-9).
- `SETTINGS_COMPARISON.md:71`: the dropped `previous_play` column and MAGPIE's
  removed refusal (F2-1). Only that row; the anchors stay with F-15.
- `e2e/tests/e12-saved-positions.spec.ts:8`: "Position Recorder" (O7-60).

**Flagged as uncertain or for the owner:** nothing new. F-4, F-12, F-15 and
F-16 remain pending.

### (h) Python-worker-as-production-client corrections

None needed. Reviewers F and H found no code, doc, config or CI step that
treats `worker/fake_worker.py` as a production or dev client. Reviewer H
compared the fake's key sets with real MAGPIE fixtures: every key the fake
sends is one MAGPIE sends (only the solver fields `mean_spread` and
`fidelity_plies` are real-only, since the fake never solves), and B-3 and B-4
hold by construction. `scripts/fake-worker-fixtures.sh --check` passed for
reviewers G and H (under Python 3.8). B2-3 changes no shape the fake emits.

### (i) `birdtest-contribute` changes, pin and version floor

All of these are committed on `birdtest-contribute` as `1a0ae932`,
on top of `c3c6a875`, and are **not pushed**.

| Change | Why | Files |
|---|---|---|
| `get_top_simming_move` reports whether it simulated; a forced turn is recorded static (one play, no inference); the sim counters and print-boards output follow | B2-3: a forced simmer turn carried a stale simulation | `simmer.c`, `simmer.h`, `autoplay.c`, `contribute_test.c` |
| The games executor no longer applies the movegen margin, with a comment saying why | C2-1: autoplay never reads it | `config.c` |
| A simmer with no win% model name is a server error; `validate_common`'s dead NULL guards and comment fixed; malformed or unknown-role `expected_data` entries refused at claim time | C2-3: the C-5 remnants | `config.c`, `contribute.c`, `contribute.h`, `contribute_test.c` |
| `impl_contribute` checks `chttp_is_available()` first | F2-10: dead function, and a missing libcurl looked like a transport error | `config.c` |

- **Pin.** `docker/Dockerfile` `ARG MAGPIE_COMMIT=` moves from `c3c6a875` to
  `1a0ae932`. CI (two jobs) and the nightly read it from that line.
- **Version and floor.** `MAGPIE_VERSION` stays 0.1.1 and `MIN_MAGPIE_VERSION`
  stays 0.1.1 in every copy (U-CFG-5 now also reads `backend/.env.example`).
  B2-3 changes what a worker submits for a forced simmer turn, which under a
  deployed system would call for a bump of both; it is not bumped under the
  owner rule (nothing deployed; both sides change together). PD-9 stands: bump
  both before a first deployment.
- **Protocol and fixtures.** No key added or removed on either side (a forced
  turn's `analysis: "static"` is an existing value; the margin key stays
  required), so `contract-fixtures/` were not recaptured and stay
  byte-identical to MAGPIE's `test/birdtest_contract/`.
- **Derived data.** No builder or conversion code changed, so builder
  versions and derived-data hashes are unaffected.
- **MAGPIE checks (X2):**
  - **Run:** `python3 format.py --write` (clang-format-20);
    `make magpie_test BUILD=dev -j3`, clean with `-Werror`;
    `./bin/magpie_test contribute`, passed (7 m 11 s; the first run crashed in
    the new test, which had not loaded a win% table, and was fixed);
    `./bin/magpie_test autoplay`, passed (3 m 54 s);
    `make magpie BUILD=portable_release -j3` (only the existing LTO
    stringop warnings in `move.h`).
  - **Not run:** cppcheck and clang-tidy (standing owner instruction); the
    `BOARD_DIM=21` build; the wasm tests.
  - **Correction to AUDIT_FINDINGS_29 (C2-7).** AF29 (i) said the contribute
    changes were "written to be BOARD_DIM-agnostic" and that "CI's 21 shard is
    the first real check". That is wrong: at `BOARD_DIM == 21`, MAGPIE's
    `test/test.c` `main` runs only `run_all_super()` (position lengths, bit
    racks, the super board layout) and ignores its arguments. The `contribute`
    suite never runs in a 21 build, in MAGPIE's CI or anywhere. For the
    contribute changes (iteration 1's and this one's), CI's 21 shard is **a
    compile check only**; the contribute suite is 15-only by MAGPIE's design.
    No code defect follows: a 21 build is answered `unsupported_build` by the
    server (`A-WORKER-22`). AF29 itself is not edited. The test comment at
    `contribute_test.c:1302` ("loads at either BOARD_DIM") still implies
    coverage that does not exist; rewording it is optional.

### (j) Findings file and count

`AUDIT_FINDINGS_30.md` — objective-7 discrepancies this iteration:
**code-wins 26 / doc-wins 10 / unresolved 0** (O7i2-36, ET2-1, TESTING's
status table, was regenerated by the coordinator at commit). Iteration 1's three unresolved discrepancies (B-1, B-2, F-15) are
unchanged and are not counted again.

### (k) Deployment blockers

No high deployment blocker. Reviewer G re-ran `terraform fmt -check`,
`validate` and `test` (50/50 before the fixes) on Terraform 1.9.8, and
re-checked IAM, ECS environment, ops scripts, RUNBOOK names and the
Dockerfile (G2-OK).

| ID | Issue | Resolution |
|---|---|---|
| G2-1 | A circuit-breaker rollback (iteration 1's G-3) was silent: no EventBridge rule on ECS deployment events, `-down` alarms need 10 minutes, `apply` returns at once. Meanwhile Terraform state names the bad revision (the next apply of any kind redeploys it), the derived builder runs the abandoned image (`arn_without_revision`), so a moved builder version leaves derived rows unbuilt (the KL-62 symptom, reached without an operator mistake), and every dump's manifest names the abandoned image. | **Fixed**: `aws_cloudwatch_event_rule.deploy_failed` (`SERVICE_DEPLOYMENT_FAILED` on the service's ARN) and a target on the alerts topic, with an `input_transformer` quoting the reason and pointing at RUNBOOK "Rolling back a deploy" and the builder skew. No IAM change (the topic policy already admits EventBridge). `S-TF-3` asserts the pattern; a mutation check fails it. RUNBOOK, README ("Check that the alarms reach you", with an `aws events test-event-pattern` check) and PLAN updated. Independent of the pending `wait_for_steady_state`. |
| G2-3 | `min_magpie_version` was the one value the backend refuses at startup that Terraform never checked: `v0.2.0` or `0.2.0-rc1` planned and applied, then the web task failed three launches (silently rolled back) and the builder failed every five minutes. | **Fixed**: a validation mirroring `Version::parse_strict` (2–3 all-digit parts, each at most 9 digits so it fits i32). Three `S-TF-1` runs (`0.2` plans; `0.2.0-rc1` and `v0.2.0` refused). `terraform test` 54/54. |
| G2-4 | README said images go to "a registry of your choice", but no task definition sets `repositoryCredentials`, so a private registry outside ECR fails every task with `CannotPullContainerError`. | **Fixed (doc)**: README and RUNBOOK §5 say ECR (any region; another account's repository must admit this one) or a public registry. |
| G2-2 | The test file's SES mock block was inert and its comment false. | **Fixed**: block and comment deleted; suite still passes. |
| — | README's new alarm-check block lacked `export AWS_PAGER=""`. | **Fixed** by the coordinator (post-fix integration). |
| — | The MAGPIE pin is not pushed. | **Open**: push `birdtest-contribute` before CI, e2e images and the nightly can pass (header). |

**Terraform variables with no default:** still eight (PD-1).

### (l) Prompt drift

Iteration 1's items, re-verified (reviewer E, with G and C):

| # | Status now | Checked in |
|---|---|---|
| PD-1 | **Holds.** Eight no-default variables: `backend_image`, `frontend_image`, `alert_email`, `acm_certificate_arn`, `mail_from_address`, `ses_domain`, `public_url` (`variables.tf`) and `derived_builder_image` (`derived.tf`). The prompt lists five | script over every `variable` block |
| PD-2 | **Holds.** The builder builds wordmaps, rack info tables and word info tables; leave-generation KLVs are built by the web process | migration, `derived.rs` header |
| PD-3 | **Holds, widened.** `plausibility.rs` also holds `check_analyses_against_players` (and now the PEG schedule bound); the pentanomial cross-check stays in `game_pair.rs` | grep |
| PD-4 | **Holds, with additions.** CI: nextest archive in 4 partitions; `terraform fmt -check`/`validate`/`test`; `fake-worker-fixtures.sh --check`; MAGPIE `contribute` plus `builderhash`. Nightly: `--run-ignored ignored-only`, `restore-roundtrip.sh`, `restore-job-check.sh`, `reapply-check.sh` **and `backup-drill-check.sh`** (missing from AF29's PD-4) | ci.yml, nightly.yml |
| PD-5 | **Holds.** Neither `dev.py` nor tier 6 has a fake-worker option to refuse | grep |
| PD-6 | **Holds, dated.** `AUDIT_FINDINGS_29.md` now exists on the local, unpushed audit branch, so this file is `_30` | `ls`, `git log` |
| PD-7 | **Resolved** by iteration 1's CHECKs and one-slot index | migration |
| PD-8 | **Holds.** The web process checks the floor at startup; `bin/build-derived.rs` does not (KL-82) | — |
| PD-9 | **Holds, sharper.** `MAGPIE_VERSION` is still 0.1.1 and every floor copy 0.1.1, while 0.1.1 builds before `c3c6a875` get a 400, and this iteration changes a forced turn's captured output without a bump. Bump both before a first deployment | MAGPIE log, U-CFG-5 copies |

New this iteration:

| # | `audit_loop.md` says | The repository says | Checked in |
|---|---|---|---|
| PD-10 | Snapshot: "birdtest `main` at `0149541`, MAGPIE `birdtest-contribute` at `dbf8df3`" | `main` is still `0149541`. MAGPIE `birdtest-contribute` is at `c3c6a875` (iteration 1) and now `1a0ae932` (this iteration), which the Dockerfile pins; both are local only, and `origin/birdtest-contribute` is still `dbf8df3b`. Expected mid-loop (reviewer C's note: the Snapshot's MAGPIE commit is now two commits behind) | `git rev-parse`, Dockerfile:6 |
| PD-11 | "Admin operations (purge, delete, activate) interact with claims through dispatch locks, a per-job merge lock, and an in-process `DispatchHolds` set" | Holds come from seeding, purge and delete, and the opening-rack **consensus edit** (`try_hold_claims_uncounted`). Activate, deactivate, complete, `set_allocations`, export start, merge and artifact rebuild take no hold; they **refuse with 409** while one is held. The consensus edit belongs in objective 2's race list | `grep dispatch_holds` |
| PD-12 | MAGPIE version control is "a server-side version floor … and version negotiation" | Negotiation also has a **build** axis: every claim states `board_dim`/`rack_size`, and anything but 15/7 gets the `unsupported_build` shutdown before the floor is checked. Objective 3's `BOARD_DIM`/`RACK_SIZE` example is now enforced server-side | `routes/worker.rs` |
| PD-13 | Objective 9's example: a `docker-compose.e2e.yml` comment about driving jobs "to an SPRT verdict" | Fixed in iteration 1; no `sprt` remains outside history comments | grep `-i sprt` |
| PD-14 | "`PLAN.md` (design, ~10k lines…)"; "audited 30+ times" | PLAN was 10,777 lines and TESTING 4,406 at `e529018`; this is the thirty-third audit, iteration 2 | `wc -l` |

All other Snapshot claims were verified true again (reviewer E's list: stack
and dark-mode SPA, fake worker as test tooling only, allocation-only deficit
scheduling, one slot, four job types, the match test off by default with
debounce 8, ratings siloed, `worker_bans`, input-data pinning, the floor
copies, the Dockerfile pin invariant, no `is_admin` API, one migration, the
documented scripts, seven tiers).

### (m) Checks run and not run

See **Checks run at the end of iteration 2** below for the end-of-pass
results.
- **Reviewers** ran no builds, except: G ran Terraform 1.9.8 in Docker on a
  scratch copy (`fmt`, `validate`, `test` 50/50, plus probe variants) and
  `fake-worker-fixtures.sh --check`; H ran `fake-worker-fixtures.sh --check`;
  D ran its measurements on a throwaway database, since dropped.
- **Fixers**, each under the shared build lock:
  - X1: clippy clean; 76 targeted nextest tests; a fail-first check reverting
    all four fixes at once (all five new or changed tests failed), then
    restoring them byte-identical.
  - X2: clippy clean; 35 + 97 targeted nextest tests; `npm run check` and
    `npm test`; MAGPIE as in (i).
  - X3: clippy `--locked` clean; 320 + 51 targeted nextest tests; PLAN schema
    blocks diffed against the migration.
  - X4: `npm run check`, `npm test` (214 at the time), `npm run build`;
    Terraform `fmt`, `init`, `validate`, `test` 54/54, with a mutation check.
  - X5: docs only; `runbook-check.sh` was run by X1 after its RUNBOOK edit
    (33 blocks).
- **Not run in iteration 2:**
  - **Tier 5 (Docker/Playwright)** and the full **tier 6**: deferred to the
    final green run (loop step 3), as in iteration 1. The e12 spec edit is a
    comment.
  - **The opt-in `magpie_leave` Rust tests.**
  - **MAGPIE cppcheck and clang-tidy** (standing owner instruction), the
    `BOARD_DIM=21` build (a compile check only, per C2-7) and the wasm tests.

---

## Checks run at the end of iteration 2

Run on the combined tree after all five fixers finished, with heavy commands
serialized behind one build lock:

| Check | Result |
|---|---|
| `cargo clippy --locked --all-targets -- -D warnings` | clean |
| `cargo test --locked --doc` | ok |
| `cargo nextest run --locked --run-ignored all` | first run: 1 failure (`submissions::solved_positions_keep_their_analysis_spread_and_depth`, U-PLAUS-9 refusing an impossible fixture; fixed, see (b)); rerun **707 / 707 passed** |
| frontend `npm run check` / `npm test` / `npm run build` | 0 errors / **214 / 214** / built |
| `scripts/runbook-check.sh RUNBOOK.md README.md` | first run failed on README's new `aws` block (no `AWS_PAGER`); fixed; 33 and 20 blocks parse |
| `scripts/dev-restore-check.sh` | passed |
| `scripts/fake-worker-fixtures.sh --check` | every fixture is what the fake emits |
| contract fixtures vs MAGPIE `test/birdtest_contract/` | 18 byte-identical |
| tier 6, `scripts/e2e_magpie_native.sh` (rebuilt `portable_release` MAGPIE) | **15 / 15 cases passed** (M-1..M-7, M-9..M-16), after iteration 1 and again after iteration 2 |
| Terraform 1.9.8 (Docker) `fmt`, `validate`, `test` (fixer X4) | clean / ok / **54 / 54** |
| MAGPIE `format.py`, `find_circ_deps.py` (clean copy) | no differences / no cycles |
| MAGPIE `make magpie_test BUILD=dev`, `./bin/magpie_test contribute` and `autoplay` (fixer X2) | clean build, passed |

**Not run, and why:** tier 5 (Playwright) is deferred to the final green run
(loop step 3), run natively, because the Docker images need the owner's
approval to build here; MAGPIE cppcheck and clang-tidy are never run on this
machine (the owner's standing instruction); MAGPIE's BOARD_DIM=21 build and the
wasm tests were not run this iteration.

---

## Objective 7 — every code-versus-doc discrepancy found in iteration 2

Decision key (as in AF29):
- **code wins:** behaviour kept, and the doc, comment or test note was updated
  to match it.
- **doc wins:** behaviour changed to what the doc says or needs.
- **unresolved:** neither changed beyond describing the situation; it awaits
  the owner.

Ids are `O7i2-N` to keep them distinct from AF29's `O7-N`.

| # | ID | Code does | Doc said | Decision | Reasoning |
|---|---|---|---|---|---|
| O7i2-1 | A2-4 | Consensus edits take a `DispatchHold` (iteration 1) | KL-82, PLAN's primary/secondary note and `desired_count`'s description named only purge, delete, seeding | code wins | Lists completed |
| O7i2-2 | C2-1 | MAGPIE's autoplay never reads a player's `movegen_margin` or `recorder_type`; only opening-rack static analysis does | PLAN called both result-changing for games and the margin a shared run-wide setting; `I-JOB-2` asserted the margin refusal | code wins | MAGPIE's behaviour is the truth; birdtest's margin refusal, which followed the doc, was dropped (coordinator), and PLAN, the form hint and I-JOB-2 rewritten |
| O7i2-3 | C2-2 | MAGPIE's tests pin the claim body and every shutdown reason | KL-48: only assignment shapes are pinned; the rest is caught by hand | code wins | KL-48 retitled and narrowed to the decline body and `print_shutdown` |
| O7i2-4 | C2-4 | A claim without a derived pin is a `derived_mismatch` decline | `scheduler.rs`/`derived.rs` comments: the worker "falls back" unchecked | code wins | Comments reworded (C-5 removed the fallback) |
| O7i2-5 | C2-5 | The claim body requires `board_dim`/`rack_size`; a non-15/7 build gets `unsupported_build` first | PLAN client-loop step 2 omitted both | code wins | Step rewritten |
| O7i2-6 | C2-6 | MAGPIE's `convert rackequity2klv` builds the KLV | `leave_gen.rs` doc pointed at a nonexistent `klv::FullRackLeaves` | code wins | Comment rewritten |
| O7i2-7 | D2-4 | The public worker list reads per-identity counters | PLAN's cost table: "Public worker list, all claims, 93 ms" as current | code wins | Row labelled "as it was" |
| O7i2-8 | E2-1 | `io_util.c` (this branch) and `transposition_table.h` (MAGPIE main) carry platform `#if`s outside `src/compat/` | PLAN: the invariant "holds exactly" | code wins | PLAN names both exceptions; moving `io_util.c`'s into compat is optional MAGPIE work, not done (Issues, E2-1) |
| O7i2-9 | E2-2 | `derived_data` holds `wmp`, `rit` and `wit` | Seven PLAN places and the migration comment (with its PLAN copy) said wordmaps and rack info tables | code wins | "and word info tables" everywhere; schema copy re-diffed identical |
| O7i2-10 | E2-3 | `JobStats` carries `job.name`, `leave_generation.generations_closed`, `completion`, and an always-present `eta_seconds` | PLAN's shape lacked the three and marked `eta_seconds` optional | code wins | Shape updated (`eta_seconds: number \| null`) |
| O7i2-11 | E2-4 | The export panel shows for every job (a running job exports a snapshot) | Frontend Routes: "for a completed job the export panel" | code wins | Row reworded |
| O7i2-12 | E2-5 | RUNBOOK §2.6 is titled "A deleted rating pool" | PLAN quoted "Restoring a deleted rating pool" | code wins | PLAN cites §2.6 by its title |
| O7i2-13 | E2-6 | `unsupported_jobs` defaults to `[]` | PLAN (three places) and the 400 message read as if it were required; "neither field" left over from two fields | code wins | PLAN and the message text (only) corrected |
| O7i2-14 | E2-7 | `infra/tests/variables.tftest.hcl` exists; CI runs `terraform test`, the fixture check and `builderhash`; two lib tests test files outside lib | PLAN's directory tree and its comments | code wins | Tree and comments updated |
| O7i2-15 | E2-8 | 18 contract fixtures; MAGPIE copies all and checks assignments, results, the claim body and shutdowns | PLAN listed fewer fixtures and said MAGPIE checks only assignment keys | code wins | Paragraph rewritten |
| O7i2-16 | E2-10 | The fleet counts open claims made in the week | PLAN, KL-32 and the fleet page: "those open now" | code wins | All three say "claimed in that week and still open" (the bound is right under lazy reclamation, KL-1) |
| O7i2-17 | ET2-4 | `a_purge_waiting_on_a_rating_fit_holds_up_no_submissions`; the WIT derived test | Doc comments cited `A-ADMIN-20` (a different guarantee) and `I-DERIVED-9` (TESTING names I-DERIVED-11); the purge/fit guarantee had no entry | code wins | Comment fixed to I-DERIVED-11; new `A-ADMIN-29` entry |
| O7i2-18 | ET2-5 | §0's query prints the `rating_pool.deleted.census` row; a deleted letter distribution or layout (or `added_by` admin) breaks the `\copy` | RUNBOOK §2.6 named the plain `.deleted` row and listed two refusals | code wins | Step 1 and the refusal list corrected; X5 found the `added_by` case and judged "re-import" wrong (a re-import gets a new id), so the step says to remake the pool on current rows |
| O7i2-19 | ET2-7 | CI's `scripts` job runs `fake-worker-fixtures.sh --check` | TESTING CI item 6 listed two steps | code wins | Added |
| O7i2-20 | ET2-8 = E2-9 = F2-2 = H2-3 (AF29 O7-60) | The checkbox is "Position Recorder" | e12 spec comment: "Save the positions played" | code wins | Fixed (carried from AF29; now closed) |
| O7i2-21 | ET2-9 | One audit | TESTING named it two ways ("Audit of 2026-10-05" / "Thirty-third audit, pass 1"); three I-OR entries had no note; the `artifacts.rs` coverage row said tier 2 only | code wins | One form throughout (and KL-62's closure); notes added; row "1 + 2" |
| O7i2-22 | F2-1 | No `previous_play` column or MAGPIE refusal | `SETTINGS_COMPARISON.md:71` described both | code wins | Row rewritten; anchors left to F-15 |
| O7i2-23 | F2-8 | The claim retry covers leave generation's random seeds and `LostRace`; the transition no longer retries | Comments and PLAN's "One thing restarts an attempt" cited the transition and a cursor-seed race "a lock that can time out" allows | code wins | Rewritten; the old race recorded as history |
| O7i2-24 | F2-9 | One slot per task | Reclaim/release comments: "at capacity"; an unannotated N-slot arm | code wins | Worded as drift defence |
| O7i2-25 | H2-2 | A capturing static player ranks up to `num_plays_recorded` plays every turn (MAGPIE raises `num_plays`), as the fixtures show | Job form: "A static player records only the move it played"; PLAN 3241-3266: the static analysis "does not exist yet", override relaxation "not yet done" | code wins | Form help and PLAN (including "Two details that will otherwise bite") corrected, relaxation marked done |
| O7i2-26 | G2-4 | No `repositoryCredentials` | README: "a registry of your choice" | code wins | ECR or a public registry |
| O7i2-27 | A2-1 | An export spanning a reopen and a re-completion was stored final | PLAN: a final export is the completed job's corpus; "What the edit holds" described only the narrow window | doc wins | Code fails the running export on reopen; PLAN "Exports" and "Three races" updated |
| O7i2-28 | D2-1 | The in-flight read scanned the job's whole reissue history | `next_reissue` docstring "bounded by the batch, not by the job"; PLAN cost row; `I-OR-REISSUE-1` | doc wins | Code reads only open tasks; docstring, PLAN rows and `I-OR-REISSUE-2` |
| O7i2-29 | D2-2 | The "unseen first" sort could hash the identity's history | PLAN: "custom or generic plan alike, and the same whether the identity analysed every rack or none"; TESTING I-OR-REISSUE-1 | doc wins | Scalar subquery; PLAN qualified "with current statistics"; RUNBOOK §5 analyze step |
| O7i2-30 | B2-1 | Plausibility refused exhaustive PEG's real depth 40 | PLAN: plausibility "rejects the impossible, no false positives"; `MAX_FIDELITY_PLIES` documented as MAGPIE's ceiling | doc wins | Per-solver and per-schedule bounds; PLAN table row and validation list updated |
| O7i2-31 | H2-1 | A 5xx after sign-in resolved the session to signed-out | `refreshSession`'s doc and `F-AUTH-1`: a 5xx "says nothing about the session cookie" | doc wins | `resetSession()`; `F-AUTH-3` |
| O7i2-32 | ET2-2 | U-CFG-5 skipped `backend/.env.example`'s live copy | TESTING U-CFG-5: "every hand-kept copy" | doc wins | Test reads it; entry names it |
| O7i2-33 | ET2-3 | I-OR-REISSUE-1's assertions also passed against the old full walk | TESTING: the reissue "looks at a window of racks, not every one" | doc wins | Test pushes B's racks out of the window and asserts A gets its own; verified to fail with the window widened to a full walk |
| O7i2-34 | ET2-6 | U-CFG-5 and the SES alarm test read files at run time | TESTING tier 1: tests "may not … read a file" | doc wins | Both switched to `include_str!` (the nextest archive now carries the bytes); the rule names its two remaining exceptions (`email.rs`'s loopback SES stand-in, `U-ARCHIVE-11`'s `/proc/self/status`) and allows reading back a test's own tempdir |
| O7i2-35 | G2-2 | The SES mock block did nothing | TESTING: the mocks give the plan what it needs "and nothing else"; the block's comment claimed `outputs.tf` indexes it | doc wins | Block and false comment deleted, so TESTING's sentence holds |
| O7i2-36 | ET2-1 | 701 backend tests at `e529018` (tier 1 246, tier 2 180, tier 3 244), more after this iteration | TESTING status table: 245 / 179 / 240 with stale per-file counts; total "687" (the rows summed to 695) | doc wins | No fixer package covered it; the coordinator regenerated the table from `cargo nextest list --run-ignored all` and vitest's JSON report on the final tree: tier 1 248, 1F 214, tier 2 182, tier 3 246, contract 15, tier-6 Rust 16; 707 backend tests |

**Count: code-wins 26 / doc-wins 10 / unresolved 0**.

---

## Issues and Recommended Solutions

Each entry gives the context (file and location, how it was found), the
problem, the options considered, and the recommendation and outcome. Reviewer
ids: A races, B backend bugs, C MAGPIE, D performance and storage, E PLAN and
drift, ET (reviewer E2) TESTING/RUNBOOK/README, F dead code, G deployment, H
frontend, e2e and the fake worker. Fixer ids X1–X5 as in the header.

### Races and locking

**A2-1 (medium). An export spanning a reopen and a re-completion was stored final.**
- **Context:** `exports.rs` `mark_ready` (~648), `read_snapshot`'s `is_final`
  (~575), `unfinalize` (~780); `routes/admin.rs` (the edit calls
  `unfinalize`); `newest_ready`; `routes/public.rs` results stream. Reviewer A,
  reasoning about the ordering iteration 1's A-3 did not cover.
- **Problem:** the snapshot is taken while the job is completed. An edit then
  reopens it (`unfinalize` touches only `is_final` rows, so the running export
  is untouched), the few unsettled racks are reissued and analysed, and the
  job completes again before the upload finishes. `mark_ready` reads
  `completed` and stores the pre-edit corpus as final, which the stream then
  serves; no second export can be started while it runs. A large export takes
  minutes; a small edit re-completes in seconds to minutes.
- **Options:** (a) `unfinalize` fails the job's running exports; (b) a
  `final_vetoed` column set by `unfinalize` and ANDed into `is_final`.
- **Outcome:** (a), X1. `mark_ready`'s guard then matches nothing and its
  existing branch deletes the objects. Lock order verified (EvalPlanQual
  re-check, no deadlock). `I-EXPORT-15` now expects `failed`; new
  `I-EXPORT-16` reproduces the full interleaving and fails without the fix.
  PLAN "Exports" and "What the edit holds" ("Three races").

**A2-2 (low). The consensus edit locked before validating.**
- **Context:** `routes/admin.rs` `update_consensus` / `consensus_body`.
  Reviewer A.
- **Problem:** a 400 or a no-op 200 still took the hold, the dispatch lock and
  every open claim, then left the reclaim grace (the hold is never
  `committed()`), parking the job's lapsed claims for five minutes, a games
  job's included.
- **Options:** validate unlocked first and re-validate under the locks; or
  also call `committed()` on refusal paths.
- **Outcome:** the first (X1): `consensus_request` (type check, merge,
  `consensus_problems`, player validation) runs unlocked before the hold and
  locked inside; `unchanged_consensus` answers the no-op. The locked no-op
  path (reachable only by a race) keeps the grace, which the reviewer called
  defensible. `I-OR-EDIT-5` added; PLAN "What an edit takes no hold for".

**A2-3 (low). A committed edit could answer 5xx.**
- **Context:** `update_consensus` post-commit `?` on `request_derived_data` and
  `load_job`. Reviewer A.
- **Problem:** a pool timeout after the commit answered 503, and skipped
  `rearm_idle`, the finish check and the push. A retry is a harmless no-op.
- **Options:** log and continue, as `purge_body` does; leave it.
- **Outcome:** log and continue; answer from the committed row (X1). No test:
  the failure is not injectable without test hooks.

**A2-4 (low, doc).** Covered as O7i2-1.

**A2-OK (checked and found correct).**
- Iteration 1's A-1 (completion predicate), A-2 (uncounted hold, 409s,
  `run_to_completion`, hold dropped before `finish_idle_job`), A-3 (no
  deadlock with the edit or purge) and A-5 (every writer of the counters and
  task `state` enumerated; no interleaving reaches the one-slot index or a
  CHECK: reclaim vs late submit, decline vs reclaim, heartbeat vs reclaim,
  claim vs reclaim, two claims on one task, purge/delete, reissue).
- Lock order on claim, submission, decline, heartbeat, reclaim, purge,
  delete, consensus edit, activate, `set_allocations`, deactivate,
  force-complete, `complete_unless_purged`, leave close and merge,
  `lift_passed_over` and export start: no cycle.
- Consensus bookkeeping, finish checks and debounce, leave generation,
  ratings vs admin, exports and backups, the derived builder, single-instance
  enforcement, the artifact KLV cache, and errors under contention.

### Backend bugs and validation

**B2-1 (medium). Exhaustive PEG accepted, then refused as implausible.**
- **Context:** `routes/admin.rs` `resolve_solver_settings`;
  `plausibility.rs` `MAX_FIDELITY_PLIES = 25`; MAGPIE `peg.c:2775`,
  `peg_defs.h:43`, `autoplay_solvers.c`, `config.c`. Reviewer B, tracing the
  schedule from the admin body into `peg_solve`.
- **Problem:** creation allowed `[2147483647]`, MAGPIE's exhaustive mode, which
  tags every play at fidelity 40; plausibility refused anything over 25, so a
  capturing job with such a player wedged on false positives. Iteration 1's B-4
  had kept 25 for both solvers without tracing exhaustive mode.
- **Options:** (1) cap each stage at `PEG_CAND_LIST_CAP` at creation, making
  exhaustive unreachable; (2) give PEG its own bound.
- **Outcome:** (2), widened by the coordinator to a per-job bound (X2): PEG
  0..=40 and endgame 0..=25 in `check_analysis`; a PEG position no deeper than
  the deepest PEG player's schedule reaches (40 for `[i32::MAX]`, else stages
  + 1). A one-stage `i32::MAX - 1` schedule is not exhaustive (2 plies). New
  `U-PLAUS-9`; `U-PLAUS-5` extended; PLAN updated. The post-fix integration
  found one test fixture that MAGPIE could not have produced (`{8,4}` with a
  depth-4 move) and corrected it to `{8,4,2}`.

**B2-2 (low). `bingo_bonus` unbounded above.**
- **Context:** `routes/admin.rs` `validate_job_body` (only `b < 0` refused);
  migration `bingo_bonus INT NOT NULL`; plausibility's absolute bounds;
  MAGPIE's `int32` equity. Reviewer B.
- **Problem:** from about 1,700 a batch's mean score fails `MAX_SCORE_MEAN`;
  around 5,000 a leave rack mean, around 100,000 a move score; past about 2.1
  million MAGPIE's equity overflows. A typo wedges the job.
- **Options:** bound the field; scale the plausibility bounds by the job's
  bonus (needs job context in `process_response`, which deliberately has none).
- **Outcome:** bounded at 0..=500 (coordinator) at creation, in a schema
  CHECK and in the form; PLAN states why. `I-JOB-1g`.

**B2-3 (low, MAGPIE).** Covered in (c) and (i). Fixed in MAGPIE; no version
bump (owner rule).

**B2-4 / F2-3 (low). The rack size written three times.** Covered in (b).
Fixed by X3, with a unit test of the bound.

**B2-OK (checked and found correct).**
- Iteration 1's B-3 against MAGPIE's pair logic (swapped first mover; overtime
  penalties only for play-chooser players, which contribute never sets); B-4
  against MAGPIE's per-turn choice (except B2-1); empty-bag sims; the opening
  rack's inferred analysis kind; the C-1 build check; G-6 presign; B-8 names;
  B-9 rack lookup; consensus arithmetic (exhaustively compared with exact
  rationals over every share in tenths); H-1; D-7; A-3's SQL.
- `plausibility.rs` in full; PEG win% sentinels; auth (CSRF, PASETO with
  generation check, rate limits, confirmation and reset tokens, API-key and
  anonymous identity); `routes/admin.rs` validation; `next_available`'s
  exclusion; `config.rs` and `U-CFG-5`; `inputdata.rs` archive walking;
  pagination.

### MAGPIE arguments and `birdtest-contribute`

**C2-1 (low). `movegen_margin` and `recorder_type` inert outside opening racks.**
- **Context:** MAGPIE `gameplay.c:938-951`, `autoplay.c:847-851`,
  `simmer.c:179`, `autoplay_solvers.c:132`, `config.c:9205-9213` and the only
  readers at `config.c:3019, 3038`; birdtest `validate_shared_player_options`,
  PLAN:4049, 4104-4109, 4129, TESTING I-JOB-2. Reviewer C.
- **Problem:** birdtest refused a games job whose players disagreed on a
  setting games never read, and documented both settings as result-changing.
- **Options:** drop the refusal and fix the docs; keep it "in case" as a
  documented KL.
- **Outcome:** dropped (coordinator), docs corrected, form hint corrected,
  I-JOB-2 inverted (X2). MAGPIE's games executor no longer applies the margin;
  the key stays required.

**C2-2 (low, doc).** Covered as O7i2-3.

**C2-3 (low). C-5 remnants in MAGPIE.** Covered in (c). Fixed (X2); four new
refused cases in `test_an_unverifiable_assignment_is_refused`. No unit test of
the win% refusal: the executors are static and reachable only through
`impl_contribute`.

**C2-4, C2-5, C2-6 (low, doc).** Covered as O7i2-4, -5 and -6.

**C2-7 (low, record). AF29 overstated CI's `BOARD_DIM=21` coverage.**
- **Context:** AF29 (i) and C-1; MAGPIE `test/test.c:273-294`,
  `.github/workflows/run-tests.yml:181-189`, `contribute_test.c:1302`.
  Reviewer C.
- **Problem:** at 21 MAGPIE's test binary runs only the board/rack "super"
  tests, so the contribute suite never runs; the 21 shard is a compile check
  for the contribute changes.
- **Options:** correct the record; also reword the test comment.
- **Outcome:** corrected here, in (i) (AF29 is not edited). The test comment
  still says "loads at either BOARD_DIM"; rewording it is optional and was not
  done. No code defect: the server answers a 21 build `unsupported_build`.

**C-OK (checked and found correct).** The full settings table re-traced at
`c3c6a875`; `c3c6a875`'s diff reviewed for memory, error paths and AGENTS.md
style (no finding; one `size_t i` naming nit matching its neighbours); routes,
bodies, decline reasons, assignment and shutdown fields match both sides;
every claim sender states `board_dim`/`rack_size`; the server-side MAGPIE
commands; `magpie_defaults.rs` equals MAGPIE's defaults; no outcome-affecting
process global; no unused `CONTRIBUTE_*` define; no SPRT, chi-square,
redundancy or self-update remnant; fixtures byte-identical.

### Performance and storage

**D2-1 (medium). The reissue's in-flight read walked the job's reissue history.**
- **Context:** `jobs/opening_rack.rs` `next_reissue`'s first query (~315);
  its docstring; PLAN:1390; TESTING I-OR-REISSUE-1. Reviewer D, measured.
- **Problem:** see (e) #2. W1's iteration-1 measurement seeded no completed
  reissues, so could not show it.
- **Options:** read from the two partial indexes bounded by what is open; add
  `state` to an index; wait for D-4's `task_claims` job index.
- **Outcome:** the first (X1). Equivalence: a non-completed task is
  `available` or `claimed`, a claimed task has exactly one claimed claim, and
  first-pass tasks have `racks IS NULL`. The second arm scans the fleet's open
  claims, the bound reclaim already pays (KL-4); D-4's index would make it
  exact. Measured 2–5 ms against 27–44 ms (X1's dataset), same racks. New
  `I-OR-REISSUE-2` (fails without the `available` arm). PLAN rows and
  paragraph.

**D2-2 (medium). The "unseen first" sort could hash the identity's history.**
- **Context:** `next_reissue`'s `ORDER BY EXISTS (…)` (~327); PLAN:1390;
  TESTING I-OR-REISSUE-1. Reviewer D, with statistics deliberately understated
  inside a rolled-back transaction.
- **Problem:** see (e) #1. The trigger is real in operation: `pg_restore` in
  PG16 restores no statistics, so the first claims after a disaster restore
  would plan the hashed form.
- **Options:** a scalar subquery or `LEFT JOIN LATERAL … LIMIT 1` (never
  hashed); plus an `ANALYZE` after restores.
- **Outcome:** the scalar form, with a comment saying why not `EXISTS` (X1);
  RUNBOOK §5 step 3 runs `vacuumdb --analyze-only --jobs=4` before step 4
  starts the service. `restore-drill.sh` (starts no service) and
  `restore-job.sh` (writes into production, which keeps its statistics) need
  nothing. X1 notes an optional `ANALYZE` of §2.1's scratch copy as cheap
  insurance; not done. TESTING notes the plan shape is measured by hand.

**D2-3 / F2-4 (low). Two indexes with no reader.** Covered in (f). Dropped
(X3), after re-verifying no `ON CONFLICT` targets either and no backend query
filters `input_data` by role or name. PLAN's schema copy matches.

**D2-4 (low, doc).** Covered as O7i2-7.

**D-OK (checked and found correct).** The critical-path trace in (d);
iteration 1's plausibility checks' cost; the debounce; ratings off the path;
D-1's hashed in-flight exclusion and window; the edit's effects; export
`FOR SHARE` cost; the rack lookup bound (an index condition); the fleet page;
pools; no blocking I/O on request paths; storage growth all under accepted
KLs; backups. The rebuilt index-to-query map is in reviewer D's report.

### Docs (objective 7)

Every E2-*, ET-* doc item, H2-2, and the doc halves of A2-4, C2-1, C2-2, C2-4,
C2-5, C2-6, D2-4, F2-1, F2-8, F2-9 and G2-4 are recorded one per row in the
Objective 7 table (O7i2-1 to O7i2-36), with the code, the doc, the decision
and the reasoning. In summary:
- **Code wins:** 26, made by X5 (most), X1 (A2-4, D2-4), X3 (C2-4, C2-6,
  F2-8, F2-9, ET2-4), X2 (C2-1) and X4 (H2-2, G2-4, the e12 comment). PLAN's
  schema copy was re-diffed byte-identical after each migration edit (X2, X3,
  X5).
- **Doc wins:** 10, each also covered under its own id (ET2-1 below).

**ET2-1 (low). TESTING's status table and backend total are stale. Fixed by the coordinator.**
- **Context:** TESTING.md:71-89. Reviewer ET, counting test attributes per
  file.
- **Problem:** iteration 1 added tests (U-RACK-12, I-EXPORT-15, I-OR-EDIT-3/4,
  I-OR-REISSUE-1, A-WORKER-22) without updating the rows: tier 1 reads 245
  (246 at `e529018`), tier 2 179 (180), tier 3 240 (244), total 687 (701; the
  rows themselves summed to 695). This iteration adds more (I-EXPORT-16,
  I-OR-EDIT-5, I-OR-REISSUE-2, I-JOB-1g, U-PLAUS-9, the rack-size unit test,
  F-AUTH-3 ×3), so the row counts drift further.
- **Options:** patch the numbers by hand; regenerate the table from
  `cargo nextest list --run-ignored all` and `vitest`.
- **Recommendation:** regenerate it from the final tree, as the prose says it
  is. **Done** at commit by the coordinator (no fixer package covered it):
  tier 1 248, 1F 214, tier 2 182, tier 3 246, contract 15, tier-6 Rust 16,
  707 backend tests in all, from `cargo nextest list --run-ignored all` and
  vitest's JSON report.

**E2-1 (low). The `src/compat/` platform rule. Doc fixed; MAGPIE half optional.**
- **Context:** PLAN.md:3713-3716; MAGPIE `src/util/io_util.c:590`
  (`#if defined(__APPLE__)`, from this branch's `68a74611`) and
  `src/ent/transposition_table.h:35` (from MAGPIE main). Reviewer E.
- **Problem:** PLAN said the rule "holds exactly"; this branch broke it, and
  the `st_mtim`/`st_mtimespec` split has no Windows arm (relevant only once the
  "written, not compiled" WinHTTP build is compiled).
- **Options:** move the accessor into `src/compat/` (e.g. `compat/cstat.h`
  with a Windows arm) and reword PLAN; or correct the doc only.
- **Outcome:** the doc names both exceptions (X5). The MAGPIE move was not
  made. **Recommendation:** make it in a later pass on `birdtest-contribute`,
  then drop the `io_util` sentence from PLAN. Low; no behaviour on supported
  platforms changes.

**ET2-6 (low).** Covered as O7i2-34. Note for the owner: `include_str!` means
an edit to an `infra/*.tf`, compose file or script the test compares
rebuilds the test binary; X3 judged that acceptable (the Docker release build
compiles no tests).

**ET-OK (checked and found correct).** All 555 TESTING entries' *(Covered: …)*
Rust names resolve and none is wrongly `#[ignore]`d; no duplicate or undefined
ids beyond ET2-4; every `F-*`, `E-*`, `M-*` id resolves; iteration 1's new
tests each prove their stated guarantee (except ET2-3, now fixed); RUNBOOK
§0–§6 and README checked against the migration, Terraform outputs, scripts,
routes, UI labels and flags (`runbook-check.sh` 32 and 19 blocks at
`e529018`); the rollback paragraph against `ecs.tf`.

### Dead and out-of-date code

**F2-1, F2-2 (low).** Covered as O7i2-22 and O7i2-20.

**F2-3 / F2-4 (low).** Covered under B2-4 and D2-3.

**F2-5 (low). `jobs.deactivated_at` write-only.** Covered in (f) and (g).
Options were drop it or keep it for ad-hoc SQL; dropped (X3), since the
`job.deactivated` audit row is the record of who and when, and PLAN's
deactivate row now says so.

**F2-6 (low). axum `macros` unused.** Removed (X3); the backend's only axum
attribute is the unconditional `async_trait` re-export. `clippy --locked`
clean; the lock only loses `axum-macros`.

**F2-7 (low). `tslib` unused.** Uninstalled (X4); `npm run check`, `npm test`
and `npm run build` pass.

**F2-8, F2-9 (low).** Covered as O7i2-23 and O7i2-24. The optional
random-seed collision test (I-SCHED-13 family) was not added (low value).

**F2-10 (low). MAGPIE `chttp_is_available()` never called.** Options were
delete it or call it; called (X2), see (g). No test: libcurl is present on the
test host.

**F-OK (checked and found correct).** Iteration 1's removals hold (grep for
every removed name); no SPRT, redundancy, priority, chi-square or self-update
remnants outside deliberate history; no TODO/FIXME/HACK; every Rust item,
enum variant, `Config` field and deserialized field used (except F2-5); every
Cargo feature used (except F2-6); every npm devDependency used (except F2-7);
Python clean; every route has a consumer except the pending F-4 and F-12;
every frontend component, prop and lib export used; every table read; every
env var, Terraform variable and local, script, Dockerfile stage and fixture
referenced.

### Deployment

**G2-1 (medium), G2-3 (low), G2-4 (low), G2-2 (low).** Covered in (k).
Options for G2-1 were an EventBridge alert (no workflow change) or
`wait_for_steady_state` (pending the owner since iteration 1, G-3); the alert
was made, and the two are independent. What is not testable offline (that the
rule's resource is the live service ARN, and delivery) is stated in TESTING's
"Terraform, beyond its variables"; README gives an `aws events
test-event-pattern` check against the live stack.

**G2-OK (checked and found correct).** Iteration 1's G-1 to G-6 hold; the
backend's ALB keep-alive (hyper sets no header-read timeout without a timer);
the `outputs.tf` splat equals `[0]` on a real apply; the breaker against
`desired_count` 0/1 and `min 0% / max 100%` (no rollback loop; the previous
image starts on a newer schema with `set_ignore_missing`); ECS environment
against `config.rs`; IAM against every AWS call; ops-script outputs; RUNBOOK
names; Dockerfile pin verification and smoke check; the fixture check's Python
stability; compose, CI and nightly; production auth cookies; no unfinished
markers. `postgres:16` is Debian trixie and still installs `awscli`.

### Frontend, e2e and the fake worker

**H2-1, H2-4 (low).** Covered in (b). For H2-1, X4 also routed the import
watcher's `signedOut` through `resetSession()`; the trade-off is that the admin
layout shows "Loading…" until `/api/me` answers, and the page resumes the kept
import id from localStorage. The optional "server is busy, retrying…" message
was not added. For H2-4, the optional shared `nameProblem` / `F-NAME-1` was not
added.

**H2-2 (low, doc).** Covered as O7i2-25.

**H2-3 (low).** Covered as O7i2-20.

**H-side note (not a finding).** `backend/src/jobs/testdata/README.md:10` and
`racks.rs:572,599` call `english.csv` part of `data-20251004.tgz`; the
contract fixtures pin the same hash under `20260925`. A stale date in
comments; recommended for a later pass.

**H-OK (checked and found correct).** Every `api.*` wrapper's route and
method; iteration 1's contract changes on both sides; `query()`;
`consensusFields`/`consensusProblem`; the job form against `CreateJobBody`;
`refreshSession`'s retry apart from H2-1; error handling and SSE; CSRF; the
job list's "racks settled"; `ConsensusEditor`; `firstClaim`; e2e claim bodies;
E-12/E-13/E-18 against the fake; the fake's key sets against MAGPIE;
`fake-worker-fixtures.sh`; `e2e/run.sh`; the new contract fixtures; the
TESTING entries for iteration 1's frontend and fake tests.

### Open items carried forward

**Pending the owner** (genuine trade-offs, unchanged from AF29; they do not
count as fixable for the loop):
- **B-1:** rating-pool scoping by bingo bonus and sim cutoff (KL-75). B2-2's
  0..500 bound narrows the range but does not scope pools.
- **B-2:** a consensus edit reopens a force-completed job.
- **D-3:** batch defaults (KL-93).
- **D-4:** `task_claims` job-scoped indexes (would also make D2-1's second arm
  exact).
- **D-5:** the FK drops and the KL-10 re-measurement.
- **D-8:** export snapshot versus vacuum (KL-94).
- **F-4 / A-RATE-6:** the rating-history endpoint.
- **F-12:** the admin results stream.
- **F-15 and F-16:** `SETTINGS_COMPARISON.md`'s anchors and the root plan
  docs.
- **G-3:** `wait_for_steady_state`.
- **W2a:** the even-batch CHECK (B-5) and exact-mover attribution (B-4).
- **Also noted, not blocking:** the D-7 fleet semantics (now documented,
  E2-10); production `jit` (narrowed by D2-2); bumping `MAGPIE_VERSION` and the
  floor together before a first deployment (PD-9, now also for B2-3's output
  change).

**New pending items from this iteration:** none. No fix report flagged a
decision for the owner.

**Fixable, carried to iteration 3:**
- **E2-1 (MAGPIE half, optional):** move `io_util.c`'s platform `#if` into
  `src/compat/` and drop the sentence PLAN now carries about it.
- **Optional, low:** C2-7's test comment ("at either BOARD_DIM"); the
  testdata `data-20251004` date note; X1's optional `ANALYZE` of RUNBOOK
  §2.1's scratch copy.

**Blocking CI until done (owner action):** push `birdtest-contribute` with
`c3c6a875` and `1a0ae932`.
