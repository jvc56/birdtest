# AUDIT_FINDINGS_31 — the thirty-third audit (2026-10-05), pass 3 (iteration 3)

Branch `audit/birdtest-2026-10-05`, the same branch as iterations 1 and 2.
Iteration 1's work is committed on it as `fe701c8`, `deaacd8` and `e529018`,
and iteration 2's as `6c6c191`. This iteration audited `6c6c191` and commits on
top of it.

MAGPIE: `~/MAGPIE` is on `birdtest-contribute`. The iteration started at
`1a0ae932` (iteration 2's commit), which `docker/Dockerfile` pinned. This
iteration's MAGPIE changes are committed on `birdtest-contribute` as
`baa82cfc`, and the Dockerfile pin moves to that commit.
`MAGPIE_VERSION` stays 0.1.1, and so does `MIN_MAGPIE_VERSION` in every copy.

> **STILL NOT PUSHED.** Neither `birdtest-contribute` nor the birdtest branch
> is on a remote; `origin/birdtest-contribute` is still `dbf8df3b`. Until
> `birdtest-contribute` (now with `c3c6a875`, `1a0ae932` and
> `baa82cfc`) is pushed, these fail at the step that fetches the pin:
> - CI's `magpie-contract` job;
> - the e2e image builds (tier 5) in CI;
> - the nightly job, on **both** e2e legs: the pin leg at its MAGPIE checkout,
>   and the head leg at "Start the stack", whose backend image is built from
>   `docker/Dockerfile` and fetches the pinned commit too (G3-1 corrected
>   TESTING, which said the head leg ran).
>
> As before, the pin cannot be reverted to the pushed commit: the server
> refuses a pre-`c3c6a875` claim (no `board_dim`/`rack_size`) with a 400.

**Builds on** AUDIT_FINDINGS_29 (iteration 1) and AUDIT_FINDINGS_30
(iteration 2), both on this branch, and through them AUDIT_FINDINGS_7 to _28
and PLAN.md's Known Limits KL-1 to KL-94. **No KL is added this iteration**,
so the set is still KL-1 to KL-94 (KL-95 is free). KLs revisited (none
renumbered):
- KL-17 and KL-18: a sentence each on the leave universe probe (D3-1);
- KL-59: **closed**, implemented (H3-1);
- KL-60: narrowed (E3-3);
- KL-68: rewritten, because its premise no longer holds (C3-2);
- KL-89: a redundancy remnant dropped (F3-4).

**Process.** The run follows `audit_loop.md`. This file records iteration 3,
which ran in three stages:
1. Nine read-only reviewers, each re-auditing its whole scope and reading
   iterations 1 and 2's diffs (`git diff 0149541..6c6c191`, with
   `e529018..6c6c191` in particular):
   - A: races and locking;
   - B: backend bugs;
   - C: MAGPIE arguments and `birdtest-contribute`;
   - D: critical path, performance and storage (with measurements);
   - E: PLAN against the code, and prompt drift;
   - E2: TESTING, RUNBOOK and README;
   - F: dead and out-of-date code;
   - G: deployment;
   - H: frontend, e2e and the fake worker.
