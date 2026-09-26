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
plus KL-74 carried. All fixed and verified; the adversarial checks are 1.6.

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

**The versions the adversarial checks broke** (1.6 has the detail):
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

**Verified:** 30 unit tests (`U-STATS-5`, each adversarial case among them)
fail against the version they broke and pass; the analytic `U-STATS-4` errors
still hold; the ratings and public API integration tests pass.

### 1.6 Adversarial checks of the pass's fixes

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

### 1.7 Tests

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
