# AUDIT_FINDINGS_28 — the thirty-second audit (2026-09-25), a loop of passes

Branch `audit/birdtest-2026-09-25-2`, off `audit/birdtest-2026-09-25`
(`a229895`), the tip of the thirty-first audit. `main` (`5d35525`) holds none of
the eleventh-to-thirty-first audits' commits, so the run is taken off their tip
rather than off `main`. MAGPIE is on `birdtest-contribute` at `7400184f`, which
`docker/Dockerfile` pins. **Still unpushed** (AUDIT_FINDINGS_8, header).

**Builds on** AUDIT_FINDINGS_7 to _27, and on PLAN.md's Known Limits
(KL-1 to KL-77), which are not re-flagged unless the code under them changed.
The thirty-first audit stopped at its four-pass budget with one medium open,
KL-74 (the rating fit's bias and understated errors), and named it the first
thing to take up next; this run takes it up in pass 1.

This run follows `BIRDTEST_AUDIT_LOOP_PROMPT.md` with no pass cap: pass 1 is
full, later passes cover the diff plus one named area, and the loop ends only
on a clean confirmation full pass that follows a clean follow-up pass.

## Summary (kept current)

- **Pass 1 (full):** 0 high and 3 medium from the five reviewers, plus the
  carried medium KL-74 (the rating fit), all fixed and verified. The
  adversarial checks found 4 high and 12 medium in the pass's own fixes —
  one medium in the rest (a `terraform fmt` break), the others in the rating
  fit over ten rounds — all fixed within the pass. KL-74 closed; KL-78 and
  KL-79 added; KL-17, 44, 54, 60, 63, 68, 75, 76 updated. The loop continues
  (pass 1 found medium).
- **Pass 2 (follow-up: pass 1's diff, and the backup pipeline and restore
  drill):** 0 high and 2 medium from the reviewers (E-8 could not pass; PLAN
  said the drill writes a result row), both fixed and verified; the
  adversarial check found no high or medium. Lows mostly fixed; KL-80 added,
  KL-17, 65, 74, 75, 76 and 79 updated. The loop continues (pass 2 found medium).
- **Pass 3 (follow-up: pass 2's diff, and job creation and player-config
  validation):** 0 high and 2 medium from the reviewers (E-1 and E-5 could not
  pass — tier 5 now run natively, 11 of 11; job creation accepted jobs no
  worker can run), both fixed; the adversarial check found 1 medium (the play
  cap too low), fixed. KL-2, KL-60, KL-68 and KL-80 updated. The loop
  continues.
- **Pass 4 (follow-up: pass 3's diff, and the live stats stream and cache):**
  0 high and 4 medium from the reviewers (live pushes not spaced; the finish
  check's read seven times its documented cost; the job form refusing valid
  SPRT settings; `dev-restore.sh` skipping the scrub on `SCRUB=yes`), all fixed
  and verified; the adversarial check found no high or medium. KL-2, KL-10,
  KL-60 and KL-78 updated. The loop continues.
- **Pass 5 (follow-up: pass 4's diff, and the account lifecycle):** 1 high
  (Argon2 had no concurrency bound: a flood of sign-ins took the process to
  2.6 GB; now four turns on four threads, 129 MB) and 3 medium from the
  reviewers (no way to recover a forgotten username; PLAN's password-score
  claim; `game_pair_stats`'s cost understated), all fixed and verified; the
  adversarial check found 2 medium (aborted requests escaped the Argon2
  bound; the confirmation mail carried the registrant's text), both fixed.
  KL-34, KL-60 and KL-81 updated. The loop continues.
- **Pass 6 (follow-up: pass 5's diff, and leave generation):** 2 high and 3
  medium from the reviewers — password scoring ran on the executor (`/health`
  8.5 s from one address); MAGPIE's rack spelling meant no blank rack was ever
  counted and an English generation could never close; usernames carried text
  into mail; the tail could close a generation short; a merge's temporary
  files grew with the backlog — all fixed and verified; the adversarial check
  found 1 high (scoring on sign-in's turns), fixed. KL-2, 13, 34, 37, 78 and 81
  updated. The loop continues.
- **Pass 7 (follow-up: pass 6's diff, and derived-file builds):** 0 high and 7
  medium from the reviewers — a failed build retried at once, spending its
  attempts in seconds; a stalled S3 read held the builder for good; a
  wordmap job on a three-blank distribution never dispatched; a leave rack
  spelled before its length was checked; PLAN putting scoring back on
  sign-in's turns; RUNBOOK calling a differing KLV hash legitimate; merges
  eleven minutes on the default database — all fixed and verified; the
  adversarial check found 3 medium (a damaged object's remedy, invisible
  username twins, merge turns held by waiters), fixed. KL-37, 57, 78 and 81
  updated. The loop continues.
- **Pass 8 (follow-up: pass 7's diff, and the task lifecycle):** 0 high and 5
  medium from the reviewers — the builder's role could not delete a damaged
  input; one failing task was every claim of its job; RUNBOOK said
  contributors resume after a restore; positions stored for a job that did
  not capture them; Retry's documented body reset other builders' rows — all
  fixed and verified; the adversarial check found 2 medium (the decline skip
  forcing leave racks twice; an expired twin holding a name), fixed. KL-2,
  40, 57 and 78 updated. The loop continues.
- **Pass 9 (follow-up: pass 8's diff, and admin operations):** 0 high and 3
  medium from the reviewers — PLAN counting a `derived_mismatch` toward the
  five-failure stop; KL-56 saying a ban stops an identity-less client; a
  forced rebuild stopped part-way leaving no audit row — all fixed and
  verified; the adversarial check found 2 medium (refused exports logged as
  started; RUNBOOK counting identities on the wrong database), fixed. KL-40,
  56 and 78 updated. The loop continues.

---

## Pass 1 — full pass

**Plan.** Every objective across the whole repository, split between five
reviewers who report only:
1. dispatch, races and public-route cost;
2. auth, statistics and the frontend;
3. storage, performance, RUNBOOK and README;
4. deployment, CI and security;
5. MAGPIE (`birdtest-contribute`) and objectives 3 and 8.

Plus the carried open medium KL-74, taken up by the run itself.

**Findings: 0 high, 3 medium** from the five reviewers (dispatch 1 medium, 2
low; auth, statistics and frontend none, 9 low, 1 unconfirmed; storage,
performance and docs 1 medium, 4 low, 1 unconfirmed; deployment, CI and
security none, 4 low, 1 unconfirmed; MAGPIE 1 medium, 3 low, 1 unconfirmed),
plus KL-74 carried. All fixed and verified; the adversarial checks are 1.5.

### 1.1 Medium — a job whose last results landed while it was inactive never completed (dispatch reviewer)

**Code updated.** Only a submission ran the finish check, and only while its
job was `active`. A games or pairs job at its cap, or an opening-rack job with
its rack space handed out, deactivated while its last tasks were out: the
results are accepted (KL-1) with no check, and once reactivated the job has
nothing to generate and nothing will submit again. It stayed `active` at its
allocation for good — refusing other activations against it, never
exportable, heading every claim's candidate list. A last submission whose check
failed (logged, never retried) left the same state; the comment "the next
submission's check picks the job up" was false for the last one. PLAN says a
job completes "automatically when the finish condition is met".
**Fix:** a claim that finds a games, pairs or opening-rack job with nothing to
hand out spawns its finish check (`finish_idle_job`): skipped while any claim is
in flight (that claim's submission checks), guarded by the same purge witnesses
as a submission's (`complete_unless_purged`), paced to once per job every ten
seconds, re-armed by an activation. **Verified:** two tests
(`finish::a_job_whose_last_results_landed_while_inactive_completes_once_reactivated`,
`…_a_games_job_at_its_cap_…`, `I-STATS-9f`) fail with the claim-path hook
removed (the job still `active`) and pass with it; the games job stores
`terminated_at_max` as its verdict. PLAN's SPRT paragraph and the comment
corrected.

### 1.2 Medium — the home page's advice for a second contributor process could not start (MAGPIE reviewer)

**Docs updated.** The home page said a second process needs a `contribute.txt`
"most simply in a directory of its own", and the README said a directory of its
own is simplest "since MAGPIE also writes `settings.txt` there". MAGPIE loads
its default board from `./data` before it parses anything, so such a directory
fails at startup (reproduced: error 33 / 153, with or without `-path`), and a
one-shot `magpie contribute` writes no `settings.txt`. This advice was the
thirty-first audit's own low fix (its §3.5). **Fix:** both say to run
`./bin/magpie contribute second.txt` in the same directory, the README noting
why a bare directory fails. `F-DOCS-1` gains a check (the page names the
command, no page says "directory of its own"): it fails on the committed page
and passes.

### 1.3 Medium — `scripts/leave-gen-bench.sh` failed at its first statement, and PLAN sent operators to it (storage reviewer)

**Script removed, PLAN updated.** The script's throwaway job omitted
`bingo_bonus` and `sim_cutoff`, both `NOT NULL` with no default: reproduced,
"null value in column bingo_bonus". PLAN called it "safe to run against
production" and "one command", and described the universe as *copied* from the
previous generation, a choice the script existed to settle and the code has
since made (the universe is generated from the letter distribution and `COPY`ed
in, `seed_generation`). **Fix:** the script is removed, PLAN's paragraph
describes what runs now, and its repository tree lists the five scripts it was
missing (`restore-job.sh`, `restore-job-check.sh`, `backup-drill-check.sh`,
`e2e_magpie_native.sh`, `capture_contract.py`).

### 1.4 Low findings

**Fixed:**
- the move-score and equity plausibility caps (2,000 and 5,000) assumed the
  15×15 board; the 21×21 `standard21` has quadruple-word corners (an edge-long
  word ×144), and a false positive refuses the same seeded task on every retry
  — now 100,000 and 200,000 (auth/statistics reviewer's unconfirmed suspicion;
  no real play over 2,000 was found, so low);
- a self-play `game_pairs` job was counted in a run's `pairs_used` and
  `jobs_used` though the fit ignores it (new test fails before, passes after);
- `GET /api/workers` read `offset + limit` rows of each arm for a page past the
  end (0.3 s at 300,000 contributors); it now answers such a page from the count;
- the SPRT label "terminated at max games" read wrong for pairs jobs ("stopped
  at its cap"); a password reset now clears the tab's session; the new
  player-config form reports a failed load; the pool page reloads after a
  membership change whose refit failed; an unmeasured error shows as ±∞, not
  ±1.8e308;
- the scheduler roles' `ecs:RunTask` now lists the revisionless family ARN as
  well as `:*` (the deployment reviewer could not settle which IAM evaluates
  for a revisionless schedule target; both is safe either way);
- README's image builds name `--platform linux/amd64`; `backend_image` and
  `frontend_image` refuse an empty value; `nightly.yml`'s "two databases"
  (three); `restore-job.sh`'s sequence step no longer prints empty result sets;
  PLAN's "neither side holds a generation in memory" (MAGPIE's `RackList` does);
  the seed-tiling comment in `jobs/game.rs` (MAGPIE draws a batch's game seeds
  from a stream seeded with the task's seed, not S..S+N-1); stale MM comments.

**Recorded:** KL-17 (late-lap sweep selection 48–100 ms warm, cold unmeasured),
KL-44 (`rackequity2klv`'s 375 MB, unbounded concurrency), KL-54 (a result of up
to 1 MiB has a fixed 30 s), KL-60 (the job form's version-load message), KL-63
(unscrubbed non-database 500 messages), KL-68 (MAGPIE's 0.1.1 bump and
intermediate commits, no SIGINT handler, Windows rename), KL-75 (history ranks
removed and unrated configs), KL-76 (rating charts' fixed margins on narrow
phones), and new KL-78 (a force-completed job's SPRT panel, two purge races
reasoned about and not reproduced, player-config delete's unindexed foreign-key
scans, the worker list just short of its end).

### 1.K Medium, carried — the rating fit biased large or thinly linked groups and understated their errors (KL-74, closed)

**Code updated.** Opened by the thirty-first audit's last pass and left for a
statistical decision. `stats/bradley_terry.rs` solved by minorization-
maximization capped at 10,000 iterations, with two virtual draws for every
member against the anchor's strength and errors from the diagonal of the
information. **Failure shown first:** six new tests with noiseless evidence
(`U-STATS-5`) against the committed fit — a 12-member group joined by one
300-pair job, a 20-config chain, a 30-member group, a well-played island, a
clean sweep, and a hundred-member pool — five failed: unconverged at 10,000
iterations (group, 30-member group, island), the chain's first step 26 Elo low,
and 1.7 s for the hundred-member pool in a debug build.

**Fix (decision made), in ten versions.** The solver and the errors were
right first time; the prior and its errors took nine adversarial rounds, each reproduced
with a failing test before the next version, and was redesigned rather than
re-weighted once it reached its third correction.
- **Solver:** Newton's method on the log-strengths relative to the anchor,
  every step moving the whole pool through the full curvature; a step bounded
  at 8 natural-log units (about 1,400 Elo), a step under 0.5 taken whole,
  convergence judged on the full step (1e-6), Levenberg–Marquardt damping if
  the curvature fails to factor, and a backtracking line search.
- **Errors:** the diagonal of the inverse of the full data-only information
  over the anchor's component, so a group carries its link's uncertainty.
- **Prior, as it ended:** virtual drawn games for every config, the anchor
  included, against a virtual config at the pool's *centre* — the plain mean
  of every rating — on a logistic twice as wide as real games' (about 350
  Elo): two for a config with no games, `2 / (1 + g/200)`, at least 0.2, for
  one with `g`; convergence on the full step or the Newton decrement. Each
  error is widened by the prior's pull on that config. Strictly concave, so
  one answer, continuous in the scores; no pull toward the anchor, so a far
  field or group stays put; no virtual link between configs except through the
  centre. Its costs are KL-79.
- Runs record `method = 'bradley_terry_newton'` (schema default too).

**The versions the adversarial checks broke** (1.5 has the detail):
1. Two draws per config against the anchor, with an unbounded Newton step:
   clean sweeps contradicting a pool threw one config 18,000 Elo into
   saturation (medium); fixed by the step bound and damping.
2. Two per config spread over its opponents (`1/deg(i) + 1/deg(j)`): halved a
   newcomer's shrinkage in a star (medium); then `2/min(deg)`: a config over a
   gauntlet of lightly played opponents 2–4 errors low (medium), and a fit at
   the answer marked unconverged at million-pair head-to-heads (medium, fixed
   by the full-step rule).
3. Draws only on head-to-heads crossing the strongly connected components of
   "took points from" (where the maximum likelihood diverges): a newcomer
   conceding a quarter point rose 175–240 Elo (high), and baselines everyone
   swept tied the rest together (medium).
4. Firth's penalty (½ log det I): not concave, so a config between far-apart
   opponents had two maxima and a quarter point moved it 600–900 Elo (high);
   and it converged linearly, 76 s at 437 members (medium).
5. Two draws per config against a fitted pool centre, on a logistic twice as
   wide (chosen by a Monte Carlo of every shape found, against the old prior
   and at five scales): a strong tier joined to the rest by one job was pulled
   back by half a game per member, 345 Elo and nearly four errors low (high).
6. The same, each config's virtual games faded with its real ones,
   `2 / (1 + g/200)`: in a mature pool the fitted centre followed a newcomer's
   unfaded virtual games, so its 3-pair sweep rose 460 Elo as the others played
   on (high); and a million-pair sweep, faded to nothing, left a fit at the
   answer stored as unconverged (medium).
7. The centre as the plain mean of every rating (no longer a fitted strength),
   the virtual games floored at 0.05: a young strong tier joined thinly 1.6 to
   2.6 errors low (medium; the fitted centre had held it near one), and a
   block that swept or was swept by everything floated on unrelated configs'
   pulls, every error infinite, some fits falsely unconverged (medium).
8. The floor raised to 0.2, and convergence also on the Newton decrement: no
   floating block, no false unconverged run in the check's fuzzing. The
   remaining tier bias was first accepted on the ground that real gaps are a
   few hundred Elo — which the check showed false (medium): tiers of lightly
   played configs 400–800 Elo apart, joined by one small job, sit 1 to 3
   errors low, the 95% interval covering the truth 30–65% of the time.
9. No weighting removes that — it is the thin-link limit of any prior that
   pulls config by config, and the old prior was worse (510 Elo low where this
   is 260) — so the error bar is made honest instead: each config's error is
   widened by the prior's one-step shift, `I⁻¹ · ∇prior`, added to its
   variance. The 600-Elo case goes from 1.74 errors to 1.13; but at 800 the
   one step underestimated a saturated pull, coverage 64–86% (medium).
10. The shift taken one and a half times (the check's own measurement on the
    same data): 95–100% coverage at 600 and 800 Elo, a well measured config's
    error a few percent wider at most. The rest is KL-79, with the check's
    figures.

**Verified:** the 18 `U-STATS-5` tests (each adversarial case among them) fail
against the version they broke and pass, as do the 12 fit tests that predate
the run; the analytic `U-STATS-4` errors
still hold; the ratings and public API integration tests pass.

### 1.5 Adversarial checks of the pass's fixes

Two reviewers who did not write the fixes, each given only them.

**The rating fit (KL-74).** Ten rounds, each against the version before; 1.K
lists what each round broke. The check's final state: across some 4,000 fuzzed
pools (1 to 120 configs, up to a million pairs a head-to-head, anchors at
±9,999), no fit unconverged, no non-finite rating, agreement with an
independent numpy reference to about 0.001 Elo; pair-flip monotonicity broken
about once in 500, by under an Elo; release timing 0.007 s at 100 members,
0.3 s at 400, 2–3 s at 800.

**Everything else** (the idle finish check, the docs and script fixes, the
lows). **Medium, fixed — `terraform fmt -check` failed.** The new IAM resource
lists left `actions` padded for an alignment group it no longer had, so CI's
terraform job would have failed (HEAD passed): reproduced with
`hashicorp/terraform:1.9`, fixed with `terraform fmt`; `fmt -check` and
`validate` pass. **Held:** the idle finish check against purge, delete,
deactivate, reactivate and force-complete (the witness order, the `active`
guard), SPRT below `min_units`, declined opening-rack tasks, leave jobs, a
failed check retried ten seconds later; its cost (one aggregate per job every
ten seconds, only with nothing in flight); the self-play skip (no refit loop);
the second-process advice against a dummy server; the plausibility caps
against the columns and the frontend; `restore-job-check.sh` on its own
Postgres; the KL entries against the code. **Lows fixed:** PLAN's SPRT label;
TESTING's tier-2 counts; the README's `dev.py` paragraph (MAGPIE writes no
`settings.txt` for `contribute`); PLAN's "stops with a clear error" (it stops
on the board layout first); a doc comment above the wrong function; the
finish-check counters forgotten on a force-complete; the second process's file
must be a copy of the first; the nightly-only checks marked as such. **Also
found while testing:** two worker-route tests used values between the old and
new plausibility caps, and a rate-limit test assumed its 150 requests finished
inside a refill (it failed at load average 13): both corrected.

### 1.6 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included
  (`MAGPIE_BIN` the `portable_release` build of `7400184f`): **560 of 560**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 117 of 117.
- `terraform fmt -check -recursive` and `validate` (`hashicorp/terraform:1.9`,
  a copy of `infra/` and `scripts/`): clean after the fix above.
- `scripts/restore-job-check.sh` (changed script): passed, on its own
  Postgres, in the adversarial check.
- MAGPIE did not change, so its test table and tier 6's native run were not
  required.

---

## Pass 2 — follow-up pass

**Plan.** The diff since the previous pass's base (`a229895..3fdbc27`; MAGPIE
unchanged), reviewed against every objective, one reviewer per part it
touches: backend (the rating fit above all, and the idle finish check);
frontend; docs and procedures (PLAN, TESTING, README, `scripts/`); infra.
Plus one area not examined in recent passes: **the backup pipeline and the
restore drill** — `scripts/backup.sh` and `restore-drill.sh`, their manifests
and the `backups` row, `infra/backup.tf` (schedules, IAM, the drill's disk and
alarms), `routes/admin.rs`'s backups view, RUNBOOK §§ on restore, and the
nightly CI jobs that exercise them.

**Findings: 0 high, 2 medium** from the five reviewers (backend none, 4 low;
frontend 1 medium, 5 low; docs and procedures none, 7 low, 2 unconfirmed;
infra none, 2 low; backups 1 medium, 11 low, 2 unconfirmed). Both fixed and
verified; the adversarial check is 2.4.

### 2.1 Medium — E-8 could not pass (frontend reviewer)

**Test updated.** E-8 looked for the SPRT line `… — LLR …` on the settled
seeded game-pairs job, but a job the finish check completes has stored its
verdict since the eleventh audit, and the page then shows `Completed: …`
instead — so the locator timed out on every run. Pass 1's label change
("stopped at its cap") added a second mismatch to its regex. TESTING lists E-8
as covered; tier 5 has not run in any audit since AUDIT_FINDINGS_14 (image
builds), which is how it went unseen. **Fix:** E-8 asserts the `Completed:`
paragraph. **Verified** against a build of the pages with the API mocked,
using E-8's exact locator and regex: the old spec matched none of the settled
cases, the new one matches a job decided `passed` and one `terminated_at_max`.
Tier 5 itself was not run (image builds). With it: the decided sentence reads
"Completed: <verdict>, LLR x after N pairs" (it read "… at its cap at LLR"),
the admin line likewise, and the SPRT badge shows "at its cap" rather than the
raw `terminated at max`.

### 2.2 Medium — PLAN said the monthly restore drill writes a result row; it writes nothing (backups reviewer)

**PLAN updated.** The drill task has no database credentials by design and
leaves only its exit status (mailed by `restore-drill-failed`), an unalarmed
`DrillSuccess` metric and its logs: reproduced with `backup-drill-check.sh`,
no row anywhere. PLAN's "Verifying a restore" also listed a check ("no
completed task with insufficient accepted claims") that neither the drill nor
RUNBOOK §4 runs. **Fix:** PLAN's Drills paragraph says what the drill records,
and that nothing shows when it last passed (added to KL-65, with the unalarmed
metric); the verification list names the counter check the drill does run.

### 2.3 Low findings

**Fixed:**
- `scripts/restore-roundtrip.sh` seeded any table that was empty, so on a
  developer's database with jobs but no tasks it added a completed task, a
  claim, a result and an `ok` backups row to every job: reproduced on an
  isolated stack (jobs|tasks|claims|results|backups 3|0|0|0|0 → 3|3|3|3|1).
  It now seeds only a database with no jobs and round-trips any other as it
  is: re-run twice, counts unchanged; an empty database is still seeded;
  `backup-drill-check.sh` passes.
- The rating fit runs on a blocking thread (`spawn_blocking`), not the
  runtime's (0.23 s at 400 members, 4 s for a 400-rung ladder of sweeps, on a
  one-vCPU service); leave rack means keep their own plausibility cap (5,000),
  which pass 1's 21×21 widening had loosened for no reason; the worker page's
  query re-indented; a stale test comment.
- The drill's "not enough disk" message pointed at a RUNBOOK procedure that
  does not exist and at server mode, whose `DATABASE_URL` is production's in
  the ops task — it now names KL-46 and warns; its header's "7.6" reference.
- `backup_ephemeral_storage_gib` gains the 21–200 validation its sibling has.
- Headers: `backup.sh` needs `_sqlx_migrations`; `dev-restore.sh` scrubs every
  restore; `dev.py`'s "settings.txt" claim; the IAM comment cites the service
  authorization reference; README's image-build paragraph after its block, and
  a line on emulating amd64 on arm64 Linux; the second contributor's file
  without the first one's `uuid` line; PLAN's "the row is exactly the
  manifest"; KL-74's summary (errors widened, figures in KL-79); KL-79's
  mature-tier figures as ranges by layout; KL-17's justification (cold cache
  unmeasured); TESTING names the self-play and ±∞ tests.

**Tried and not kept:** a `TERM` trap in `backup.sh` and `restore-drill.sh`
so an ECS StopTask would run the EXIT trap — measured in `postgres:16`, bash
defers the trap while a foreground child (`pg_dump`) runs and the SIGKILL
comes first (exit 137, trap not run), so it would claim a fix it does not
make. Recorded in KL-80.

**Recorded:** KL-65 (the drill's record), new KL-80 (backup timing excludes
the upload, the StopTask case above, the check's leftover bucket, two
unreproduced suspicions), and the frontend's remaining lows in KL-75 (the
history's jump at a pool's first Newton fit) and KL-76 (a clamped error bar
has no mark).

### 2.4 Adversarial check of the pass's fixes

**No high or medium.** **Held:** E-8 against every way the seeded job can
settle (a completed games or pairs job always stores its verdict; only a
force-complete, which no spec calls, does not; the fake workers' 60% win rate
ends it `passed` or at its cap), and no other spec or unit test depended on
the old wording; the blocking fit (no deadlock, a panic is a 500 and a
rollback); the rack-mean cap in practice; the worker page's re-indentation
(whitespace only); `restore-roundtrip.sh` seeded on CI's fresh schema and
unchanged on a re-run; `backup-drill-check.sh`; the storage validation's
bounds through `terraform console`; the docs. **Lows fixed:**
`restore-roundtrip.sh` still seeded a database holding users or input data but
no jobs (reproduced: an active job and an admin appeared) — it now seeds only
an empty one, and its two content comparisons get unique tiebreakers; the
script, README and RUNBOOK say it seeds only a fresh schema and wants an idle
stack; the rack-mean comment (the mean is a best play's equity, not a leave
value; the bound still holds with 15-letter lexicons); PLAN's drill runs the
*SQL* referential checks. **Recorded (KL-80):** the round trip's counts are
read outside the dump's snapshot; a dropped refit request leaves its fit
running on the blocking pool.

### 2.5 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **560 of 560**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 117 of 117.
- `terraform fmt -check -recursive` and `validate`: clean.
- `scripts/restore-roundtrip.sh` (changed) on an isolated stack through its
  failure (the committed script seeding a database with jobs) and its cases
  (users only, empty, re-run); `scripts/backup-drill-check.sh`: passed.
- E-8's new assertion against a build of the pages with the API mocked. Tier 5
  itself not run (image builds).
- MAGPIE unchanged.

---

## Pass 3 — follow-up pass

**Plan.** The diff since the previous pass's base (`3fdbc27..9753314`; MAGPIE
unchanged), reviewed against every objective, one reviewer per part it
touches: backend; frontend and the e2e spec; docs and procedures (PLAN,
TESTING, README, RUNBOOK, `scripts/`); infra. Plus one area not examined in
recent passes: **job creation and player-config validation** — the admin
create-job and player-config routes, their validation of every parameter,
`magpie_defaults`, the job templates they produce, the admin forms, and what a
malformed or extreme config does downstream (dispatch, MAGPIE, the fit).

**Findings: 0 high, 2 medium** from the five reviewers (backend none, 3 low;
frontend and e2e 1 medium, 3 low; docs and procedures none, 6 low; infra none,
3 low; job creation 1 medium, 7 low, 1 unconfirmed). Both fixed and verified;
the adversarial check is 3.4.

### 3.1 Medium — tier 5 still failed: E-1 and E-5 asserted text their pages dropped in the thirteenth and fourteenth audits (frontend reviewer)

**Tests updated.** E-1 looked for `units` in the job list's row, which since
the fourteenth audit says `pairs` (or games, racks, generations); E-5 looked
for a form label "User ID or anonymous UUID", which the thirteenth audit split
into "Anonymous UUID" and "User ID". So CI's per-PR `e2e` job failed on every
run, as E-8 did (2.1). **Verified by running tier 5** — for the first time
since AUDIT_FINDINGS_14 — natively, without images: the backend built and run
on its own database with the e2e compose file's settings, the built pages
behind nginx with the image's `/api` proxy, the GitHub fixtures served
locally, `seed.py` with `run.sh`'s arguments, and the fake workers with the
compose arguments. The committed specs: 2 failed (E-1, E-5), 9 passed; with
the two lines changed: 11 of 11, and again 11 of 11 on a fresh database.
E-8's new assertion passed in both runs (the seeded job decided `passed`).
**Lows fixed:** the badge's lookups use own keys only; "after 1 pairs"; E-8's
comment.

### 3.2 Medium — job creation accepted jobs no worker can run, and five failures in a row stop `magpie contribute` (job-creation reviewer)

**Code updated.** Three configurations passed validation and failed on every
contributor: a 21×21 layout (it ships beside the super-board lexica; every
MAGPIE the fleet runs is built 15×15 — "invalid number of rows … expected 16,
got 22"); a `movegen_margin` or `inference_margin` past MAGPIE's largest
equity ("server sent an invalid movegen_margin"); and `num_plays` or
`num_plays_recorded` of millions (MAGPIE allocates them all up front: 2e9 was a
16 GB malloc and a core dump). The server does nothing with `task_failed`
declines and MAGPIE ends `contribute` after five in a row, so such a job alone
on offer took the fleet down within seconds. Reproduced through the API
(created and dispatched) and against the real binary behind a stand-in server.
PLAN says birdtest "must not be able to build a job MAGPIE would refuse to
load". **Fix:** a job's layout is checked as MAGPIE's loader checks it (a
start square on the board, then exactly 15 rows of 15 known squares); margins
stop at MAGPIE's largest equity, 2,147,483.645, which it takes; `num_plays` at
200,000 and `num_plays_recorded` at 32,767 (a stored rank is a `SMALLINT`);
and, a low, SPRT `alpha` and `beta` at 0.000001 (a subnormal alpha made the
bound infinite and the job page threw). The first version counted rows and
capped `num_plays` at 32,767 too; the adversarial check (3.4) broke both. **Verified:**
`a_config_or_job_no_worker_can_run_is_refused` (`A-ADMIN-21`, now at every
boundary) fails against the committed validation (the super-board job
created, 201) and passes, as does a unit test of the layout check on the
cases where MAGPIE's loader and a row count disagreed (CRLF, blank lines,
trailing whitespace, widths, squares, start squares). A confirmation round
compared the check with the real binary on 43 files: it never accepts what
MAGPIE refuses, and differs only on six files MAGPIE accepts through parser
quirks (a coordinate past `int`, a whitespace-only or `\v`-led coordinate, a
NUL after the last row, a byte above 0x7f, which MAGPIE reads out of bounds —
KL-68).
The test helper's layout is now the real `standard15.txt`; the full suite
passes.
KL-2 records the deeper gap: nothing acts on a job every worker fails.

### 3.3 Low findings

**Fixed:** the refit's panic text no longer reaches the `500` body; the
`recompute` doc; the rack-mean comment; README's `dev-restore.sh` example uses
`SCRUB=0` for your own snapshot and says every restore is scrubbed;
`restore-roundtrip.sh`'s and KL-80's "before and after" (after the dump);
RUNBOOK's "byte-identically"; TESTING's `± ∞`; the pass-2 summary's KL list;
the drill header's wrap; PLAN's "seeds S, S+1, …" (MAGPIE draws a batch's
seeds from a stream seeded with S); the variant field is a select; Fargate
CPU/memory pairs, the builder's ephemeral storage (21–200) and RDS's
`max_allocated_storage` (capped at 65,536 GiB) are validated at plan — a mock
`terraform test` refuses the bad pairs and passes the defaults.

**Recorded:** KL-2 (a job every worker fails), KL-60 (no page shows a config
in full; a duplicate name's generic 409; no audit row for a new config; a leave
job created twice when its KLV build fails); KL-68 (MAGPIE's out-of-bounds read
of a layout byte above 0x7f) and KL-80 (the round trip's counts, after the
dump) corrected.

### 3.4 Adversarial check of the pass's fixes

**Medium, fixed — `num_plays` capped below what real opening racks need.** The
32,767 cap was the recorded rank's, applied to plays generated too, and
blank-heavy opening racks have more: 63,585 plays for ??EIRST in CSW24
(measured with the pinned binary), so a static player could no longer rank
every opening play, where PLAN promises `num_moves` keeps the whole count.
100,000 plays ran without trouble. **Fix:** `num_plays` stops at 200,000 (some
11 MB a player a thread) and only `num_plays_recorded` at 32,767; the test
creates a config at 63,585.

**Lows fixed:** the layout check counted rows, where MAGPIE also checks row
widths, squares and the start square, ignores blank lines and refuses a
trailing whitespace line — six layouts it refuses were accepted and two it
loads refused; it now ports MAGPIE's check. MAGPIE takes a margin of exactly
2,147,483.645 (it refuses only above), which the first cap refused.
`Object.hasOwn` is newer than the build's browser targets (own-key checks now
use `hasOwnProperty.call`). PLAN's seed sentences at the Task and design-table
entries contradicted the corrected one; its creation rules still said "strictly
between 0 and 1" and "six rules". TESTING's `A-ADMIN-21` said alpha only.
`db_allocated_storage`'s description names RDS's ceiling. **Recorded:** batch
and generation sizes have no ceiling (KL-2); the job form defaults to the
newest layout, which could be one creation refuses (KL-60). **Held:** the
Fargate table and its boundaries in a 61-run mock `terraform test`; every
default; `fmt` and `validate`; 227 backend tests with the real MAGPIE; and
**tier 5 on the whole working tree, natively: 11 of 11**, the seed's layout
accepted by the new check.

### 3.5 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **562 of 562**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 117 of 117.
- `terraform fmt -check -recursive`, `validate`, and a mock `terraform test`
  of the new validations: clean.
- **Tier 5, natively (no image builds): 11 of 11**, twice by the frontend
  reviewer with the E-1 and E-5 fixes and once by the adversarial check on the
  whole working tree.
- MAGPIE unchanged.

---

## Pass 4 — follow-up pass

**Plan.** The diff since the previous pass's base (`9753314..7e75361`; MAGPIE
unchanged), one reviewer per part it touches: backend; frontend and the e2e
specs; docs and procedures; infra. Plus one area not examined in recent
passes: **the live stats stream and the stats cache** — `sse.rs`, the job
stream route and its caps, `push_stats_until_idle` and its coalescing, the
stats payload cache (`JOB_STATS_CACHE_SECONDS`), `jobstats::compute`'s cost,
and the pages' `EventSource` handling and reconnection.

**Findings: 0 high, 4 medium** from the five reviewers (backend none, 4 low;
frontend and e2e 1 medium, 6 low — tier 5 natively 11 of 11; docs and
procedures 1 medium, 11 low; infra none, 4 low; the live stats stream 2
medium, 6 low, 2 unconfirmed). All fixed and verified; the adversarial check
is 4.6.

### 4.1 Medium — live pushes were not spaced (live-stats reviewer)

**Code updated.** The push loop paused only after a build during which another
submission had asked for one; when none had, it forgot the job, and the next
submission started a fresh, uncached build at once. So a job whose submissions
came slower than one build was rebuilt and pushed for each — reproduced, six
full builds in 2.1 s under a ten-second `JOB_STATS_CACHE_SECONDS` — where PLAN
said pushes are "spaced at least `JOB_STATS_CACHE_SECONDS` apart" (and, in
other places, once a second). At 400,000 results a build is 0.7–1 s. **Fix:**
every build is followed by the cool-down with the push still in flight, so a
submission meanwhile only marks it to go round again; an admin's change (a
per-job `Notify`) cuts the cool-down short, so a deactivation or completion
still reaches the page at once. **Verified:**
`live_pushes_are_spaced_by_the_stats_interval_but_admin_changes_are_not`
(`A-PUBLIC-6d`) fails with the loop as committed and passes, including a
mid-interval deactivation pushed within seconds; `an_urgent_push_cuts_the_cool_down_short`;
PLAN now says one rule.

### 4.2 Medium — PLAN's cost for the finish check's read understated it some sevenfold (live-stats reviewer)

**PLAN updated.** `game_stats` at 400,000 result rows — a job of 400,000 games
at the form's default batch of one — measured 340–620 ms warm (its sort spills
at the default `work_mem`), where PLAN's table said 50 ms and KL-10 "tens of
milliseconds"; KL-10's justification ("moving it gains milliseconds") rested
on that. The read gates dispatch, so rewriting it was not taken lightly within
a pass: PLAN's table and summary give both figures, and KL-10 now records the
real cost and the options (running win/loss/tie and pentanomial totals beside
`games_completed`, a stride that grows with the job, a leaner query at about
200–280 ms), open.

### 4.3 Medium — the job form refused SPRT settings the server accepts (frontend reviewer)

**Code updated.** α and β took only multiples of 0.01 and the Elo bounds only
whole numbers (the inputs' `step`), so the browser blocked, with no request
and no page message, α = 0.025 or the whole [0.000001, 0.01) range pass 3
documented, and bounds such as [0.5, 2.5]. Reproduced against the native
tier-5 stack. **Fix:** `step="any"` with the server's bounds on α and β, and on
the Elo bounds; with it, `min_*` may be 0, the player-config form's 0.1 steps
are `any`, both forms clear a server error on the next edit (a submit the
browser blocked left the previous one showing), and "until 1 games" reads
right. Checked by the adversarial check (4.6) driving the forms.

### 4.4 Medium — `dev-restore.sh` scrubbed only when `SCRUB` was exactly 1 (docs reviewer)

**Code updated.** README (pass 3) and the header said every restore is scrubbed
unless `SCRUB=0`, but `SCRUB=true`, `yes` or `TRUE` turned the scrub off —
reproduced with a stub `COMPOSE`: no `scrub.sql` — restoring a production
dump's real addresses and password hashes onto a laptop. **Fix:** only `0` or
`1` is taken, anything else refused before any change, and the scrub runs
unless `0`. Re-run with the stub: unset and `1` scrub, `0` does not, `true`,
`yes` and `TRUE` exit 2 with no compose call.

### 4.5 Low findings

**Fixed:** no `spawn_blocking` join error's text (a panic's message) reaches a
`500` body any more (`AppError::task_failed`, six sites); a rating pool on a
board no job can use is refused; the layout test's comment; KL-2 (checks run
at creation only); Fargate CPU sizes validated, the memory message gives the
whole table, and `db_allocated_storage` stops at 59,578 GiB (past it RDS's
ceiling is under a tenth above the allocation) — a mock `terraform test` of
11 runs passes; RUNBOOK §1's ceiling wording; a push after a delete ends
quietly; `sse.rs`'s and PLAN's "an idle server holds no per-job state"; the job
page's "live, on every accepted result"; TESTING's `A-ADMIN-21` claims now
tested (β, the 0.000001 boundary, the inference margin); PLAN's schema seed
comment, the player-config paragraph and API row, the generation-0 KLV wording
in PLAN and `create_job`, KL-60's sentence; README's scrub list; the findings
file's section numbering, KL lists and a dangling pointer. **Recorded:** KL-60
(errors by API name at the foot of the form; the layout checked after the other
rules), KL-78 (a push overtaken by a page build rebuilds; a deleted job's page
says nothing). The mock `terraform test` guards `outputs.tf`'s DKIM index with
`try()` in its own copy — the committed file cannot be mocked as it stands,
which matters only if a `terraform test` is ever put in CI.

