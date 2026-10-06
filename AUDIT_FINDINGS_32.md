# AUDIT_FINDINGS_32 — the thirty-third audit (2026-10-05), pass 4 (iteration 4, the last)

Branch `audit/birdtest-2026-10-05`, the same branch as iterations 1 to 3.
Iteration 1's work is committed on it as `fe701c8`, `deaacd8` and `e529018`,
iteration 2's as `6c6c191` and iteration 3's as `77c7b52`. This iteration
audited `77c7b52` and commits on top of it.

MAGPIE: `~/MAGPIE` is on `birdtest-contribute`. The iteration started at
`baa82cfc` (iteration 3's commit), which `docker/Dockerfile` pinned. This
iteration changed one MAGPIE file, `test/birdtest_contract/README.md` (a doc
correction, C4-2). It is committed on `birdtest-contribute` as
`02a91dfe`, and the Dockerfile pin moves to that commit so that the
pin stays equal to the branch head. `MAGPIE_VERSION` stays 0.1.1, and so does
`MIN_MAGPIE_VERSION` in every copy.

> **STILL NOT PUSHED.** Neither `birdtest-contribute` nor the birdtest branch
> is on a remote; `origin/birdtest-contribute` is still `dbf8df3b`. Until
> `birdtest-contribute` (now with `c3c6a875`, `1a0ae932`, `baa82cfc` and
> `02a91dfe`) is pushed, these fail at the step that fetches the pin:
> - CI's `magpie-contract` job;
> - the e2e image builds (tier 5) in CI;
> - the nightly job, on both e2e legs (the pin leg at its MAGPIE checkout, the
>   head leg at "Start the stack", whose backend image fetches the pin).
>
> The pin cannot be reverted to the pushed commit: the server refuses a
> pre-`c3c6a875` claim (no `board_dim`/`rack_size`) with a 400.

**Builds on** AUDIT_FINDINGS_29 (iteration 1), _30 (iteration 2) and _31
(iteration 3), all on this branch, and through them AUDIT_FINDINGS_7 to _28
and PLAN.md's Known Limits KL-1 to KL-94. **No KL is added this iteration**,
so the set is still KL-1 to KL-94 (KL-95 is free). KLs revisited (none
renumbered):
- KL-18 and KL-25: who still reads the rows their retention options would
  drop (D4-3);
- KL-62: the closure extended to `github_token_parameter_arn` (G4-1);
- KL-68: the `MAGPIE_VERSION` bullet's premise corrected (C4-1).

**Process.** The run follows `audit_loop.md`. Iteration 4 ran in two stages:
1. Nine read-only reviewers, each re-auditing its whole scope from scratch and
   reading iterations 1 to 3's diffs (`git diff 0149541..77c7b52`, with
   `6c6c191..77c7b52` line by line): A races and locking; B backend bugs;
   C MAGPIE arguments and `birdtest-contribute`; D critical path, performance
   and storage (with measurements); E PLAN against the code, and prompt drift;
   E2 TESTING, RUNBOOK and README; F dead and out-of-date code; G deployment;
   H frontend, e2e and the fake worker. The reviewers were interrupted once by
   an API spend limit and resumed with their context.
2. Four fixers, working in one tree:
   - Z1: backend (D4-1, D4-2, B4-1, B4-2, F4-1, ET4-2, G4-3);
   - Z2: frontend (H4-1 to H4-4);
   - Z3: docs and infra (G4-1, G4-2, C4-1, C4-2, ET4-1, E4-1 to E4-4, F4-2,
     F4-3, D4-3);
   - Z4: a follow-up the coordinator added after Z2 reported a gap it could
     not close from the frontend: the pool detail API returned no member
     list, so a member added since the last fit was offered under Add and had
     no Remove button. Z4 added `members` to `GET /api/rating-pools/:id` and
     rebuilt the page's membership from it (`A-RATE-10`, `F-RATE-1`).

**Findings this iteration, after de-duplication:** **0 high, 0 medium, 23 low**
(23 reported, no duplicates), plus Z4's follow-up to H4-3. Reviewer A (races)
found nothing new. Every finding is fixed; nothing new was sent to the owner.

**Loop end: the loop stops here without reaching a clean pass.** During
iteration 4 the owner instructed that no further pass be started: finish this
pass and stop. So the loop ends after iteration 4 although iteration 4 found
(and fixed) issues, which by `audit_loop.md` step 2 would otherwise call for
iteration 5. Stated plainly:
- No pass of this loop came back clean. The confirmation pass that step 3
  presumes was not run.
- The trend across passes was **4 high + ~9 medium** (iteration 1) →
  **0 high + 5 medium** (iteration 2) → **0 high + 1 medium** (iteration 3) →
  **0 high + 0 medium** (iteration 4, 23 low).
- Iteration 4's findings are low-severity: doc sentences left behind by
  earlier redesigns, two "one index probe" queries of the D3-1 class, two
  malformed-input edges, and small UI and form gaps. A further pass would
  likely find more low items of the same kinds, at a falling rate; it would be
  unlikely, on this trend, to find a high or medium one, but that has not been
  shown.
- The end-of-loop checks (step 3) are recorded under **Final checks (loop
  step 3)**.

---

## Pass summary

### (a) Branch, prior findings and versions

- **Branch:** `audit/birdtest-2026-10-05` (unchanged; one branch per loop run).
- **Prior findings:** as listed in AUDIT_FINDINGS_29 (a): `_7`–`_26` on
  `origin/audit/birdtest-2026-09-24-pass21`, `_27` on local
  `audit/birdtest-2026-09-25` (and `_27`/`_28` in `main`'s history until
  `7e74935`), and `_29`, `_30` and `_31` on this branch (local, unpushed). The
  highest number on any ref is 31 (reviewer E, `git ls-tree` over every local
  and remote ref), so this file is number 32.
- **What iteration 3 left open, and what happened to it:**
  - F3-8 (the `backups` RDS-snapshot branch): still pending the owner, not
    re-flagged.
  - The optional carried items (E2-1's MAGPIE half, C2-7's test comment, the
    `data-20251004` date note, an `ANALYZE` of RUNBOOK §2.1's scratch copy,
    KL-10's re-measured numbers, README's floor prose in U-CFG-5, CI's
    Terraform bump for S-TF-3, a decline-and-continue for a malformed
    `expected_data`): **none attempted**, none re-raised as a defect; they stay
    optional (Open items).
- **KLs added:** none. **KLs revisited:** KL-18, KL-25, KL-62, KL-68.

### Coordinator decisions on flagged items

- **H4-3's remaining gap → Z4.** Z2 fixed the client type and built the Add
  list from configs that are neither rated nor the anchor, but reported that
  the pool detail served only the latest fit's ratings, so a member added
  after the fit (or whose refit failed after the membership committed) was
  still offered under Add and had no Remove button. Rather than leave it, the
  coordinator added package Z4: `GET /api/rating-pools/:id` serves `members`
  (`player_config_id`, `name`; not `added_by` or `added_at`), and the page
  takes its membership from it. Z2's "Already a member: nothing changed. Use
  Recompute to refit." notice stays and is now reachable only through a race.