2. A tier-5 runner. **Tier 5 ran natively for the first time in this loop**,
   against `6c6c191` (before iteration 3's fixes): 22 / 22 Playwright tests
   (E-1 to E-18 with their b/c variants, plus the admin setup) passed, and the
   backend log was clean. No Docker image was built: the reusable script
   `scratchpad/tier5/run-native.sh` (outside the repository) runs the backend
   as the cargo-built debug binary, the fake worker and `seed.py` on the host,
   and stock `nginx:1.30-alpine` containers for the web and fixture servers,
   against a database and bucket of their own in the audit's existing Postgres
   and MinIO.
3. Four fixers, working in one tree:
   - Y1: backend performance and bugs (D3-1, D3-2, A3-1, A3-2, B3-1, B3-2,
     C3-1, F3-1, F3-2's backend half, F3-5, F3-7, F3-9, ET3-1/E3-2);
   - Y2: MAGPIE and the contract (F3-6, F3-3, F3-11);
   - Y3: frontend (H3-1 to H3-4);
   - Y4: docs (C3-2, C3-3, E3-1, E3-3 to E3-7, ET3-2, ET3-3, G3-1, F3-4,
     F3-10, F3-2's comment half).

**Findings this iteration, after de-duplication:** 0 high, 1 medium, 33 low
(35 reported).
- **Medium:** D3-1, a leave claim's universe probe was planned as a sequential
  scan of every leave job's every generation, under the dispatch lock.
- **Duplicates merged:** E3-4 = F3-10 (the `MAGPIE_NO_NETWORK` macro). ET3-1
  and E3-2 are separate findings (a test and a PLAN row about the same floor
  copies) fixed together.

Every finding is fixed except F3-8, which needs a small owner decision (keep
or drop the never-written RDS-snapshot branch of the `backups` table).

**Loop decision:** iteration 3 found and fixed issues (one medium, the rest
low), so iteration 4 runs a full audit again.

---

## Pass summary

### (a) Branch, prior findings and versions

- **Branch:** `audit/birdtest-2026-10-05` (unchanged; one branch per loop run).
- **Prior findings:** as listed in AUDIT_FINDINGS_29 (a): `_7`–`_26` on
  `audit/birdtest-2026-09-24-pass21`, `_27`/`_28` in `main`'s history until
  `7e74935`, and `_29` and `_30` on this branch (local, unpushed). So this file
  is number 31.
- **What iteration 2 left open, and what happened to it:**
  - Fixable, optional, carried: E2-1's MAGPIE half (`io_util.c`'s platform
    `#if` into `src/compat/`), C2-7's test comment ("at either BOARD_DIM"), the
    testdata `data-20251004` date note, and an `ANALYZE` of RUNBOOK §2.1's
    scratch copy. **None was attempted this iteration**; no reviewer re-raised
    any of them, and they stay optional (Open items).
  - Pending the owner: unchanged and not re-flagged (see "Open items carried
    forward").
- **KLs added:** none. **KLs revisited:** KL-17, KL-18, KL-59 (closed), KL-60,
  KL-68, KL-89.

### Coordinator decisions on flagged items

- **A3-1, the export's `consensus` gate.** Y1 applied the reviewer's optional
  second change as well as the fix: a line's `consensus` object is present
  when `c.results > 1 OR o.max_results_per_rack > 1`, not only on the job's
  current maximum, so a job whose maximum was lowered to 1 still exports the
  disagreements its `racks_without_consensus` counts. Accepted.
- **C3-1, an optional schema CHECK** (`num_plies = 0 OR num_plays >= 2`) was
  not added. Validation at creation suffices: player configs are made only
  through that route, and the CHECK would add migration and PLAN-copy churn
  for no reachable case.
- **H3-1, `refreshSession` rather than `resetSession`.** The brief said
  `resetSession`; Y3 used `refreshSession` in `signOut`'s failure path.
  `resetSession` first sets the store to `undefined`; during a deploy both the
  logout and `/api/me` fail, so the store would stay unresolved, and the
  header then shows neither the user nor the Sign out button, leaving nothing
  to retry with. `refreshSession` keeps the signed-in user on a 5xx or network
  failure and goes to `null` on a 401/403. Accepted.
- **F3-6, the claim's `bytes`.** Removed on both sides rather than documented,
  under the owner rule that nothing is deployed and both sides change together.
- **F3-8, the `backups` RDS-snapshot branch.** Sent to the owner (new pending
  item).
- **F3-2, the migration comment.** Y4 named the `builder` column rather than
  the reviewer's `KLV_BUILDER_VERSION`, which does not exist in `backend/src`.
  Accepted.

### Coordinator post-fix integration

- **H3-2's remaining call: a muted row was still bolded as a difference.** Y3
  muted the recorder and movegen-margin rows wherever the job never reads
  them, but `PlayerSettingsTable` still set `differs` on a muted row, so two
  players with `best` and `all` recorders in a games job (which play
  identically) still had that row bolded. Y3 left it as a design call. The
  coordinator made `marked()` in `frontend/src/lib/jobSettings.ts` clear
  `differs` for unused rows (a setting the job never reads cannot tell its
  players apart), added an assertion to F-SET-1's test (two margins in a games
  job: the Movegen Margin row's `differs` is false), and added a sentence to
  TESTING's F-SET-1 entry ("a row marked unused is never marked as a
  difference").

### (b) Bugs and fixes made in birdtest

#### Race conditions

Reviewer A re-traced the documented lock order on every path: merge lock,
dispatch lock, claim, task, job, contributor rows. **No lock-order deadlock was
found**, and iterations 1 and 2's A-1, A-2, A-3, A-5, A2-1, A2-2 and A2-3 fixes
hold. Neither finding below is an ordering bug.

| ID | Sev | Race | Fix | Test |
|---|---|---|---|---|
| A3-1 | low | A consensus edit of a **completed** job that leaves every rack settled (lowering `consensus_pct` from 90 to 60, or changing `max_results_per_rack`) still restates rack standings, but `exports::unfinalize` ran only when the edit reopened the job. The final export kept serving pre-edit standings as "the completed job's corpus", and an export running during such an edit was stored final with its pre-edit snapshot. `I-OR-EDIT-1` performs exactly such an edit and asserted nothing about exports. | `consensus_body` calls `unfinalize` whenever the job was `Completed` and the settings changed. Final exports become snapshots; a running one fails with "the job's consensus settings changed while this export was building: export it again". Plus the export gate change (coordinator decisions). | `I-OR-EDIT-1` extended (a ready final and a running export; neither final afterwards, the running one failed; the stream's 8 lines all carry `consensus` with `results` 2 after the maximum drops to 1). `I-EXPORT-15`'s message assertion updated |
| A3-2 | low | A lifecycle action (activate, deactivate, force-complete, allocations) that took the job's row before a consensus edit reached it found `claims_held` true and was told "the job was purged while this waited for it". Nothing was purged: since iteration 1 the edit holds claims through the uncounted hold. | `refuse_if_purged_since` answers `PURGED_WHILE_WAITING` only when the purge count changed, and `ALREADY_RUNNING` (which names the consensus change) when only a hold is held. Both still refuse. It now takes `&DispatchHolds`, so it can be unit-tested. | New `I-OR-EDIT-6` (unit test) |

The paths A checked and found correct are under A3-OK in the Issues section.

#### Other bugs fixed

| ID | Sev | Bug | Fix | Test |
|---|---|---|---|---|
| D3-1 | medium | See (e). | | `I-LEAVE-25` |
| D3-2 | low | See (e). | | `I-STATS-9j` |
| B3-1 | low | A NUL in a text parameter (`?worker=%00` on the public results route, `?rack=`, a leave cursor, a login username, a reset address, an API-key label) reached Postgres, which refuses it (SQLSTATE 22021), and `From<sqlx::Error>` answered `500` with an ERROR log line. The public routes are unauthenticated and not rate limited, so anyone could write error lines at will. | `error.rs` maps 22021 (`CHARACTER_NOT_IN_REPERTOIRE`) and 22P05 (`UNTRANSLATABLE_CHARACTER`) to `400 bad_request`, "the request holds a character that cannot be stored (a NUL)". PLAN's API conventions gain "So is a NUL." | New `U-ERR-8`, `A-PUBLIC-8` |
| B3-2 | low | Re-adding an existing pool member (a double click, or the anchor) wrote a `rating_pool.member_added` audit row and stored a new refit with a `membership` trigger for a change that did not happen. The removal's second click was fixed in pass 22. | `add_member` answers `{"run_id": null}` with no audit row and no refit when the insert affects no row. PLAN's API row says so. | New `A-RATE-4c` |
| C3-1 | low | See (c). | | `A-ADMIN-3` extended |
| H3-1 | low | A failed sign-out (a deploy's 502/503, a network blip, a 403 with no CSRF cookie) cleared the store in a `finally`, so the page said "signed out" while the HttpOnly session cookie, which only a successful logout response removes, stayed valid for the session TTL. The rejection was unhandled too. This was KL-59, open since the thirty-first audit. | `signOut` clears the store only on success; on failure it awaits `refreshSession()` and rethrows. `handleSignOut` stays on the page and shows "Could not sign out (…); you are still signed in. Try again." KL-59 closed as implemented. | `F-AUTH-2` rewritten (3 failure cases; 17 tests in `auth.test.ts`) |
| H3-2 | low | Job pages showed a games or pairs player's Move Recorder and Movegen Margin as live settings, bolded when the players differed, though autoplay reads neither (iteration 2's C2-1). Likewise an opening-rack simmer's. | `jobSettings.ts` mutes both rows for every games and pairs job and for an opening-rack job whose player simulates; a static opening-rack player keeps them live. Plus the coordinator's `differs` change. SETTINGS_COMPARISON's "Never read by" column for rows 4 and 23 updated. | `F-SET-1`'s test extended |
| H3-3 | low | Three inputs bounded differently from the server: (a) the pool page's anchor rating had no bounds, and `mutate` showed only "rating pool details are invalid" and then reloaded over what was typed; (b) the API-key label had no `maxlength`; (c) Confidence % had `min="50.1" max="99.99"`, which blocked values the server accepts (50.05, 99.995), the pattern AF29 H-4 removed from the consensus share. | (a) `min`/`max` ±10000, a range check in `saveAnchor`, `errorText(e)` in `mutate` and `removePool`, the typed values restored on failure; (b) `maxlength="100"`; (c) bounds removed, new `confidenceProblem` in `matchTest.ts` (finite, strictly between 50 and 100, the server's rule) shown as a field error and refused on submit. | New `F-TEST-3` for (c); (a) and (b) are markup |
| H3-4 | low | Stale or detached doc comments in `api.ts`, `routes/mod.rs`, `jobstats.rs` and `jobs/mod.rs`. See O7i3-16. | Comments only. | — |

### (c) MAGPIE argument coverage (objective 3)

Reviewer C re-traced every outcome-affecting setting from the server's rows to
the executors' reads at `1a0ae932`. Iteration 2's table holds row for row:
the movegen margin is now applied only by the opening-rack executor; a NULL
win% name is a server error with no default-table branch left; the capture
counts' fallbacks are unreachable (both keys required). Typed fallbacks
(`json_get_*_or` after a presence check) would take the reset default for a
value of the wrong JSON type, which birdtest's serde-typed structs cannot
send, and none loads an unverified file; noted, not flagged. In
`contribute.txt` only `threads` affects outcomes (KL-14, accepted).

| ID | Sev | Gap | Fixed |
|---|---|---|---|
| C3-1 | low | A player config with `num_plies >= 1` and `num_plays = 1` passed every check. MAGPIE creates the move list with capacity `num_plays`, and `get_top_simming_move` returns a one-play list without simulating, so in a games or pairs job the player played statically on every turn while pinning a win% model, being shown and rated as a simmer, and (since `1a0ae932`) having every captured position recorded `static`. The server already refuses the mirror case (simulation settings with no plies) for exactly this reason. | Yes (birdtest): player-config creation refuses `num_plays < 2` for a simmer, on the field `num_plays`, "a simming player needs at least 2 candidate plays". Leaving it out still takes the default of 100. PLAN states the rule beside the plies rule. The optional schema CHECK was not added (coordinator decisions). MAGPIE needs no change. |

The determinism claims in the docs remain accurate (PLAN 166-174, 2416-2418,
2975, KL-14, README 436): only static, non-solving players are deterministic.
`1a0ae932` changes nothing there: a forced turn is the same pass either way.

### (d) Critical-path and async changes (objective 4)

Reviewer D re-traced claim, heartbeat, decline, submit and completion at
`6c6c191`. Nothing purely observational runs inline beyond the documented
exceptions (KL-5, KL-28, the display columns of the job `UPDATE`). Ratings are
still neither read nor written on the claim, submit or completion path. The
match-test debounce is safe (every 8th submission, any submission that finds no
open claim, the 10 s idle check re-armed on activation and edit, guarded by the
purge witnesses; it reads results, so it is late, never wrong).

Changes this iteration that touch the path:
- **Leave claims (D3-1).** The universe probe, run on every leave claim inside
  `registry::acquire` under the dispatch lock, is now an index-only scan:
  0.02–0.08 ms where it cost 133–279 ms measured and, at full size on the
  default instance, an estimated seconds per claim.
- **Opening-rack completion (D3-2).** The finish check on the submission that
  settles the last rack (inline, before the worker is answered) reads three
  indexes: 0.32–0.37 ms where it cost 395–607 ms at 3.2 M tasks.
- **Consensus edit (A3-1).** One more `UPDATE job_exports` in the edit's
  transaction when a completed job's settings change; admin-side only.
- **Pool membership (B3-2).** A no-op add no longer runs a refit; admin-side.

### (e) Performance, most to least severe (objective 5)

Measured by reviewer D on a throwaway database (PG 16.15 at defaults, `jit`
on, `synchronize_seqscans = off` for worst-case scan starts unless stated) and
re-checked by Y1 with EXPLAIN on a scratch database; both were dropped.

| # | ID | Issue and expected impact | Status |
|---|---|---|---|
| 1 | D3-1 | `universe_exists` (`SELECT EXISTS (… leave_rack_progress WHERE job_id = $1 AND generation = $2)`), called on every leave claim under the job's dispatch lock, is planned as a **sequential scan** once the current generation is in the statistics: with few distinct `job_id`/`generation` values the planner expects a match within a few rows, but the current generation's rows lie after every closed generation of every leave job (KL-18 keeps them). Measured on 5 M rows (two leave jobs): 133–156 ms, custom and generic plans alike; with the production default `synchronize_seqscans = on` and two jobs alternating, 135–279 ms per claim. At full English size (3.2 M racks a generation) a job at generation 5 crosses about 13 M rows (~1 GB) per claim, which `db.t4g.micro` cannot cache: **seconds per claim**, the job's other claims waiting out the 2 s lock and going `Busy`, and the I/O slowing every other job. `any_rack_below_target` (a tail claim that selected nothing) is the same shape (142 ms). | **Fixed** (Y1): `UNIVERSE_PROBE` = `SELECT 1 … ORDER BY rack LIMIT 1` and `BELOW_TARGET_PROBE` = `… AND occurrence_count < $3 ORDER BY occurrence_count LIMIT 1`, outside an `EXISTS` (which drops the `ORDER BY`): index-only scans on the pkey and the pick index, 0.02–0.13 ms, custom or generic. `seed_generation` calls `universe_exists`. The "One index probe" comments explain the shape. PLAN measured rows; KL-17 and KL-18 sentences. |
| 2 | D3-2 | The opening-rack finish check's `EXISTS (… tasks WHERE job_id = j.id)` and `NOT EXISTS (… AND state <> 'completed')` are planned as sequential scans of the whole `tasks` table (every job's history, KL-29), the second reading the entire table exactly when the job is done. Measured at 3.2 M tasks over 11 jobs: 395–414 ms, 607 ms with realistic claims, 383–401 ms with reissues in flight. Linear in the whole table: seconds after a year of jobs. Runs once per job completion inline on the submission, on the idle and post-edit checks, and every eighth submission while an edit has settled every rack with reissues out. The migration comment said `tasks_seed_unique_idx` served it; the query does not use `seed`. | **Fixed** (Y1): `OPENING_RACK_FINISHED` uses D2-1's equivalence (a non-completed task is `available` or has exactly one `claimed` claim): a scalar `ORDER BY seed LIMIT 1`, `NOT EXISTS` an `available` task, `NOT EXISTS` a `claimed` claim of the job, served by `tasks_seed_unique_idx`, `tasks_queue_idx` and `task_claims_open_idx`: 0.32–0.37 ms done, 1.0 ms in flight. Migration comment and PLAN schema copy corrected; PLAN measured rows. |
| 3 | KL-10 note | The match-test reads, "not re-measured" in KL-10, were re-measured: `plain_game_stats` 59–168 ms and `game_pair_stats` 117–175 ms over 300,000 batch-1 rows in a 1.5 M-row `game_results`, well under the 340–620 ms documented for the old read. | **Informational**; recording the numbers in KL-10 is optional and was not done |
| 4 | D-4 (pending) | The finish check's in-flight `EXISTS` over 1,100 open claims: under 1 ms warm, 143 ms cold. D-4's job-scoped `task_claims` index would bound it (and D3-2's claimed arm). | **Pending the owner**, not re-flagged |

### (f) Storage (objective 6)

**Fixed:** nothing structural. Iteration 3's schema edits are comments only
(the finish-check comment, D3-2; the `klv::build` comment, F3-2).

**Flagged:** F3-8, the `backups` table's never-written RDS-snapshot branch
(owner). D-3 (KL-93), D-4, D-5 and D-8 (KL-94) remain with the owner.

**Verified:** iteration 2's drops have no remaining reader; every
`ON DELETE CASCADE` FK on a large table still has an index leading with its
column; rating-run thinning runs hourly and keeps each pool's first, each
day's last and the newest run; unbounded growth stays under accepted KLs
(KL-18, KL-23 to KL-30, KL-38, KL-94); RUNBOOK §5's `vacuumdb --analyze-only`
exists in the ops image; backup, drill and round-trip scripts name no dropped
column or index.

### (g) Dead or out-of-date code removed, and items flagged as uncertain

**Removed (each verified against its callers, then by compiler and tests):**
- `LetterDistribution.machine_letters` and `machine_letter()`: written for the
  deleted `klv.rs`, read only by tests that therefore proved the order of a
  vector production never reads. The tests now assert positions in
  `letters()`, the numbering `canonical_rack` and the board use (F3-1).
- `games.win_pct` / `loss_pct` / `draw_pct` in `JobStats`: their one reader,
  the job page's W/L/D line, went in `699912a`. Removed from the builder, the
  TS type, PLAN's shape and three asserts. The `TestResult` Elo fields stay
  (PLAN records them as deliberately API-only) (F3-5).
- `bytes` on every `expected_data.files[]` and `derived[]` entry of a claim:
  MAGPIE, the fake and every script never read it. Removed from both wire
  structs and their SELECTs; contract fixtures recaptured; the admin views
  read sizes through their own structs, and the schema columns stay (F3-6).
- `scripts/dev.py`'s `CONTRIBUTE_FLAGS` (`-ritmmap true`), a shim for MAGPIE
  checkouts that predate mapping by default and so also predate `c3c6a875`,
  whose claims the server refuses (F3-7).
- `scripts/e2e_magpie.py`'s skip of `.wmp.src` sidecars, which nothing writes
  since `c3c6a875` (F3-3, optional half).

**Made live instead of removed:** MAGPIE's copy of `decline-missing-data.json`
was read by no MAGPIE test, so the decline body was pinned on birdtest's side
only. MAGPIE now builds it through `contribute_decline_body` and
`contribute_missing_file_json`, and a new test checks it against the fixture
(F3-11; see (i)).

**Stale comments and text corrected:**
- The migration's `klv::build` (and its PLAN copy) and `backups.rs`'s
  `STALE_AFTER` (F3-2).
- MAGPIE comments citing birdtest's deleted `MAGPIE_DEPENDENCY.md` and
  `MAGPIE-CLIENT.md`, and `hash.h`'s "wordmap sidecar" (F3-3).
- KL-89's "a slot of a task it already holds" (F3-4).
- Derived-builder descriptions that omitted word info tables:
  `docker-compose.yml`, `infra/derived.tf` (header and the schedule's
  `description`, an in-place change that shows in a plan), README and
  `dev.py`'s `--build-threads` help (F3-9).
- PLAN's nonexistent `MAGPIE_NO_NETWORK` macro (E3-4 / F3-10).
- `api.ts`, `routes/mod.rs`, `jobstats.rs` and `jobs/mod.rs` comments (H3-4).
- The "server that checks nothing" reason in four places (E3-6).

**Flagged for the owner:** F3-8 (the `backups` RDS-snapshot branch).
F-4, F-12, F-15 and F-16 remain pending. Recorded, not flagged: `ratings.rs`'s
now-trivial `job_results` CTE; `.dockerignore`'s `worker/.venv`; test-only
helpers that guard real guarantees (`jobstats::worker_contributions`,
`extract::*::available_kib`, `racks::enumerate_leaves`).

### (h) Python-worker-as-production-client corrections

None needed. Reviewers F and H found no code, doc, config or CI step that
treats `worker/fake_worker.py` as a production or dev client;
`--server-url` has no default, and every remaining flag and mode has a user.
`scripts/fake-worker-fixtures.sh --check` passed for reviewers G and H and for
Y2 after the `bytes` removal. Reviewer H noted (not a defect) that the fake
never models MAGPIE's "one legal play", so a fake simmer's pass-only position
still says `sim`; the server accepts either shape and no spec depends on it.

### (i) `birdtest-contribute` changes, pin and version floor

All of these are committed on `birdtest-contribute` as `baa82cfc`, on
top of `1a0ae932`, and are **not pushed**.

| Change | Why | Files |
|---|---|---|
| `contribute_missing_file_json` (one `missing[]` entry, now shared by `expected_data_matches` and `contribute_record_derived_mismatch`) and `contribute_decline_body` (used by `decline_over_http`); byte-for-byte the same output. New `test_the_decline_body_matches_the_decline_fixture` | F3-11: the decline fixture was copied but read by no MAGPIE test | `contribute.c`, `contribute.h`, `contribute_test.c`, `test/birdtest_contract/README.md` |
| The 18 contract fixtures recopied from birdtest (no `bytes`) | F3-6 | `test/birdtest_contract/*.json` |
| Comments re-pointed to birdtest's README "MAGPIE on the server" / "Leave-generation KLVs" and PLAN "Leave generation on the client"; `hash.h`: "Used to verify a fetched leave-generation KLV, and in tests" | F3-3 | `Makefile`, `builder_defs.h`, `convert.c`, `autoplay.c`, `hash.h`, `builder_hash_test.c` |

- **A difference the new test pinned.** MAGPIE leaves a not-found file's
  `actual` out of its decline entry, while the fixture writes `"actual": null`.
  The server reads both alike. birdtest's `contract_fixtures::decline_parses_as_a_decline_body`
  (new `C-10`) also parses the fixture with the null keys removed and checks
  the result is the same; MAGPIE's test accepts the omission where the
  fixture's `actual` is null and asserts the fixture covers both cases.
- **Pin.** `docker/Dockerfile` `ARG MAGPIE_COMMIT=` moves from `1a0ae932` to
  `baa82cfc`. CI (two jobs) and the nightly read it from that line.
- **Version and floor.** `MAGPIE_VERSION` stays 0.1.1 and `MIN_MAGPIE_VERSION`
  stays 0.1.1 in every copy (U-CFG-5 now also reads the root `.env.example`).
  F3-6 drops a key the client never read, and F3-11 and F3-3 change no
  behaviour, so no bump is called for even under a deployed system; PD-9
  (bump both before a first deployment) stands for iteration 2's B2-3.
- **Protocol and fixtures.** `bytes` is gone from every assignment fixture and
  `expected-data.json`; the hand-written fixtures were edited and the captured
  ones recaptured (`scripts/e2e_magpie_native.sh --cases capture`, 8 fixtures,
  with a freshly built `portable_release` MAGPIE). All 18 are byte-identical to
  MAGPIE's copies (`cmp`).
- **Derived data.** No builder or conversion code changed, so builder versions
  and derived-data hashes are unaffected.
- **MAGPIE checks (Y2):**
  - **Run:** `python3 format.py --write` (clang-format-20; diff limited to the
    changed files); `make magpie_test BUILD=dev -j3` and
    `./bin/magpie_test contribute`, passed before and after copying the new
    fixtures; `make magpie BUILD=portable_release -j3` (only the existing LTO
    warnings).
  - **Not run:** cppcheck and clang-tidy (standing owner instruction); the
    `BOARD_DIM=21` build (a compile check only for contribute code, per AF30
    C2-7); the wasm tests.

### (j) Findings file and count

`AUDIT_FINDINGS_31.md` — objective-7 discrepancies this iteration:
**code-wins 18 / doc-wins 12 / unresolved 0** (O7i3-30, TESTING's status
table, is regenerated by the coordinator at commit). Iteration 1's three
unresolved discrepancies (B-1, B-2, F-15) are unchanged and are not counted
again.

### (k) Deployment blockers

No high deployment blocker. Reviewer G re-ran Terraform 1.9.8 on a scratch
copy (`fmt -check`, `init -backend=false`, `validate`, `test` **54 / 54**) and
a probe showing that even with `override_resource` the deploy-failed target's
ARN and the pattern's service ARN are unknown at plan on 1.9 (so S-TF-3's
scope is right; ET3-3 corrected its wording). G re-checked iteration 2's
deploy-failed rule against AWS's documented event, the `min_magpie_version`
validation against `Version::parse_strict`, ECS environment against
`config.rs`, IAM against every SDK call, S3 replication and lifecycles, SES,
ALB health and keep-alive, container-definition sizes, ops scripts and the
provider lock (G3-OK).

| ID | Issue | Resolution |
|---|---|---|
| G3-1 | TESTING's "While the pin is unpushed" said the nightly's head leg builds the branch head and fails on the floor. Its "Start the stack" builds the backend image from `docker/Dockerfile`, which fetches the pinned commit, so it fails at the same fetch as the pin leg. | **Fixed (doc)**: both legs fail; `restore-roundtrip` and `backup-drill` build no image and are unaffected. CI step 4 names the images' real sources. |
| F3-9 | `infra/derived.tf`'s schedule `description` omitted word info tables. | **Fixed**; an in-place attribute change that shows in the next plan. |
| — | The MAGPIE pin is not pushed. | **Open**: push `birdtest-contribute` before CI, the e2e images and the nightly can pass (header). |

**Terraform variables with no default:** still eight (PD-1).

### (l) Prompt drift

Iteration 1 and 2's items, re-verified (reviewer E, with C, F and G):

| # | Status now | Checked in |
|---|---|---|
| PD-1 | **Holds.** Eight no-default variables: `backend_image`, `frontend_image`, `alert_email`, `acm_certificate_arn`, `mail_from_address`, `ses_domain`, `public_url` (`variables.tf`) and `derived_builder_image` (`derived.tf`). The prompt lists five | script over every `variable` block; README step 4 and the test file's `variables {}` list the same eight |
| PD-2 | **Holds.** The builder builds wordmaps, rack info tables and word info tables; leave-generation KLVs are built by the web process. F3-9 was its in-repo counterpart outside PLAN | migration role CHECK, `derived.rs`, `leave_gen.rs` |
| PD-3 | **Holds.** `plausibility.rs` also holds `check_analyses_against_players` with the PEG schedule bound; the pentanomial cross-check stays in `game_pair.rs` | grep |
| PD-4 | **Holds.** CI: clippy, doc tests, a nextest archive in partitions, `npm run check`/`test`/`build`, three image builds, `e2e/run.sh --no-build`, `terraform fmt -check`/`validate`/`test`, `dev-restore-check.sh`, `runbook-check.sh`, `fake-worker-fixtures.sh --check`, MAGPIE `contribute` plus `builderhash` and the conversions check. Nightly as in AF30. The prompt's "Terraform checks" are now fmt, validate and `terraform test` (G) | `ci.yml`, `nightly.yml` |
| PD-5 | **Holds.** Neither `dev.py` nor tier 6 has a fake-worker mode to refuse | grep |
| PD-6 | **Holds, dated.** `_29` and `_30` exist on the local, unpushed audit branch, so this file is `_31` | `ls`, `git log` |
| PD-7 | **Resolved** (the one-slot CHECKs and index) | migration |
| PD-8 | **Holds.** The web process checks the floor at startup; `bin/build-derived.rs` does not (KL-82) | — |
| PD-9 | **Holds.** `MAGPIE_VERSION` is `"0.1.1"` (`config.c:1024`) and every floor copy 0.1.1. Bump both before a first deployment | MAGPIE `config.c`; U-CFG-5 copies |
| PD-10 | **Holds, one more commit.** `main` is still `0149541`; MAGPIE `birdtest-contribute` is now three commits past the Snapshot's `dbf8df3` (`c3c6a875`, `1a0ae932`, `baa82cfc`), which the Dockerfile pins; none is pushed | `git rev-parse`, Dockerfile:6 |
| PD-11 | **Holds.** Holds come from seeding, purge, delete and the consensus edit (now taken only after the edit's pre-check); other lifecycle actions refuse with 409 while one is held (A3-2 corrected which 409 message they give) | `admin.rs` |
| PD-12 | **Holds.** The claim's build axis (`board_dim`/`rack_size`, `unsupported_build`) is checked before the floor | `routes/worker.rs` |
| PD-13 | **Holds (fixed).** No `sprt` outside history comments | `grep -ri sprt` |
| PD-14 | **Updated.** PLAN.md was 10,908 lines and TESTING.md 4,495 at `6c6c191` (about 10,960 and 4,600 after this iteration's edits); this is the thirty-third audit, iteration 3 | `wc -l` |

New this iteration:

| # | `audit_loop.md` says | The repository says | Checked in |
|---|---|---|---|
| PD-15 | Objective 8: keep `MIN_MAGPIE_VERSION` "(backend config default, Terraform variable, and `docker-compose.e2e.yml`)" consistent | The floor has more hand-kept copies, and `U-CFG-5` (`config::tests::every_copy_of_the_version_floor_agrees`) checks each: the config default; Terraform's `min_magpie_version` (passed through by `ecs.tf` and `derived.tf`); both compose files (`docker-compose.yml` twice); `scripts/e2e_magpie_native.sh`; `backend/.env.example`; the root `.env.example` (from this iteration, ET3-1); the `jobs` table's column defaults in the migration; and `scripts/dev.py`'s note. A human following objective 8's list would miss several; the test catches it | `config.rs` U-CFG-5; TESTING U-CFG-5 |

All other Snapshot claims were verified true again (reviewer E's list: the
stack and dark-mode-only SPA; the fake as test tooling only; allocation-only
deficit scheduling, `CLAIM_ROUNDS` 8, `JOIN_SETTLE` 1 h; one slot; four job
types; the match test off by default, `TEST_CHECK_EVERY` 8, stored
`test_decided_*`; no SPRT or chi-square; ratings siloed, sweep every 120 s;
`worker_bans`; input-data pinning; the Dockerfile pin invariant; no API grants
`is_admin`; one migration; the documented scripts; seven tiers).

### (m) Checks run and not run

See **Checks run at the end of iteration 3** below for the end-of-pass
results.
- **Reviewers** ran no builds, except: G ran Terraform 1.9.8 in Docker on a
  scratch copy (`fmt`, `init`, `validate`, `test` 54/54, plus a probe),
  `runbook-check.sh` (33 and 20 blocks), `dev-restore-check.sh` and
  `fake-worker-fixtures.sh --check`; H ran `fake-worker-fixtures.sh --check`;
  D measured on a throwaway database, since dropped.
- **Tier-5 runner:** tier 5 natively against `6c6c191`, **22 / 22 passed**,
  backend log clean (see Process). It ran before iteration 3's fixes, so
  iteration 3's frontend changes (sign-out, settings tables, form bounds) are
  not covered by that run.
- **Fixers**, each under the shared build lock:
  - Y1: clippy `--locked` clean; `cargo nextest run --run-ignored all`
    **713 passed** (556 + 157), 0 failed; `npm run check` 0 errors, `npm test`
    216 / 216; `py_compile scripts/dev.py`; EXPLAIN checks on a scratch
    database (`y1perf`, dropped).
  - Y2: clippy clean; 45 targeted nextest tests (21 contract/fake);
    `fake-worker-fixtures.sh --check`; the tier-6 capture run (8 fixtures);
    MAGPIE as in (i).
  - Y3: `npm run check` 0 errors, `npm test` 216 / 216, `npm run build`;
    clippy clean after its backend comment edits.
  - Y4: docs and comments only; no tests. Terraform was not installed for Y4,
    so its `variables.tftest.hcl` edit (a comment and an assert message) was
    not run by the fixer.
- **Not run in iteration 3 by the fixers:** the full tier 6; Terraform after
  Y1's `derived.tf` description edit and Y4's test-file edit; MAGPIE cppcheck
  and clang-tidy (standing owner instruction), the `BOARD_DIM=21` build and the
  wasm tests. The end-of-pass checks below record what the coordinator ran on
  the combined tree.

---

## Checks run at the end of iteration 3

Run on the combined tree after all four fixers finished (and the
coordinator's `marked()` change), with heavy commands serialized behind one
build lock:

| Check | Result |
|---|---|
| `cargo clippy --locked --all-targets -- -D warnings` | clean |
| `cargo test --locked --doc` | ok |
| `cargo nextest run --locked --run-ignored all --no-fail-fast` | **713 / 713 passed** |
| frontend `npm run check` / `npm test` / `npm run build` | 0 errors / **216 / 216** / built |
| `scripts/runbook-check.sh RUNBOOK.md README.md` | 33 and 20 blocks parse |
| `scripts/dev-restore-check.sh` | passed |
| `scripts/fake-worker-fixtures.sh --check` | every fixture is what the fake emits |
| contract fixtures vs MAGPIE `test/birdtest_contract/` | 18 and 18, byte-identical |
| MAGPIE `format.py`, `find_circ_deps.py` (clean copy) | no differences / no cycles |
| MAGPIE `make magpie_test BUILD=dev` + `./bin/magpie_test contribute` (fixer Y2) | clean build, passed |
| Terraform 1.9.8 (Docker): `fmt -check -recursive`, `validate`, `test` | clean / valid / **54 / 54** |
| tier 6, `scripts/e2e_magpie_native.sh` (rebuilt `portable_release` MAGPIE) | **15 / 15 cases passed** |
| tier 5, native (`scratchpad/tier5/run-native.sh`, against `6c6c191`, before this iteration's fixes) | **22 / 22 passed** |
| TESTING status table | regenerated from `cargo nextest list --run-ignored all` and vitest's JSON report: tier 1 250, 1F 216, tier 2 184, tier 3 248, contract 15, tier-6 Rust 16; 713 backend |

**Not run, and why:** tier 5 has not yet been rerun on this iteration's tree
(it is rerun in the final green run); MAGPIE cppcheck and clang-tidy are never
run on this machine (the owner's standing instruction); MAGPIE's BOARD_DIM=21
build and the wasm tests were not run.

---

## Objective 7 — every code-versus-doc discrepancy found in iteration 3

Decision key (as in AF29 and AF30):
- **code wins:** behaviour kept, and the doc, comment or test note was updated
  to match it.
- **doc wins:** behaviour changed to what the doc says or needs.
- **unresolved:** neither changed beyond describing the situation; it awaits
  the owner.

Ids are `O7i3-N`, distinct from AF29's `O7-N` and AF30's `O7i2-N`.

| # | ID | Code does | Doc said | Decision | Reasoning |
|---|---|---|---|---|---|
| O7i3-1 | E3-1 | `validate_shared_player_options` compares only the win% model (iteration 2) | PLAN's job-creation list: two configs must agree on `win_pct_model` and `movegen_margin` | code wins | Bullet rewritten; it notes margins were refused until pass 2 |
| O7i3-2 | E3-2 | U-CFG-5 reads `backend/.env.example` (and, after ET3-1, the root `.env.example`) | PLAN's `MIN_MAGPIE_VERSION` row listed neither | code wins | Row lists both |
| O7i3-3 | E3-3 | The job form sends `capture_positions`; `/player-configs/[id]` shows every setting; distribution and layout start empty | KL-60: none of these | code wins | KL-60 narrowed to `racks_per_batch`/`rack_size` and the remaining gaps, with a dated note |
| O7i3-4 | E3-4 = F3-10 | MAGPIE's `chttp.c` selects the wasm stub with `#if defined(__wasm__)` | PLAN: the compat layer defines `MAGPIE_NO_NETWORK` (never existed) | code wins | Mechanism named correctly |
| O7i3-5 | E3-5 | Games seeds are read and inserted under the dispatch lock; no race | PLAN's games claim step 1: "the loser retries" | code wins | The unique index described as a backstop |
| O7i3-6 | E3-6 | MAGPIE refuses a player's derived file with no pinned hash (`derived_mismatch`), setting the job aside | PLAN, TESTING `I-DERIVED-3`, a `worker_api.rs` doc comment and a `worker.rs` test comment: an absent `derived` entry "reads as a server that checks nothing" | code wins | All four give the current reason (the `worker.rs` one: an empty list is the fixture's shape, read like a missing key) |
| O7i3-7 | E3-7 | `validate_capture_play_cap` refuses a capture job whose players differ on `num_plays_recorded` or `num_plies_recorded` (A-ADMIN-26) | PLAN's job-creation list did not mention it | code wins | Bullet extended |
| O7i3-8 | C3-2 | Since `1a0ae932`, a malformed or unknown-role `expected_data` entry fails the claim and ends the run, with no decline | KL-68: an unknown role "is skipped silently"; client-loop step 3 listed fewer refusals | code wins | KL-68 rewritten (a new role must ship with a floor raise), step 3 extended |
| O7i3-9 | C3-3 | MAGPIE's tests: a forced turn is recorded static; the unverifiable-assignment test refuses four more shapes; a new forced-turn test | TESTING's account of MAGPIE's contribute tests predated `1a0ae932` | code wins | Paragraph updated; notes `chttp_is_available` has no unit test |
| O7i3-10 | ET3-2 | Account deletion anonymizes the `users` row and keeps it, so `added_by` always resolves | RUNBOOK §2.6: blank `added_by` if the admin was deleted | code wins | Clause replaced; consistent with §0 |
| O7i3-11 | ET3-3 | S-TF-3 asserts the rule has a target | TESTING and the assert message: "has the topic's target" | code wins | Reworded to what is proved; the target's ARN is known only at apply on Terraform 1.9.8 (G's probe) |
| O7i3-12 | G3-1 | The nightly head leg builds the backend image from `docker/Dockerfile`, which fetches the pin | TESTING: the head leg builds the branch head and runs | code wins | Both legs fail while the pin is unpushed; CI step 4's image sources corrected |
| O7i3-13 | F3-4 | Under one slot a held task is never `available` | KL-89: a worker passes over "a slot of a task it already holds" | code wins | Phrase dropped; the decline case is the whole gap |
| O7i3-14 | F3-2 | KLVs are built by MAGPIE's `convert rackequity2klv`; the constant is `STALE_AFTER_HOURS` | Migration comment (and PLAN copy): `klv::build`; `backups.rs` doc: `STALE_AFTER` | code wins | Both corrected; PLAN schema copy re-diffed identical |
| O7i3-15 | F3-9 | The derived queue builds and pins wordmaps, rack info tables and word info tables | Compose, `derived.tf`, README and `dev.py` help: two file kinds | code wins | All say three (or "derived file") |
| O7i3-16 | H3-4 | Two endpoints page by cursor; a rack lookup's `total` is its count and its list is cut per analysis; a running job exports a snapshot; an opening-rack job completes when every rack is settled | `api.ts` / `routes/mod.rs` / `jobstats.rs` / `jobs/mod.rs` comments: one cursor endpoint, `total` always -1, the whole list, 409 unless completed, "analysed" | code wins | Comments moved onto their items and reworded |
| O7i3-17 | F3-3 | birdtest's `MAGPIE_DEPENDENCY.md` and `MAGPIE-CLIENT.md` were folded into README and PLAN; the sidecar is gone | MAGPIE comments cited both files and the sidecar | code wins | Comments re-pointed (MAGPIE) |
| O7i3-18 | F3-7 | MAGPIE maps the rack info table by default | README:79: "dev.py runs them with `-ritmmap true`, so it is mapped" | code wins | Shim removed; README says MAGPIE maps it by default |
| O7i3-19 | A3-1 | A consensus edit that left a completed job completed kept its final export, serving pre-edit standings | PLAN "Exports": a final export is the completed job's corpus, built once | doc wins | `unfinalize` on any change of a completed job; PLAN "Exports" and "Editing the consensus" updated; the export gate described |
| O7i3-20 | A3-2 | An action that waited on an edit's hold was told the job was purged | Iteration 1 (AF29 A-2): the 409s name the consensus change when it holds the job | doc wins | Message chosen by what changed |
| O7i3-21 | B3-1 | A NUL in a public parameter or unauthenticated body was a 500 | PLAN API conventions: a malformed request is a 400, a 500 a server bug; passes 21 and 22 fixed the same class elsewhere | doc wins | 22021/22P05 map to 400; PLAN "So is a NUL." |
| O7i3-22 | B3-2 | Re-adding a member logged and refitted | Pass 22's rule for the removal's second click, and `update_pool`'s no-op | doc wins | No-op add answers `run_id: null`; PLAN row updated |
| O7i3-23 | C3-1 | A simmer with `num_plays` 1 was accepted and never simulates | PLAN and `admin.rs`: a config carried as a simmer must simulate (the mirror case is refused for that reason) | doc wins | Refused at creation; PLAN sentence |
| O7i3-24 | D3-1 | `universe_exists` and `any_rack_below_target` were sequential scans | Their doc comments: "One index probe"; KL-18: closed generations cost only storage | doc wins | Index-only forms; comments, PLAN measured rows, KL-17/KL-18 |
| O7i3-25 | D3-2 | The opening-rack finish check sequentially scanned `tasks` | Migration comment: served by `tasks_seed_unique_idx` and the heap | doc wins | Index-served query; comment corrected (and its PLAN copy) |
| O7i3-26 | ET3-1 | U-CFG-5 skipped the root `.env.example` | TESTING U-CFG-5: "every hand-kept copy" | doc wins | Test reads it (two copies: the line and the "reports" comment); entry names it |
| O7i3-27 | F3-1 | U-RACK-1 and U-RACK-10 tested `machine_letters`, which production never reads | TESTING: they prove the numbering MAGPIE's must match | doc wins | Field removed; tests assert positions in `letters()` |
| O7i3-28 | H3-1 | A failed sign-out showed signed out | AF29 H-6's rule (only 401/403 mean signed out); F-AUTH-2's own reason ("cannot be left showing a session that is gone") | doc wins | Store follows `/api/me` on failure; F-AUTH-2 rewritten; KL-59 closed as implemented |
| O7i3-29 | H3-2 | Job pages showed the recorder and margin as live for games, pairs and simming opening-rack players | PLAN (iteration 2, C2-1): autoplay reads neither; only an opening-rack static analysis does | doc wins | Rows muted, never bolded as a difference; F-SET-1 and SETTINGS_COMPARISON updated |
| O7i3-30 | TESTING status table | Iteration 3 adds tests (I-LEAVE-25, I-STATS-9j, I-OR-EDIT-6, U-ERR-8, A-PUBLIC-8, A-RATE-4c, F-TEST-3, F-AUTH-2's cases, among others) | The status table's counts as of `6c6c191` | doc wins | **Done**: the coordinator regenerates the table at commit from `cargo nextest list --run-ignored all` and vitest |

**Count: code-wins 18 / doc-wins 12 / unresolved 0**.

Not counted (dead code with code and docs in agreement, updated together):
F3-5, F3-6, F3-11. F3-8 is not a discrepancy (the schema and its PLAN copy
agree; nothing writes the branch) and is pending the owner.

---

## Issues and Recommended Solutions

Each entry gives the context (file and location, how it was found), the
problem, the options considered, and the recommendation and outcome. Reviewer
ids: A races, B backend bugs, C MAGPIE, D performance and storage, E PLAN and
drift, ET (reviewer E2) TESTING/RUNBOOK/README, F dead code, G deployment, H
frontend, e2e and the fake worker. Fixer ids Y1–Y4 as in the header.

### Races and locking

**A3-1 (low). A consensus edit that restated a completed job without reopening it left a stale final export.**
- **Context:** `routes/admin.rs` `consensus_body` (`unfinalize` only inside
  `if reopened`); `jobs/opening_rack.rs` `restate_racks`; `exports.rs`
  `OPENING_RACK_CORPUS` (the `consensus` object gated on the current
  `max_results_per_rack > 1`), `newest_ready`; the results stream's redirect.
  Reviewer A, reasoning past AF29 A-3 and AF30 A2-1, which closed the
  reopen orderings only.
- **Problem:** each export line carries its rack's live standing. An edit that
  leaves every rack settled still changes standings (a lower share turns
  "without consensus" racks into agreed ones; a changed maximum flips the
  gate), but the job stays completed and its export final, so the stream
  keeps serving pre-edit standings while the job page shows the new counts. An
  export running during the edit is stored final with its pre-edit snapshot.
- **Options:** unfinalize on any committed change of a completed job; gate on
  `restate_racks` reporting changed rows (more precise, more code); separately,
  gate the export's `consensus` object on the rack's own results.
- **Outcome (Y1):** unfinalize whenever the job was completed and the settings
  changed (simpler and never wrong); the running export fails with "the job's
  consensus settings changed while this export was building: export it
  again". The optional gate (`c.results > 1 OR o.max_results_per_rack > 1`)
  was applied too, so a job lowered to one analysis still exports its
  disagreements. `I-OR-EDIT-1` extended, `I-EXPORT-15`'s
  message assertion updated; PLAN "Exports", "Editing the consensus" and the
  opening-rack export sentence updated.

**A3-2 (low). A lifecycle action that waited on an edit was told the job was purged.**
- **Context:** `routes/admin.rs` `refuse_if_purged_since`,
  `PURGED_WHILE_WAITING`; callers `activate_job`, `set_allocations`,
  `deactivate_job`, `complete_job`. Reviewer A.
- **Problem:** the check refused on a changed purge count **or** a held claims
  hold. Since iteration 1 the consensus edit holds claims (uncounted), and it
  takes the job's row after its other locks, so an action that took the row
  first then found the hold and was told "purged". The refusal is right; the
  reason was wrong. Iteration 1 reworded the other two messages and missed
  this one.
- **Options:** distinguish by what changed; leave the message.
- **Outcome (Y1):** `PURGED_WHILE_WAITING` only on a changed count,
  `ALREADY_RUNNING` on a hold alone; both refuse. The function takes
  `&DispatchHolds` and is unit-tested: new `I-OR-EDIT-6`.

**A3-OK (checked and found correct).**
- Lock order on claim, submit, decline, heartbeat, reclaim, purge, delete, the
  consensus edit, activate, `set_allocations`, deactivate, force-complete,
  `complete_unless_purged`, `JobFinished`, `close_generation`,
  `merge_staged_in_slices`, seeding, `lift_passed_over`, export start and
  `mark_ready`, rating fits and membership, `mark_every_pool_for_refit`: no
  cycle, including the edit's reopen against activate, allocations, export
  start and `mark_ready`.
- Iteration 2's A2-1 (`unfinalize` failing running exports), A2-2 (the
  unlocked pre-check; the hold taken with no await before the spawned body),
  A2-3 (post-commit logging) and D2-1 (the in-flight read's equivalence under
  one slot) hold. Iteration 1's A-1, A-2, A-3 and A-5 hold; the one-slot index
  and counter CHECKs are unreachable by every writer.
- Consensus bookkeeping, finish checks and the debounce, leave generation,
  ratings versus admin, the derived builder, input data versus claims,
  exports versus purge, and single-instance enforcement (`desired_count`
  0..1, 0%/100%, the in-process state listed in KL-82).
- Not checked by A: `backups.rs` and the restore scripts versus running jobs,
  and account deletion versus claims beyond a skim (judged OK in iteration 1).

### Backend bugs and validation

**B3-1 (low). A NUL in a public parameter or an unauthenticated body was a 500.**
- **Context:** `error.rs` `From<sqlx::Error>`; reached from `routes/public.rs`
  (`?worker=`, `?rack=` on an opening-rack job, a leave cursor's rack half),
  `routes/auth.rs` (`login`, `request_password_reset`) and
  `routes/account.rs` (API-key label). Reviewer B, tracing each binding.
- **Problem:** Postgres refuses a NUL in text (SQLSTATE 22021), which fell to
  `AppError::internal`: a 500 and an ERROR log line for a malformed request,
  writable at will on unauthenticated, unthrottled routes. Results, declines
  and ban reasons already refuse NULs (passes 21 and 22); these paths were
  never covered. No 5xx alarm exists, so the cost was noise and a wrong status.
- **Options:** map the SQLSTATE centrally; refuse `\0` in `ApiQuery`/`ApiJson`
  (covers bodies too, but needs care around routes that store escaped text);
  a check per route.
- **Outcome (Y1):** the central mapping, for 22021 and also 22P05
  (untranslatable character): 400 `bad_request` with no database text. Every
  text the server binds that could carry a NUL is caller input, and results and
  declines refuse it before the database, so the mapping cannot mask a server
  bug. New `U-ERR-8` and `A-PUBLIC-8` (`?worker=%00`, a NUL username, a NUL
  reset address); PLAN API conventions.

**B3-2 (low). Re-adding an existing pool member logged and refitted.**
- **Context:** `routes/ratings.rs` `add_member` (`ON CONFLICT DO NOTHING`,
  result discarded). Reviewer B.
- **Problem:** a no-op add wrote a `rating_pool.member_added` row and stored a
  refit whose `membership` trigger the ratings page shows as a reason for a
  change that did not happen. `remove_member` (pass 22) and `update_pool`
  already answer their second click without either.
- **Options:** check `rows_affected()`; leave it as harmless.
- **Outcome (Y1):** `rows_affected() == 0` answers `{"run_id": null}`, no
  audit row, no refit. New `A-RATE-4c` (the rival and the anchor); PLAN's API
  row says so.

**B3-OK (checked and found correct).** Plausibility against MAGPIE at
`1a0ae932`: the PEG depth rule (`stage_idx + 1`, exhaustive only for one
`INT_MAX` stage at 40), the endgame bound, which analysis a turn gets
(including the forced-turn `static`), opening-rack sim detection, inference
bounds, the pass equity, removed position keys, leave totals, and bingo bonus
500 inside every absolute bound. `routes/worker.rs` claim and decode; auth
(CSRF on every mutating route by script, PASETO generation check, rate-limit
keys); `extract.rs` and `error.rs` apart from B3-1; admin validation; ratings;
`stats/`; exports; artifacts; derived; input data; email; SSE; config and
startup; racks; public pagination. Not re-read: `inputdata::walk_archive` and
`bradley_terry.rs` numerics (unchanged since `0149541`).

### MAGPIE arguments and `birdtest-contribute`

**C3-1 (low). A simmer with `num_plays` 1 never simulates.** Covered in (c).
- **Context:** birdtest `routes/admin.rs` player-config creation (only
  `num_plays >= 1`); MAGPIE `autoplay.c:426-429`, `move.h:670-683`,
  `simmer.c:194-196`.
- **Options:** refuse at creation (with or without a schema CHECK); refuse in
  MAGPIE's `config_contribute_apply_player_settings`.
- **Outcome (Y1):** refused at creation on `num_plays`; `A-ADMIN-3`'s test
  extended; PLAN sentence. No CHECK (coordinator decisions), no MAGPIE change
  (configs are made on the server). The form's "Moves Generated" input states
  no limits, so there was no hint to change. Not traced (moot after the fix):
  what `simulate` does with a single arm in an opening-rack job.

**C3-2 (low, doc).** Covered as O7i3-8. Optional hardening not done: turning a
malformed `expected_data` into a decline-and-continue rather than a run-ending
failure. It is unreachable while the floor discipline holds (the role set is
pinned by the schema CHECK, identical to MAGPIE's).

**C3-3 (low, doc).** Covered as O7i3-9.

**C-OK (checked and found correct).** Step 0 (branch, HEAD equal to the pin,
version equal to every floor copy, 18 fixtures byte-identical); the full
settings table at `1a0ae932`; `1a0ae932`'s diff for memory, error paths, every
caller of `get_top_simming_move` (one production caller), every consumer of a
forced turn's record (captured positions, plausibility, inference, divergence,
results, ratings, the Saved Positions display); the strict `expected_data`
check refuses no assignment the current server can build;
`config_contribute_use_win_pct`; the libcurl check; the movegen-margin
applier's remaining caller; AGENTS.md style (loop variables named `i`, as their
neighbours; not a finding); the derived-builder command lines; no new MAGPIE
dead code.

### Performance and storage

**D3-1 (medium). The leave universe probe was a sequential scan under the dispatch lock.** Covered in (e) #1.
- **Context:** `jobs/leave_gen.rs` `universe_exists` (called from
  `registry::acquire` under the dispatch lock), `seed_generation`,
  `any_rack_below_target`. Reviewer D, measured; nothing prior measured this
  probe (PLAN's leave rows measured selection only).
- **Options:** an `ORDER BY … LIMIT 1` that an index supplies, outside an
  `EXISTS`; the same `ORDER BY` inside the `EXISTS` (measured: still a
  sequential scan, because Postgres drops the `ORDER BY` there,
  `simplify_EXISTS_query`).
- **Outcome (Y1):** the `ORDER BY … LIMIT 1` forms as named constants;
  `seed_generation` reuses `universe_exists`. New `I-LEAVE-25` seeds two
  generations, `ANALYZE`s, asserts the old form plans a `Seq Scan` (so the test
  is not vacuous), and asserts the new ones use the expected index under
  `plan_cache_mode` auto and `force_generic_plan`, and answer correctly.
  KL-17 and KL-18 revisited (impact materially worse than documented, now
  fixed). Not measured: the cold-cache cost on the production instance class
  (the seconds-per-claim figure is an estimate from row counts and memory).

**D3-2 (low). The opening-rack finish check scanned every job's tasks.** Covered in (e) #2.
- **Context:** `routes/worker.rs` `finish_condition_met`
  (`JobType::OpeningRack`); the migration comment near the `tasks` indexes.
  Reviewer D.
- **Options:** D2-1's equivalence over three existing indexes (proposed and
  measured); leaving it, since it runs about once per job completion (rejected:
  its cost grows with the whole `tasks` table, not the job).
- **Outcome (Y1):** the equivalence (`OPENING_RACK_FINISHED`), with a doc
  comment; migration comment and PLAN copy corrected; new `I-STATS-9j` (old
  plan seq-scans, new plan names all three indexes custom and generic, and the
  semantics: done only when every task is completed, not with zero tasks, an
  available reissue or a claimed one). The existing `I-STATS-9c` tests pass.

**D-OK (checked and found correct).** The critical-path trace in (d); D-1,
D2-1 and D2-2 hold (the claimed-claims arm re-confirmed by EXPLAIN); the
match-test reads re-measured ((e) #3); the job page's task counts (26–58 ms,
display only, read pool); the consensus edit's cost (documented); exports,
purge and delete, leave merge and close, and the selection queries (unaffected
by D3-1); storage as in (f). Not measured at full volume: purge, merge, a
simmed opening-rack export, the derived builder's memory and disk.

### Docs (objective 7)

Every E3-*, ET3-*, G3-1 item, C3-2, C3-3, H3-4 and the doc halves of F3-2,
F3-3, F3-4, F3-7, F3-9 and F3-10 are recorded one per row in the Objective 7
table (O7i3-1 to O7i3-30), with the code, the doc, the decision and the
reasoning. In summary:
- **Code wins:** 18, made by Y4 (most), Y1 (E3-2, F3-2's comment in
  `backups.rs`, F3-7, F3-9), Y2 (F3-3) and Y3 (H3-4).
- **Doc wins:** 12, each also covered under its own id.

**ET3-1 (low). U-CFG-5 missed the root `.env.example`.**
- **Context:** TESTING U-CFG-5 ("every hand-kept copy"); `config.rs`'s test
  list; `.env.example:22-25`, the template compose users copy to `.env`.
  Reviewer ET.
- **Problem:** a floor raise that missed the root template would pass the test,
  and a developer uncommenting its line would run the compose stack below the
  real floor. Local stacks only: production reads Terraform's copy, which is
  checked.
- **Options:** add the file (and its "reports 0.1.1" comment) to the test;
  drop the number from the prose.
- **Outcome (Y1):** both copies in the root `.env.example` are checked; TESTING
  and PLAN's row name the file (with E3-2). Optional and not done: checking
  README's prose "defaults to `0.1.1`" (README:275, :838), which is true today.

**ET3-3 (low). S-TF-3 overstated what it proves.**
- **Context:** TESTING S-TF-3; `infra/tests/variables.tftest.hcl`
  `a_failed_deploy_is_alerted`. Reviewer ET; reviewer G's probe.
- **Problem:** the test asserts that a target is attached, not that it is the
  alerts topic; re-pointing it elsewhere keeps the test green.
- **Options:** reword; or raise CI's Terraform to 1.11+ and pin the topic's ARN
  with `override_resource … override_during = plan`.
- **Outcome (Y4):** reworded (TESTING, the run's comment, the assert message).
  G's probe confirmed the ARN cannot be pinned at plan on 1.9.8. The Terraform
  bump is optional and not done.

**ET-OK (checked and found correct).** All 563 TESTING entries' *(Covered: …)*
names resolve (Rust, TS, tftest, scripts, workflows, MAGPIE's
`contribute_test.c`); no duplicate or undefined ids; every `F-*` id matches a
`describe`; every validated Terraform variable has a refusal run; the status
table and the 707 backend total at `6c6c191` recounted from source; the
sampled tests (I-EXPORT-15/16, I-OR-EDIT-3/4/5, I-OR-REISSUE-1/2, I-SCHED-22,
U-PLAUS-6/7/8/9, I-JOB-1f/1g/2, A-WORKER-22, U-CFG-5, A-ADMIN-2c/29,
S-TF-1/2/3, F-AUTH-1/2/3, F-CONS-3/4, and others) each prove their entry and
would fail on revert (except as recorded); RUNBOOK §0, §2.3, §2.6, §5 and
"Rolling back a deploy" and README's deploy-failed check against the code and
infra; `runbook-check.sh` 33 and 20 blocks.

### Dead and out-of-date code

**F3-1 (low). `machine_letter()` existed only for tests.** Covered in (g) and
O7i3-27. Options were drop it and retarget the tests, or keep it; dropped
(Y1). `parse` keeps a local vector for its duplicate and alphabet-size checks.

**F3-2, F3-3, F3-4, F3-9, F3-10 (low).** Covered in (g) and the O7 table.

**F3-5 (low). `games.win_pct`/`loss_pct`/`draw_pct` had no consumer.** Options
were remove them (precedent AF29 F-11) or keep them as API conveniences
derivable from the counts beside them; removed (Y1). I-STATS-4 keeps its "no
NaN" guard through the test and the means.

**F3-6 (low). Every claim stated each file's `bytes`, which MAGPIE never read.**
- **Context:** `jobs/mod.rs` `ExpectedFile`, `derived.rs` `ExpectedDerived`,
  the assignment fixtures, PLAN's claim examples. Reviewer F's
  payload-versus-MAGPIE script (`contribute.c`'s readers).
- **Options:** remove it on both sides and recapture the fixtures; or keep it
  and say in PLAN why (a size in a decline log).
- **Outcome (Y2):** removed. `A-WORKER-7`'s test now asserts each
  `expected_data.files[]` entry has exactly the keys MAGPIE reads (`name`,
  `path`, `role`, `sha256`, `tarball_date`). Fixtures edited or recaptured and
  copied to MAGPIE; PLAN and README examples updated, with a sentence that no
  entry states a size. Same class as AF29 C-7.

**F3-7 (low). `dev.py`'s `-ritmmap true` shim.** Covered in (g) and O7i3-18.
Removed (Y1); `py_compile` only (no test tier for `dev.py`).

**F3-8 (low, owner). The `backups` table's RDS-snapshot branch is never written.**
- **Context:** `backend/migrations/0001_initial.sql` (`kind` CHECK allowing
  `'rds_snapshot'`, the `snapshot_id` column, `backups_has_single_location`),
  its PLAN copy; `backups.rs` `row_to_run`'s `or_else(snapshot_id)`; the admin
  backups page's Kind column; `api.ts`. Reviewer F's `CHECK … IN` sweep.
- **Problem:** every writer (`backup.sh` twice, `restore-roundtrip.sh`, a test)
  inserts `kind = 'pg_dump'` with an `s3_key`. Nothing records RDS snapshots
  (PLAN's backup design leaves automated backups unrecorded, and its future
  snapshot-restore dump would still be a `pg_dump` row). So `'rds_snapshot'` is
  unreachable, `snapshot_id` always NULL, the one-location CHECK reduces to
  `s3_key IS NOT NULL`, a fallback is dead, and the Kind column always reads
  `pg_dump`.
- **Options:** (a) drop `snapshot_id` and the CHECK, make `s3_key NOT NULL`,
  and drop `kind` and the Kind column (or restrict it to `'pg_dump'`),
  simplify `row_to_run`, re-copy the schema (a pre-release, in-place migration
  edit); (b) keep it as a documented placeholder for recording RDS snapshots,
  which nothing plans.
- **Recommendation:** (a), since nothing plans to write the branch and AF29's
  "write-only forensic columns are kept" rule does not apply to a column that
  is never written. **Not done: a small owner decision** (a schema change in
  the one migration), so it does not count as fixable for the loop.

**F3-11 (low). MAGPIE's decline fixture was read by no MAGPIE test.** Covered
in (g) and (i). Options were factor the body into a testable builder and test
it, or document that the decline is pinned on birdtest's side only; the first
(Y2), which also found and pinned the `actual`-omitted-versus-null difference.

**F-OK (checked and found correct).** Iterations 1–2's removals hold; remnant
greps (SPRT, redundancy, priority, chi-square, self-update, TODO/FIXME/HACK)
hit only deliberate history or why-not text; every Rust item has a production
use apart from F3-1 and the named test helpers; the `#[ignore]` sets run
nightly; every Cargo dependency and feature, npm devDependency and Python
import is used; every route has a consumer except the pending F-4 and F-12;
every frontend component, `lib` export and page is used or linked; all 95
`CONTRIBUTE_KEY_*` are used; every result, heartbeat, claim and decline
fixture key is a backend field; every table read and enum value used; every
config key, Terraform variable, local, data source, script and fixture
referenced; every `*.md` and `module::test_name` reference resolves; no
"older server" fallbacks in MAGPIE.

### Deployment

**G3-1 (low, doc).** Covered in (k) and O7i3-12.

**G3-OK (checked and found correct).** Iteration 2's deploy-failed rule
against AWS's documented event (source, detail-type, `resources` as the
service ARN, `detail.reason` and `deploymentId`, the default bus, the
unencrypted topic and its policy, the `-dr` copy); the `min_magpie_version`
validation against `parse_strict` (slightly stricter, harmlessly); the SES mock
deletion; U-CFG-5's `include_str!` paths (test-only, absent from the release
build). The whole of Terraform: validations and refusal runs, ECS environment
against every key `config.rs` reads, IAM against every SDK call, S3, SES, ALB
health and keep-alive (310 s against 300 s), `desired_count` 0..1, the
breaker and its 600 s grace, container-definition sizes, ops scripts and ECS
Exec, the provider lock's platform hashes. Docker, CI and the nightly against
TESTING (apart from G3-1); auth cookies and CSRF; no unfinished-work markers.
Considered and not raised: the deploy-failed mail's "rolled back to the
previous release" is inaccurate for a shared SSM failure during password
rotation, which the RUNBOOK section it points to already explains. Not
checked: real AWS behaviour and Fargate quotas in a fresh account.

### Frontend, e2e and the fake worker

**H3-1 (low, security-adjacent). A failed sign-out said signed out.** Covered
in (b) and O7i3-28.
- **Context:** `frontend/src/lib/auth.ts` `signOut`, `+layout.svelte`
  `handleSignOut`, TESTING F-AUTH-2, PLAN KL-59 (open since the thirty-first
  audit, "needs a UI decision"). Reviewer H.
- **Options:** keep the session and say the sign-out failed; retry; for the
  re-ask, `resetSession` or `refreshSession` (coordinator decisions).
- **Outcome (Y3):** the first, with `refreshSession`: on failure the store
  follows `/api/me` (a live session stays signed in; a 401 on the re-ask, a
  logout whose answer was lost, shows signed out), and the layout says
  "Could not sign out (…); you are still signed in. Try again." without
  navigating. KL-59's UI decision is made and recorded as implemented. Not
  done: special handling of a 403 for a missing CSRF cookie (the session can
  then never be ended through the UI); the error message covers it, and it is
  rare.

**H3-2 (low). Recorder and margin shown as live where never read.** Covered in
(b) and O7i3-29. Y3 verified PLAN's tables against MAGPIE (`config.c`: a
simmer generates with `MOVE_RECORD_ALL`; `move_gen.c` reads the margin only for
`WITHIN_X_EQUITY_OF_BEST`) before muting. The coordinator's `differs` change
completed it. E-10 only asserts the row is visible, so it is unaffected.

**H3-3 (low). Three inputs bounded differently from the server.** Covered in
(b). Options for (c) were widen the browser bounds or drop them for a script
check; dropped, with `confidenceProblem` mirroring the server as
`consensusProblem` does for the share. New `F-TEST-3`.

**H3-4 (low, comments).** Covered as O7i3-16.

**H-OK (checked and found correct).** Iteration 2's `resetSession` (one retry,
two callers, the 401/403-only rule); H2-4's `maxlength`; the bingo-bonus and
sim-cutoff inputs; the job form against `validate_job_body` field by field;
the player-config form against `CreatePlayerConfigBody` and
`resolve_solver_settings`; the consensus editor and `PATCH /consensus`; every
`api.*` wrapper's route, method and shape; CSRF; SSE; error handling; the e2e
suite (21 tests, claim bodies with `board_dim`/`rack_size`, page caps, the
match-test and capture rules); the fake against the contract and MAGPIE's
fixtures; `fake-worker-fixtures.sh`; the nginx template. Not opened line by
line: the allocation, derived-data, backups, audit-log and users pages and
`charts/*` (untouched since prior passes).

### Open items carried forward

**Pending the owner** (genuine trade-offs; they do not count as fixable for the
loop). Unchanged from AF29 and AF30:
- **B-1:** rating-pool scoping by bingo bonus and sim cutoff (KL-75).
- **B-2:** a consensus edit reopens a force-completed job.
- **D-3:** batch defaults (KL-93).
- **D-4:** `task_claims` job-scoped indexes (would also make D2-1's and D3-2's
  claimed arms exact).
- **D-5:** the FK drops and the KL-10 re-measurement (D's numbers in (e) #3
  can feed it).
- **D-8:** export snapshot versus vacuum (KL-94).
- **F-4 / A-RATE-6:** the rating-history endpoint.
- **F-12:** the admin results stream.
- **F-15 and F-16:** `SETTINGS_COMPARISON.md`'s anchors and the root plan
  docs.
- **G-3:** `wait_for_steady_state`.
- **W2a:** the even-batch CHECK (B-5) and exact-mover attribution (B-4).
- **Also noted, not blocking:** the D-7 fleet semantics; production `jit`;
  bumping `MAGPIE_VERSION` and the floor together before a first deployment
  (PD-9).

**New pending item from this iteration:**
- **F3-8:** the `backups` table's `rds_snapshot` kind, `snapshot_id` column
  and single-location CHECK, never written: drop them (recommended) or keep
  them as a placeholder.

**Closed this iteration:** KL-59 (a failed sign-out), whose open "UI decision"
H3-1 made; the owner may wish to review the wording the layout shows.

**Fixable, optional, carried to iteration 4** (none attempted this iteration):
- E2-1's MAGPIE half: move `io_util.c`'s platform `#if` into `src/compat/`
  and drop PLAN's sentence about it.
- C2-7's test comment ("at either BOARD_DIM"); the testdata `data-20251004`
  date note; an `ANALYZE` of RUNBOOK §2.1's scratch copy.
- New optional items, not done: KL-10's re-measured match-test numbers (D);
  README's floor prose in U-CFG-5 (ET3-1); CI's Terraform bump to pin S-TF-3's
  target (ET3-3); a decline-and-continue for a malformed `expected_data`
  (C3-2).

**Blocking CI until done (owner action):** push `birdtest-contribute` with
`c3c6a875`, `1a0ae932` and `baa82cfc`.