### 4.6 Adversarial check of the pass's fixes

**No high or medium.** **Held:** the spacing test fails against the committed
loop (six pushes in 2.6 s) and passes; on a native backend with a 30-second
interval and four fake workers, pushes came exactly 30 s apart; a SIGTERM with
a loop asleep in its cool-down exits in 0.1 s; no lost wake-up (a `Notify`
permit waits for the next `notified()`), no second loop per job, completion,
the scheduler's `JobFinished` and a leave generation closing all push urgently;
`dev-restore.sh` through 14 values with a stub and for real on an isolated
stack (`yes` refused, the database untouched; `0` kept real addresses and a
key; unset and `1` scrubbed); the forms, driven with Playwright, submit α =
0.025, α = β = 0.0000015, Elo 0.5→2.5 and −0.5→2.5, `min` 0, and margins of
2.55; the Terraform validations in a 12-run mock test; TESTING's counts; and
**tier 5 natively on the working tree: 11 of 11** (twice). **Lows fixed:** a
job deleted with a round pending left its push entry for good (now abandoned);
an admin's change during a build left a wake-up that skipped the next
cool-down for an identical push (spent after the rebuild); two more joins let
a panic's text reach an admin (the purge/delete operation, an export's stored
error); the refit's failure log names its pool again; an edit cleared a
failure to load the form's choices as well as a submit's error (only the
submit's now); "live, at most every few seconds" (now plain "live"); the remaining "on every
accepted result" and "one a second" in PLAN, `public.rs` and `sse.ts`; a stray
comma in RUNBOOK §1. **Unconfirmed, recorded under KL-78:** a page view in the
gap between the cache's expiry and the next push can cost a second build per
interval.

### 4.7 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **565 of 565**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 117 of 117.
- `terraform fmt -check -recursive`, `validate`, and a mock `terraform test` of
  the new validations: clean.
- `scripts/dev-restore.sh` through its values with a stub `COMPOSE`, and for
  real on an isolated stack (adversarial check).
- **Tier 5, natively: 11 of 11** — on the committed pass-3 tree by the frontend
  reviewer, and on the working tree by the adversarial check.
- MAGPIE unchanged.

---

## Pass 5 — follow-up pass

**Plan.** The diff since the previous pass's base (`7e75361..dd8ae36`; MAGPIE
unchanged), one reviewer per part it touches: backend (the live-push loop
above all); frontend; docs and procedures; infra. Plus one area not examined
in recent passes: **the account lifecycle** — registration, email
confirmation, password reset, sign-in and sessions, account deletion and its
tombstone, API keys (creation, use, revocation), and the account page.