- **H4-1, the label.** Z2 did not branch on `snapshot_at` against
  `completion.at` (the reviewer's alternative): a job reopened by an edit and
  completed again has a newer `completion.at`, and its demoted export would
  still read "taken while running". It names both possible reasons for an
  opening-rack job instead (only opening-rack jobs can be demoted). Accepted.
- **H4-4, extra minimums.** Z2 also put `min="1"` on the simmer's iterations,
  plies and plies-recorded inputs, matching `validate_player_config_body`.
  Accepted.
- **F4-1, delete rather than keep a test seam.** Z1 deleted
  `lock_claim_decisions` and made `jobs::try_lock_job_dispatch` `pub` (with a
  doc line saying it is public for the tests), which the two tests now call.
  Accepted.
- **B4-1, no global 22008 mapping.** As the reviewer advised, a 22008 from
  server arithmetic would be a real fault, so the cursor parser drops
  impossible times instead.
- **MAGPIE pin.** The C4-2 README edit changes no code or fixture, so no
  birdtest behaviour depends on it; the pin moves to `02a91dfe`
  anyway, so that Step 0's "HEAD equals the pin" stays true.

### (b) Bugs and fixes made in birdtest

#### Race conditions

Reviewer A re-traced, from scratch, the documented lock order (merge,
dispatch, claim, task, job, contributor rows) on every writer in scope:
claim, submit, decline, heartbeat, reclaim, purge, delete, the consensus edit,
activate, `set_allocations`, deactivate, force-complete, leave close, merge,
staging and seeding, exports, ratings, the derived builder and input data.
**No new finding.** No lock-order cycle, lost update, double dispatch or
counter drift; iteration 3's A3-1, A3-2, B3-2, D3-1 and D3-2 hold under
concurrency, and iterations 1 and 2's race fixes still hold. Two near-misses A
examined and judged correct are recorded under A4-OK.

#### Other bugs fixed

| ID | Sev | Bug | Fix | Test |
|---|---|---|---|---|
| D4-1 | low | See (d) and (e). | | `I-STATS-9k` |
| D4-2 | low | See (e). | | `I-EXPORT-17` |
| B4-1 | low | `GET /api/jobs/:id/results?cursor=` (public, unauthenticated, unmetered) accepted a cursor holding a time before 4713 BC: chrono holds it, Postgres's `timestamptz` does not, and the bind failed with SQLSTATE 22008, answered `500` with an ERROR log line. The documented contract is that anything not a cursor this server produced reads as "start from the beginning". The B3-1 class, on the last public path found. | `micros_to_time` keeps only times at or after 1970 (`.filter(|t| t.timestamp() >= 0)`; no result predates it), so the cursor reads as no cursor. `error.rs` unchanged. | New `A-PUBLIC-3d` |
| B4-2 | low (dev stack only) | `/api/dev/login`'s `next` filter (starts with `/`, not `//`, no `\`) let `/\t/evil.example` through, which a browser reads as `//evil.example` (an open redirect off the local stack), and `/\n` made axum's `Redirect::to` panic on an invalid header value (no `CatchPanic` layer, so the connection dropped). `DEV_LOGIN` is refused beside `SECURE_COOKIES=true`. | The filter also requires every byte to be visible ASCII (`0x21..0x7f`); anything else redirects to `/`. | `A-AUTH-13` extended (`/\t/evil.example`, `/\n`, `/jobs\r\n` → `303` to `/`) |
| H4-1 | low | Since iteration 3 any consensus edit of a completed job demotes its final exports, but the export card labelled every non-final export of a completed job "taken while the job was still running", which a demoted export was not. `JobExport.is_final`'s doc said the same. | `exportSummary`: a completed opening-rack job's snapshot reads "not the job's final results: read before it completed, or before its consensus settings last changed"; other job types (never demoted) "read before the job completed". `is_final` doc reworded. | `F-FMT-14` reworded; exact-label cases in `format.test.ts` |
| H4-2 | low | The settings tables bolded "Moves Generated" between two static players in a games or pairs job that captures nothing, though autoplay never reads a static player's `num_plays` there (it only sizes the move list, raised to the capture cap when capturing; the move played is the top-equity move). Likewise shown live for a leave job. The H3-2 class. | `num_plays` added to `LEAVE_UNUSED`; new `UNCAPTURED_STATIC_UNUSED` for a non-capturing games or pairs job with no simmer. A simmer, or capture, keeps the row live. | `F-SET-1`'s test extended (four cases) |
| H4-3 + Z4 | low | Iteration 3's no-op `add_member` answers `{"run_id": null}`, but (a) the client type said `string`, and (b) the pool page derived membership from the latest fit's ratings, so a never-fitted pool offered its own anchor under Add, and a member added since the fit was offered too, both now a silent no-op, with no Remove button for such a member. | (a) `post<{ run_id: string \| null }>`; a null shows "Already a member: nothing changed. Use Recompute to refit." (b) Z4: `pool_detail` serves `members` from `rating_pool_members` (name order, primary-key lookup); new `lib/ratingPool.ts` `poolMembership` gives members, unrated members and the Add list; unrated members get a row ("not yet rated", Remove for admins except on the anchor); a rated config removed since the fit shows "removed" instead of a Remove that would 404. PLAN's API row and `/ratings/[id]` bullet. | New `A-RATE-10`, `F-RATE-1` |
| H4-4 | low | The player-config form's simmer "Moves Generated" had no `min`, so the server's iteration-3 rule (a simmer needs at least 2) surfaced only as an error after submit. | `min="2"` and help text ("At least 2: with one, MAGPIE plays it without simulating."); `min="1"` on iterations, plies and plies recorded. | Markup; no id |
| ET4-2 | low (test gap) | U-CFG-5 checked the root `.env.example`'s "`birdtest-contribute` reports 0.1.1" sentence (ET3-1) but not the same sentence in `backend/.env.example` and `docker-compose.yml`, so a floor raise could leave them stale with the test green. | Two rows added to `every_copy_of_the_version_floor_agrees` (a shared `REPORTS` prefix); TESTING names both sentences. | `U-CFG-5` |
| G4-1 | low | See (k). | | S-TF-2 (three new runs) |
| F4-1 | low | See (g). | | — |

### (c) MAGPIE argument coverage (objective 3)

Reviewer C re-traced every outcome-affecting setting at `baa82cfc`.
`config.c` and the executors are unchanged since `1a0ae932`, so iteration 3's
table holds row for row, re-verified directly:
- `contribute_required_player_keys` and `contribute_required_simmer_keys`
  require every setting a `PlayerSpec` carries (the PEG and nested-PEG keys
  required when PEG is on and refused when off);
- every `json_get_*_or` in the contribute region (23 sites) is a required key
  read after `contribute_require_keys`, a derived-file flag pinned or declined
  through `derived`, or a capture flag birdtest always serializes;
- `config_contribute_reset_player_settings` resets every per-player field
  before applying;
- `seed` is required in all three executors, and autoplay's per-game seeds
  make a static games task independent of the thread count;
- the solver bounds agree (`MAGPIE_MAX_ENDGAME_PLIES` 25 = `MAX_VARIANT_LENGTH`,
  `MAGPIE_PEG_MAX_BAG` 4 = `PEG_MAX_BAG`); board and rack size are stated in
  the claim; C3-1's simmer rule holds and no seed, e2e or test script
  contradicts it.

The only contributor-local input remains `threads` (KL-14, accepted). **No
gap found.** The determinism claims (PLAN 171-175, 2423-2427, KL-14) remain
accurate: only static, non-solving players are deterministic.

### (d) Critical-path and async changes (objective 4)

Reviewer D re-traced claim, heartbeat, decline, submit and completion at
`77c7b52`. Nothing purely observational runs inline beyond the documented
exceptions (KL-5, KL-28, the display columns of the job `UPDATE`); ratings are
neither read nor written on the claim, submit or completion path; the
match-test debounce is unchanged and safe.

Changes this iteration that touch the path:
- **The "anything in flight" probe (D4-1).** `should_check_finish` runs it
  inline on 7 of 8 submissions (the worker waits for it), and
  `finish_idle_job`, the export's settling and `is_final` checks, the job
  list's `stalled` and two leave reads (`claims_in_flight` under the leave
  dispatch lock, `furthest_below_target`'s `out_now`) ask the same thing. It
  joined `tasks` to filter by job; it now reads `task_claims.job_id`, which is
  written at claim time and never changed (the form D3-2 already relies on).
  See (e) #1.
- **Pool detail (Z4).** One indexed read more on a public display route; off
  the critical path.

Noted, not flagged: a leave submission writes a `leave_records` row that
duplicates the completed claim (`rack_count`, `submitted_at`) and is read only
by the delete census (`admin.rs`), a sub-millisecond observational insert;
dropping the table would touch the census format, `restore-job.sh`, RUNBOOK
§2.2 and four tests (Open items). Also noted: an on-demand task is inserted
`available` and updated to `claimed` in the same transaction (one extra tuple
version; millisecond-level).

### (e) Performance, most to least severe (objective 5)

Measured by reviewer D on a scratch database (`d4perf`, PG 16.15 at defaults,
`jit` on, `synchronize_seqscans = off`, custom and forced-generic plans:
2.2 M tasks, 2.33 M claims with 600 open, 2.1 M game results, 5 M leave
progress rows, 1.8 M position records), and re-checked by Z1 with EXPLAIN on
`z1scratch`; both were dropped.

| # | ID | Issue and expected impact | Status |
|---|---|---|---|
| 1 | D4-1 | The in-flight probe (`task_claims c JOIN tasks t … WHERE t.job_id = $1 AND c.state = 'claimed'`) walks the fleet-wide open-claims index with one random `tasks_pkey` probe per open claim until it finds the job's; for the submissions that decide completion (the job's last claims landing) and for `finish_idle_job`, that is nearly every open claim in the fleet: 4.8 ms warm after a restart, 69 ms fully cold at 600 open claims (about 0.1 ms per open claim cold, so ~0.5 s at 5,000). **Without statistics** (after a `pg_restore` until ANALYZE, or while autovacuum lags a fast job) the plan flips to a bitmap of the job's whole task history: 311 ms for a 250,000-task job, 101 ms for 100,000. Low because RUNBOOK §5 analyzes before the service starts and §1's PITR keeps statistics. | **Fixed** (Z1): `JOB_HAS_OPEN_CLAIM` (`routes/worker.rs`) = `… FROM task_claims c WHERE c.job_id = $1 AND c.state = 'claimed'`, used by `should_check_finish`, `finish_idle_job` and `exports::refuse_unsettled`; the same filter in `read_snapshot`'s `is_final`, `public.rs`'s `stalled` and both leave reads (which keep `JOIN leave_requests`). 0.04–0.27 ms with or without statistics, custom or generic. Not D-4: no schema change. PLAN measured rows. Other `JOIN tasks` sites (admin, jobstats, scheduler, worker) were not in the finding and are unchanged. |
| 2 | D4-2 | An export's "has this job captured positions" `EXISTS (… position_analysis_records WHERE job_id = $1)` is planned as a sequential scan for a job that did capture (the D3-1 shape): measured 129–147 ms behind 1.5 M older records; at tens of millions of rows (an English opening-rack job is 3.2 M records times its analyses; a 400,000-game capture job about 9 M positions, KL-23) a few seconds warm, tens of seconds of cold I/O on `db.t4g.micro`, evicting the claim path's cache and lengthening the export snapshot's hold on vacuum (KL-94). Once per games or pairs export, off the claim and submit paths. | **Fixed** (Z1): `exports::POSITIONS_CAPTURED` = `SELECT 1 … WHERE job_id = $1 ORDER BY submitted_at DESC, id DESC LIMIT 1`, read with `fetch_optional(..).is_some()`: an index-only scan of `position_analysis_records_feed_idx`, 0.02–0.05 ms custom and generic. `I-EXPORT-14`'s wait-for-statement prefix updated. PLAN measured rows. |
| 3 | KL-10 note | `plain_game_stats` (the finish check's read every 8th submission) re-measured: 26–37 ms at 100,000 rows of 2.1 M, 264 ms for a 250,000-row job with a cold index; grows with the square of the job's rows, as KL-10 says. | **Informational**, not re-flagged (KL-10, D-3/KL-93) |
| 4 | D-4 data | The job page's ETA read joins `tasks` for `job_id`: 165 ms (100,000 tasks, 220,000 recent completions) against 46 ms with `c.job_id`. Display only, read pool. | **Data for D-4** (pending), not a new finding |
| 5 | jit note | A full English leave merge pass compiled JIT every pass, about 230–250 ms each over 8 passes (7.1–7.5 s for 266,694 rack updates). | The pending "production `jit`" note, not re-flagged |

Iteration 3's rewrites were re-verified on the fresh dataset: `UNIVERSE_PROBE`
0.03–1.4 ms (including with `pg_statistic` deleted), `BELOW_TARGET_PROBE`
0.02–0.9 ms, `OPENING_RACK_FINISHED` 0.04 ms warm, 1.1 ms cold, 0.7 ms without
statistics. D hunted every other `EXISTS`, `COUNT`, `LIMIT 1` and `MIN`/`MAX`
on the growing tables; apart from D4-1 and D4-2, each is served by an index
leading with its filter or bounded by the fleet's open claims (D4-OK).

### (f) Storage (objective 6)

**Fixed:** nothing structural; no schema change this iteration.

**Doc corrected (D4-3):** KL-18 and KL-25's retention options understated who
reads the rows they would drop (see the Issues section). The retention
decisions themselves stay with the owner.

**Flagged:** nothing new. F3-8, D-3 (KL-93), D-4, D-5 and D-8 (KL-94) remain
with the owner.

**Verified:** `task_claims` has 8 indexes, as its fillfactor comment says,
and the one-slot and open-claims indexes are both needed;
`leave_rack_progress.updated_at` is read by the public leave feed; rating-run
thinning, export expiry, backup snapshot and counts are unchanged since
iteration 3; KL-24, KL-27 to KL-31 and KL-38 still describe the code; the
derived builder hashes outputs in 8 MiB chunks, so the 1.9 GB RIT is never
resident.

### (g) Dead or out-of-date code removed, and items flagged as uncertain

**Removed (verified against callers, then by compiler and tests):**
- `leave_gen::lock_claim_decisions`, a one-line wrapper over
  `try_lock_job_dispatch` with a 28-line doc, called only by two tests since
  `e25f460` (2026-09-14). Its rationale (why leave generation most needs the
  dispatch lock) moved onto `registry::acquire`'s lock comment, and
  `claim_transition`'s safety argument now cites the lock `registry::acquire`
  takes (F4-1).
- The pool page's `candidates` and `poolMembers()`, replaced by
  `poolMembership` over the served members (H4-3, Z4).

**Stale text corrected:**
- PLAN "Position Capture From Games › MAGPIE changes": the typed-accessor API
  (`AutoplayPosition`, `autoplay_results_get_positions()`, "mirrors
  `autoplay_results_get_game_summary()`", "parses `-hr false` summary
  lines"), which MAGPIE removed on 2026-08-28 (`bc620e55`, `62f6ecbb`),
  replaced by the JSON-writer design; `pair_game_number` is "0 for an unpaired
  game; 1 or 2 within a pair"; "seven function pointers" → eight (F4-2).
- PLAN Backups Layer 3's "harmless if `restored_at` context is displayed"
  (nothing has a `restored_at`) (F4-3).
- PLAN's "eight rules" left over from the redundancy removal (E4-1), and the
  pre-consensus finish-condition sentence (E4-2).
- TESTING's two "wordmaps and rack info tables" sentences (ET4-1) and
  `dev.py`'s queue docstring and log line (G4-3), which iteration 3's
  word-info-table sweep missed.

**Flagged for the owner:** nothing new. F-4, F-12, F-15 and F-16 remain
pending; F-15's `SETTINGS_COMPARISON.md` has 96 migration line anchors, all
stale again. Recorded, not flagged: test seams that guard real guarantees
(`scheduler::reclaim_expired`, `auth::public_anon_id`,
`jobstats::worker_contributions`, `extract::available_kib`,
`racks::enumerate_leaves`); the unreachable `GREATEST(active_claim_count - n,
0) > 0` arms, kept as drift defence (AF29); the `leave_records` table (only
the delete census reads it; see (d)); other two-item "wordmap and rack info
table" mentions that are correct in context (the provenance section, the
claim's pinned hashes).

### (h) Python-worker-as-production-client corrections

None needed. Reviewers F and H found no code, doc, config or CI step that
treats `worker/fake_worker.py` as a production or dev client; `--server-url`
has no default and every flag and mode has a user. `scripts/fake-worker-fixtures.sh
--check` passed for reviewers F, G and H against iteration 3's recaptured
fixtures (no `bytes`).

### (i) `birdtest-contribute` changes, pin and version floor

| Change | Why | Files |
|---|---|---|
| "The `result-*` fixtures except `result-games-inference.json` (written by hand in MAGPIE's key layout), and the `heartbeat`, … were captured" | C4-2: the README called `result-games-inference.json` captured; birdtest's README (correctly) says it was written by hand | `test/birdtest_contract/README.md` |

- **Commit.** `02a91dfe`, on top of `baa82cfc`, **not pushed**.
- **Pin.** `docker/Dockerfile` `ARG MAGPIE_COMMIT=` moves from `baa82cfc` to
  `02a91dfe`, keeping the pin equal to the branch head. CI (two jobs)
  and the nightly read it from that line.
- **Version and floor.** `MAGPIE_VERSION` stays 0.1.1 and `MIN_MAGPIE_VERSION`
  stays 0.1.1 in every copy (U-CFG-5 now also checks the "reports 0.1.1"
  sentences of `backend/.env.example` and `docker-compose.yml`). A doc-only
  change calls for no bump; PD-9 (bump both before a first deployment) stands.
- **Protocol, fixtures and derived data.** Unchanged: no fixture, no builder
  and no conversion code changed, so the 18 fixtures stay byte-identical to
  MAGPIE's copies and no derived-data hash changes.
- **MAGPIE checks.** A README edit changes no code, so MAGPIE's build and
  tests are unaffected by it; what was run at the end is under **Final
  checks**. cppcheck and clang-tidy are never run on this machine (the
  owner's standing instruction).

### (j) Findings file and count

`AUDIT_FINDINGS_32.md` — objective-7 discrepancies this iteration:
**code-wins 13 / doc-wins 8 / unresolved 0** (O7i4-21, TESTING's status
table, is regenerated by the coordinator at commit). Iteration 1's three
unresolved discrepancies (B-1, B-2, F-15) are unchanged and are not counted
again.

### (k) Deployment blockers

No high deployment blocker. Reviewer G re-ran Terraform 1.9.8 (the version CI
pins) in Docker on a scratch copy: `fmt -check -recursive` clean,
`init -backend=false` (lock file unchanged), `validate`, `test` **54 / 54**;
after Z3's change, **57 / 57**. G re-checked the fresh-account path (README
"Deploying" steps 1–8), ECS environment against `config.rs`, IAM against every
AWS call, RDS, ALB health and keep-alive, the circuit breaker, backup and
restore wiring, ops scripts, Docker, compose, nginx, CI and the nightly
against TESTING, and auth (all 39 mutating session routes call
`csrf::verify`) (G4-OK).

| ID | Issue | Resolution |
|---|---|---|
| G4-1 | `github_token_parameter_arn` (expected in production: PLAN says unauthenticated ref resolution is 60 calls an hour) had no validation and README no recipe. A parameter **name** passes plan and fails at `PutRolePolicy` half-way through the apply (IAM `MalformedPolicyDocument`), the class KL-62's closure fixed for `acm_certificate_arn`; another region's ARN applies cleanly and every web task then fails at start; a customer-managed KMS key fails the same way (no `kms:Decrypt`). | **Fixed** (Z3): validation (empty, or `^arn:aws[a-z-]*:ssm:${var.region}:[0-9]{12}:parameter/.+$`); description says ARN, region and default `aws/ssm` key; three S-TF-2 runs (an in-region ARN plans; a name and another region's ARN are refused); README "Deploying" gains the optional third parameter's recipe (read-only fine-grained token, default key, `get-parameter --query Parameter.ARN`); KL-62's closure extended; TESTING's S-TF-1 run count 54 → 57. |
| G4-2 | `alert_email`'s description said the topic carries backup alarms; it carries every alarm the stack raises (site down, deploy rollback, RDS, SES, backups, restore drill). An operator could route it to a low-attention mailbox. | **Fixed (doc)**: description lists every alarm; README's closing note says "alarms". An in-place description change. |
| G4-3 | `dev.py` announced the queue as "wordmap / rack info table file(s)". | **Fixed (doc)**. |
| — | The MAGPIE pin is not pushed. | **Open**: push `birdtest-contribute` before CI, the e2e images and the nightly can pass (header). |

**Terraform variables with no default:** still eight (PD-1).

### (l) Prompt drift

Iterations 1 to 3's items, re-verified (reviewer E, with C, F and G). **No
new item this iteration.**

| # | Status now | Checked in |
|---|---|---|
| PD-1 | **Holds.** Eight no-default variables: `backend_image`, `frontend_image`, `alert_email`, `acm_certificate_arn`, `mail_from_address`, `ses_domain`, `public_url` (`variables.tf`) and `derived_builder_image` (`derived.tf`). Objective 10 lists five | script over every `variable` block (E, G) |
| PD-2 | **Holds.** The builder builds wordmaps, rack info tables and word info tables; leave KLVs are built by the web process. ET4-1 and G4-3 were in-repo leftovers of the same omission | `derived.rs`, migration |
| PD-3 | **Holds.** The pentanomial cross-check is in `game_pair.rs`; `plausibility.rs` holds the shared checks | grep |
| PD-4 | **Holds.** CI uses a nextest archive, adds `terraform test`, `fake-worker-fixtures.sh --check` and MAGPIE `builderhash`/conversions; the Terraform job is fmt, init, validate and test | `ci.yml`, `nightly.yml` |
| PD-5 | **Holds.** Neither `dev.py` nor tier 6 has a fake-worker mode to refuse | grep |
| PD-6 | **Holds, dated.** `_29` to `_31` are on the local, unpushed audit branch; this file is `_32` | `git ls-tree` over every ref |
| PD-7 | **Resolved** (earlier iterations) | — |
| PD-8 | **Holds** (KL-82) | — |
| PD-9 | **Holds.** `MAGPIE_VERSION` `"0.1.1"` (`config.c:1024`), every floor copy 0.1.1. The protocol changed between `dbf8df3b` and `baa82cfc` without a bump, consistent with the owner's "nothing in prod" rule; bump both before a first deployment | MAGPIE `config.c`; U-CFG-5 |
| PD-10 | **Holds, one more commit.** `main` and `origin/main` are `0149541`; `birdtest-contribute` is four commits past the Snapshot's `dbf8df3` (`c3c6a875`, `1a0ae932`, `baa82cfc`, `02a91dfe`), which the Dockerfile pins; none is pushed | `git rev-parse`, Dockerfile:6 |
| PD-11 | **Holds.** Holds come from seeding, purge, delete and the consensus edit; lifecycle actions, exports, merge-progress and artifact rebuilds refuse with `409` while one is held (E4-3 brought PLAN's API rows into line) | `admin.rs` |
| PD-12 | **Holds.** The build axis is checked before the floor | `routes/worker.rs` |
| PD-13 | **Holds.** No `sprt` or chi-square code; only why-not history (`match_test.rs`, `plausibility.rs`, `admin.rs`); the compose SPRT comment objective 9 cites is gone | `git ls-files \| xargs grep -il` |
| PD-14 | **Updated.** PLAN.md was 10,963 lines and TESTING.md 4,595 at `77c7b52` (about 10,980 and 4,660 after this iteration's edits); this is the thirty-third audit, iteration 4 | `wc -l` |
| PD-15 | **Holds.** The floor has more hand-kept copies than objective 8 lists; U-CFG-5 checks each, and now also the two "reports" sentences (ET4-2) | `config.rs` U-CFG-5 |

All other Snapshot claims were verified true again (reviewer E's list: the
stack; the fake as test tooling only; allocation-only scheduling,
`CLAIM_ROUNDS` 8, `JOIN_SETTLE` 3600 s; one slot; four job types; the match
test off by default, `TEST_CHECK_EVERY` 8; ratings siloed, sweep 120 s;
`worker_bans`; input-data pinning; the Dockerfile pin invariant; no API grants
`is_admin`; one migration; the lock order; the root plan docs and scripts;
seven tiers).

### (m) Checks run and not run

See **Final checks (loop step 3)** below for the end-of-loop results.
- **Reviewers** ran no builds, except: G ran Terraform 1.9.8 in Docker on a
  scratch copy (54/54), `runbook-check.sh` (33 and 20 blocks),
  `dev-restore-check.sh`, `fake-worker-fixtures.sh --check` and
  `dev.py --help`; E2 ran `runbook-check.sh`; F and H ran
  `fake-worker-fixtures.sh --check`; B ran read-only `SELECT`s on the audit
  Postgres; D measured on a scratch database, since dropped.
- **Incident (reviewer D).** D restarted the shared `bt-audit-pg` container
  once (`docker restart`, 2026-10-06 00:56:28 UTC) for cold-cache timings. A
  normal restart: nothing on disk was lost and no other database was touched,
  but any reviewer or fixer session open at that moment would have seen its
  connection drop. None reported a failure from it.
- **Fixers**, each under the shared build lock:
  - Z1: clippy `--all-targets -D warnings` clean; the `finish`, `exports`,
    `public_api`, `auth_routes`, `leave_gen` and `leave_generation` binaries
    (120 passed) and `worker_api`, `admin_api`, `jobs`, `scheduler` plus the
    new and changed tests by name (156 passed); `py_compile scripts/dev.py`;
    EXPLAIN checks on `z1scratch` (dropped). Not the full nextest suite.
  - Z2: `npm run check` 0 errors, `npm test` 217 / 217, `npm run build`.
  - Z3: Terraform 1.9.8 in Docker on a scratch copy: `fmt -check -recursive`
    clean, `init -backend=false` (lock file identical), `validate`,
    `test` **57 / 57**. Docs only otherwise.
  - Z4: clippy clean; the `ratings` binary 33 / 33, plus
    `authz::the_route_table_is_every_route_the_router_serves` and
    `worker_api::a_pools_residuals_are_the_ones_its_latest_fit_stored`;
    `npm run check` 0 errors, `npm test` 220 / 220, `npm run build`.
- **Not run by the fixers:** the full nextest suite on the combined tree,
  tiers 5 and 6, and MAGPIE's tests (a README edit). The final checks below
  record what was run on the combined tree.

---

## Final checks (loop step 3)

The loop's step 3: every check CI and the nightly run, on the final tree
(birdtest after iteration 4's fixes, MAGPIE `02a91dfe`), run natively on this
machine with heavy commands one at a time.

| Check (CI / nightly job) | Result |
|---|---|
| `cargo clippy --locked --all-targets -- -D warnings` (backend-lint) | clean |
| `cargo test --locked --doc` (backend-lint) | ok |
| `cargo nextest run --locked --run-ignored all` — tiers 1–4 and the opt-in tier-6 Rust tests against `~/MAGPIE/bin/magpie` (backend-test, nightly's opt-in step) | **717 / 717 passed** |
| frontend `npm run check` / `npm test` / `npm run build` | 0 errors / **220 / 220** / built |
| `scripts/runbook-check.sh RUNBOOK.md README.md` | first run failed (iteration 4's new README `put-parameter` block lacked `export AWS_PAGER=""`); fixed; 33 and 21 blocks parse |
| `scripts/dev-restore-check.sh` | passed |
| `scripts/fake-worker-fixtures.sh --check` | every fixture is what the fake emits |
| contract fixtures, birdtest vs MAGPIE `test/birdtest_contract/` | 18 and 18, byte-identical |
| Terraform 1.9.8 in Docker: `fmt -check -recursive`, `init -backend=false`, `validate`, `test` | clean / ok / valid / **57 / 57** |
| tier 6, `scripts/e2e_magpie_native.sh` (M-1..M-7, M-9..M-16) against a `portable_release` MAGPIE | **15 / 15 passed** |
| tier 5, Playwright E-1..E-18, natively (`scratchpad/tier5/run-native.sh`: the e2e compose env, nginx, the fake workers) | **22 / 22 passed** |
| MAGPIE `python3 format.py`; `find_circ_deps.py` on a clean copy (iteration 3) | no differences; no cycles |
| MAGPIE `make magpie_test BUILD=dev` + `./bin/magpie_test contribute` (magpie-contract) | clean build; passed |
| `scripts/restore-roundtrip.sh` (nightly restore) | passed |
| `scripts/restore-job-check.sh` (nightly restore) | passed |
| `scripts/reapply-check.sh` (nightly restore) | passed |
| `scripts/backup-drill-check.sh` (nightly backup-drill; `BACKUP_METRICS=false`) | passed (its deliberately failed backup recorded `ok=false`) |
| TESTING status table | regenerated from `cargo nextest list --run-ignored all` and vitest's JSON report: tier 1 250, 1F 220, tier 2 186, tier 3 250, contract 15, tier-6 Rust 16; 717 backend |

**Not run, and why:**
- **The Docker image builds** (CI's images job, tier 5's and the nightly's
  compose builds): the owner's standing instruction is to ask before building
  the backend and derived-builder images on this machine. Tier 5 and tier 6 ran
  natively instead, against the same code. The backend image would also fail to
  fetch `MAGPIE_COMMIT` until `birdtest-contribute` is pushed.
- **MAGPIE cppcheck and clang-tidy** (two of MAGPIE's CI jobs): never run here,
  by the owner's standing instruction (they froze the machine).
- **MAGPIE's `BOARD_DIM=21` unit tests and the wasm tests**: not run. (At 21,
  MAGPIE's test table runs only board and rack tests; AF30 C2-7.)
- **The builderhash and conversion steps of CI's magpie-contract job**: not run
  separately; nothing in this loop changed a builder.

---

## Objective 7 — every code-versus-doc discrepancy found in iteration 4

Decision key (as in AF29 to AF31):
- **code wins:** behaviour kept, and the doc, comment or test note was updated
  to match it.
- **doc wins:** behaviour changed to what the doc says or needs.
- **unresolved:** neither changed beyond describing the situation; it awaits
  the owner.

Ids are `O7i4-N`, distinct from earlier iterations' ids.

| # | ID | Code does | Doc said | Decision | Reasoning |
|---|---|---|---|---|---|
| O7i4-1 | C4-1 | `origin/birdtest-contribute` (`dbf8df3b`) reports 0.1.1, so builds of commits that changed results without a bump exist anywhere; none can claim, because `ClaimBody` requires `board_dim`/`rack_size` | KL-68: 0.1.1 lives only in unpushed commits, "the remote is still 0.1.0", so the floor has nothing to exclude | code wins | Same conclusion, right reason: the bullet now names the claim body; no bump while nothing is deployed, and the first result-changing commit after a deployment bumps version and floor together |
| O7i4-2 | C4-2 | MAGPIE checks its claim and decline bodies against `claim-request.json` and `decline-missing-data.json`; `result-games-inference.json` was written by hand | `contract-fixtures/README.md`: MAGPIE checks only assignments and results; MAGPIE's README: every `result-*` was captured | code wins | Both READMEs corrected (the MAGPIE one in `02a91dfe`) |
| O7i4-3 | D4-3 | A closed leave generation's rows are read by the export and the public results feed; `position_analysis_plies` is also read by the public rack lookup (`FIRST_PLIES`, since `a12fdd7`) | KL-18: "never read again once that generation's artifact is verified" (weighed against `rebuild-artifacts` only); KL-25: plies are read only by the export | code wins | Readers named, and each option's consequence for them stated; the retention decisions stay with the owner |
| O7i4-4 | E4-1 | Job creation enforces seven rules (the redundancy rule went in October 2026) | PLAN: "creation enforces eight rules" | code wins | "seven" |
| O7i4-5 | E4-2 | The opening-rack finish check counts settled racks; a skipped rack has no progress row, so nothing reissues it and the job never completes | PLAN: the finish condition "only asks whether every task completed", so a short batch leaves a hole | code wins | The bullet's reason rewritten; the check it justifies stands |
| O7i4-6 | E4-3 | Every job action answers `409` while a purge, delete **or consensus change** of the job is running | PLAN's API rows: purge and delete `409` only on a purge or delete; the edit's row no `409` | code wins | Rows reworded and one sentence above the job rows; the narrative already said so |
| O7i4-7 | E4-4 | `variables.tftest.hcl` holds S-TF-1 to S-TF-3, the deploy-failed alert among them | Directory Structure: "S-TF-1/2: every variable validation" | code wins | Comment names S-TF-1..3 and the alert |
| O7i4-8 | ET4-1 | The server builds word info tables from the `kwg` too; tier 5 switches all three off | TESTING `I-INPUT-8` and the fixture-tarball paragraph: wordmaps and rack info tables only | code wins | Both name word info tables; `A-ADMIN-23` (correctly two) left |
| O7i4-9 | F4-1 | Every claim takes the dispatch lock in `registry::acquire`; `lock_claim_decisions` has no production caller | `claim_transition`'s doc: safe "because `lock_claim_decisions` holds the job's lock"; the function's doc read as leave generation's own lock | code wins | Function deleted, rationale moved onto `registry::acquire`, the safety argument cites the lock actually taken |
| O7i4-10 | F4-2 | MAGPIE's recorders write the result JSON directly; `config_contribute_games` submits `autoplay_results_get_json()`; `pair_game_number` is 0, or 1 or 2 within a pair | PLAN: a typed `AutoplayPosition` accessor mirroring `autoplay_results_get_game_summary()`, parsed `-hr false` lines, `pair_game_number` 0 or 1 | code wins | Section rewritten from MAGPIE at `baa82cfc`; "why not formatted output" kept |
| O7i4-11 | F4-3 | No `restored_at` exists; restored backup rows show as they were; staleness reads the newest successful run | PLAN: harmless "if `restored_at` context is displayed" | code wins | Sentence says what happens and why it is harmless |
| O7i4-12 | G4-2 | `aws_sns_topic.alerts` carries every alarm the stack raises | `alert_email`'s description: backup failure and staleness alarms | code wins | Description and README's closing note list them |
| O7i4-13 | G4-3 | The derived queue includes word info tables | `dev.py`'s docstring and log line: wordmaps and rack info tables | code wins | "derived files (wordmaps, rack info tables, word info tables)" |
| O7i4-14 | B4-1 | A pre-4713 BC cursor time was a `500` (SQLSTATE 22008) | `routes/mod.rs` and `public.rs`: anything not a cursor this server produced reads as the first page; PLAN API conventions: a malformed request is a 400, a 500 a server bug | doc wins | Impossible times read as no cursor |
| O7i4-15 | ET4-2 | U-CFG-5 skipped two "reports 0.1.1" sentences | TESTING U-CFG-5: "every hand-kept copy" | doc wins | Test reads them; entry names them |
| O7i4-16 | G4-1 | `github_token_parameter_arn` unvalidated; no recipe | KL-62's closure: a cross-variable mistake is refused at plan, not half-way through the apply; PLAN: the token is set in production | doc wins | Validation, three refusal/acceptance runs, README recipe, KL-62 sentence |
| O7i4-17 | H4-1 | A demoted export of a completed job was labelled "taken while the job was still running" | PLAN "Exports" (iteration 3): an edit of a completed job demotes its final export | doc wins | Label names both reasons for an opening-rack job; `is_final` doc and `F-FMT-14` reworded |
| O7i4-18 | H4-2 | Job pages showed a static player's `num_plays` live and bolded it in a non-capturing games or pairs job, and live in a leave job | PLAN "What is capturable": a static player's move count only sizes its list; SETTINGS_COMPARISON's "Never read by" | doc wins | Row muted there; F-SET-1 and SETTINGS_COMPARISON row 5 updated |
| O7i4-19 | H4-3 + Z4 | The client typed the add answer as `string`; the page offered members (a never-fitted pool's anchor, a member added since the fit) under Add | PLAN's member-add row (iteration 3): a repeat answers `run_id: null` with no refit; the pool's membership is `rating_pool_members` | doc wins | Type widened; detail serves `members`, the page's controls follow it; PLAN API row and `/ratings/[id]` bullet |
| O7i4-20 | H4-4 | The simmer "Moves Generated" input had no minimum | PLAN and `admin.rs` (iteration 3): a simmer needs at least 2 candidate plays | doc wins | `min="2"` with help; `min="1"` on the other simmer inputs |
| O7i4-21 | TESTING status table | Iteration 4 adds tests (`I-STATS-9k`, `I-EXPORT-17`, `A-PUBLIC-3d`, `A-RATE-10`, `F-RATE-1`, cases in `A-AUTH-13`, `F-FMT-14`, `F-SET-1`, `U-CFG-5`) | The status table's counts as of `77c7b52` | doc wins | **Done**: the coordinator regenerates the table at commit from `cargo nextest list --run-ignored all` and vitest |

**Count: code-wins 13 / doc-wins 8 / unresolved 0**.

Not counted (no code-versus-doc disagreement; behaviour or performance fixes
whose PLAN measured rows were added with them): B4-2, D4-1, D4-2.

---

## Issues and Recommended Solutions

Each entry gives the context (file and location, how it was found), the
problem, the options considered, and the recommendation and outcome. Reviewer
ids: A races, B backend bugs, C MAGPIE, D performance and storage, E PLAN and
drift, ET (reviewer E2) TESTING/RUNBOOK/README, F dead code, G deployment, H
frontend, e2e and the fake worker. Fixer ids Z1–Z4 as in the header.

### Races and locking

**No finding.**

**A4-OK (checked and found correct).**
- Lock order on every writer in scope, as in (b): no cycle. Contributor rows
  against the job row are not a cycle (a purge or delete must first lock every
  open claim of its job). Exports, ratings membership and fits, leave close,
  merge, staging and seeding, the derived builder's leases and input-data
  imports all checked.
- `DispatchHolds`: one hold per job; the edit's hold uncounted;
  `refuse_if_purged_since` (A3-2) tells a purge from an edit; claims skip held
  jobs; the reclaim grace; `run_to_completion`.
- The finish checks: `OPENING_RACK_FINISHED` gives the old predicate's answer
  under every interleaving (one statement, one snapshot; nothing can put work
  back between the check and `complete_unless_purged`); the debounce; the idle
  check.
- Near-misses examined and judged correct: (1) a finish check suppressed by a
  consensus edit's hold when the edit then fails or finds nothing to change:
  the next idle claim's check (`IDLE_FINISH_CHECK_EVERY`, 10 s) completes the
  job, by design; (2) a dropped transaction's row locks outlive its
  `DispatchHold` by one queued `ROLLBACK`: a claim or submission in that window
  waits on the bounded lock waits and is answered `Busy` or `503`, both
  retried.
- Not checked: `backups.rs` and the restore scripts against running jobs;
  `sse.rs`'s push coalescing (display only); real plan and lock behaviour (all
  from reading against documented Postgres semantics).

### Backend bugs and validation

**B4-1 (low). A results-feed cursor before 4713 BC was a 500 on a public route.** Covered in (b) and O7i4-14.
- **Context:** `routes/public.rs` `micros_to_time` (used by
  `opening_rack_cursor` and `game_result_cursor`, bound as `$4` in
  `job_results`); `error.rs` `From<sqlx::Error>`. Reviewer B, confirmed
  against the audit Postgres (`SELECT '30000-01-01 BC'::timestamptz` →
  22008) and sqlx-postgres's encoder (no range check beyond `i64`).
- **Problem:** chrono holds ±262,143 years; `timestamptz` starts at 4713 BC.
  A cursor such as hex(`"-1000000000000000000\x1f1"`) parsed and failed at the
  bind: a `500` and an ERROR line, at will, on an unauthenticated unmetered
  route. Positive times stay inside Postgres's range; the leave and positions
  cursors are integers and cannot do this.
- **Options:** filter in the parser; map 22008 globally (rejected: a 22008
  from server arithmetic is a real fault).
- **Outcome (Z1):** `.filter(|t| t.timestamp() >= 0)` with a doc comment; new
  `A-PUBLIC-3d` (an opening-rack and a games job, with and without
  `?worker=`, each `200` with the uncursored first page).

**B4-2 (low, dev stack). `/api/dev/login`'s `next` let a tab through and panicked on a line break.** Covered in (b).
- **Context:** `routes/auth.rs` `dev_login`; axum 0.7.9
  `response/redirect.rs` (`HeaderValue::try_from(..).expect(..)`); `http`'s
  header byte rule (tab allowed); the WHATWG URL parser (strips tab and
  newline). Reviewer B.
- **Problem:** `/\t/evil.example` passed the `//` and `\` checks and reached
  the browser as `//evil.example`, an open redirect after signing in as the
  named user; `/\n` passed and panicked the handler, dropping the connection.
  Only where `DEV_LOGIN` is on, which startup refuses beside
  `SECURE_COOKIES=true`.
- **Options:** visible-ASCII only; percent-encode; parse as a URI.
- **Outcome (Z1):** every byte must be in `0x21..0x7f`, else `/`; a comment
  names the tab and line-break cases; `A-AUTH-13` extended.

**B-OK (checked and found correct).** Iteration 3's B3-1 (22021/22P05 → 400
without the database's text), B3-2, `refuse_if_purged_since` and
`OPENING_RACK_FINISHED`; the export's consensus gate and `unfinalize`; every
other caller-input path to the database (typed binds, enum extractors,
saturating pagination, length caps on every indexed caller text,
plausibility-bounded integers); C3-1's simmer rule (configs are immutable and
made only through `create_player_config`; nothing deployed predates the
rule); public routes; `canonical_rack` after `machine_letters`' removal; auth,
account, ratings and worker routes; `artifacts.rs`; `inputdata.rs` name
handling; `error.rs` scrubbing; `models/`. Not re-derived: `bradley_terry.rs`
and `match_test.rs` numerics; plausibility bounds against MAGPIE
(`1a0ae932..baa82cfc` touches nothing a result's analysis depends on).

### MAGPIE arguments and `birdtest-contribute`

**C4-1 (low, doc; KL-68 revisited).** Covered as O7i4-1.
- **Context:** PLAN KL-68's `MAGPIE_VERSION` bullet (written 2026-09-25,
  before `dbf8df3b` existed); `git -C ~/MAGPIE branch -r --contains 68a74611`;
  `routes/worker.rs` `ClaimBody` (no `serde(default)` on the build fields).
  Reviewer C.
- **Problem:** the bullet's premise ("the remote is still 0.1.0") no longer
  holds: anyone can build an intermediate 0.1.1 commit, including `dbf8df3b`,
  which captures a forced turn as the previous simulation (B2-3). The
  conclusion (nothing for the floor to exclude) still holds, for a different
  reason the next reader would need.
- **Options:** reword; or raise the version and floor (rejected: the owner's
  "nothing in prod" rule).
- **Outcome (Z3):** reworded as proposed.

**C4-2 (low, doc).** Covered as O7i4-2 and (i). Outcome (Z3): birdtest's
README lists MAGPIE's claim-body and decline-body checks (both tests verified
registered in `contribute_test.c`); MAGPIE's README excepts
`result-games-inference.json` (committed as `02a91dfe`).

**C-OK (checked and found correct).** Step 0 (branch, HEAD equal to the pin
at the start, version equal to every floor copy, 18 fixtures byte-identical
with equal file sets); `baa82cfc` line by line (the decline and missing-file
JSON byte-identical to before the refactor; no leaks or unchecked error paths;
both directions of C-10 pinned; comments re-pointed to headings that exist;
`bytes` removed consistently, never read by MAGPIE); the whole
`dbf8df3b..baa82cfc` range again; the settings table in (c); the derived
builder command lines; no dead MAGPIE function or orphaned error code. Not
run: any MAGPIE build or test (read-only brief). Cosmetic, not flagged: a
duplicated doc comment above `produces_every_fixture_key` in
`contribute_test.c`.

### Performance and storage

**D4-1 (low). The in-flight probe joined `tasks` when `task_claims.job_id` answers it.** Covered in (d) and (e) #1.
- **Context:** `routes/worker.rs` `should_check_finish` and `finish_idle_job`;
  `exports.rs` (settling and `is_final`); `routes/public.rs` `stalled`;
  `jobs/leave_gen.rs` `claims_in_flight` and `furthest_below_target`.
  Reviewer D, measured.
- **Options:** filter on `c.job_id` (no schema change); D-4's job-keyed index
  (pending the owner, would make these reads exact; not pre-empted).
- **Outcome (Z1):** verified first that `task_claims.job_id` is `NOT NULL`,
  bound by the one production INSERT (`scheduler::issue_claim`) from the job
  whose task `registry::acquire` returned, and set by every test INSERT. Then
  `JOB_HAS_OPEN_CLAIM` and the same filter at the other sites, as in (e).
  New `I-STATS-9k` (30,000 completed tasks of one job with claims, 200 open
  claims of another): the old form reads `tasks` without statistics
  (`pg_statistic` deleted and `reltuples = -1` in a rolled-back transaction);
  the constant uses `task_claims_open_idx` with no `tasks` scan and no
  sequential scan, custom and generic, with and without statistics, and
  answers false and true correctly. PLAN measured rows.

**D4-2 (low). An export's positions probe scanned every older job's records.** Covered in (e) #2.
- **Context:** `exports.rs` `snapshot` (games and pairs jobs,
  `may_capture_positions`), inside the export's REPEATABLE READ snapshot.
  Reviewer D.
- **Options:** `ORDER BY … LIMIT 1` on the feed index outside an `EXISTS`
  (D3-1's settled form; inside an `EXISTS` Postgres drops the `ORDER BY`).
- **Outcome (Z1):** `POSITIONS_CAPTURED`; new `I-EXPORT-17` (the old form
  plans a `Seq Scan`, the new one reads the feed index custom and generic, and
  answers correctly for a job with records and one without); `I-EXPORT-14`'s
  statement prefix updated.

**D4-3 (low, doc; KL-18 and KL-25 revisited).** Covered as O7i4-3.
- **Context:** PLAN KL-18 (`exports.rs`'s leave corpus has no generation
  filter; `routes/public.rs`'s leave feed steps down every generation); KL-25
  (`FIRST_PLIES`, used by the rack lookup and saved positions). Reviewer D,
  `git log -S` dating both statements before their readers.
- **Problem:** acting on either retention option as written would silently
  empty the export and feed of every closed generation, or break the rack
  lookup's plies.
- **Outcome (Z3):** both Contexts name every reader, and the options say what
  each would also have to rewrite. No option chosen.

**D4-OK (checked and found correct).** Iteration 3's three rewrites on the
fresh dataset ((e)); every other `EXISTS`/`COUNT`/`LIMIT 1`/`MIN`/`MAX` on a
growing table (`next_available`, `MAX(seed)`, `reclaim_expired_for`,
`record_consensus`'s reads, `racks_after`, the merge pass, `refresh_summary`,
`plain_game_stats`, the completed-job audit read, the staging, transition,
artifact and per-generation probes, the positions search, the random
position, the rating-run newest); the critical path in (d); storage in (f);
the derived builder's memory by code read. Not measured: production instance
class, full-volume purge or delete (KL-12), a full English merge or
transition, a full opening-rack export, D2-1/D2-2's reissue reads on this
dataset, the builder's actual peak, MAGPIE `contribute`'s runtime.

### Docs (objective 7)

Every E4-*, ET4-1, C4-1, C4-2, D4-3, F4-1 to F4-3, G4-2 and G4-3 item is
recorded one per row in the Objective 7 table, with the code, the doc, the
decision and the reasoning. In summary:
- **Code wins:** 13, made by Z3 (most), Z1 (F4-1's comments, G4-3) and, in
  MAGPIE, Z3's README edit.
- **Doc wins:** 8, each also covered under its own id.

Brief entries for the doc findings not covered elsewhere:

**E4-1 (low).** PLAN's "creation enforces eight rules" counted the redundancy
rule deleted in October 2026 (bullet count 8 at `81b9e28`, 7 at every commit
since `0149541`). Fixed: "seven".

**E4-2 (low).** PLAN's opening-rack batch bullet gave a pre-consensus reason
(`269e666`, 2026-09-13). Fixed: a skipped rack has no `opening_rack_progress`
row, nothing reissues it, and the job, whose finish check
(`OPENING_RACK_FINISHED`) counts settled racks, never completes.

**E4-3 (low).** PLAN's purge, delete and consensus API rows omitted the edit's
`409` (the `refuse_while_purging` callers, `hold_for_purge_or_delete`,
`ALREADY_RUNNING`). Fixed: rows reworded and one sentence above the job rows,
linking "Opening-rack consensus".

**E4-4 (low).** The Directory Structure's comment on `variables.tftest.hcl`
named S-TF-1/2 only. Fixed.

**ET4-1 (low).** TESTING's `I-INPUT-8` and fixture-tarball paragraph,
missed by iteration 3's sweep. Fixed (O7i4-8).

**ET4-2 (low).** Covered in (b) and O7i4-15. The ET3-1 class, two files short;
fixed by Z1.

**E-OK and ET-OK (checked and found correct).** Reviewer E's mechanical
checks: the schema copy byte-identical to the migration (1,849 lines); every
route has a PLAN row and every row a route (73 rows, 74 registrations; the
extra two `GET /api/dev/login` and `/health`); 30 frontend routes; the
Directory Structure; the config table (31 keys plus `MAGPIE_SCRATCH_DIR`);
every anchor; KL-1..94 unique and complete; every `PLAN.md, "…"` citation in
code; every test id PLAN cites; audit actions both ways; the 18 fixtures.
Iteration 3's PLAN, README and RUNBOOK edits each checked against the code.
Reviewer E2: every *(Covered: …)* name resolves (including five MAGPIE
tests); no undefined id cited anywhere; the status table and the 713 backend
total recounted from source at `77c7b52`; each of iteration 3's tests read in
full and judged to fail on a revert of its fix; TESTING's CI section against
both workflows; RUNBOOK and README script paths, routes, audit actions and
`seed.py` choices; `runbook-check.sh` 33 and 20 blocks. Noted for whoever
raises the floor, not a finding: `scripts/e2e_magpie.py` sends two hard-coded
`"magpie_version": "0.1.1"` claims, which tier 6 would catch loudly.

### Dead and out-of-date code

**F4-1 (low). `lock_claim_decisions` was a test-only alias with a stale safety citation.** Covered in (g) and O7i4-9.
- **Context:** `jobs/leave_gen.rs` (the function and `claim_transition`'s
  doc); `registry.rs` `acquire`; `tests/leave_generation.rs`,
  `tests/leave_gen.rs`. Reviewer F; earlier sweeps counted the doc mentions as
  references.
- **Options:** delete it and call `try_lock_job_dispatch` from the tests; or
  keep it as a named test seam with a one-line doc.
- **Outcome (Z1):** deleted; `try_lock_job_dispatch` made `pub`; rationale
  moved; "under its claim lock" in `registry::acquire` corrected to "under
  this same lock". The guarantee itself always held.

**F4-2 (low). PLAN described a MAGPIE accessor API removed on 2026-08-28.** Covered in (g) and O7i4-10.
- **Context:** PLAN "Position Capture From Games › MAGPIE changes"
  ("Emitting it", the C block, the `RecorderArgs` comment); MAGPIE
  `bc620e55`, `62f6ecbb`, `config.c:9295`, `autoplay_results.c`,
  `autoplay.c:595`. Reviewer F's doc-identifier sweep.
- **Outcome (Z3):** rewritten from MAGPIE at `baa82cfc`: each recorder has a
  `json_func` beside its `str_func`; the positions recorder keeps captures per
  worker thread and renders them on consolidation into `positions`, plays
  through `autoplay_results_write_ranked_plays_json`; `config_contribute_games`
  submits `autoplay_results_get_json()`'s object; no text parsing. Option
  mapping kept. Not re-verified line by line: every other prose claim in the
  "MAGPIE changes" sections (for example `game_get_cgp` versus MAGPIE's
  current `game_get_cgp_string`).

**F4-3 (low).** Covered in (g) and O7i4-11 (Z3; `backups.rs`'s `is_stale`
reads the last success, verified).

**F-OK (checked and found correct).** Iteration 3's removals hold (`win_pct`
and friends, `machine_letter()`, claim `bytes`, the `-ritmmap` shim, the
`.wmp.src` skip); iterations 1–2's removals hold; remnant greps (SPRT,
redundancy, priority, chi-square, self-update, TODO/FIXME/HACK) hit only
deliberate history or why-not text; every Rust item has a production use
apart from F4-1 and the named test seams; no unread Deserialize/FromRow
field; the Serialize fields no consumer reads are recorded as deliberate
(`TestResult.elo*`, `Job.min_magpie_*`, `Job.activated_at`, `Builders.*`);
every frontend export, component prop and page used or linked; e2e helpers
used and no assertions on removed UI; every contract fixture read on both
sides; every `contribute.h`/`contribute_defs.h` item used; every table read;
every Cargo, npm and Python dependency used; no unused Terraform variable,
local or data source; every env var and `.env.example` key read; every script
and fixture referenced (the `.ab` half of a split tarball aside); 622
*(Covered: …)* names resolve.

### Deployment

**G4-1 (low).** Covered in (k) and O7i4-16.
- **Context:** `infra/variables.tf` (`github_token_parameter_arn`),
  `infra/ecs.tf` (`execution_ssm` policy `resources`, `secrets[].valueFrom`),
  README "Deploying" step 8, PLAN KL-62. Reviewer G.
- **Options:** a validation like `acm_certificate_arn`'s plus refusal runs
  and a README recipe; or documentation only (rejected: the KL-62 closure's
  rule is to refuse at plan).
- **Outcome (Z3):** as in (k). The DR copy (RUNBOOK §5) already overrides the
  variable to `""`, so it is unaffected.

**G4-2, G4-3 (low, doc).** Covered in (k), O7i4-12 and O7i4-13.

**G4-OK (checked and found correct).** As in (k), plus iteration 3's changes
in scope: `dev.py` without `-ritmmap` still starts a working contribute
invocation (MAGPIE maps by default since `c7e516ef`, and every MAGPIE the
server admits postdates it); `e2e_magpie.py` without `.wmp.src`; the
`derived.tf` description (67 characters, plans); S-TF-3's wording; the pin;
U-CFG-5's root `.env.example` rows; MAGPIE's `dlopen`ed libcurl in the
`portable_release` image; `DATA_VERSION` against `download_data.sh`; RDS
`force_ssl` met by sqlx and libpq `prefer`. Not checked: real AWS behaviour
(EventBridge delivery, breaker timing, maintenance windows, whether the
`postgres:16` image's `awscli` sends the checksum Object Lock requires),
Fargate quotas in a fresh account, image builds and the e2e and nightly tiers.

### Frontend, e2e and the fake worker

**H4-1 (low).** Covered in (b) and O7i4-17. Context: `frontend/src/lib/format.ts`
`exportSummary`, `api.ts` `JobExport.is_final`, TESTING `F-FMT-14`,
`admin.rs` `consensus_body`, `exports.rs` `unfinalize`. Options were a label
true in both cases, or a branch on `snapshot_at` against `completion.at`
(rejected, coordinator decisions). Outcome (Z2) as in (b).

**H4-2 (low).** Covered in (b) and O7i4-18. Context: `jobSettings.ts`
`LEAVE_UNUSED`, `UNCAPTURED_UNUSED`, `unusedPlayerSettings`; MAGPIE
`autoplay.c:381-398`, `game_runner_get_best_move`,
`autoplay_results.c:1770-1772`. Z2 verified in MAGPIE that a simmer's
`num_plays` is read (`game_runner_get_top_simming_move`), so the row stays
live whenever a simmer plays, and that capture reads a static player's
count (a captured position's `num_moves`). SETTINGS_COMPARISON row 5 updated.

**H4-3 (low) and Z4.** Covered in (b), the coordinator decisions and
O7i4-19.
- **Context:** `api.ts` `addRatingPoolMember`; `routes/ratings/[id]/+page.svelte`
  (`candidates`, `poolMembers()`); `routes/ratings.rs` `add_member`,
  `pool_detail`. Reviewer H; Z2's report of the remaining gap.
- **Options:** frontend only (type, Add list from non-anchor, unrated
  configs, a notice on null), which still offered a member added since the
  fit; or serve the membership (chosen, Z4).
- **Outcome (Z2, Z4):** as in (b). `members` is public-safe (the endpoint was
  public and `ratings` already exposed rated configs' names; `added_by` and
  `added_at` are not served). E-7 unaffected (`addMember` waits for one row
  per name, which still holds).

**H4-4 (low).** Covered in (b) and O7i4-20.

**H-OK (checked and found correct).** AF31's H3-1 to H3-4 hold; iteration 3's
contract changes (`GameStats`, `bytes`, `run_id: null`, the export demotion,
the simmer rule, NUL as 400) against every type and caller; every `api.*`
wrapper's route and method; the export card after an edit (apart from H4-1's
label); auth and session (`refreshSession` 401/403 only, login during a
deploy, the guards); CSRF; SSE; the job, player-config, pool and consensus
forms against the server; the e2e suite (E-7, E-10, E-12 to E-15, E-18 against
iteration 3's changes); the fake worker against the protocol and the
plausibility rules; `fake-worker-fixtures.sh --check`. Not re-opened line by
line: the allocation, derived-data, backups, audit-log, users and workers
pages and `charts/*`.

### Open items carried forward

**Pending the owner** (genuine trade-offs; they do not count as fixable for the
loop). Unchanged from AF29–AF31:
- **B-1:** rating-pool scoping by bingo bonus and sim cutoff (KL-75).
- **B-2:** a consensus edit reopens a force-completed job.
- **D-3:** batch defaults (KL-93).
- **D-4:** `task_claims` job-scoped indexes (would make D2-1's, D3-2's and
  D4-1's open-claim reads exact; D4's ETA numbers in (e) #4 feed it).
- **D-5:** the FK drops and the KL-10 re-measurement. Optional data for it:
  the match-test reads measured 59–175 ms over 300,000 batch-1 rows (D3) and
  `plain_game_stats` 26–37 ms / 264 ms cold (D4, (e) #3).
- **D-8:** export snapshot versus vacuum (KL-94).
- **F-4 / A-RATE-6:** the rating-history endpoint.
- **F-12:** the admin results stream.
- **F-15 and F-16:** `SETTINGS_COMPARISON.md` (its 96 migration line anchors
  are stale again) and the root plan docs.
- **G-3:** `wait_for_steady_state`.
- **W2a:** the even-batch CHECK (B-5) and exact-mover attribution (B-4).
- **F3-8:** the `backups` table's never-written `rds_snapshot` kind,
  `snapshot_id` column and single-location CHECK: drop them (recommended) or
  keep them as a placeholder.
- **Also noted, not blocking:** the D-7 fleet semantics; production `jit`
  ((e) #5); bumping `MAGPIE_VERSION` and the floor together before a first
  deployment (PD-9).

**New pending items from this iteration:** none.

**Fixable, optional, carried** (none attempted in iterations 3 or 4):
- E2-1's MAGPIE half: move `io_util.c`'s platform `#if` into `src/compat/`
  and drop PLAN's sentence about it.
- C2-7's test comment ("at either BOARD_DIM"); the testdata `data-20251004`
  date note; an `ANALYZE` of RUNBOOK §2.1's scratch copy.
- KL-10's re-measured numbers (above); README's floor prose in U-CFG-5
  (ET3-1); CI's Terraform bump to 1.11 or later to pin S-TF-3's target
  (ET3-3); a decline-and-continue for a malformed `expected_data` (C3-2).
- New this iteration, recorded and not flagged: `leave_records` is read only
  by the delete census (D; dropping it touches the census format,
  `restore-job.sh`, RUNBOOK §2.2 and four tests, and would remove D-5's
  `leave_records.task_id` question); the other `JOIN tasks` sites D4-1 did not
  cover (admin, jobstats, scheduler, worker); the on-demand task's
  insert-then-update; `e2e_magpie.py`'s hard-coded `magpie_version` claims (to
  change with a floor raise); the duplicated MAGPIE test doc comment.

**Blocking CI until done (owner action):** push `birdtest-contribute` with
`c3c6a875`, `1a0ae932`, `baa82cfc` and `02a91dfe`.

**Loop status (owner instruction):** the loop stopped after this pass at the
owner's instruction, without a clean confirmation pass (see the header). A
later audit should start with a full pass over this branch's head.