**Findings: 1 high, 3 medium** from the five reviewers (backend none, 3 low;
frontend none, 4 low — tier 5 natively 11 of 11; docs and procedures 1 medium,
9 low; infra none, 3 low; the account lifecycle 1 high, 2 medium, 5 low, 1
unconfirmed). All fixed and verified; the adversarial check (5.6) found 2
medium, both fixed.

### 5.1 High — Argon2 had no concurrency bound: a flood of sign-ins took the process past its memory (account reviewer)

**Code updated.** Every password hash and verify (19 MiB each, the crate's
defaults) ran on the blocking pool with no limit, and the rate limits are per
address: seven addresses' worth of wrong-password logins and registrations —
within every limit, needing no account, the target usernames public — held
140 runs at once. Reproduced on a native backend: RSS to 2.6 GB and held there
(glibc keeps the memory), past the single 2 GiB task, which is OOM-killed with
every claim and stream and the rate limiters' state. **Fix:** Argon2 runs on
four threads of its own, admitted by a semaphore of four; a request that waits
ten seconds for a turn is `503` with `Retry-After`; a taken username is
answered before the hash (it bought nothing there). The first version — the
semaphore alone, on the blocking pool — still peaked at 694 MB: glibc raises
its mmap threshold to the last large block freed, after which each 19 MiB
buffer came from a per-thread arena and stayed (eleven arenas, 600 MB, seen in
`smaps`), and dedicated threads alone still left 420 MB. So the allocator's
mmap threshold is pinned at 1 MiB at startup, and every such buffer is given
back. **Verified:** the same flood, 2.6 GB before; 129 MB peak and 52 MB after
now (logins alone 129 MB); `argon2_runs_wait_for_a_turn` (`U-AUTH-9`).

### 5.2 Medium — someone who had forgotten their username had no way back (account reviewer)

**Code updated.** Sign-in asks for a username, and neither the reset mail nor
the notice sent when someone registers a taken address named it; PLAN promises
that "a real person who has forgotten they signed up still finds out". Both
now name the account (both go only to the address's owner; the notice's lookup
runs in its own task, so the request's timing does not change), and so did the
confirmation mail, so an owner could see an account someone else registered on
their address (KL-34) — reverted in 5.6, and usernames restricted in 6.3. **Verified:** `the_owner_of_a_taken_address_is_told_their_username`
(`A-AUTH-4e`) and the reset test's new assertion.

### 5.3 Medium — PLAN said the registration form shows the server's password score; it does not (account reviewer)

**Docs and page updated.** The form's meter is length and character classes,
not zxcvbn, and disagrees with the server both ways (`Qwerty123456789!` shown
strong and refused; a long passphrase shown good and taken). PLAN and the page
now call it a rough guide that the server's check overrides.

### 5.4 Medium — PLAN's cost for `game_pair_stats` had the same sevenfold understatement (docs reviewer)

**PLAN updated.** Pass 4 corrected the games row only; the pairs row said 54 ms
where 400,000 one-pair rows (the form's default batch) measured 430–520 ms.
Both rows now carry the measured figure and say when they run (every eighth
submission, an idle job's check, every live build), and KL-10 says games or
pairs.

### 5.5 Low findings

**Fixed:** a reset and an admin's delete of one account deadlocked (reproduced
in psql; the delete now removes the account's rows first, as a reset takes
them — rerun, both finish); an admin change made during a third superseded
build waited out the cool-down (the wake-up is now drained before each build,
not after the publish); a pool on a letter distribution no job can use is
refused; a test that submissions during a cool-down are pushed when it ends;
RUNBOOK's PITR and DR ceilings capped at RDS's bounds and §1's wording;
`scripts/dev-restore-check.sh` (the SCRUB rule against a stub, in a new CI
`scripts` job; fails against pass 3's script, `7e75361`); the check-email page's
developer hint only in development; form errors announced (`role="alert"`);
stopping % bounded in the form; "acted on from the first pair" when the
minimum is 0; PLAN's heading "An event per round", the deletion rationale, the
scrub list, the schema's seed comment (the migration's copy left for its
checksum); TESTING's `A-ADMIN-21` boundaries now all tested, `A-RATE-3b`,
`S-BACKUP-6`; the findings file's RUNBOOK section names. **Recorded:** KL-60
(α/β arrow keys), KL-81 (a reset holds its transaction while it waits for a
turn — resolved in 5.6; no SES timeout; a known address's reset writes a row),
KL-34.

### 5.6 Adversarial check of the pass's fixes

**2 medium, both fixed and verified.**

- **Medium — a client that hung up escaped the Argon2 bound.** The request
  held the turn, so an aborted request gave its turn back while its run stayed
  queued on the four threads: 2,000 abandoned logins left a fresh one waiting
  46 s, and no request was ever told `503`. **Fix:** the turn goes with the
  run and is given back when the run ends, and a run whose requester has gone
  is skipped (`api_key::on_an_argon2_thread`). **Verified:** the new
  `abandoned_argon2_runs_do_not_queue_past_the_turns` (a thousand aborted
  requests, then a fresh hash) fails on the old code (the run took 8.4 s) and
  passes, under a second.
- **Medium — the confirmation mail carried the registrant's text to any
  address.** Naming the account (5.2's companion change for KL-34) let anyone
  put 32 characters of their own, newlines included, in a mail from
  birdtest's sender to an address they typed. **Fix:** the confirmation mail
  does not name the account; the taken-address notice and the reset mail
  still do, since they go to the account's own address. **Verified:**
  `A-AUTH-1` now asserts no line but `To:` carries the username; KL-34
  rewritten.

**Lows fixed:**
- **The reset/delete deadlock came back with two reset links.** 5.5's fix
  held for one link only: a reset spends link B while the delete removes link
  A, then waits for B, and the reset's spend of the account's other links
  waits for A. Reproduced in psql with the adversary's script (the delete was
  the victim). Both now lock the account's row first: the delete
  `FOR UPDATE`, the reset `FOR NO KEY UPDATE` — which the account's own
  submissions do wait on (their `UPDATE users` takes the same lock; corrected
  in pass 6), so the reset holds it only for a few statements. Rerun in both
  start orders: both commit, and a reset that starts second finds its link
  gone.
- **The reset hashes before its transaction.** The new lock is held for a few
  statements; hashed inside, it would have held the account's row through a
  wait of up to ten seconds for an Argon2 turn, against the account's own
  submissions, which count its tasks on that row. KL-81's first item is
  resolved this way.
- The pool test gained its letter-distribution case (fails with the check
  removed).
- `main.rs`'s allocator pin is `cfg(all(target_os = "linux", target_env =
  "gnu"))`.
- PLAN records the pin's cost, 12–18% of Argon2's throughput.
- The player-config form's stopping % takes the server's range, 0 to 100,
  instead of a stricter one.
- With a minimum of 0 the job page now says "SPRT can stop the job as soon as
  a bound is crossed".

**Held:** tier 5 natively on the working tree, 11 of 11; the full suite, 568
of 568; a flood that hangs up does not stall sign-in once the fix is in
(0 turns held after 2,000 dropped requests; a fresh hash in 36 ms).

### 5.7 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **569 of 569**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 117 of 117.
- `scripts/dev-restore-check.sh`: passes, and fails against pass 3's script
  (`7e75361`).
- The reset/delete race replayed in psql, old order and both new orders.
- **Tier 5, natively: 11 of 11** (reviewer and adversarial check).
- Terraform and MAGPIE unchanged since pass 4.

---

## Pass 6 — follow-up pass

**Plan.** The diff since the previous pass's base (`dd8ae36..6a609da`; MAGPIE
unchanged), one reviewer per part it touches: backend (the Argon2 threads and
turns, the reset's order); frontend (tier 5 natively); docs and procedures;
infra and CI. Plus one area not examined in recent passes: **leave
generation** — its universe, dispatch, results, merges, generation close and
MAGPIE's side of it.

**Findings: 2 high, 3 medium** from the five reviewers (backend 1 high, 5 low;
frontend none, 4 low — tier 5 natively 11 of 11; docs and procedures 1
medium, 11 low; infra none, 3 low; leave generation 1 high, 2 medium, 3 low, 2
unconfirmed). All fixed and verified; the adversarial check (6.7) found 1
high in the fixes, fixed.

### 6.1 High — password scoring ran on the executor: one address stalled the server for seconds (backend reviewer)

**Code updated.** zxcvbn's cost depends on the characters, not only the
length it truncates to: a hundred characters of its substitution letters take
it about 0.9 s (release). It ran on the async executor, in registration and —
before any token was looked at — in the reset. Pinned to one CPU, twenty such
resets from one address, within its limits, held `/health` for up to 8.5 s. The
earlier audits that held "zxcvbn's input cap" measured length, not this.
Scoring now runs off the executor (`too_weak_off_the_executor`) — first on
the four Argon2 threads, under sign-in's turns, then, after 6.7, on two turns
of its own — and the reset reads its link first, without a
lock, so a wrong link costs neither a score nor a hash; it then scores and
hashes outside any transaction, and only then locks the account and spends the
link. **Verified:** `scoring_a_crafted_password_does_not_stall_the_server`
(`A-AUTH-3b`: a single-threaded runtime ticks throughout; on the old code the
gap was 1.08 s) and a wrong link refused in under 100 ms with the crafted
password; zxcvbn is built optimized in tests so they time what production
does.

### 6.2 High — MAGPIE spells a rack blanks last, the universe blanks first: no blank rack was ever counted (leave-generation reviewer)

**Code updated.** The server forced `?AEINST`; MAGPIE played it and reported
`AEINST?` (`rack_get_string(…, blanks_first = false)`); the merge matched racks
exactly and dropped what matched nothing, silently. 715,540 of English's
3,199,724 racks hold a blank: none gained an occurrence, each was forced on
every lap, and an English generation could never close. German and Polish
failed a second way (MAGPIE orders `AÄB…`, the universe by code point). No test
could see it: the tier-6 tests submit the forced racks as sent, and M-4 only
checked that something landed. A reported rack is now spelled as the universe
spells it before anything reads it (`leave_gen::canonical_rack`), one rack
under two spellings is a duplicate, and a merge logs racks that matched no row.
**Verified:** `a_rack_counts_however_the_worker_spells_it` (`I-LEAVE-20`: the
whole universe reported reversed; on the old code the merge updated 0 racks);
the unit test covers German's `Ä`; and tier 6's M-4, now asserting that every
rack a real MAGPIE reported matched and that a blank rack counted, **fails on
the old code — 73 of 870 reported racks matched nothing — and passes**.

### 6.3 Medium — a stranger's text still reached an address through the reset mail and the notice (docs reviewer)

**Code updated.** 5.6 took the username out of the confirmation mail, but once
a mail scanner confirms an account registered on someone else's address
(KL-34), the reset mail and the taken-address notice name it — 32 characters of
the registrant's own, newlines included, from birdtest's sender, repeatable
five times an hour each. A username may no longer hold a control character, a
line or paragraph separator or a format character (bidi overrides, zero-width
marks); an older account's are mailed as `?`. **Verified:**
`a_username_cannot_carry_a_message_into_mail` (`A-AUTH-4f`) fails on the old
code (the newline name registered, `201`) and passes; a unit test covers the
list. KL-34 says what still reaches the owner: a name, only as a name.

### 6.4 Medium — in the tail, a decline or a merge between two reads closed a generation short (leave-generation reviewer)

**Code updated.** The tail's selection holds out racks in flight or staged;
when it comes back empty, two later statements ask whether anything is in
flight or staged. A claim declined or lapsed, or a merge committed, between the
selection and those reads left racks the first held out and the others no
longer saw: the generation closed with them below target — a declined task's
at zero, which the KLV weighs like a measured zero. KL-13's justification said
this could not happen. The tail now reads, last, whether any rack is below
target holding nothing out, and a rack that is answers "no work yet" for the
next claim to hand it out. **Verified** with the reviewer's instrumentation (a
sleep after an empty selection, in a private copy): on the old code both
triggers started the transition with two racks below target; with the fix
neither did, and the controls were unchanged. No committed test: the window
cannot be widened without a hook in the code.

### 6.5 Medium — a merge's temporary files grew with the backlog (leave-generation reviewer)

**Code updated.** Measured on a full-size generation, one merge statement of
200 staged results of 150,000 racks wrote 2.9 GB of temporary files (159 s),
on a 20 GiB volume, and a backlog after an outage is larger. Two things
spilled: `UNNEST(a, b, c)` in `FROM` materialized every array (1.5 GB for 200,
found by `EXPLAIN`), and the sum over every element staged. The arrays are now
unnested in the select list, where they stream, and a merge sums a slice of
the racks by hash per pass — in the final version (6.7), one pass per 400,000
racks of the generation, each slice's hash table held in memory. **Verified:**
eight passes over a full English generation, 153 s for 200 staged results and
253 s for 600, with no temporary files at all (the single statement: 159 s and
2.9 GB at 200); `a_merge_of_a_backlog_sums_it_in_passes_exactly` (`I-LEAVE-21`: three passes, every total
exact); tier 6's M-4 merges a real generation. Two versions came first: one
kept the `FROM`-clause `UNNEST` (13 GB over eight passes; the plan showed
why), and one took a pass per fifty results, whose time the adversarial check
showed grows as the square of the backlog.

### 6.6 Low findings

**Fixed:**
- An email confirmation and an admin's delete of one account deadlocked (the
  confirmation took its code, then the account). Confirmations, resets and
  deletes now all lock the account first, and `A-AUTH-9b` and `A-ADMIN-22` pin
  it: with the account held, each waits holding none of its links. Both fail
  on the old order.
- A reset requested while its account was being deleted inserted a token for
  the tombstone and mailed a link. The insert now takes a key-share lock on a
  live account only.
- The reset's hash moved again, out of the transaction: pass 5's lock order
  had made the account's own submissions wait through the Argon2 turn. 5.6's
  claim that they "do not wait on" the lock was wrong; corrected.
- `racks_per_task` is capped at 10,000, in the server and the form.
- The admin job page no longer says a KLV whose hash differs has "almost
  certainly moved on": a closed generation's rows do not change.
- `dev-restore-check.sh` hung when run by hand in a terminal (its stub read the
  tty); CI's `scripts` job has a timeout; TESTING and PLAN list it.
- Accessibility and wording:
  - `role="alert"` on the public auth forms' errors;
  - the check-email page's console-mail hint is back for `dev.py` and compose,
    decided at run time from the host;
  - the `503` says "the server is busy checking passwords" (it reaches
    registration and resets too);
  - the SPRT line with a minimum of 0 says "checked as pairs arrive".
- Docs:
  - RUNBOOK §1 drops the storage ceiling past 59,578 GiB, where none is legal;
  - PLAN's password-refusal, reset-flow and cost wording, KL-37's measured
    1.6 ms registration gap, KL-81;
  - TESTING's CI and scripts tables and counts;
  - pass 5's record: 5.2 and 5.5 marked as changed by 5.6, "the committed
    script" named, and U-AUTH-9b's failure described as observed.

**Recorded:** KL-78, for a push whose build fails after spending its wake-up,
and a hostile leave result's reach (unconfirmed); KL-2 (the new cap).
**Unconfirmed, left:** a universe seeding that runs after a purge (KL-78
already holds its twin).

### 6.7 Adversarial check of the pass's fixes

**1 high, fixed and verified.**

- **High — scoring on sign-in's turns let one reset link keep sign-in down.**
  6.1 moved scoring onto the four Argon2 turns, the ones sign-in's verify
  waits on. A reset link refused a weak password still works, and a weak
  password can still be slow to score (a username of substitution letters,
  three times over: 0.78 s), so one confirmed account's link, replayed from
  eight addresses at their limit, kept logins at 10 s and `503` on one vCPU
  (`/health` fine). **Fix:** scoring has two turns of its own, on the
  blocking pool, with its own `503`; and each scoring a reset link buys is
  counted against the link, five an hour from any number of addresses.
  **Verified:** `scoring_never_holds_up_a_sign_in_and_a_link_buys_few`
  (`A-AUTH-3c`): with sixteen crafted scorings queued, a sign-in waited 3.9 s
  on the old code and answers at once now; the link's sixth attempt is `429`.

**Lows fixed:**
- The merge's passes follow the generation's size, not the backlog (see 6.5):
  measured, a pass per fifty results would take 7.7 minutes at a thousand
  staged and fall behind at a result a second.
- SES parts are sent as UTF-8: with no charset SES reads 7-bit ASCII, and the
  mails now carry names such as `李小龍`.
- Usernames may not hold default-ignorable characters that are not Cf
  (variation selectors, the Hangul fillers, `U+034F`) or the blank Braille
  pattern: `walker` plus one of them registered as a second `walker`.
- `merge_staged`'s comment described one statement.

**Recorded:** KL-81 — a link's five scorings an hour; composed and decomposed
lookalike usernames (normalizing only new names would lock out existing
ones). **Held, by reasoning:** no deadlock among the account paths; no
livelock in the tail's recheck; no staged row lost between passes (purges take
the merge lock). **Left:** results staged before a deploy of 6.2 keep MAGPIE's
spelling and are dropped at the next merge, logged (nothing is deployed yet).

### 6.8 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **578 of 578**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 117 of 117.
- Tier 6's M-4, natively against a real MAGPIE: fails on the old spelling (73
  of 870 racks unmatched), passes with the fix, and passes after the merge's
  redesign.
- **Tier 5, natively: 11 of 11** on the committed pass-5 tree (frontend
  reviewer).
- The tail-close race, instrumented in a private copy: old code closes short,
  new code does not.
- Merge benchmarks on a full English generation (`p6-merge/` in the scratch
  directory).
- `scripts/dev-restore-check.sh` under a pseudo-terminal: passes (it hung).
- Terraform unchanged (fmt and validate clean, infra reviewer); MAGPIE
  unchanged.


---

## Pass 7 — follow-up pass

**Plan.** The diff since the previous pass's base (`6a609da..92544dc`; MAGPIE
unchanged), one reviewer per part it touches: backend; frontend (tier 5
natively); docs and procedures; infra and CI. Plus one area not examined in
recent passes: **derived-file builds and the worker data gate** — the build
queue and the `build-derived` task, the gate that holds a job until its files
are built, and how workers prove they hold the right data.

**Findings: 0 high, 7 medium** from the five reviewers (backend 1 medium, 4
low, 2 unconfirmed; frontend 1 medium, 4 low — tier 5 natively 11 of 11; docs
and procedures 1 medium, 8 low; infra and CI 1 medium, 2 low, 1 unconfirmed;
derived builds 3 medium, 9 low, 1 unconfirmed). All fixed and verified.

### 7.1 Medium — a failed build was retried at once, so one run spent all three attempts in seconds (derived-builds reviewer)

**Code updated.** A failed build went back to `pending` and, still the oldest
row, was taken again by the same run: an S3 outage of a few seconds used up
`MAX_ATTEMPTS` and left the row `failed` until an admin pressed Retry, its job
handing out nothing meanwhile (reproduced: three attempts in 5 s, then a second
run with S3 back built nothing). A failed attempt now waits 5 and then 15
minutes before the next (`leased_until` holds the time while the row is
pending), and the builder takes a pending row only once it has passed.
**Verified:** `I-DERIVED-7` now asserts the row pending with a five-minute wait
and not taken again; it fails on the old code. `infra/derived.tf`'s comment and
RUNBOOK (re-import, *then* Retry) say what happens.

### 7.2 Medium — a stalled S3 read held the builder task for good (derived-builds reviewer)

**Code updated.** The S3 client sets a connect timeout only; a connection that
stalled after it held `build-derived` — a 4 vCPU, 8 GB task — past a 200-second
test, and each five-minute run that met the same stall stranded another. An
input fetch now gives up after five minutes, a whole build after seventy (its
MAGPIE killed), and the lease is 75 minutes, longer than both (a rack info
table is two converts of up to 30). **Verified** against a listener that
accepts and never answers: the run ended after 300 s, the row `pending` with
"fetching … took longer than 5 minutes" and its five-minute wait.

### 7.3 Medium — a wordmap job on a distribution with more than two blanks was created and never dispatched (derived-builds reviewer)

**Code updated.** MAGPIE aborts building a wordmap for more than two blanks
(`english_super`), so such a job's build failed three times and it never
dispatched, with nothing on its page to say why. Job creation now refuses a job
that needs a wordmap or a rack info table on such a distribution, naming the
distribution's field. **Verified:** `A-ADMIN-23` fails without the check (`201`)
and passes; the same job with no wordmap is created.

### 7.4 Medium — a leave result's rack was spelled before its length was checked (backend reviewer)

**Code updated.** 6.2's spelling copied every reported rack before plausibility
refused one that was too long: a 60 MiB "rack" cost 300 MB and 0.35 s in a
standalone reproduction, past PLAN's quarter gigabyte per large-result slot. A
rack is now spelled in place, and only when it could be one (at most 28 bytes,
no brackets); anything else reaches the refusal uncopied. **Verified:** the
unit test holds a 3,000-character string unchanged; `I-LEAVE-20` and M-4 still
pass.

### 7.5 Medium — PLAN, TESTING and 6.1 put scoring back on the password threads (docs reviewer)

**Docs updated.** PLAN's reset flow, `A-AUTH-3b` and 6.1 described the
arrangement 6.7 removed as a high; a reader matching the code to PLAN would
have put it back. They now say scoring has two turns of its own, and the reset
flow names the per-link bucket.

### 7.6 Medium — RUNBOOK §3 called a differing KLV hash legitimate (frontend reviewer)

**Docs updated.** §3 still said the results "have moved on since the generation
closed, so a rebuild legitimately produces different bytes" — the reasoning
6.6 removed from the admin page: a closed generation's rows do not change. It
now says a difference means the object or the rows were damaged or replaced,
and to find out which before forcing.

### 7.7 Low findings

**Fixed:**
- Derived builds:
  - the list returns each row's input ids, the page keys rows by them, and
    Retry resets the one row it names (`A-ADMIN-24`), not every failed row of
    a name, other builders' included;
  - the builder checks an input's bytes against the hash imported before
    building from them;
  - S3 errors carry their causes (they read "unhandled error");
  - a claim reports the build target that built the hash, not the web
    process's;
  - `derived_builder_memory` refuses less than 4 GB;
  - the page's remedy text, README and compose ("up to eight files a run"),
    KL-57 (a failed build is not bounded by the build), and PLAN's claim that
    an unpinned table means "no table" (MAGPIE declines it).
- Accounts:
  - the reset link's bucket is charged only once a scoring turn is held, so a
    request turned away busy does not spend the owner's tries;
  - the reset page says when a link is out of tries and offers a new one;
  - the register form's field errors are announced;
  - joiners and variation selectors are allowed where scripts and emoji use
    them (between or after non-ASCII characters), refused beside Latin
    letters.
- Merges: one with nothing staged runs no pass and counts nothing; passes are
  sized from the generation's summary.
- Docs:
  - merge memory figures (650 MB for a whole generation's hash table; 74–90 MB
    a slice, 130 MB for the backend);
  - the rate-limit table and `ratelimit.rs`'s comments;
  - KL-37 ("all four"), KL-81;
  - the "one statement" wording in PLAN and `plausibility.rs`;
  - TESTING's `A-AUTH-4e`/`4f`;
  - 5.2's pointer to 6.3;
  - PLAN's `ci.yml` tree comment.

### 7.8 Medium — on the default database a merge took eleven minutes (infra reviewer)

**Code updated.** Each pass summed its slice in a hash table and fed the sums to
the update in hash order, so the primary key was probed at random. With the
generation in memory that costs nothing, which is where 6.5's figures were
measured; on 1 GiB — the Terraform's `db.t4g.micro`, reproduced in a container
of that size with three full generations — a merge took 11 minutes (658 s; the
unsorted single statement before pass 6, 522 s). Each slice's sums are now
sorted by rack, so the update walks the key: 132–145 s on the same machine.
And at most two merges run at once across every job, since each pass's
backend holds about 130 MB and nothing bounded merges across jobs. PLAN's
"What a merge costs" gives both figures and asks for one measurement on the
real instance. **Verified:** the reviewer's timings with the committed SQL
and with the sorted variant (plan: a 34 MB quicksort over the hash
aggregate); the leave suite and M-4 pass.

**Lows fixed:** every CI job has a timeout; `hash_mem_multiplier` is named as
what keeps a slice in memory. **Checked:** `RESET` inside the transaction
leaves the pooled connection at its defaults; zxcvbn's optimization affects
tests only (the image is a release build); SES's policy allows the charset.

### 7.9 Adversarial check of the pass's fixes

**3 medium, fixed and verified.**

- **Medium — the hash check's remedy did not repair a damaged object.** 7.7's
  check refuses an input whose object holds other bytes than those imported,
  and told the admin to import again; an import skips an object that exists,
  so after re-import and Retry the build failed the same way. **Fix:** the
  build deletes an object under its own content address whose bytes are
  wrong, and says so; the next import uploads it again. **Verified:**
  `a_damaged_lexicon_object_is_replaced_by_the_next_import` (`I-INPUT-8b`)
  fails without the deletion (the object survives the re-import) and passes.
- **Medium — usernames could again be invisible twins.** 7.7 allowed joiners
  and selectors beside any non-ASCII character, and so beside `ë`, `é` or a
  Cyrillic letter (`zoë` and `zoë` plus a selector both registered); the
  unassigned default-ignorable ranges were missing from the list. **Fix:** a
  joiner stands only between letters of a script written with joiners
  (Arabic, Syriac, NKo, Mongolian, Brahmic) or inside an emoji sequence; a
  variation selector only after a pictograph, an ideograph or on a keycap; an
  ideographic selector only after an ideograph; and U+2065, U+FFF0–FFF8 and
  the unassigned tags are refused. **Verified:** the unit test covers the
  reviewer's twins and the names that must pass (Persian, Devanagari, Sinhala
  `ශ්‍රී`, `🏳️‍🌈`, `❤️‍🔥`, `👁️‍🗨️`, a keycap, an ideograph's variant).
- **Medium — merges waiting on one job held both merge turns.** 7.8's turn was
  taken before the job's lock, so two callers waiting on job X's running
  merge — an export's settle, the public stream of a completed job, an
  admin's merge, X's transition — held both turns, and every other job's
  merges gave up or waited behind them. **Fix:** the job's lock first, then
  the turn. **Verified:** `merges_waiting_on_one_job_leave_other_jobs_free_to_merge`
  (`I-LEAVE-22`) fails with the old order (Y's merge gave up) and passes.

**Lows fixed:** the reset page's out-of-tries message no longer promises an
hour (it may be the address's limit, and the link refills a try every 12
minutes); Retry's audit row names the row it reset; KL-57's worst case is
three attempts of up to 70 minutes. **Held:** a build stopped at its deadline
kills its MAGPIE and leaves an empty scratch directory (a 4-second deadline, a
fake MAGPIE); the lease exceeds the deadline on every path; nothing else reads
`leased_until` on a pending row; tier 6's M-10 built a wordmap and a rack info
table through the new hash check, a real import's objects matching their
recorded digests. **Recorded (KL-78):** an admin's "merge now" or an export's
settle may wait for other jobs' merges as well as its own. **Unconfirmed,
left:** a wordmap for Polish (33 letters, past a 32-letter bit rack) — its
build could not be run in the memory available; the blank is taken as `?`
where MAGPIE takes row 0 (every shipped file puts `?` first). The derived-data
page does not show when a waiting row will next be tried.

### 7.10 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **582 of 582**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 117 of 117.
- Tier 6 natively: M-4 and M-10 pass (a real leave merge; a real wordmap and
  rack info table built through the input hash check).
- **Tier 5, natively: 11 of 11** on 92544dc (frontend reviewer).
- The stalled-S3 replay against `build-derived` (300 s, then `pending`).
- `terraform fmt -check -recursive` and `validate`: clean.
- MAGPIE unchanged.


---

## Pass 8 — follow-up pass

**Plan.** The diff since the previous pass's base (`92544dc..9b28b9c`; MAGPIE
unchanged), one reviewer per part it touches: backend; frontend (tier 5
natively); docs and procedures; infra and CI. Plus one area not examined in
recent passes: **the task lifecycle for games, game pairs and opening
racks** — generation, claims, leases, heartbeats, reclaims, declines,
submissions, redundancy and finishing, and MAGPIE's `contribute` side.

**Findings: 0 high, 5 medium** from the five reviewers (backend none, 4 low, 2
unconfirmed; frontend none, 5 low — tier 5 natively 11 of 11; docs and
procedures 1 medium, 9 low; infra and CI 1 medium, 3 low; task lifecycle 3
medium, 7 low, 2 unconfirmed). All fixed and verified; the adversarial check
(8.7) found 2 medium in the fixes, fixed.

### 8.1 Medium — the builder's role could not delete a damaged input, so 7.9's repair never happened in production (infra reviewer)

**Code updated.** 7.9 had the build delete a damaged `inputs/` object so a
re-import would replace it; the builder's IAM role granted only `GetObject`
and `ListBucket`, the failed delete was swallowed, and an admin following
RUNBOOK re-imported (skipped: the object existed) and retried into the same
failure. Reproduced with a MinIO user holding the Terraform's policy, and with
the committed test run under credentials denied `DeleteObject`. **Fix:** the
role may delete `inputs/*` (and only that); a failed delete is logged and the
row's error says why and names the manual `aws s3 rm` RUNBOOK §2.4 now gives;
the stale "read-only" and "never deletes" comments are rewritten.
**Verified:** `terraform validate`; `I-INPUT-8b` still passes; the policy is
the only change the reproduction lacked.

### 8.2 Medium — one task that failed everywhere was every claim of its job (task-lifecycle reviewer)

**Code updated.** A declined task goes back to `available` and, the oldest,
was handed straight back to whoever claimed next — the worker that had just
failed it included — ahead of new work; MAGPIE stops after five failures in a
row, so one such task stopped every contributor claiming from its job
(reproduced on the server, six claims in a row the same seed, and with a real
MAGPIE against a stub). **Fix:** a worker is not offered a task it declined
within the hour (`registry::next_available`, keyed on the declined claim's
last heartbeat; no schema change). Another worker still is; I-SCHED-15's
index trap still holds an hour on. **Verified:** `A-WORKER-19` fails on the
old code and passes; KL-2 rewritten.

### 8.3 Medium — RUNBOOK said contributors pick up by themselves after a point-in-time restore (task-lifecycle reviewer)

**Docs updated.** A contributor whose UUID, key or account was made after the
restore point meets a `401` on its next claim, which ends its run (reproduced
with a real MAGPIE against a stub). RUNBOOK §1 now says so, with each kind of
contributor's remedy and how to count them on the old instance.

### 8.4 Medium — positions from a job that does not capture them were stored (task-lifecycle reviewer)

**Code updated.** PLAN lists "positions present only when the job set
`capture_positions`" as server-side validation; nothing checked it, and the
job's export then carried a positions file nobody asked for. Such a result is
now refused, and so is one with two positions for one turn of one game (the
row kept the first's position and the last's moves). **Verified:**
`A-WORKER-20` fails on the old code (`accepted: true`) and passes; a unit test
covers the duplicate.

### 8.5 Medium — Retry with the body PLAN documents reset other builders' rows (docs reviewer)

**Code updated.** 7.7 made Retry precise only when the caller sent the row's
builder and ids; PLAN documented `{ role, name }`, which still reset every
failed row of that name. Retry now requires the row whole (`klv_id` null for a
wordmap) and answers `400` otherwise; PLAN documents the full body.
**Verified:** `A-ADMIN-24` asserts `400` for `{ role, name }` and for a partial
set.

### 8.6 Low findings

**Fixed:**
- Accounts:
  - a name differing from a taken one only in joiners or variation selectors
    is taken (`A-AUTH-4g`; it fails on the old check), since a joiner between
    letters that join anyway, or a selector on an emoji, changes nothing a
    reader sees;
  - a Malayalam chillu written with a trailing joiner is allowed;
  - S3 error causes are logged, not sent in response bodies (they carried
    request ids and, on a DNS failure, the bucket's host).
- Derived builds:
  - the derived-data page shows each row's files and tarball dates, and
    offers no Retry on a row no builder of this version takes;
  - `derived_builder_cpu` no longer accepts 256, which no permitted memory
    pairs with;
  - I-DERIVED-7 asserts the 15-minute wait too;
  - RUNBOOK says a row between attempts retries on its own;
  - README says a run straight after a failure builds nothing.
- The reset page says how many minutes to wait (`ApiError` carries
  `Retry-After`; F-API-3).
- The job form's three-blank refusal no longer repeats itself.
- PLAN:
  - KL-40 (invisible characters now refused);
  - the reset flow's order (turn, then the link's bucket);
  - KL-57 and KL-78's waits;
  - the builder "drains" wording;
  - the three-blank refusal;
  - the claim loop (reclaim once; `JobFinished` is leave-only;
    `NeedsLeaveMerge` commits);
  - where the batch-size check runs;
  - which declines add a job to the unsupported set.
- KL-78: SPRT on a run with no variance.

**Recorded:** KL-2 (a claim has no maximum age: an executor that hangs while
heartbeating holds its task; reasoned). **Unconfirmed, left:** a correct input
object deleted by a slow builder that read the damaged one before a
re-import (bounded: the next build fails and a re-import repairs it; a
conditional delete would close it); merge waiters holding pool connections
(about 15 leave jobs transitioning at once to matter); several processes on
one key starving a long task's heartbeats. **Left:** `divergent_games`
unchecked against the pentanomial buckets (diagnostic only); MAGPIE's comment
above `config_contribute_ensure_rack_info_table` (MAGPIE's side).

### 8.7 Adversarial check of the pass's fixes

**2 medium, fixed and verified.**

- **Medium — the decline skip forced a leave task's racks twice.** 8.2's
  skip also applied to leave generation, where a declined task sits
  `available` with no claim: the decliner's claim went to rack selection,
  which does not count an available task's racks as out, so the same racks
  went out again on a second claim, and in the tail every decline made another
  task of them for the same worker — the failure 8.2 set out to end. **Fix:**
  leave generation keeps the old rule, a declined task reissued as it stands.
  **Verified:** `a_declined_leave_task_is_reissued_as_it_stands`
  (`I-LEAVE-23`) fails with the skip ("racks forced by two open claims") and
  passes; KL-2 and `A-WORKER-19` say which jobs skip.
- **Medium — an expired twin held a username for good.** 8.6's twin check
  refused a name differing only in joiners or selectors, but the release of
  an expired, unconfirmed account matched exact names only, so such a twin
  was never released (and a phone keyboard's U+FE0F after an emoji makes one
  by accident). **Fix:** the release matches as the check does.
  **Verified:** `A-AUTH-4g` registers an expired twin and then the name; it
  fails with the old release (`409`) and passes.

**Lows fixed:** the derived build's stored error keeps the S3 cause (a
builder-only fetch; responses keep the plain message); RUNBOOK §1's account
count uses `users.created_at`, which exists; the authz table sends Retry's
full body; `derived.tf`'s unreachable 256-CPU arm and message are gone and
its comment states the delete's real scope. **Recorded (KL-40):** the twin
check scans every name (274 ms at 200,000) and is check-then-insert; it
merges Persian names a non-joiner visibly separates. **Held:** an honest
MAGPIE never reports two positions for one turn, and sends positions only
when asked; the Retry page sends the full body. **Unconfirmed, recorded
(KL-2):** a small fleet may wait an hour on a job's last task after a
transient failure; a worker could skip outcomes it dislikes by declining
(outside the threat model: it could forge them anyway).

### 8.8 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **587 of 587**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 118 of 118.
- `terraform fmt -check -recursive` and `validate`: clean.
- **Tier 5, natively: 11 of 11** on 9b28b9c (frontend reviewer).
- Real MAGPIE against stubs: the poison-task stop and the post-restore `401`
  (task-lifecycle reviewer).
- MAGPIE unchanged.


---

## Pass 9 — follow-up pass

**Plan.** The diff since the previous pass's base (`9b28b9c..3f05fce`; MAGPIE
unchanged), one reviewer per part it touches: backend; frontend (tier 5
natively); docs and procedures; infra. Plus one area not examined in recent
passes: **admin operations** — activating, deactivating, completing, purging
and deleting jobs, allocation, player configs, bans, the fleet and workers
pages, account deletion by an admin, the audit log, CSRF and admin
authentication.

**Findings: 0 high, 3 medium** from the five reviewers (backend none, 6 low, 1
unconfirmed; frontend none, 4 low — tier 5 natively 11 of 11; docs and
procedures 1 medium, 8 low; infra none, 2 low; admin operations 2 medium, 10
low, 1 unconfirmed). All fixed and verified; the adversarial check (9.5)
found 2 medium in the fixes, fixed.

### 9.1 Medium — PLAN said a `derived_mismatch` decline counts toward MAGPIE's five-failure stop (docs reviewer)

**Docs updated.** 8.6's rewrite of the decline paragraph said a KLV mismatch
counts as a failure, five in a row ending the run; MAGPIE never counts it (a
real MAGPIE against a stub declined `derived_mismatch` eight times, setting
the job aside for 1, 2, …, 128 s, and never stopped). The paragraph now says
which declines set a job aside, which count, and that leave generation
reissues a declined task.

### 9.2 Medium — KL-56 said a ban stops an identity-less client; none can (admin reviewer)

**Docs updated.** A client that sends no identity mints a new one with every
claim, so banning any of them changes nothing (reproduced: five of five
claims after the ban got tasks), and a banned account's owner can claim with
no key. KL-56, PLAN's ban list and the workers page now say so; the lever
that would work, a cap on open claims per address, stays an option in KL-56.

### 9.3 Medium — a forced artifact rebuild that stopped part-way left no audit row (admin reviewer)

**Code updated.** The rebuild rewrites one generation at a time and logged
only at the end, so one stopped by an S3 error, a failed build or the load
balancer's timeout (KL-19) had replaced objects workers played with no record.
An export also started before its row, and Retry reset a build before its
row, breaking PLAN's same-transaction promise. **Fix:** a rebuild logs
`job.artifacts_rebuild_started` (with `force`) before anything, and its
counts row at the end; an export is logged in the transaction that records
it (9.5); Retry's reset and row share a transaction; PLAN names the rebuild
as the exception. **Verified:**
`a_forced_rebuild_is_logged_before_it_rewrites_anything` (`A-ADMIN-11b`, tier
6) fails without the early row and passes.

### 9.4 Low findings

**Fixed:**
- Derived builds:
  - the builder's delete of a damaged input and an import's or export's S3
    failure store the SDK's whole cause again (they are read only by an
    admin); only the object fetch a worker's request makes keeps the plain
    message;
  - an input whose key is not its content address is told the right remedy;
  - Retry refuses (`404`) a row no builder of this version takes;
  - the derived-data page's banners count only such rows, and a later
    success clears an earlier error.
- `Retry-After` is rounded up: a client that waited the seconds it was told
  was refused again.
- The registration twin check keeps its plan stable (its argument no longer
  recomputed per row once Postgres switches to a generic plan).
- Docs:
  - PLAN's validation-placement text;
  - the reset flow;
  - the Retry row's 400/404;
  - PLAN's 401/403 for admin routes and the input-data refusal's wording;
  - README's Retry button;
  - RUNBOOK's retry wait (about 20 minutes), when to count identities after
    a restore, the census claim and §0's query;
  - the account-deletion dialog (a ban discards nothing);
  - `derived.tf`'s 90-day note.

**Recorded:** KL-40 (the chillu look-alike and the Malayalam and Persian
merges; the twin check's cost); KL-78 (the admin audit log's remaining gaps;
old allocations shown beside inactive jobs). **Unconfirmed, left:** a decline
loop generating tasks up to a job's cap (a hostile account); activating a job
during a forced rebuild.

### 9.5 Adversarial check of the pass's fixes

**2 medium, fixed and verified.**

- **Medium — refused exports were logged as started.** 9.3 moved the export's
  audit row ahead of `exports::start`, which refuses routinely (the job not
  completed, claims still out, one already running): each refusal left a
  `job.export_started` row for an export that never existed (reproduced:
  three refusals, three rows, no export). The export's start is one insert in
  a transaction of its own. **Fix:** the row is written in that transaction.
  **Verified:** `A-ADMIN-15b` (a refusal logs nothing, a begun export one row)
  fails with the pre-start row and passes.
- **Medium — RUNBOOK's advice on when to count identities counted the wrong
  database.** It said to count "before step 5 repoints the service"; §1 has
  no step 5, and the rename, which comes before the repoint, moves the
  endpoint, so the service's `DATABASE_URL` then reaches the restored instance
  — the one lacking exactly the identities counted. **Fix:** count before the
  rename, with `scripts/prod-sql.sh`; the comment that cited step 5 is
  reworded.

**Lows fixed:** presigning keeps the plain message (a public download's
failure reached an anonymous caller with the credential provider chain);
PLAN's admin preamble (worker credentials without a session get `401`); the
decline paragraph notes a task handed back on stopping counts as nothing.
**Held:** the rebuild's started row follows every refusal; Retry's builder
check agrees with `buildable`; `Retry-After`'s rounding never exceeds the
period rounded up, and a client waiting exactly that long passes; the twin
check's subquery holds its plan (about 210–250 ms at 200,000 users against
370); RUNBOOK §0's query runs against the schema.

### 9.6 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **588 of 588**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 118 of 118.
- `terraform fmt -check -recursive` and `validate`: clean (infra reviewer).
- **Tier 5, natively: 11 of 11** on 3f05fce (frontend reviewer).
- The builder policy modelled in MinIO: `inputs/` deletable, `exports/`,
  `leaves/` and `derived/` not (infra reviewer).
- MAGPIE unchanged.

