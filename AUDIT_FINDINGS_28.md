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
- **Pass 10 (follow-up: pass 9's diff, and the public read API):** 0 high and 2
  medium from the reviewers — a job page waiting about fifty seconds on a
  saturated display pool; an account deletion's census read as counting what
  is lost — both fixed and verified; the adversarial check found 1 medium
  (PLAN's backup section still calling account deletion destructive), fixed.
  The loop continues.
- **Pass 11 (follow-up: pass 10's diff, and input-data import):** 1 high and 5
  medium from the reviewers — symlink aliases copying their targets past every
  archive cap; any fork's commit importable under the upstream's name; viewers
  queued on a failed stats build waiting in turn; RUNBOOK's guard closing the
  operator's shell; two PLAN passages wrong — all fixed and verified; the
  adversarial check found 1 medium (the ref check still passing fork commits),
  fixed by resolving refs only among the repository's own branches and tags.
  KL-70 updated. The loop continues.
- **Pass 12 (follow-up: pass 11's diff, and startup, configuration and the
  background loops):** 0 high and 2 medium from the reviewers — a heartbeat
  timeout below MAGPIE's cadence accepted, lapsing live claims; PLAN's
  `MAIL_BACKEND` row — both fixed and verified; the checks of the fixes found
  2 high (E-3 broken by a label rename; a RUNBOOK §1 command that no longer
  parsed) and 3 medium (the rename block not refusing without `STAMP`; a
  job's creator widening its page; the alias docs), all fixed —
  `scripts/runbook-check.sh` now parses RUNBOOK's blocks in CI. KL-70 updated,
  KL-82 added. The loop continues.
- **Pass 13 (follow-up: pass 12's diff, and request authentication and rate
  limiting):** 0 high and 6 medium from the reviewers — the RUNBOOK check
  reading 16 of 25 blocks; the heartbeat floor's stated reason; MAGPIE sending
  credentials in the clear to an `http://` server, followed through the ALB's
  redirect (the ALB now refuses `/api/*` on port 80); PLAN's worker rate limit;
  PLAN's alias rule; the AWS CLI pager swallowing a pasted `wait` — all fixed
  or, where MAGPIE must change, bounded and recorded; the adversarial check
  found 4 medium (fence variants the check skipped — it now parses fences;
  §6's drill broken by §5's new line; the rest of the blocks that call `aws`
  without the pager
  off, now enforced; the floor's reason again), all fixed. KL-76, 81 and 82
  updated; KL-83 and KL-84 added. The loop continues.
- **Pass 14 (follow-up: pass 13's diff, and MAGPIE's contribute client):** 0
  high and 5 medium from the reviewers — a server-assigned UUID written into
  `contribute.txt` as it came (a newline added settings every later run
  obeyed); PLAN promising size bounds the worker does not keep; a settings
  file MAGPIE could not write losing the identity silently; README's password
  block with the pager on; the pager check not checking "first" — all fixed,
  the MAGPIE ones in `5753e212` on `birdtest-contribute`, now pinned; the
  adversarial check found 2 medium (fences on nested list lines; README's ACM
  block hiding its CNAME), both fixed. KL-68 and KL-84 updated; KL-85 added.
  The loop continues.
- **Pass 15 (follow-up: pass 14's diff, and the build and CI):** 0 high and 5
  medium from the reviewers — a UUID save cut short by a full disk locking the
  worker out; README's ACM wait giving up after five minutes; the RUNBOOK
  check passing an unterminated heredoc and a trailing backslash; a new
  migration alone not rebuilding the backend; the image's shared cargo cache
  able to ship another checkout's binary — all fixed (MAGPIE `8ba24b7b`,
  pinned; `backend/build.rs`); the adversarial check found 2 medium (a
  closing fence less indented than its opener; zsh without
  `interactive_comments`), both fixed. KL-64 updated. The loop continues.
- **Pass 16 (follow-up: pass 15's diff, and the admin frontend):** 0 high and
  2 medium from the reviewers — the admin job page showing defaults as the
  server's state after a failed first read, and MAGPIE printing an API key
  written on the wrong line — both fixed (a new journey, `E-11`; MAGPIE
  `3565279b`, pinned); the adversarial check found 4 medium (a retry
  replacing a typed allocation; a read landing after an action; the checker
  hanging on a line of backslashes; PLAN's output promise), all fixed. KL-64
  and KL-68 updated; KL-86 added. The loop continues.
- **Pass 17 (follow-up: pass 16's diff, and the infrastructure as a whole):**
  0 high and 4 medium from the reviewers — the database password and session
  signing key decrypted into Terraform's state (now referenced, not managed,
  and moved out of an existing state by `removed` blocks); a typed allocation
  lost to any action and a slow read landing over a live payload (the admin
  job page's state redesigned, with `E-11b`); a MAGPIE message still quoting a
  value (MAGPIE `e98a5244`) — all fixed; the adversarial check found 1 medium
  (a commented-out key refused; MAGPIE `f3fc1927`, pinned), fixed. KL-64,
  KL-65, KL-68 and KL-86 updated. The loop continues.
- **Pass 18 (follow-up: pass 17's diff, and the job statistics):** 1 high and 3
  medium from the reviewers — a `games` job at the default batch of 1 gave
  player 1 every first move, so SPRT passed identical players (a games batch
  is now even, default 2); MAGPIE's comment rule refusing ordinary comments
  and missing the key it was for (redesigned: warn, never refuse; MAGPIE
  `65ad3348`); a job deleted while its admin page was open keeping its
  actions (`E-11c`) — all fixed; the adversarial check found 1 medium
  (MAGPIE's test not compiling in the dev build CI uses; MAGPIE `8f2f5d75`,
  pinned), fixed. KL-86 updated; KL-87 added. The loop continues.
- **Pass 19 (follow-up: pass 18's diff, and task dispatch and allocation):** 0
  high and 2 medium from the reviewers — a newcomer joining level with a job
  that had stopped moving and taking every claim; allocation being a share of
  claims where PLAN said the fleet (stated, KL-88) — both addressed; the
  adversarial check found 1 medium (joining at the leader starved a newcomer
  in a split fleet), fixed by redesign: every claim bounds every job's lag to
  a window, so neither join point can take over or starve (`I-SCHED-3c` to
  `3f`). KL-87 updated and KL-88 added. The loop continues.
- **Pass 20 (follow-up: pass 19's diff, and exports and the results stream):**
  2 high and 1 medium from the reviewers — pass 19's lag bound scrambled jobs
  that lagged together (978 : 22) and made a concurrent burst to a small job
  permanent (83 where 30 is fair); a results stream cut off by the database
  read as a complete download — all fixed (the bound removed; a failed stream
  ends in an error, and what can fail before its first row is a status). The
  two adversarial checks found 3 high and 2 medium in what replaced the bound,
  each fixed in the pass: a job now joins at the lowest ratio served and
  settles, for an hour, against each class of workers that claims it; a
  decline that says a worker cannot run a job undoes that; a job with nothing
  to hand out is lifted as it is passed over; and each claim is checked for
  its turn under the job's dispatch lock, so concurrency makes no bursts
  (`I-SCHED-3c` to `3q`, `I-EXPORT-10`, `11`). KL-89 added. The loop
  continues.
- **Pass 21 (follow-up: pass 20's diff, and result validation):** 0 high and 3
  medium from the reviewers — equal jobs claimed together left workers idle
  while work existed; a busy job cost every claim 16 s and a `204`; through
  the compose Nginx a cut results stream still read as complete — all fixed
  (`I-SCHED-3r`, `3s`, `F-NGINX-1`). Result validation found no way past it
  and nine lows, fixed: a capturing job's result must cover every game, and no
  result may hold a NUL or an unbounded play, tile or position (`A-WORKER-21`).
  The adversarial check found 2 medium (a busy large job handing a small one
  its claims, `I-SCHED-3t`; the batch cap breaking the fixture capture), fixed.
  KL-20 closed; KL-89 updated. The loop continues.
- **Pass 22 (follow-up: pass 21's diff, and the audit log and retention):** 0
  high and 3 medium from the reviewers — credential changes (keys, a reset, a
  confirmation) left no audit row; a full restore silently undid security
  actions, with no RUNBOOK step to re-apply them; repeated busy spells let a
  job's settling forgive leads — all fixed (`A-ACCOUNT-8`, `A-AUTH-12`, a new
  RUNBOOK §1 step replayed, `I-SCHED-3u`). The adversarial check found 3
  medium (that step copied the damage back; an unsettled busy newcomer took a
  split fleet over; the key toggle grew the log at request rate), fixed — the
  step redesigned around reviewed audit rows (`I-SCHED-3v`). KL-89 updated;
  KL-90 added. The loop continues.

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


---

## Pass 10 — follow-up pass

**Plan.** The diff since the previous pass's base (`3f05fce..b2d9962`; MAGPIE
unchanged), one reviewer per part it touches: backend; frontend (tier 5
natively); docs and procedures; infra. Plus one area not examined in this
run: **the public read API and pages** — `/api/jobs`, a job's stats, users,
workers and `?worker=`, the results feed and its cursors, download redirects,
the read pool and the stats cache, and the pages built on them.

**Findings: 0 high, 2 medium** from the five reviewers (backend none, 5 low;
frontend none, 3 low — tier 5 natively 11 of 11; docs and procedures 1
medium, 8 low; infra none, 3 low; public read API 1 medium, 5 low, 1
unconfirmed). All fixed and verified; the adversarial check (10.4) found 1
medium in the fixes, fixed.

### 10.1 Medium — on a saturated display pool a job page waited about fifty seconds, not a quick `503` (public read API reviewer)

**Code updated.** PLAN promises that the display pool's short acquire timeout
turns a saturated pool into a quick `503`. A stats build took a connection
per statement — about eight — and on a saturated pool each could wait most of
the timeout and then succeed, so a build took twenty seconds; and a request
that arrived during a slow build refused it for having started before it
asked, so a burst of viewers built twice (reproduced on a seeded database with
the pool held by `?worker=` probes: 24–52 s per job page). **Fix:** a build
runs on one connection, so it waits for the pool once; and a waiting request
takes any build that was no older than `max_age` when it asked. A job page
still waits for the pool twice at most — the route's own read of the job, and
the build. **Verified:** `a_stats_build_takes_one_connection` (`I-STATS-10`)
counts the pool's acquires: 8 on the old code, 1 now; and on the seeded
database under load (10.4), twenty viewers were answered by one shared build
in about fifteen seconds, where the old code took sixty-four and answered six
with `503`.

### 10.2 Medium — RUNBOOK and PLAN said an account deletion's census counts what is lost (docs reviewer)

**Code and docs updated.** Deleting an account anonymizes it and keeps its
claims and results, but its census read `claims=… accepted=… api_keys=…`
under a sentence saying `reason` holds "the row counts that were about to be
lost" — an operator would scope a restore around claims that were all still
there (reproduced: a deleted account's claim survives its census). The census
now reads `destroyed: api_keys=… confirmations=… reset_tokens=…; kept:
claims=… accepted=…`; RUNBOOK §0, PLAN and the code say which goes.

### 10.3 Low findings

**Fixed:**
- Errors and their causes:
  - S3 service errors stored for an admin carry the service error, not the
    raw response it came in (about 2 KB of headers and body);
  - a forced rebuild's object read and presigning carry the whole cause (both
    are admin-only, the reason 9.5 gave for presign was wrong);
  - a hand-edited object key is told to set the key back, not to delete the
    row and everything that pins it.
- Registration: the expired-twin release uses the twin check's scalar
  subquery.
- Admin pages:
  - the account-deletion dialog and the users page no longer promise that a
    ban stops work, and name job deletion as discarding results too;
  - a failed Retry reloads the list and says the row may have been retried
    already.
- Public:
  - `/api/users` counts first and answers a page past the end without its
    query;
  - a worker's label shows its whole sixteen-character pseudonym (what
    `?worker=` takes; eight characters collided at 300,000 contributors), and
    the e2e patterns follow;
  - a completed leave job's page no longer says "live".
- RUNBOOK §1:
  - the identity count is a step in the procedure before the rename;
  - the restore time is set once;
  - how to count after the rename is given.
- PLAN and TESTING:
  - the audit table lists `input_data.deleted` and `player_config.deleted`;
  - admin routes answer a worker credential with `401`;
  - the validation-placement paragraph;
  - a job deletion's census gaps;
  - the stale `task_claims`-has-no-`job_id` note;
  - TESTING's "largest tier" and A-ADMIN-15b (a second export is tested).

**Left:** the home page lists only the first fifty active jobs, 0%-allocation
ones included (KL-78 already covers old allocations beside status); the job
page's REST answer can briefly overwrite a newer pushed payload (reasoned,
not reproduced).

### 10.4 Adversarial check of the pass's fixes

**1 medium, fixed.**

- **Medium — PLAN still said account deletion destroys work, and promised an
  account restore no procedure covers.** 10.2 corrected the audit section and
  RUNBOOK §0 but left PLAN's backup section: "`delete_job` / `delete_user` are
  similarly total", the census "of what they are about to destroy", and a
  restore row for a mistakenly deleted *user* — while RUNBOOK §2 restores jobs
  only, and §0 now said a restore "recovers" an account's name and password
  (reproduced: a deleted account's claim and result survive, its census says
  so). **Fix:** PLAN says `delete_user` anonymizes and keeps the work; the
  restore table gives a deleted account its own row — none documented, the
  owner registers again — and RUNBOOK §0 says so.

**Lows fixed:** the census is taken after the account is locked (a key made in
between was destroyed uncounted); a stored S3 service error is its code and
message (`AccessDenied: …`, not 800 characters of repeated metadata); RUNBOOK
§1 counts in one ops task, and `RESTORE_TIME` must be set rather than
defaulting to the example date; a paused leave job's page no longer says
"live"; 10.1 and `I-STATS-10` say a job page waits for the pool twice at most.
**Held:** under the seeded load, twenty viewers of one job were answered by one
shared build (about fifteen seconds, the database's CPU the rest) where the old
code took sixty-four and answered six with `503`; no stale payload could be
served (`forget` and the kept-entry rule unchanged); tier 5 natively on the
patched tree, 11 of 11. **Recorded (trade-off):** at a one-second stats cache,
holding one connection per build slows other display-pool routes under heavy
job-page load while serving about twice as many pages; at the default ten
seconds the two are the same. **Unconfirmed, left:** waiters on a build that
fails its acquire each try again in turn.

### 10.5 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **589 of 589**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 119 of 119.
- **Tier 5, natively: 11 of 11** on b2d9962 (frontend reviewer) and on the
  patched tree with the E-1/E-5 patterns at sixteen characters (adversarial
  check).
- `terraform fmt -check -recursive` and `validate`: clean (infra reviewer).
- RUNBOOK §1's count, run against a migrated schema with a stub `prod-sql.sh`.
- MAGPIE unchanged.


---

## Pass 11 — follow-up pass

**Plan.** The diff since the previous pass's base (`b2d9962..40380f9`; MAGPIE
and infra unchanged), one reviewer per part it touches: backend; frontend
(tier 5 natively); docs and procedures. Plus one area not examined in this
run: **input-data import** — fetching a MAGPIE-DATA release, walking and
staging it, confirming, uploading lexicon bytes, deleting an input row, the
import page.

**Findings: 1 high, 5 medium** from the four reviewers (backend 1 medium, 4
low; frontend none, 1 low — tier 5 natively 11 of 11; docs and procedures 3
medium, 6 low; input-data import 1 high, 2 medium, 6 low, 2 unconfirmed). One
medium was found by two reviewers. All fixed and verified; the adversarial
check (11.8) found 1 medium in the fixes, fixed by redesign.

### 11.1 High — a symlink alias copied its target's bytes, and no archive cap counted the copies (import reviewer)

**Code updated.** An alias at a pinned path is resolved to a file in the
archive and its row carries its own copy of the target's bytes; each alias was
a zero-byte entry, so the total, the ratio and the per-entry cap saw the target
once, and nothing bounded a distribution or layout below the 128 MiB entry cap.
150 aliases of a 4 MiB layout held 604 MiB from a 263 KiB gzip; 100 of a
2 MiB one staged and inserted 202 MiB of rows. **Fix:** an alias counts as its
target's bytes against the total and the ratio, and a file kept in its row
(a letter distribution, a layout) is at most 64 KiB. **Verified:**
`aliases_count_as_their_targets_bytes_and_kept_files_are_small`
(`U-ARCHIVE-5b`) fails without the caps (a 100 KiB layout accepted) and passes;
a handful of aliases, as MAGPIE-DATA ships, still imports.

### 11.2 Medium — any GitHub user's commit could be imported under the upstream's name (import reviewer)

**Code updated.** `resolve_ref` accepted a commit sha and `pull/N/head`, which
GitHub resolves from any fork and whose files it serves under the upstream's
raw URLs (shown on a public repository): anyone who can open a pull request
could author the tarball an admin imports — with 11.1, a hostile archive for
one pasted ref, not a compromised upstream as PLAN's threat model says.
**Fix (final, 11.8):** a ref is resolved only among the repository's own
branches and tags, through GitHub's refs endpoints, never `/commits/`; PLAN's
threat model says so. Resolving is bounded as a whole (20 s) and each answer
read to 64 KiB.

### 11.3 Medium — viewers queued on a failed stats build each waited out the timeout in turn (backend and docs reviewers)

**Code updated.** Pass 10 left it unconfirmed; both reviewers reproduced it:
a failed build keeps nothing, so each viewer waiting on the job's build lock
built again, the k-th answering after k acquire (or statement) timeouts — 5,
10, 15, 20 s on the display pool; 15 to 60 s behind a slow read. **Fix:** a
build that failed after a waiter asked answers it, as busy (`503`).
**Verified:** `viewers_waiting_on_a_failed_build_are_answered_together`
(`I-STATS-10b`): six viewers on a held one-connection pool took 6.0 s on the
old code and about one second now.

### 11.4 Medium — RUNBOOK's `RESTORE_TIME` guard closed the operator's shell (docs reviewer)

**Docs updated.** The guard's `return 1 2>/dev/null || exit 1` exits an
interactive shell, and the block pasted as written (the time not yet chosen)
always tripped it; in a nested shell the rest of the paste ran in the outer
one. The first block now ends at the `LatestRestorableTime` query, the next
sets `RESTORE_TIME`, and each use is `${RESTORE_TIME:?…}`, which refuses only
that command.

### 11.5 Medium — PLAN said the contributor list reads by `task_claims.job_id`; it joins `tasks` (docs reviewer)

**Docs updated.** 10.3 replaced one wrong sentence with another (no index on
`task_claims` leads with the job; KL-58 said so). Reworded to match.

### 11.6 Medium — PLAN's import section did not describe the walk and confirm the code does (import reviewer)

**Docs updated.** PLAN said "regular files only" (aliases are accepted and
pinned — the path 11.1 used) and that confirming inserts only new rows
(collisions are inserted too). Both now match, with the new limits in PLAN's
table.

### 11.7 Low findings

**Fixed:**
- Object store errors:
  - a service error with no code (a proxy's HTML, an empty `403`) keeps its
    HTTP status and the start of its body;
  - the builder's fetch and the existence check store the short form too;
  - the hand-edited-key remedy is scoped to the one version.
- Stats cache: the freshness test cannot overflow `Instant` (a huge
  configured age would have panicked under the cache's lock).
- `/workers` drops "Last result" on a phone, where a sixteen-character
  pseudonym pushed the ranking column out of its box; E-10 now visits it.
- Docs:
  - RUNBOOK's post-rename fallback (`--output text`, `--region`, a literal
    time) and "loop" wording;
  - PLAN's account row (public credit, admin flag, sessions) and census
    wording.

**Recorded (KL-70):** a `known` row deleted between staging and confirm; "or
later" dates across releases; an input row's object kept after its delete;
case-only duplicate paths. **Unconfirmed, left:** a directory symlink written
through at extraction; a status change with no later submission leaving an
open job page stale until reload.

### 11.8 Adversarial check of the pass's fixes

**1 medium, fixed by redesign.**

- **Medium — the ref check still let a fork's commit through.** 11.2 refused
  refs shaped like a sha (7 to 40 hex characters) or a pull request's; GitHub's
  `/commits/{ref}` also resolves five- and six-character abbreviations and
  `git describe` names (`x-0-g7044a8a…`) from any fork. Shown against GitHub
  and end to end through the patched backend (a fork's commit pinned, its
  tarball fetched from the upstream's raw URL); and a real upstream branch,
  `20260101`, was refused for looking like hex. Refusing by shape was the
  second patch to this check, so it is replaced rather than patched again.
  **Fix:** `resolve_ref` asks only the endpoints that list the repository's own
  refs — `/git/ref/heads/{ref}`, then `/git/ref/tags/{ref}`, an annotated tag
  peeled through `/git/tags/{sha}` — and anything else is "no branch or tag";
  the whole resolution runs under one 20-second deadline, each answer read to
  64 KiB (the "256 bytes" of 11.2 had buffered the whole body first). The
  fixture (`fixtures/github`, `build.sh`, `nginx.conf`) and tier 6's stand-in
  answer the refs endpoint. **Verified:** `A-ADMIN-5` now sends the forms the
  check found (a whole and a five-character sha, pull refs, two describe names)
  and asserts each is not found and `/commits/` never asked; `A-ADMIN-5b`
  resolves a hex-named branch, a lightweight and an annotated tag and refuses a
  tag naming a tree — both fail on the committed resolver and pass; tier 6's
  M-10 imports through the stand-in.

**Lows fixed:** a thirty-two-character username or a tombstone no longer
pushes `/workers`' ranking out of its box (the contributor cell breaks);
PLAN's account row says the Contributors ranking keeps the tombstone's count;
the walk-limits test asserts the 64 KiB cap; RUNBOOK's identity count is a
block of its own before the renames, so a refused count stops before them.
**Held:** alias chains, aliases of aliases and aliases read before their
targets each count; hard links are refused; the real current release (195 MB,
23 symlinks) walks; the largest shipped distribution is 496 bytes; a request
arriving after a failed build is not answered busy; the worker path of the
object fetch is unchanged. **Left:** a waiter on a build that failed for a
non-transient reason (the job deleted mid-build) is told busy rather than the
builder's error; its retry gets the true answer.

### 11.9 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **592 of 592**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 119 of 119.
- **Tier 5, natively: 11 of 11** on the final working tree — the seed and E-3
  importing through `git/ref/heads/main`, never `/commits/`; E-10 visiting
  `/workers`.
- Tier 6's M-10 natively, importing through the stand-in's refs endpoint.
- RUNBOOK §1's blocks parse; its count checked against a migrated schema.
- MAGPIE and infra unchanged.


## Pass 12 — follow-up pass

**Plan.** The diff since the previous pass's base (`40380f9..0cfe839`; MAGPIE
and infra unchanged), one reviewer per part it touches: backend; frontend
(tier 5 natively); docs, procedures and fixtures. Plus one area not examined in
this run: **startup, configuration and the background loops** — `config.rs`,
`main.rs`'s startup order and reapers, the five periodic loops, graceful
shutdown, the builder binary's start.

**Findings: 0 high, 2 medium** from the four reviewers (backend none, 5 low;
frontend none, 6 low — tier 5 natively 11 of 11; docs and procedures none, 6
low; startup and loops 2 medium, 9 low, 1 unconfirmed). Both fixed and
verified.

### 12.1 Medium — a heartbeat timeout under MAGPIE's cadence was accepted, and every claim then handed out the fleet's live tasks (startup reviewer)

`HEARTBEAT_TIMEOUT_SECONDS` took any whole number. MAGPIE heartbeats every
thirty seconds, so below about a minute a live worker's claim reads as lapsed;
at 0 the restart grace went too. Shown with a private tier-2 test that parses
the value through `Config::from_lookup` and builds the state as `main` does:
worker A claims, worker B claims — at 0 and at 20 s (A's last heartbeat 25 s
old) A's claim is `abandoned` and B is handed A's task, and A's results would
be answered `accepted: false`. Nothing refused the value, though `config.rs`
promises a wrong value fails startup. A value near `u64::MAX` panicked at
startup after the migrations (`overflow when adding duration to instant`).
**Fix:** the timeout must lie in 180 s to a day — MAGPIE sends one heartbeat
attempt every thirty seconds under a 120 s request timeout, so a live claim can
go 30 + 120 + 30 s between recorded heartbeats (the first cut, 90 s, lapsed a
claim whose heartbeat stalled; 12.4) — and the session TTL (0 issued sessions already expired; past what a date holds, every
sign-in panicked) in a minute to a year; out of range fails startup naming the
setting and the range. **Verified:** `a_malformed_value_is_refused_rather_than_defaulted`
gained 0, 179, 86,401 and `u64::MAX` for the timeout, 0 and a year plus one
for the TTL, and a host name for `BIND_ADDR` (12.4) — it fails on the committed `config.rs` (the first case is
accepted) and passes; the expired-session test sets its zero TTL directly.
PLAN's table states both ranges; TESTING's U-CFG-4 names them.

### 12.2 Medium — PLAN said any `MAIL_BACKEND` but `console` or `ses` fails startup; `file` starts (startup reviewer)

Reproduced: `MAIL_BACKEND=file MAIL_OUTBOX_DIR=…` starts and listens, as the
end-to-end suite needs. PLAN's table also left out `MAIL_OUTBOX_DIR`,
`GITHUB_API_URL` and `GITHUB_RAW_URL`, which the config reads. **Fix:** the
table documents `file` (the end-to-end suite's, never production) and the three
variables.

### 12.3 Low findings

**Fixed:**
- Import refs: an explicit `heads/`, `tags/`, `refs/heads/` or `refs/tags/`
  prefix is honoured (a tag that shares a branch's name can be chosen), and an
  annotated tag is peeled through up to four tag objects (a tag of a tag
  resolved); `A-ADMIN-5b` asserts both. The form's field is "Branch or tag",
  its hint says what is resolved, and the handler's doc no longer offers a
  commit sha.
- Aliases count against the archive caps only when they copy content
  (letterdist, layout); a `.kwg` or `.klv2` alias is a symlink on the worker
  and no second copy on the server.
- An object-store error with an empty body reads `HTTP 403`, not `HTTP 403: `.
- A second process on the same address now exits before the startup reapers:
  it had failed the running instance's import (shown with the committed
  binary: `failed|the server restarted while this import was running`, then
  `Address already in use`; with the fix the row stays `running`).
- The public Users list breaks a long username, as `/workers` does (a
  32-character name pushed "Tasks completed" out of the card, 484 px of 359);
  pagination reads "Page 1 of 1", not "Page 1of 1".
- E-10 asserts a contributor row before measuring (an empty table fits any
  page), then answers both rankings with a 32-character username, a tombstone
  and a pseudonym and measures `/workers` and `/users` — the `break-all` half
  of pass 11's fix had no test.
- Docs: the e2e compose comment names the refs endpoint; RUNBOOK §1 says a
  fresh shell needs `STAMP` as well as `RESTORE_TIME`, and refuses without it
  (an unset one made the rename loop wait forever on `birdtest-damaged-`;
  the form of the refusal is 12.4's); PLAN's KL-70 justification, the calls a
  resolution costs against the 60-per-hour limit, the startup order (the
  MAGPIE probe, the address, the five loops) and the 120 s stop timeout
  (PLAN and `state.rs` still said thirty seconds); E-10's title.

**Recorded (KL-82):** whether ECS starts a new task while the old one drains
(unconfirmed; the single-instance invariant rests on it), no panic guard on the
background loops, detached mail tasks at shutdown, the NDJSON stream not ended
by the shutdown signal, a `TRUSTED_PROXY_HOPS` too high keying every limit on
the ALB, the builder's missing version floor.

### 12.4 Adversarial check of the pass's fixes

Two checks: tier 5 run natively against the frontend fixes, failure first; and
an adversarial reviewer on the rest. **2 high and 3 medium, all fixed.**

- **High — the rename to "Branch or tag" broke E-3**, which found the field by
  its old label: tier 5 was 10 of 11. **Fix:** E-3 finds the new label.
- **High — a RUNBOOK §1 command no longer parsed.** The `${STAMP:?…}` message
  added to the backup-settings command held an apostrophe, which bash reads as
  an opening quote inside the expansion: `unexpected EOF while looking for
  matching `''`, in every bash, with `STAMP` set or not; pasted, it left a
  continuation prompt that swallowed what came next, so the restored instance
  never got its retention or deletion protection. **Fix:** no apostrophe; and
  `scripts/runbook-check.sh`, run in CI, parses every bash block in RUNBOOK
  (`bash -n`) — it fails on the broken block (`the bash block starting at line
  131 does not parse`) and passes on HEAD's and the fixed RUNBOOK (16 blocks).
- **Medium — with `STAMP` unset the rename block did not refuse.** In an
  interactive shell a failed `${STAMP:?}` stops one command: the renames were
  refused, and `terraform state rm` and `import` then ran on the damaged
  instance, and a failed import would leave no database in state. Shown by
  pasting the block through a pty into `bash -i` with logging stubs. **Fix:**
  the renames and the state surgery are one `if` with `&&` between the steps.
  Replayed with the same harness, bracketed paste on and off: `STAMP` unset,
  no call at all; set, every call in order; the first rename failing, nothing
  after it.
- **Medium — a job's "Created by" widened the job page** past a phone's
  screen with a 32-character name of wide letters (421 px on a 393 px
  screen). **Fix:** the name breaks; the job page's contributor table breaks a
  long name too, so its count stays in view; `/users` drops "Joined" below
  `sm` (at 320 px the count left the box whatever the name) and keeps the
  admin badge whole. E-10 loads the job page again with that name as creator
  and contributor and measures both, and uses 32 `W`s for every list. **Shown:**
  E-10 fails on HEAD's job page at the new step (`the layout viewport was
  widened to fit the page`) and passes on the fix; with HEAD's `/users` it
  fails at 579 px of 359; with an empty contributor list its new row check
  fails.
- **Medium — PLAN and TESTING said every alias counts as its target's
  bytes** after 12.3 limited that to distributions and layouts (a private
  test: 60 aliases of a 60 KiB `.kwg`, ratio 60, accepted). **Fix:** both say
  which aliases count.
- **Unconfirmed, fixed:** the 90 s floor tolerated one lost heartbeat but not
  one stalled for MAGPIE's 120 s request timeout (read from `contribute.c` and
  `http_client.c`, not reproduced); the floor is 180 s.

**Lows fixed:** PLAN's startup order says a second process migrates before it
finds the address taken; a ref costs one call for a branch, two for a tag and
one per tag object peeled (up to six), in PLAN and the handler; `A-ADMIN-5b`
and PLAN describe the prefixes and nested tags; the form's hint names "the data
repository", not MAGPIE-DATA; a `BIND_ADDR` that is not an IP address and port
fails in the config, before the MAGPIE probe and the migrations. **Held:** the
range checks (nothing in the repo sets a refused value; a day's timeout is
safe in every use); the bind before the reapers (on 127.0.0.1, 0.0.0.0 and
`[::]`); the ref prefixes (`refs/heads/` alone, `heads/../tags/v1`,
`tags/refs/heads/main` refused; five tag hops and a self-referencing tag end in
an error); the alias gate (alias rows carry no bytes); `sdk()`.

### 12.5 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **592 of 592**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 119 of 119.
- **Tier 5, natively: 11 of 11** on the final working tree.
- `scripts/runbook-check.sh`: 16 blocks parse; RUNBOOK §1's modify and swap
  blocks replayed through a pty with logging stubs (above).
- A second backend on a taken address, committed binary against the fix
  (12.3).
- MAGPIE and infra unchanged (MAGPIE read only).

## Pass 13 — follow-up pass

**Plan.** The diff since the previous pass's base (`0cfe839..7c88905`; MAGPIE
and infra unchanged), one reviewer per part it touches: backend; frontend
(tier 5 natively); docs, procedures and CI. Plus one area not examined in this
run: **request authentication and rate limiting** — API keys, anonymous worker
identities, bans, session cookies, CSRF, the admin guard, the client address
and the rate limiters.

**Findings: 0 high, 6 medium** from the four reviewers (backend 2 medium, 6
low; frontend none, 6 low — tier 5 natively 11 of 11; docs and procedures 3
medium, 7 low, 1 unconfirmed; authentication and rate limiting 2 medium, 4
low, 2 unconfirmed). One medium was found by two reviewers. All fixed or, where
the fix is MAGPIE's, bounded and recorded.

### 13.1 Medium — `runbook-check.sh` read 16 of RUNBOOK's 25 bash blocks (backend and docs reviewers)

Its fence pattern was anchored at column 0; the nine blocks indented inside
numbered lists — all of §5 and §6 — were never read, and a broken one passed
(`ok: 16 bash blocks`). Widened, the check failed on HEAD: §5's
`DR_REGION=<the region the copy is built in>` placeholders do not parse.
**Fix:** blocks are read at any indent with the fence's indent removed; a shell
block fenced `sh`, `shell`, `console` or `zsh`, or a bash fence never closed,
is refused; the count read must equal the fences. §5's placeholders are
`DR_REGION=''  # …` (and `DR_ACCOUNT`), which its `if` already refuses empty.
**Verified:** 25 blocks parse; HEAD's RUNBOOK fails at line 975, an indented
block with an apostrophe'd `${…:?}` fails at 1042, an unclosed fence and a
`shell` fence fail. (13.8 replaced this fix — the fence patterns and the
count, and the `DR_ACCOUNT` placeholder.)

### 13.2 Medium — the heartbeat floor's reason was wrong: MAGPIE's 120 s bounds a stall, not a request (backend reviewer)

Pass 12 set the floor to 180 s on "30 + 120 + 30". `chttp.h` says the 120 s is
a no-progress timeout; the exchange may run an hour. With MAGPIE's own libcurl
options against a server that stalled twice mid-reply, every heartbeat
succeeded, 250 s apart. No floor short of an hour covers a crawling link, and
the default 300 s does not either. A third change of the number would be a
patch on a patch; instead what the floor does is stated: it keeps a timeout
above MAGPIE's cadence and one stalled heartbeat, and nothing more is promised.
**Fix:** `config.rs` and PLAN's row say so; KL-83 records the slow-link case
and the real fix (a whole-exchange bound on MAGPIE's heartbeat, a MAGPIE
release).

### 13.3 Medium — MAGPIE sent its credential in the clear to an `http://` server, silently (authentication reviewer)

MAGPIE accepts any scheme, and the ALB's port 80 redirected every path to
https, which libcurl followed. Shown with `strace` on MAGPIE: an anonymous
worker's `X-Worker-UUID` crossed in the clear on every request and everything
worked; a keyed worker's `Authorization: Bearer bt_…` crossed in the clear,
was dropped at the scheme change, and its heartbeats and results were refused
— while MAGPIE printed "as an authenticated worker". **Fix:** the ALB answers
`/api/*` on port 80 with `426` and a message ("served over https only … treat
any API key sent over http as disclosed") instead of redirecting; the page is
still redirected. **Verified:** MAGPIE against a stand-in giving that answer
stops after one request: `claiming a task failed with HTTP 426: birdtest's API
is served over https only…`; `terraform fmt` and `validate` pass. The first
request still discloses; the client-side refusal needs a MAGPIE release
(KL-84).

### 13.4 Medium — PLAN's worker rate limit was one bucket per identity; each credential has two (authentication reviewer)

The thirty-first audit split claims from work in hand and never updated the
table: one key made ten requests in a burst where PLAN allowed five, and the
key's bucket is keyed by its hash, not its id. **Fix:** the table has a row
for claims and one for work in hand, with why; the "Rate limiting" paragraph
matches.

### 13.5 Medium — PLAN's import step still said every alias counts against the caps (docs reviewer)

12.4 corrected the limits table and not the step above it. **Fix:** the step
says which aliases count; `aliases_count_as_their_targets_bytes_and_kept_files_are_small`
now also walks sixty aliases of a lexicon, which counting them would refuse.

### 13.6 Medium — the AWS CLI's pager swallowed a pasted `wait` (docs reviewer)

AWS CLI v2 pages output longer than a screen through `less`, which read the
rest of a paste (without bracketed paste) as keystrokes: the restore's
`wait` never ran, and the next blocks met an instance still being created.
Shown with a stub that pages as v2 does. **Fix:** §1, §5 and §6 begin with
`export AWS_PAGER=""`, and the header says to set it first.

### 13.7 Low findings

**Fixed:**
- Worker authentication: the `Bearer` scheme is matched in any case (a
  lowercase `bearer` with a deactivated key minted an anonymous identity and
  got `204` — `A-ACCOUNT-4b`, which fails on HEAD); a banned credential is no
  longer remembered as known, so it pays its address's bucket every time; an
  `X-Forwarded-For` with a byte that is not visible ASCII is read as bytes,
  not dropped (it keyed the request on an ALB node — a unit test, failing on
  HEAD); the CSRF comparison takes the same time wherever the tokens differ.
- RUNBOOK §1: the rename block checks the restored instance exists before
  renaming production (a mistyped `STAMP` renamed production and then found
  nothing — now it stops at the check, shown with the stubs); `renamed()`
  gives up after fifteen minutes (it looped for ever on expired credentials);
  what to do if the block stops part-way.
- Ref resolution: a chain of more than four tags says so; PLAN says a branch
  named `tags/…` is given as `heads/tags/…`.
- Frontend: the import form is one column on a phone (three columns of 74 px
  at 320); the unban notice breaks a long name (the one spill in a 156-page
  sweep); E-10's helpers are in order under their comments.
- `birdtest listening` logs the bound address; `fake_worker.py`'s abandon
  mode names the 180 s floor; KL-82 no longer says the Terraform requires one
  binary for both images; TESTING's E-10 text; 12.3 and 12.4's counts.

**Recorded:** KL-76 (E-10 measures at 393 px only; a pool's ratings table at
320 px scrolls in its box); KL-81 (the CSRF cookie is not `__Host-`); KL-83;
KL-84 (with the unconfirmed case of a client's second `X-Forwarded-For` header
passed by the ALB). **Unconfirmed, left:** the 180 s floor equals the worst
case after one stalled heartbeat rather than exceeding it by the nap's
overshoot — covered by KL-83's statement that the floor promises nothing more.

### 13.8 Adversarial check of the pass's fixes

**4 medium, all fixed.**

- **`runbook-check.sh` still passed on fence variants** — `bash title="x"`,
  `Bash`, `` ``` bash ``, ```` ````bash ````, `~~~bash`, `sh title`, a quoted
  fence: none was read, counted or refused (`ok: 1`, rc 0, around a block that
  does not parse). Its patterns had been widened once already (13.1), so they
  are replaced: the script now **parses** every fence — any run of backticks
  or tildes at any indent opens one, which closes only on the same character
  at least as long — and an opener must be labelled exactly `bash` (read) or
  `sql` (skipped); anything else, unlabelled or quoted, is refused, as is a
  fence never closed. **Verified:** each of the thirteen variants now fails
  (refused, or read and found not to parse); a heredoc holding fence text
  inside a block reads correctly; RUNBOOK passes with 25 blocks.
- **§6's drill could no longer run as written.** 13.1 made §5's `REPLICA` a
  computed line from a new `DR_ACCOUNT`, overwriting the staging bucket the
  drill sets. **Fix:** `REPLICA=''` is itself the quoted placeholder (no
  `DR_ACCOUNT`), the refusal names it, and §6 says to fill it in. **Shown:**
  step 1 replayed with stubs reads the drill's bucket when filled and refuses
  when empty.
- **"Rotating the database password" and thirteen other blocks had no
  `AWS_PAGER`** (13.6 covered §1, §5, §6): a pasted rotation set the new
  password in RDS and never reached SSM. **Fix:** every block that calls `aws`
  begins with `export AWS_PAGER=""` (fourteen added), and `runbook-check.sh`
  refuses one that does not — HEAD's RUNBOOK fails with 17 such blocks.
- **The heartbeat floor's reason was wrong again.** libcurl averages speed over
  a few seconds, so a stalled heartbeat is abandoned at about 127 s, not 120
  (`curl` with MAGPIE's options: 127.03 s), and one stalled heartbeat leaves
  some 187 s between recorded ones — past the 180 s floor. The floor is not
  moved a third time: `config.rs`, PLAN's row and KL-83 now say it keeps a
  timeout above MAGPIE's cadence and nothing more, and that the default, 300 s,
  covers one stalled heartbeat.

**Lows fixed:** README and `ecs.tf`'s comments say port 80 refuses the API
rather than only redirecting (and that a copied `http://…/api/…` link reads the
`426`); the unban notice wraps only the long name (`overflow-wrap:anywhere`,
not `break-all`). **Held:** the listener rule (`fmt`, `validate`; the provider
takes only `2xx`/`4xx`/`5xx`; priorities are per listener; health checks do not
pass through it; MAGPIE sends no `/api` path it would miss, and does not retry
a `426`); the bearer parse (two spaces authenticate the trimmed key; a tab,
`Bearerx` or `Basic` fall to the UUID path as before; no panic); the gate
(a banned credential pays its address's bucket once its known entry expires);
the byte-wise `X-Forwarded-For` (the trusted hop's entry stays out of the
client's reach); `same_token`; the tag chain; the resume guidance in every stop
state of the rename block; 594 tests listed.

### 13.9 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **594 of 594** (two new: `A-ACCOUNT-4b` and the `X-Forwarded-For` unit test,
  each failing on HEAD; the tag-loop and lexicon-alias assertions added to
  existing tests, the first failing on HEAD).
- `npm run check`: 0 errors, 0 warnings; `npm test`: 119 of 119.
- **Tier 5, natively: 11 of 11** on the final working tree.
- `terraform fmt -check` and `validate` (offline): pass.
- `scripts/runbook-check.sh`: 25 blocks; the variants, HEAD's RUNBOOK and a
  stalled `aws` pager check as above; RUNBOOK §1's rename block replayed with
  `STAMP` set, mistyped and unset, and with its wait never satisfied (it gives
  up after 90 tries); §5's step 1 empty and filled.
- MAGPIE against a stand-in answering `426` (one request, the message shown).
- MAGPIE unchanged (read only).

## Pass 14 — follow-up pass

**Plan.** The diff since the previous pass's base (`7c88905..d511393`; MAGPIE
unchanged), one reviewer per part it touches: backend; frontend (tier 5
natively); docs, procedures and scripts; infra. Plus one area not examined in
this run: **MAGPIE's contribute client** — the claim, heartbeat and submission
loop, the settings file, the HTTP client, and what a server's answers become on
the contributor's machine.

**Findings: 0 high, 5 medium** from the five reviewers (backend none, 3 low;
frontend none, 2 low — tier 5 natively 11 of 11; docs and procedures 2
medium, 6 low; infra none, 3 low, 2 unconfirmed; MAGPIE's client 3 medium, 9
low, 2 unconfirmed). All fixed, the MAGPIE ones on `birdtest-contribute`.

### 14.1 Medium — a server-assigned `worker_uuid` was written into `contribute.txt` as it came (MAGPIE reviewer)

MAGPIE checked only that the UUID was present, then appended `uuid <value>` to
the settings file and sent the value as `X-Worker-UUID`. A value with newlines
added settings of the server's choosing, which every later run obeyed (the
last `server` line wins), and its CRLF reached the wire as header injection.
**Shown** with the committed binary against a stand-in:
`contribute.txt` gained `server http://127.0.0.1:1/elsewhere` and `threads 1`.
**Fix:** only a canonical UUID (8-4-4-4-12 hex digits) is adopted; anything
else is ignored with a line saying so, and the next claim asks again.
**Verified:** the same stand-in against the fixed binary leaves the file as it
was and prints "the server sent a worker identity that is not a UUID";
`contribute_test.c` checks the form against ten values, a newline and a CR
among them.

### 14.2 Medium — PLAN promised the worker bounds what the server asks for; nothing does (MAGPIE reviewer)

Response bodies, `num_games`, `num_plays` and the recorded-play counts are
checked only for being positive: under `ulimit -v`, a 2 GiB artifact, two
billion plays and a 10^15-game task each ended in a failed allocation and an
abort. AUDIT_FINDINGS_19 had weighed this and left it — the worker trusts its
server's sizes — but PLAN still said the opposite. **Fix:** PLAN's "Client
security" states the trust model; KL-85 records the gap and the bounds that
would close it.

### 14.3 Medium — a settings file MAGPIE could not write lost the worker's identity silently (MAGPIE reviewer)

With `contribute.txt` read-only, the issued UUID lasted one run and each
restart was a new anonymous worker, with nothing printed — the case PLAN's
design exists to prevent. **Fix:** the failed write is reported, with the line
to add by hand. **Verified:** read-only, the run prints "could not save this
worker's identity to contribute.txt; add the line uuid 6f3d7198-…"; writable,
the line is appended as before; a test writes to a path that cannot exist.

### 14.4 Medium — README's password block had the pager problem RUNBOOK's had (docs reviewer)

README's first-deploy password block (which rotation reuses) called
`modify-db-instance` with the pager on: without bracketed paste, `less` ate
the `wait` and the endpoint lookup, and `put-parameter` wrote
`postgres://birdtest:…@:5432/birdtest` to SSM. **Fix:** README's `aws` blocks
(three then, four once 14.7 split one) turn the pager off, its example
`contribute.txt` is labelled `text`, its ACM block parses (the ARN captured,
not `<arn>`), and CI runs the check over README too.

### 14.5 Medium — the pager check did not check "first" (docs reviewer)

`runbook-check.sh` asked only that `export AWS_PAGER=""` appear somewhere in a
block, and its `aws` pattern missed a tab, backticks, a line continuation and
a quoted name: a block paging before its export passed, and replayed, the
`wait` never ran. **Fix:** a block that mentions `aws` anywhere but a comment
line must have the export as its first line of code; the pattern is widened;
no pipes (one closed early under `pipefail` passed a large block). A fence on a
list item's own line is refused, and a label's CR is shown. **Verified:** of
the reviewer's 34 cases, every one that should fail fails (`aws` before the
export, tab, backtick, continuation, `sudo`, `command`, `xargs`, a heredoc or
`if false` export, `"aws"`, `'aws'`); the exceptions are an `AWS_PAGER=''`
export, which is accepted, and an indented code block or `<pre>`, which are
not fenced; RUNBOOK (25 blocks) and README (18) pass; HEAD's README fails.

### 14.6 Low findings

**Fixed:**
- Backend: the bearer test also shows a live key sent as `bearer` is looked up
  and its use recorded; an empty CSRF cookie and header no longer match.
- MAGPIE (`5753e212`, which `docker/Dockerfile` now pins; `MAGPIE_VERSION`
  stays 0.1.1, since nothing computed changes): a negative `maxtasks` is
  refused; `bonus_square_from_char` indexes as unsigned (a byte above 0x7f
  was a negative index) — both from KL-68, which said they would go with the
  next MAGPIE change.
- Docs: PLAN's HTTP requirements say `timeout_seconds` bounds a stall, not the
  exchange; the eight `dlopen`ed symbols listed; where `impl_contribute`
  lives and what `contribute.c` holds; "known values" for lexicon names is now
  the character set the code checks; README's HTTPS sentence keeps its reason
  with it; `ecs.tf`'s comment; KL-84 notes the `426` has no `Upgrade` header.

**Recorded:** KL-68 (cross-host redirects, a leave task with no lexicon, the
unchecked double-to-int cast, terminal escapes, the per-task mutex, exit 0
after errors, a non-JSON `200` counted as accepted, a decline's answer unread,
`maxtasks` truncation); KL-85. **Left:** the import form's button 2 px off
its inputs; the staged-files table scrolling in its box on a phone.

### 14.7 Adversarial check of the pass's fixes

**2 medium, both fixed.**

- **A fence on a nested or quoted list item's line was still skipped** —
  `- - ```bash`, `1. - ```bash`, `> 1. ```bash`: none matched the list-marker
  rule or the opener, and unclosed (a list's end closes it) the block rendered
  as bash and was never read (`ok`, rc 0; rendered with micromark). **Fix:**
  outside a block, any fence run after anything but blanks is refused —
  quote marks, list markers, nesting and prose alike; neither RUNBOOK nor
  README has one. **Verified:** the three cases fail; of the 55 case files
  from this pass and the last, those accepted are only an `AWS_PAGER=''`
  export, an export with leading blanks or a trailing comment, a call by full
  path after the export, and three the header names as out of reach (an
  indented code block, `<pre>`, a block that unsets the pager after turning
  it off).
- **README's ACM block, made to paste whole, hid the CNAME and blocked.**
  `describe-certificate` ran before ACM had a record (`null`), then `wait`
  sat — five minutes with the current CLI, forty with older ones (15.2) — on
  a certificate that could not validate, the ARN shown only after. **Fix:** two blocks — the request prints the ARN and
  polls (up to five minutes) until the CNAME exists, printing it; the second,
  run once the CNAME is in DNS, waits and prints the `prod.tfvars` line. Each
  refuses its empty placeholder. **Shown** with a stub that answers `None`
  twice: the ARN, then the CNAME, then the wait; with nothing filled in, both
  blocks refuse and nothing is called.

**Lows fixed:** the `aws` pattern catches a call by path or through `${AWS:-aws}`;
the first line may carry leading blanks. **Held:** MAGPIE's canonical-UUID
check (a truncated value must still pass all 36 positions); a rejected UUID
followed by a task ends in five failures and an exit, nothing spins; valgrind
clean on the rejected and read-only paths; no documented use of a negative
`maxtasks`; nothing else names the old pin; the CSRF cookie is always 48 hex
characters, so no real request carries an empty token; mawk and busybox awk
agree with the checker's results.

### 14.8 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included,
  `MAGPIE_BIN` the `portable_release` build of `5753e212`: **594 of 594**.
- **MAGPIE's suite** (`no_pgo_release`, each default test alone): **70 of 70**;
  `format.py` and `find_circ_deps.py` (on a clean copy) pass.
- **Tier 6, natively, every case** (M-1 to M-7, M-9 to M-11) with `5753e212`:
  passed.
- MAGPIE against stand-ins: the injected UUID (committed binary writes the
  `server` line; fixed binary ignores it), a read-only settings file (the
  warning), a writable one (the line appended).
- `npm run check`: 0 errors, 0 warnings (the frontend is unchanged this
  pass); tier 5 was run by the frontend reviewer on `d511393`: 11 of 11.
- `terraform fmt -check` and `validate`: pass.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks; the case
  files as above; README's password and alert blocks replayed with the paging
  stub (no call paged), and its ACM blocks with a stub.

## Pass 15 — follow-up pass

**Plan.** The diff since the previous pass's base (`d511393..97fd887`), and
MAGPIE's `7400184f..5753e212`, one reviewer per part it touches: MAGPIE;
backend and scripts; docs and procedures. Plus one area not examined in this
run: **the build and CI** — `docker/Dockerfile` and the frontend's, the
compose files, `.github/workflows/`, `e2e/run.sh`'s build.

**Findings: 0 high, 5 medium** from the four reviewers (MAGPIE 1 medium, 4
low; backend and scripts 1 medium, 3 low; docs and procedures 1 medium, 6 low;
build and CI 2 medium, 14 low, 2 unconfirmed). All fixed.

### 15.1 Medium — a UUID save cut short by a full disk left a line every later run sent (MAGPIE reviewer)

On a full filesystem the append of `uuid <36>` could land partly: the warning
printed (14.3), but `uuid 6f3d7198-1` stayed at the end of the file, and every
later run sent it, was refused `400` by the server, and ended — after the disk
was freed too. Shown in a user-namespace tmpfs of 8 KiB. **Fix** (MAGPIE
`4c09e1a3`, and `8ba24b7b` after 15.7, now pinned): a `uuid` line that is not a
UUID fails startup naming the line to correct or delete; the warning says to replace any part left at the end; the
anonymous-assignment contract fixture's `worker_uuid` is checked against
`client_state_is_worker_uuid`, tying the server's format to the client's
check. **Verified:** the new load test fails with the check removed
(`Assertion '!client_state_load(path, error_stack)' failed`) and passes with
it; MAGPIE's `contribute` and `layout` tests pass; formatting passes.

### 15.2 Medium — README's ACM wait gives up after four minutes, not forty (docs reviewer)

The real AWS CLI 2.37.4's `CertificateValidated` waiter polls 60 s × 5 (shown
against a fake ACM endpoint: `Max attempts exceeded`, rc 255, 241 s); README
said about forty, and DNS validation can take thirty. **Fix:** the block asks
again, up to eight times, and prints the `prod.tfvars` line only once
validated, or says to check the CNAME; it refuses an unset `REGION` as well as
`ARN`. **Shown** with a stub: failing twice then validating, it waits three
times and prints the line; never validating, eight tries and the message.

### 15.3 Medium — `runbook-check.sh` passed a heredoc never terminated and a trailing backslash (backend and scripts reviewer)

`bash -n` only warns of a heredoc whose terminator never matches (an indented
`EOF`, the twenty-fourth audit's own incident) and says nothing of a
backslash ending a block's last line; both leave a paste at a continuation
prompt. **Fix:** any output from `bash -n` fails the block, as does a last
line ending in a backslash; and a line of an indented block less indented
than its fence is refused (a renderer ends the list item there; the check read
on and swallowed a four-backtick block after it). **Verified:** the reviewer's
four new cases fail; RUNBOOK (25 blocks) and README (19) pass; of the 55
earlier cases, the same eight are accepted as before.

### 15.4 Medium — a new migration alone did not rebuild the backend (build and CI reviewer)

`sqlx::migrate!` makes cargo watch only the migration files it embedded; a new
one with no Rust changed left the crate Fresh, and the binary — which applies
its embedded migrations at startup — without it. CI builds from scratch; a
warm target (an operator's Docker builder, a native build) did not. **Shown**
in this repo: a throwaway `0002_audit_probe.sql` left `Fresh birdtest`.
**Fix:** `backend/build.rs` (`cargo:rerun-if-changed=migrations`), copied
into the image. **Verified:** with it, the crate recompiles and the new
migration's text is in the binary; removed, it is gone again.

### 15.5 Medium — the backend image's cargo cache could ship another checkout's binary (build and CI reviewer)

The target cache mount is shared by every build on the machine, and `COPY`
keeps files' mtimes: a checkout whose sources were older than the last
build's was Fresh to cargo and got the other checkout's binary (shown with
cargo and `cp -p` into one build directory). **Fix:** the build touches the
crate's own files first, so it always rebuilds; dependencies stay cached. Not
built as an image (images are built only with the user's say-so).

### 15.6 Low findings

**Fixed:**
- Build and CI: the fetched MAGPIE must be exactly the pinned commit (a branch
  name is refused — checked against a local fetch); `MAKE_JOBS` caps MAGPIE's
  LTO build, which `e2e/run.sh` passes as 3; CI's MinIO wait gives up after a
  minute; CI's image probe also requires `"build_target":"nehalem"` (tiers 5
  and 6 never run the image's own MAGPIE); the contract job clears MAGPIE's
  fixture directory before copying (a removed fixture stayed); nginx's
  directory redirects are relative and its version is not sent (`nginx -t`
  passes; `/sub` answers `Location: /sub/`); `e2e/run.sh` expands an empty
  argument list on bash before 4.4; README's release builds `--pull`;
  `.env.example` names the import and builder variables compose reads.
- Docs: TESTING's CI list and PLAN's tree name what `runbook-check.sh` covers;
  README's `REGION` says where it will be written, and pasting the request
  block again is said to request another certificate; PLAN's "response bodies
  checked for sense" corrected; 14.4, 14.7 and pass 13's summary counts.
- MAGPIE: `maxtasks`' refusal quotes what was written; "canonical" says either
  case.

**Recorded (KL-64):** base images, the Dockerfile syntax and the fake
worker's `requests` unpinned; releases built from the working tree; pages
served uncompressed; the nightly drill starting before `minio-init` ends;
`convert_lexica.sh` exiting 0 on a partial conversion. **Unconfirmed, left:**
an unset `ARG CARGO_BUILD_JOBS` exported empty (BuildKit leaves it out); CI
job timeouts on GitHub's runners. **Left:** `bash -n` does not see history
expansion (no block has a `!` in double quotes; the script's header says so);
a fence indented four columns read as a fence.

### 15.7 Adversarial check of the pass's fixes

**2 medium, both fixed.**

- **A closing fence less indented than its opener still passed.** The closer
  test ran before 15.3's indentation rule, so a list-item block closed at
  column 0 was taken as closed, while a renderer ended the list item there and
  opened an unlabelled block over the prose and the next bash block (shown
  with micromark; the check said `ok: 2`). **Fix:** the indentation rule
  applies to every line of an indented block, its closing fence first.
  **Verified:** the four closer cases fail; RUNBOOK and README pass; the
  earlier cases are unchanged.
- **In stock zsh, every commented block failed when pasted.** Without
  `interactive_comments`, `#` is a word: the first line's
  `export AWS_PAGER=""   # …` failed, and an apostrophe in a comment opened a
  quote that swallowed the rest of the paste — with bracketed paste, before
  anything ran (shown in zsh 5.9; bash correct in both modes). **Fix:** README's deployment preamble
  and RUNBOOK's header say the blocks are bash, and to run `bash` first in
  zsh.

**Lows fixed:** README's ACM wait stops at once on a terminal failure (a
certificate that failed validation, or one the region does not have) and says
a failed certificate needs a new request (shown: one try, not eight); the
trailing-backslash refusal says why; MAGPIE `8ba24b7b` accepts an empty `uuid`
value again (it always meant none — 4c09e1a3 refused it) and tells a
contributor with a hand-edited UUID to correct the line, not only delete it
(the server only ever mints lowercase hyphenated v4s). **Held:** `build.rs`
(adding, editing, adding a second and removing a migration each rebuild, a
source edit still rebuilds, an unchanged tree stays Fresh; with every mtime
set to 2020 the build stayed Fresh until the Dockerfile's touch); the touch
under dash in `debian:bookworm-slim`; no caller passes a branch as
`MAGPIE_COMMIT`; the probe and the MinIO wait's quoting; the contract
directory holds the same 16 fixtures and a README; the ACM loop under `set -e`
and `set -u`. **Left:** a comment ending in a backslash is refused as a
continuation; a tab counted as one column of indentation.

### 15.8 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **594 of 594** (with `build.rs`).
- **MAGPIE's suite** at `4c09e1a3`: **70 of 70**; at `8ba24b7b`, `contribute`,
  `layout` and `config` pass; `format.py` passes.
- **Tier 6, natively, every case**: passed at `4c09e1a3` and at `8ba24b7b`.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks; every
  case file from passes 13–15 as above.
- The Dockerfile's commit check against a local fetch (the pin passes, a
  branch fails); `MAKE_JOBS` under dash; the nginx template under `nginx -t`
  and a relative `Location` from the running container.
- README's ACM blocks replayed with stubs (retry, terminal failure, empty
  placeholders).
- The frontend is unchanged this pass; Terraform unchanged.

## Pass 16 — follow-up pass

**Plan.** The diff since the previous pass's base (`97fd887..a4ea3b4`), and
MAGPIE's `5753e212..8ba24b7b`, one reviewer per part: build, backend and
scripts; docs and procedures; MAGPIE. Plus one area not examined in this run:
**the admin frontend** — every page under `routes/admin/` and what they share.

**Findings: 0 high, 2 medium** from the four reviewers (build, backend and
scripts none, 7 low; docs and procedures none, 6 low, 2 unconfirmed; MAGPIE 1
medium, 4 low; admin frontend 1 medium, 11 low, 1 unconfirmed — tier 5
natively 11 of 11, every admin page driven with Playwright). Both fixed.

### 16.1 Medium — after a failed first read, the admin job page showed defaults as the server's state (admin frontend reviewer)

The page read the job, then its data gaps, then its export, one after another,
and stopped at the first failure; the stream still filled in the stats, so the
page looked whole. After a transient `503` on the first read it showed the
allocation box's initial 100, "No worker has declined this job" though the
gaps were never read, and Activate sent `{"allocation":100}`. **Fix:** the
three reads settle apart; the allocation is the job's own, seeded from the
first payload (REST or stream) and empty until one arrives; a failed read is
tried again on the next live payload and after five seconds, with its error
kept apart from an action's and cleared on success; the data-gaps card says
"Could not load" rather than "none"; actions and the artifact checks take no
second click while one runs; Activate checks for a whole number. **Verified:**
a new journey, `E-11` (the first read answered `503`, the gaps mocked), fails
on the committed page (`No worker has declined` shown) and passes; tier 5 is
12 of 12.

### 16.2 Medium — an API key on the wrong line was printed by MAGPIE (MAGPIE reviewer)

A key appended to a `contribute.txt` whose last line had no newline landed in
that line's value, and a key pasted alone was a line of its own: MAGPIE
printed it in "'0apikey bt_…' is not a valid integer", in "contributing to
https://…apikey bt_…", and in "unknown setting 'bt_…'", though PLAN promises a
key never appears in output or errors. Nothing reached the network. **Fix**
(MAGPIE `3565279b`, pinned): settings messages name the file, line and setting
only; an unknown word is shown only if it could be a setting's name; a
`server` value holding a space is refused. With it, the `uuid` check moved
after the file is read, so the last `uuid` line is the one checked (a partial
line then a whole one — what adding the line by hand after a failed save
leaves — loads). **Verified:** `test_client_state` fails on the committed
parser and passes; the three cases replayed with the built binary print no key.

### 16.3 Low findings

**Fixed:**
- `runbook-check.sh`: a closing fence indented four or more past its opener,
  or any fence-like line inside a block, is refused (a renderer shows it as
  text; in bash it is backquotes); a trailing backslash counts only as an odd
  run; a backslash followed by blanks is refused, with the line; bash 3.2's
  silence on an unterminated heredoc is in the header.
- Build: compose caps `CARGO_BUILD_JOBS` and `MAKE_JOBS` as `e2e/run.sh`
  does (`.env.example` names them).
- The contract: the server's `a_first_claim_is_assigned_a_worker_uuid` checks
  it sends the fixture's exact `worker_uuid`, so a change of form fails on both
  sides; TESTING says so.
- Admin pages: a ban's target is trimmed and a double click sends one ban;
  unbanning asks first; the job form's players are required; the input-data
  delete names the row's digest and says its derived files go with it; the
  force-complete dialog no longer contradicts purge; PLAN's admin job page row.
- README: the ACM wait retries only when the wait ran out (a failed
  certificate, a wrong region, denied access or expired credentials stop at
  once), ten rounds of about four minutes, and "waiting again" only when
  another follows; the zsh note is at the top, covering every block.
- KL-64 names the MinIO client image; 14.4's paragraph reflowed.

**Recorded:** KL-68 (a failed save's partial line — now refused naming it —
could be truncated away); KL-86 (the admin pages' remaining small things).
**Left:** `MAKE_JOBS` splitting the Docker build cache between run.sh and
other builds; a refusal inside an indented block cascading into a second
message.

### 16.4 Adversarial check of the pass's fixes

**4 medium, all fixed.**

- **A retry replaced the allocation the admin had typed.** 16.1's reload
  seeded the allocation on every successful read, the retries included, so a
  value typed after a failed read was put back to the job's (shown: typed 55,
  six seconds later 7, Activate sent 7). **Fix:** typing marks the box edited;
  reads leave it alone until an action's own read. **Verified:** E-11 now
  types 55 while two gap reads fail and asserts 55 is kept and sent; with the
  guard removed it fails at that assertion.
- **A read started before an action could land after it.** The retry and an
  action's reload ran unordered: a slow retry finishing after Deactivate put
  back `active` and cleared the error, and an inactive job gets no payload to
  correct it (shown). **Fix:** each read is numbered and one overtaken by a
  newer is dropped; the retry skips while a read runs.
- **`runbook-check.sh` hung on a line of only backslashes** — 16.3's
  odd-run loop read past the start of the line in every awk (mawk, nawk,
  busybox). **Fix:** the loop is bounded. **Verified:** both cases now fail
  at once instead of hanging.
- **PLAN promised no settings value appears in output;** the status line
  prints `server`, and a bare key glued to it with no space shows there
  (shown with the built binary). **Fix:** PLAN says no settings *error*
  quotes a value, and that the status line prints `server`; KL-68 records the
  URL shape MAGPIE does not check.

**Lows fixed:** Purge, Force complete, Merge, Export and Delete take no
second click and show no dialog while an action runs; an export poll's error
is kept apart and cleared by the next good poll; `runbook-check.sh` leaves a
shorter fence inside a longer one alone (the CommonMark way to quote a fence),
refuses a continuation into a blank or comment line (an option dropped), and
its advice no longer points the wrong way; README's ACM loop retries only a
certificate still pending (a timed-out one stopped too) and its last message
names credentials; the contract comment says hex, either case; the unban
dialog says the reason stays in the audit log; PLAN's admin row names the data
gaps and ETA. **Held:** E-11 fails on the committed page and passes;
navigating away mid-retry sends nothing more; `busy` resets in `finally`; the
real CLI's timed-out wait says "Max attempts exceeded" and, still pending,
names `PENDING_VALIDATION`; MAGPIE loads CRLF, tabs and uppercase UUIDs and
quotes no key in any settings error; every doc block parses under bash 3.2.

### 16.5 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **594 of 594**.
- **MAGPIE's suite** at `3565279b`: **70 of 70**; `format.py` passes.
- **Tier 6, natively, every case** at `3565279b`: passed.
- **Tier 5, natively: 12 of 12** with `E-11` on the final working tree; `E-11`
  fails on the committed page and on the page without the edit guard.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 119 of 119.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks; every
  case file from passes 13–16 as above, none hanging.
- README's ACM wait replayed with a stub carrying the real CLI's messages:
  pending then issued, never issued, timed out, failed, denied.

## Pass 17 — follow-up pass

**Plan.** The diff since the previous pass's base (`a4ea3b4..20a27fd`), and
MAGPIE's `8ba24b7b..3565279b`, one reviewer per part: frontend (tier 5
natively); MAGPIE; scripts, docs and backend. Plus one area not examined as a
whole in this run: **the infrastructure** — every file in `infra/`, and the
scripts that run as its tasks, against their roles.

**Findings: 0 high, 4 medium** from the four reviewers (frontend 2 medium, 5
low, 1 unconfirmed; MAGPIE 1 medium, 10 low; scripts, docs and backend none, 5
low, 1 unconfirmed; infrastructure 1 medium, 6 low, 2 unconfirmed). All fixed;
the admin job page's state handling, patched in passes 16 and 16.4, is
redesigned rather than patched a third time.

### 17.1 Medium — the database password and the session signing key sat in plain text in Terraform's state (infrastructure reviewer)

The two SSM parameters were Terraform resources with a placeholder value and
`ignore_changes = [value]`, which suppresses the diff but not the read: every
refresh wrote the decrypted values into the state — the RDS password inside
the URL, and the key that mints a session for any user. README said the state
held no secret and to copy it off the machine. **Shown** with the stack's own
`ssm.tf` against a local SSM emulator: after README's second apply, both
values in `terraform.tfstate`, and `plan` said nothing. **Fix:** Terraform no
longer manages them; the task definitions and roles reference them by an ARN
built from their names, and `removed` blocks take them out of an existing
stack's state without deleting them. README says the parameters are created
by `put-parameter`, and that a stack applied before this pass must rotate
both. **Verified:** against the emulator, the old config puts both values in
the state; the new one drops them (state holds neither) and both parameters
remain in SSM; `fmt` and `validate` pass. With it: the cluster's ECS Exec
logging is off (its default asked the ops role for CloudWatch Logs
permissions it lacks, and a transcript would hold what an operator echoes).

### 17.2 Medium — a typed allocation was thrown away by any action (frontend reviewer)

16.4 kept a typed allocation through retries but cleared the edit flag after
every action, so Deactivate, Purge, Force complete and Merge put the job's
value back over one the admin had typed and not sent, and Activate sent the
old one (shown against the real backend: typed 12, Deactivate, Activate sent
7). The third fix to how the page keeps the allocation in step, so the
approach changed (17.4).

### 17.3 Medium — a read started before a live payload landed over it (frontend reviewer)

16.4 ordered reads against each other, not against the stream: a retry's slow
read that started before the stream said `completed` landed after it and put
`active` back, and a completed job sends nothing more (shown with mocks).
Fixed by the same redesign (17.4).

### 17.4 The admin job page's state, redesigned

- The job's stats come from the REST read and the stream, neither ordered: a
  REST read is applied only if it is the newest started and no stream payload
  arrived while it was out.
- The allocation box is the admin's alone: filled once, from the first
  payload that says the job's allocation, and never touched by a read after
  that; the job's current allocation is shown beside it ("Now: 7%").
- The job, its gaps and its export are read apart; export results go through
  one function, one poll timer at a time, and a 404 clears the export error; a
  deleted job (404) says so and stops the retries; a validation error or a
  delete clears the last action's notice.

**Verified:** `E-11` now also types a value, deactivates, and asserts the value
kept and the job's shown beside it; a new `E-11b` holds a retry's read until
the stream has said `completed` and asserts the page still says completed.
Both fail on the committed page (at the kept value, and at `completed`) and
pass; four repeats of both pass; tier 5 is 13 of 13.

### 17.5 Medium — PLAN said no settings error quotes a value; the negative-`maxtasks` one did (MAGPIE reviewer)

It quoted only a whole negative number, so never a key, but broke the
guarantee as written. **Fix** (MAGPIE `e98a5244`, pinned): it quotes nothing.
With it: whole-number settings are refused past an `int`'s range rather than
truncated (`maxtasks 4294967296` read as 0, "no limit"); a UTF-8 byte-order
mark is skipped (it made `server` an unknown setting); a comment holding
`apikey bt_` is refused (a key appended to a last comment line was swallowed
and the run went anonymous — narrowed in 17.7, and replaced by a warning in
pass 18); an `apikey` holding a space is refused; a
setting's name in the wrong case is shown, saying settings are lowercase; the
fixture README says which files birdtest's CI replaces. **Verified:**
`test_client_state` fails on the committed parser and passes; MAGPIE's
`contribute`, `config` and `layout` pass.

### 17.6 Low findings

**Fixed:**
- Infrastructure: variables refuse what would break an apply or the stack —
  `db_backup_retention_days` outside 1–35 (0 turns point-in-time recovery
  off), `backup_retention_days` of 30 or less (before the Glacier move),
  `backup_object_lock_days` and `backup_dump_jobs` below 1, a negative
  `desired_count`, and `name_suffix = "-backup"` (the one suffix that rebuilds
  a production name, `birdtest-backup-task`; checked against every name the
  stack builds); RUNBOOK §2.1 no longer offers reading another bucket, which
  the ops role cannot.
- `runbook-check.sh`: a closer indented past its opener is refused; code (not
  comments) must be printable ASCII — a no-break space after a backslash
  ended the command; `sed -i` replaced for macOS; the header says a `<pre>` or
  comment can hide a fence.
- The nightly's image build uses the runner's four cores.
- MAGPIE (above): the byte-order mark, ranges, comment and case messages.
- Admin job page (above): the export's 404 clears its error, a deleted job
  stops retrying, stale notices are cleared.

**Recorded:** KL-64 (trust policies without a source condition; the builder
given `SESSION_SIGNING_KEY` only for the shared configuration); KL-65 (S3
replication has no metrics or failure alarm); KL-68 (`server` not held to a
URL's shape; a NUL byte hides the lines after it). **Unconfirmed, left:**
whether an ECS Exec session sees the container's environment (RUNBOOK §2
relies on it); whether ACM ever reports a failed certificate with a pending
domain (the ACM loop would then wait its ten rounds).

### 17.7 Adversarial check of the pass's fixes

**1 medium, fixed.**

- **MAGPIE refused a key commented out on purpose.** 17.5's rule refused
  any `#` line holding `apikey bt_`, so `# apikey bt_old (laptop,
  deactivated)` — an ordinary edit, and PLAN says `#` lines are ignored —
  stopped the run; and it still missed the case it was written for when the
  key had `dev.py`'s aligned spacing (`apikey   bt_…`). **Fix** (MAGPIE
  `f3fc1927`, pinned): only `apikey` glued to the text before it, then blanks
  and `bt_` — what appending the setting to a last comment line with no
  newline makes — is refused. **Verified:** the new cases (a glued key with
  either spacing refused; three commented-out keys loaded, no key set) fail
  on `e98a5244`'s rule and pass.

**Lows fixed:** RUNBOOK §5 says `put-parameter` creates the parameters and §6's
teardown deletes them (Terraform no longer does, and the drill's session key
may be production's); `rds.tf`'s comment; README names
`terraform.tfstate.backup` among the state copies that hold the old values;
the checker's refusal of an over-indented closer says to indent it as its
opener; a deleted job's page disables its actions; MAGPIE's range refusal
says the range. **Recorded (KL-86):** while an export read fails its poll and
the retry both ask; a poll already out can briefly restore "Building…".
**Held:** the built ARNs match the provider's (a mock-provider test, and a
plan after migration showing only "no longer managed"), including the DR
copy; `removed` needs Terraform 1.7 and the repo pins 1.9; no default or
documented value fails the new validations; E-11 and E-11b pass repeatedly and
fail on the committed page; a deleted job stops retrying; MAGPIE's range check
takes `+5`, `007` and `2147483647` and refuses `0x10`, `1e3` and
`2147483648`; every real contributor's settings file loads.

### 17.8 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **594 of 594**.
- **MAGPIE's suite** at `e98a5244`: **70 of 70**; at `f3fc1927`, `contribute`,
  `config` and `layout` pass; `format.py` passes.
- **Tier 6, natively, every case**: passed at `e98a5244` and `f3fc1927`.
- **Tier 5, natively: 13 of 13** on the final working tree; `E-11` and
  `E-11b` fail on the committed page, and pass four times over.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 119 of 119.
- `terraform fmt -check`, `validate`, and each new validation refusing its
  bad value while the defaults pass; the state migration replayed against a
  local SSM emulator.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks.

## Pass 18 — follow-up pass

**Plan.** The diff since the previous pass's base (`20a27fd..ec43aef`), and
MAGPIE's `3565279b..f3fc1927`, one reviewer per part: frontend (tier 5
natively); MAGPIE; infrastructure, docs and scripts. Plus one area not examined
in this run: **the job statistics** — the SPRT, the pentanomial, how results
become counts, the finish check, and what the job page shows.

**Findings: 1 high, 3 medium** from the four reviewers (frontend 1 medium, 6
low; MAGPIE 2 medium, 6 low; infrastructure, docs and scripts none, 11 low, 1
unconfirmed; statistics 1 high, 6 low, 1 unconfirmed). All fixed; MAGPIE's
comment rule, patched in 17.5 and 17.7, is redesigned rather than patched a
third time.

### 18.1 High — a `games` job at the default batch gave player 1 every first move, and SPRT passed identical players (statistics reviewer)

MAGPIE gives player 1 the first move in a run's first game and alternates, and
every task is a run of its own. At the default `games_per_batch` of 1 player 1
moved first in every game of the job. **Shown** with MAGPIE's own executor
path on the seeds birdtest dispatches, two identical players: 3,000 one-game
tasks scored 0.560 for player 1 (+41.9 Elo), where two-game tasks scored 0.490;
walked through birdtest's LLR at ±10, the job passed H1 after 358 games. Any
odd batch leans the same way, by less (+13.9 at 3, +8.3 at 5). Game pairs were
unaffected. **Fix:** a `games` job's batch must be even, and defaults to 2;
the form steps by two and says why; PLAN explains it, and KL-87 gives the query
that finds jobs made before with an odd batch, whose verdicts are biased.
**Verified:** `a_games_batch_must_be_even` fails on the committed validation
and passes; E-5 and tier 6's cases now use even batches.

### 18.2 Medium — MAGPIE's comment rule refused ordinary comments and missed the key it was for (MAGPIE reviewer)

17.7's rule refused a comment holding `apikey` glued to other text before
`bt_` — `# paste yours as "apikey bt_..." below`, `# old key:apikey bt_x` —
though PLAN says `#` lines are ignored; and it missed a key appended to a
comment ending in a blank, a tab, a bare `#` or a banner, which then ran
anonymous. The rule's third fault, so the approach changed. **Fix** (MAGPIE
`65ad3348`, pinned): no comment is refused; one holding `apikey`, blanks and
`bt_` is recorded, and a run with no `apikey` prints that line's number after
its "contributing to …" line (as a new or a returning anonymous worker), never
the key. PLAN's settings
section states it and the rules the parser keeps (lowercase names, the range,
a key's shape, the BOM). **Shown:** with `f3fc1927` the doc comment is refused
and the banner case silent; with `65ad3348` both load and name their line.

### 18.3 Medium — a job deleted while its admin page was open kept its actions (frontend reviewer)

Only a failed read set the page's "gone"; a job deleted by another admin went
on offering every action, each answering "no such job", though KL-86 said a
deleted job disabled them. **Fix:** a 404 from any read, action or the stream
(`subscribeToJob` gained an `onRefused` callback) marks the job gone: the page
says so, stops its retries and polls, and disables its actions. **Verified:**
a new `E-11c` deletes the job through the API while its page is open, then
clicks Deactivate: it fails on the committed page and passes; the sse unit
test for `onRefused` passes.

### 18.4 Low findings

**Fixed:**
- Statistics: Elo hypotheses are bounded to ±1000 (past it both are an
  expected score of 1 and the job runs to its cap); the job page says "LLR x,
  bounds [lo, hi]" rather than "within" when x is outside, and stops saying
  "not acted on until N" once N are in; PLAN no longer credits fishtest with
  the exact approximation.
- MAGPIE: an `apikey` is `bt_` then letters, digits and underscores (a
  no-break space made a key the server refused); the range refusal names the
  range; a negative `maxtasks` is tested as refused without its value; the
  fixture README rewrapped.
- Admin job page: after starting an export, a failed read is the export's to
  retry, not the action's error; a job that never existed shows no
  "Loading…"; an inactive job's allocation reads "Set: 7% (offered to nobody
  while inactive)"; E-11b asserts its held read carried the old status, with a
  wider margin.
- `runbook-check.sh`: a backslash before blanks and a comment is refused; the
  header's HTML-block and quoted-`#` limits are stated plainly.
- Docs: README says a missing parameter's backup fails before starting (only
  the staleness alarm reports it), that `removed` works on an apply (a
  `destroy` first deletes), and to rotate after the migrating apply; RUNBOOK
  §6 names a drill's `GITHUB_TOKEN` parameter; PLAN's tree and KL-86 wording;
  the `name_suffix` message names `-backup` alone; 17.5 notes it was
  superseded.

**Recorded:** KL-87 (games jobs made with an odd batch; the normal
approximation's LLR in near-zero-variance samples; type I error with no
minimum and wide bounds); KL-86's deleted-job wording.
**Left:** the checker's refusal of a closer indented 1–3 columns and of
non-ASCII data in a heredoc (both fail safe); ARNs in `ecs.tf` and `backup.tf`
hard-coding the `aws` partition.

### 18.5 Adversarial check of the pass's fixes

**1 medium, fixed.**

- **MAGPIE's test did not compile in the dev build.** `65ad3348`'s new loop
  declared a second `contents` inside `test_client_state`; the dev build's
  `-Wshadow -Werror` refused it, so `make magpie_test` — what birdtest's
  contract job and MAGPIE's CI build — failed at the pinned commit. This
  audit's MAGPIE runs had built `no_pgo_release`, which lacks the flags.
  **Fix** (MAGPIE `8f2f5d75`, pinned): the variable renamed. **Verified:**
  `make magpie_test BUILD=dev` builds, and `magpie_test contribute` passes
  under its sanitizers; the dev build is now part of every MAGPIE check here.

**Lows fixed:** after starting an export the button stays off until a read
shows it (a failed read left it on, and a second click got a 409); a job found
gone clears the last action's notice and the export error; a completed job's
allocation reads "Was: 4% (completed)"; the job form's Elo inputs carry the
±1000 limits; the ±1000 comment gives the right reason (saturation begins near
6,400 Elo; the bound is policy); TESTING's `I-JOB-1b` says the bound covers
game pairs too; PLAN names an empty `apikey` and points at the even-batch
explanation; the checker's header names the quoted `\ #` it refuses.
**Held:** an even batch is balanced whatever the threads, capture or
redundancy, and a short task cannot count (a games result must carry exactly
`games_per_batch` games) — measured with MAGPIE, identical players: batch 1
0.561 (+42 Elo), batch 2 0.496, batch 4 0.503; rating pools read only
game-pairs jobs, so odd-batch games jobs never reached a rating; no API caller
makes an odd games batch (some scheduling-only test fixtures insert one
directly, which the schema's default allows, KL-87); switching the form's type to games bumps an odd
batch; no fixture or test has |Elo| past 1000; E-11, E-11b and E-11c passed
six times each; a job deleted with no click is shown gone within seconds; the
commented-key warning prints once, never the key, and not when a key is set.

### 18.6 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **596 of 596** (two new: `a_games_batch_must_be_even` and
  `elo_hypotheses_past_a_thousand_are_refused`, each failing on the committed
  validation).
- **MAGPIE's suite** at `65ad3348` (release): **70 of 70**; at `8f2f5d75`, the
  dev build and its `contribute` pass; `format.py` passes.
- **Tier 6, natively, every case** at `65ad3348` (the binary `8f2f5d75`
  builds is the same): passed.
- **Tier 5, natively: 14 of 14** (`E-11c` new; `E-8`'s pattern follows the
  job page's "bounds" wording); `E-11c` fails on the committed page.
- `npm run check`: 0 errors, 0 warnings; `npm test`: **120 of 120** (the sse
  `onRefused` test new).
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks.

## Pass 19 — follow-up pass

**Plan.** The diff since the previous pass's base (`ec43aef..b569b6d`), and
MAGPIE's `f3fc1927..8f2f5d75`, one reviewer per part: backend and frontend
(tier 5 natively); MAGPIE (both build flavours, and birdtest's CI contract job
replicated); docs, scripts and infra. Plus one area not examined in this run:
**task dispatch and allocation** — how a claim picks a job and a task.

**Findings: 0 high, 2 medium** from the four reviewers (backend and frontend
none, 5 low, 1 unconfirmed; MAGPIE none, 4 low, 1 unconfirmed; docs, scripts
and infra none, 6 low; dispatch 2 medium, 5 low, 1 unconfirmed). Both fixed or,
where the fix is a design of its own, stated and recorded.

### 19.1 Medium — a newcomer joined level with a job that had stopped moving, and took every claim (dispatch reviewer)

`join_at_parity` put a newly activated job level with the *lowest* ratio among
jobs being served. "Served" — a claim within the heartbeat timeout — is not
"keeping pace": a games job at its cap reissuing one lapsed task is served and
stands still, and after a quiet spell the fallback took the lowest of every job
on offer, often one nobody could run. **Shown** with the real
`scheduler::claim`: beside a veteran at 40% and such a capped job, a newcomer at
20% took the first 500 of 900 claims in a row (a fair split is 600 : 300); after
ten quiet minutes, 12 of 12. **Fix:** the newcomer joins level with the
*leader* — the highest ratio among jobs served, and among all in the quiet
fallback — so it is never ahead of anyone being served; and every job's lag is
bounded (19.4, where the first form of this fix was found to starve a
newcomer). **Verified:** both cases added as tests (`I-SCHED-3c`,
`I-SCHED-3d`) fail on the committed code and pass; the three earlier parity
tests still pass. PLAN's workflow text says so.

### 19.2 Medium — allocation is a share of claims, and PLAN called it a share of the fleet (dispatch reviewer)

Every claim counts one, whatever its task costs, and task sizes differ by
orders of magnitude across job types at the form's defaults. **Shown** with the
real `scheduler::claim` in simulated time: two jobs at 50/50 with tasks of 30
and 1 time units split claims 750/750 and worker time 96.8% / 3.2%. Weighting
the counter by an estimated task cost is start-time fair queuing with packet
lengths, a design of its own. **Fix:** stated — PLAN's schema comment and design
table say a share of *claims*, the admin page's field reads "Allocation % of
claims" and explains, and KL-88 records the gap and the options.

### 19.3 Low findings

**Fixed:**
- Dispatch: an `unsupported_jobs` list past 200 keeps the newest entries
  (MAGPIE appends and never prunes; keeping the first dropped the live ones —
  `A-WORKER-3` rewritten, failing on the committed code); PLAN and the code
  say expiry repeats `release_claim`'s formula rather than calling it.
- Admin job page: an action's 404 is confirmed with a read before the job is
  called gone; a reload older than an export start no longer puts "Export
  results" back; the export poll stops once the job is gone; the public job
  page says when the SPRT minimum is reached.
- `runbook-check.sh`: its grep and sed run in the C locale (an invalid byte in
  a comment after `\ ` passed under UTF-8); the header names every `\ #` it
  refuses.
- Docs: PLAN's example key is an obvious placeholder; `sprt.rs` no longer
  credits fishtest with the approximation; KL-78's quote; KL-87 notes the
  schema's column default; 18.2, 18.4 and 18.5 wording.

**Recorded:** KL-88 (allocation by claims; the generation-0 build with no
backoff; the decline skip covering MAGPIE's `stop`; a transient template error
parking a job). **Left:** MAGPIE's refusal message saying "letters and digits"
where it takes underscores too (to go with the next MAGPIE change); the dev
build's `-Wshadow` is not in this audit's release-only MAGPIE runs before pass
18 (it is now).

### 19.4 Adversarial check of the pass's fixes

**1 medium, fixed by redesign.**

- **Joining at the leader starved a newcomer in a split fleet.** With some
  workers able to run only one job (an older MAGPIE floor while a release rolls
  out), that job climbs past its share and leads, while the job the rest run
  lags it without limit; a newcomer joined at the leader got none of the next
  1,000 claims, for about 1.14 times as long as the split had lasted, and a
  newcomer only a minority can run fared the same (shown with the real
  `scheduler::claim` and the activation route). The committed rule (join at the
  lowest) was fair there and unfair in 19.1's case; neither join point is
  right while a job's lag is unbounded. `join_at_parity`'s third correction,
  so the scheduler changed instead: **every claim lifts any active job lagging
  the one it is from by more than a window — 400 claims of the whole fleet, in
  ratio units the same for every job — to that window** (`scheduler::bound_lag`,
  in the claim's transaction after the job row's update, skipping a row
  somebody holds, so a claim never waits on another job). With lags bounded,
  joining at the leader costs a newcomer at most the window. A first cut
  counted the window in each job's own claims; in testing it pinned lagging
  jobs at different distances behind the leader and the smallest allocation
  took every claim, so the window is in ratio units. **Verified:** the
  adversary's two cases added as tests (`I-SCHED-3e`, `I-SCHED-3f`: in the
  split fleet the newcomer at 40% gets 262 of the majority's 700 claims, fair
  being 311 less the lagging job's window; the minority newcomer 81 of 200).
  `I-SCHED-3c` and `3d` fail on the committed code (join at the lowest), `3e`
  and `3f` failed on join-at-the-leader alone (the adversary's run), and all
  four pass with both; the 145 scheduler, admin, worker and boundary tests
  pass; tier 6 passes. PLAN's workflow text, the schema comment and KL-88 say
  it.

**Lows fixed:** the `join_at_parity` doc, PLAN's activation and
`claims_baseline` text and a test's doc no longer say "lowest"; `worker.rs`
and PLAN say the newest 200 unsupported entries are kept; an export poll that
finds the export gone asks whether the job is; an older read's export error no
longer lands after an export start; the allocation note is visible text, not
only a tooltip. **Recorded (KL-88):** MAGPIE keeping a re-declined job's first
place in its list.

### 19.5 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **600 of 600** (four new scheduling tests; `A-WORKER-3` rewritten).
- **Tier 5, natively: 14 of 14**; **tier 6, natively, every case** (the lag
  bound runs in every claim of both).
- `npm run check`: 0 errors, 0 warnings; `npm test`: 120 of 120.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks.
- MAGPIE unchanged this pass (8f2f5d75 checked by its reviewer in both build
  flavours, with birdtest's CI contract job replicated).

## Pass 20 — follow-up pass

**Plan.** The diff since the previous pass's base (`b569b6d..c95c7d7`), one
reviewer per part: the scheduler and backend; the frontend, docs and scripts
(tier 5 natively). MAGPIE is unchanged. Plus one area not examined in this run:
**exports and the results stream** — the admin's bulk read of a job's corpus.

**Findings: 2 high, 1 medium** from the three reviewers (scheduler and backend
2 high, 5 low, 4 unconfirmed; frontend, docs and scripts none, 9 low, 1
unconfirmed; exports 1 medium, 5 low, 1 unconfirmed). All fixed: the two highs
by removing pass 19's lag bound, and the design that replaced it was redesigned
after each of two adversarial checks (20.4, 20.5).

### 20.1 High — the lag bound scrambled the jobs that lagged together (scheduler reviewer)

Pass 19's `bound_lag` set every job lagging the claimed one by more than the
window to exactly the window, which erased the jobs' positions relative to
each other; ties then went by `created_at`. **Shown** with the real
`scheduler::claim`: beside a job at 10% half the fleet can only run, two jobs
at 45% the other half runs split that half **978 : 22** (alternating claims;
654 : 347 random), and at 10% / 80%, 947 : 53 where 111 : 889 is fair. Without
the bound, the same runs split exactly by allocation.

### 20.2 High — the lag bound made a concurrent burst to a small job permanent (scheduler reviewer)

Concurrent claims read the same candidate order and land on the lowest job;
for a job at 1% each claim is a whole ratio unit, so a burst of five put it
past the window, and the bound then forgave the other job the catching up that
paid the burst back. **Shown:** 32 workers, 3,000 claims, a 1% and a 99% job:
the 1% job got **83 and 87** where 30 is fair (31 and 30 without the bound).

**Fix for 20.1 and 20.2 (first design):** `bound_lag` removed; a job joins at
the *lowest* ratio among the jobs being served, measured within the heartbeat
timeout of the latest claim rather than of now; a job passed over for want of
a task is lifted level with the job claimed (`lift_passed_over`); a job
unserved for a heartbeat timeout rejoins at parity on its first claim back.
The adversarial check broke this (20.4); the design as committed is there.
**Verified:** both reviewer cases added as tests, `I-SCHED-3h` (978 : 22 on
the pass-19 code, 500 : 500 now) and `I-SCHED-3i` (64 to 68 on the
pass-19 code in this pass's runs, 83 and 87 in the reviewer's; 31 or 32 now);
`I-SCHED-3g` added (a job nobody could run for an hour came back and, under
the bound, took all of the next 40 claims — the veteran none; 20 now); `I-SCHED-3c`
rewritten so its state is reached by real claims (its planted state is not
reachable once a capped job is lifted as it is passed over).

### 20.3 Medium — a results stream cut off by the database looked complete (exports reviewer)

`GET /api/admin/jobs/:id/results/stream` sends its `200` head before the first
row. A corpus query the database ended part-way (a failover, an operator's
`pg_terminate_backend`) was logged and the body ended normally, so curl and
browsers reported a finished download of a short file; a pool that stayed full
for the acquire timeout gave an empty one; a completed leave job whose settle
failed was streamed without its unmerged results. **Shown:** 869 of 150,000
records, then a clean end (over real HTTP, 3,928, then a clean end; an empty
`200` after 30 s with the pool held). **Fix:** the settle and the connection
are taken before the response head, so either failure is a status (`503` when
busy); a query that fails part-way yields an error, which cuts the body off
without its closing chunk. PLAN's endpoint table says a stream is complete
exactly when it ends cleanly. **Verified:** `I-EXPORT-10` fails on the
committed code and passes; over TCP, reqwest reports an unexpected EOF and
curl exits 18 (the adversary's run).

### 20.4 Adversarial check of the pass's fixes

**1 high and 1 medium, fixed by a redesign.**

- **Joining at the lowest served handed a newcomer the majority's claims.**
  A job only a minority can run (a MAGPIE floor during a rollout) lags while
  served, and is the lowest served; a newcomer everyone can run joined level
  with it and took every claim of the majority until it had caught the
  majority's job — the majority job's first claim came **331st** after 3,000
  claims of history, 661st after 6,000, so without bound in production; an
  allocation changed from 20% to 19% did the same (476th), and so did a job
  returning after an hour nobody could run it (331st). With 3e and 3f, this
  shows no single join point is right in a split fleet: each class of workers
  has its own pace. **Fix:** joining is in two steps. A job still joins at the
  lowest served ratio, below every class's pace, so it cannot be starved; and
  for an hour after joining (`JOIN_SETTLE`, from `activated_at`, which every
  join now sets — activation, purge, and a return from a spell unserved) each
  claim of it lifts it level with the claiming worker's next candidate, less
  one of that job's claims. The first claim from each faster class settles it
  at that class's pace; a structural lag is never lifted, since within the
  class that runs it a lagging job keeps pace. (Lifted to the next candidate
  exactly, a newcomer lost every tie and got 4 of 12 at 50/50; hence one claim
  short.)
- **The pass-over lift overshot by a claim of the job chosen.** Lifted to the
  chosen job's ratio after its claim, a 50% job passed over while a 1% job was
  claimed was a whole ratio unit ahead and waited **49** claims when its work
  came back. **Fix:** lifted to the chosen job's ratio before the claim, from
  the candidate list — start-time fair queuing's virtual time.

**Verified:** the adversary's cases added as tests — `I-SCHED-3j` (the
newcomer), `3k` (the allocation change), `3l` (the return), `3m` (the
overshoot) — each fails on the first design (331st, 476th, 331st, 49) and
passes (3j and 3l: the majority job's first claim within twelve, and 666 of
800 where 667 is fair; 3k: 542 where 542 is fair; 3m: P's first at once). `I-SCHED-3n` added (a job
held for 200 claims does not take the claims after in a row). Each rule
switched off in turn: the settling fails 3j, 3k and 3l; the rejoin 3g and 3l;
the pass-over lift 3n; the lift and the settling together 3c. PLAN's parity
paragraph, the schema comment and the `claims_baseline` and `activated_at`
docs say it; **KL-89** records the gaps left: a class that makes no claim
within the hour, and a lift from a pass that is one worker's alone.

**Lows fixed:** `I-SCHED-3i`'s bound tightened from 62 to 45 (the pass-19 code
gave 64 and 65, too close); PLAN says "any *other* job". An export whose
failure is ambiguous — the ready update committed and only its answer was lost
— keeps its objects: cleanup checks the row first. **Checked and holding:**
32 workers over 12 jobs, 2 capped, 1,920 claims in 4 s with no errors and no
deadlocks; the new tests stable over three runs; RUNBOOK §2.3's statement
equal to `join_at_parity` (also checked here with three fleets); the settle
before the stream's head holds nothing it did not hold before.

### 20.5 Second adversarial check

**2 high and 1 medium, fixed; the design's fourth form.** Each was shown with
the real `scheduler::claim`, and each went away with the settling switched off.

- **Settling forgave a burst's payback again.** Every job is settling for an
  hour after an activation, an allocation change or a return; a concurrent
  burst to a 1% job left the 99% job more than a claim behind it, and each
  claim of the 99% job lifted it level: the 1% job got **307 to 324** of
  2,976 where 30 is fair (I-SCHED-3i missed it: its jobs are never
  activated). The root is the burst: concurrent claims read the same list
  and all land on its first job. **Fix:** each claim is checked for its turn
  while it holds the job's dispatch lock, before the dispatch does any work —
  its ratio must not be more than one of the rival's claims past any of the
  worker's other candidates still in play; if it is, the worker goes on to
  the next, and a claim that finds every job with work outrun or busy reads
  the list again (up to eight times). A dispatch-lock timeout is now `Busy`,
  not "no work": it is not lifted as passed over. On the way: checked at the
  job's row after the dispatch, the losers held the lock for a whole dispatch
  and 32 workers on two jobs got 1,396 tasks of 1,920 in 27 s; checked
  against the next candidate only, a worker that found its first job outrun
  went on to its last and checked nothing (188 to 196); falling back to an
  outrun job unchecked, two jobs each outrun by the other handed one a claim
  anyway (175); with no slack, 1,232 of 1,920. Now 30, and 1,919 or 1,920 of
  1,920 on two jobs, as on the committed code.
- **A data split starved a newcomer after one decline.** The server cannot
  filter a data gap: each majority worker was issued one claim of a job only
  the minority had the data for, and that claim settled it at the majority's
  pace, past the minority's own lagging job: **none** of the minority's 400
  claims. **Fix:** a decline that says the worker cannot run the job at all
  (`missing_data`, `magpie_version`, `unknown_job_type`, `derived_mismatch`)
  undoes the settling within the hour (`scheduler::unsettle`): 81 where 80 is
  fair.
- **Settling against the next candidate skipped a job that paused.** The pace
  was the next job in the list, so with the minority's own job paused for one
  claim, a newcomer beside it was settled against the majority's job: none of
  the next 200. **Fix:** the pace is the lowest of the worker's other
  candidates, those just passed over included, less one claim of it and one
  of the job's own.

**Verified:** the three cases added as tests — `I-SCHED-3o` (the burst,
through the endpoint), `3p` (the data split, declined through
`/api/worker/decline`), `3q` (the pause) — failing on the design before (317,
0, 0) and passing (30; 81 in the adversary's form of 3p; 99). With each rule
switched off in turn, its own tests fail: the settling 3j, 3k and 3l; the rejoin 3g and 3l; the pass-over
lift 3c and 3n; the turn check 3o (and 3i, in about one run in four); the undoing on a decline 3p; the
lowest-other pace 3q. A 12-job stress with 2 capped jobs: 0 errors, 0
deadlocks, the same share of tasks as the committed code. **Lows:** a
minority-only newcomer beside an older job gets half its claims in its first
hour when the minority's claims come in pairs (119 of 200, then 200) —
recorded in KL-89 with the gaps settling leaves: a class that makes no claim
within the hour, a worker that cannot run a job and says nothing, and a lift
from a pass that is one worker's alone.

### 20.6 Low findings

**Fixed:**
- Scheduler: the lift takes `FOR NO KEY UPDATE` (not `FOR UPDATE`, which
  conflicts with every in-flight claim's key-share lock) and writes only a
  lift; the `claims_baseline` doc; a test's doc naming a constant that did not
  exist. (The migration's schema comment says "lowest", which is true again.)
- Exports: a multipart upload that fails to complete is aborted; an export
  that fails after writing its objects removes them (`I-EXPORT-11`, failing
  on the committed code with both objects left); the `429` past the
  two-stream cap says so; the admin page shows each object's SHA-256.
- Docs and scripts: RUNBOOK §2.3's baseline statement is the new rule;
  `admin.rs`'s activation comment; `runbook-check.sh`'s header puts what it
  refuses after what it cannot see; the design table credits the lift too;
  `api.ts` calls allocation a share of claims; pass 19's summary says KL-88
  was added; `split_run` lost its unused return.

**Not changed:** export download links are fetched once and the page says
they last an hour (a page left open offers dead links until reloaded; KL-72
covers the credential lifetime); a long export or slow stream holds one
snapshot throughout (admin-started, capped at two); a failed multipart
completion that in fact succeeded server-side, and an export aborted at its
six-hour limit, leave objects to the thirty-day rule.

### 20.7 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **613 of 613** (eleven new scheduling tests, `I-SCHED-3g` to `3q`, with `3c`
  and `3d` rewritten; two new export tests).
- **Tier 5, natively: 14 of 14**; **tier 6, natively, every case** (both run
  the final scheduler).
- `npm run check`: 0 errors, 0 warnings; `npm test`: 120 of 120.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks; RUNBOOK
  §2.3's statement run against `join_at_parity` in three fleets, equal.
- Concurrency: 32 workers on two jobs and on twelve (two capped), 0 errors and
  0 deadlocks, tasks handed out as on the committed code.
- MAGPIE unchanged this pass.

## Pass 21 — follow-up pass

**Plan.** The diff since the previous pass's base (`c95c7d7..3839333`), one
reviewer per part: the scheduler as redesigned; the rest of the backend and the
frontend (tier 5 natively); docs and scripts. MAGPIE is unchanged. Plus one
area not examined in this run: **result validation** — everything between a
worker's result and the rows it leaves, seen as a dishonest or buggy
contributor would.

**Findings: 0 high, 3 medium** from the four reviewers (scheduler 2 medium, 4
low, 2 unconfirmed; backend and frontend 1 medium, 2 low, 2 unconfirmed; docs
and scripts none, 8 low, 1 unconfirmed; result validation none, 9 low, 1
unconfirmed). All fixed.

### 21.1 Medium — equal jobs claimed together left workers idle while work existed (scheduler reviewer)

Pass 20's turn check lets a job run one of its rival's claims ahead. At equal
shares that is one claim, and claims made together kept finding each job a
claim past another; a claim that did for eight rounds answered `204`, and
MAGPIE slept five seconds. **Shown:** three jobs at 33%, 32 workers, 1,920
claims: **166** answered `204` (67 to 142 in the reviewer's runs at 50/50;
none on the pass-19 code). **Fix:** the eighth round takes the first job with
work without the check — a claim reaching it has lost seven rounds, so what
it can add to a burst is bounded. **Verified:** `I-SCHED-3r` fails on the
committed code and passes (0 of 1,920 idle, 600 to 680 each); `3i` and `3o`
still give 30.

### 21.2 Medium — a busy job cost every claim sixteen seconds and a `204` (scheduler reviewer)

A dispatch-lock timeout became `Busy` in pass 20, which sent the claim round
again, and every round waited the full two seconds on the busy job once more;
its ratio frozen, it also outran every other job, so nothing else could be
claimed. **Shown:** another holder keeping job A's lock, the third claim
waited 16 s and got nothing while job B had work (25 concurrent claims all
`Idle`, the slowest 46 s; on the pass-19 code, each gets B after one wait).
**Fix:** a busy job is left out of the rest of the request, as a candidate and
as a rival, and does not by itself send the claim round again. **Verified:**
`I-SCHED-3s` fails on the committed code and passes (three claims, each B
within 5 s).

### 21.3 Medium — through the compose proxy a cut results stream still read as complete (backend reviewer)

Pass 20 cut a failing stream off without its closing chunk. The Nginx in front
of the app (`frontend/docker/default.conf.template`, in compose and local
stacks; deployed, the load balancer sends `/api/` past it) had
`chunked_transfer_encoding off`, so it answered without a length and
closed the connection — a cut stream closed exactly like a finished one.
**Shown:** through Nginx, curl exited 0 after 15,641 of 150,000 lines, the last
line whole JSON (direct to the backend, exit 18); with the real template in
front of a server that aborts a chunked body, exit 0, and with the line
removed, exit 18. **Fix:** the line removed (`proxy_buffering off` is what the
SSE comment needed); tier 5 with it removed passes, E-4's live dashboard
included. **Verified:** `F-NGINX-1` pins the template and fails on the
committed one.

### 21.4 Low findings

**Fixed:**
- Result validation (`A-WORKER-21`, failing on the committed code): a
  capturing job's result must carry positions from every game of its batch
  (none were required); a NUL anywhere in a result's strings is a `400`, not a
  `500` MAGPIE retries; a play, and a captured position's previous play, at
  most 256 characters, a CGP at most 4,096, a bracketed tile at most 8, a
  previous play's score 0 to 100,000; job creation bounds a games batch at
  10,000 games (1,000 when capturing; pairs count two). Four tests' fixtures
  sent capture results no MAGPIE would, and now send one position a game.
- PLAN's submission steps say the result is decoded before the claim is
  locked, and KL-20 is closed; `plausibility.rs`'s comments and TESTING name
  `decode_result`; `check_batch_size`'s doc is its own again.
- The export panel's digest label does not break at its hyphen and says it is
  of the `.gz`.
- Docs of the scheduler: the schema comment, `JOIN_SETTLE`'s doc and a test's
  say the lowest of the worker's other candidates, not the next; a dispatch
  *hold*, not a held lock, is what is lifted as passed over; `jobs/mod.rs` says
  a lock timeout is `Busy`; PLAN's acquisition list names `Busy` and
  `NeedsZeroGeneration`; past tense where the lift changed things; the turn
  check's lookup is said to queue behind the job's claims; TESTING's 3i
  figure, the turn check failing 3i as well as 3o, and pass 20's record's test
  count and 3k/3l figures corrected.

**Recorded (KL-89):** the lift after a claim uses the chosen job's ratio from
the request's list, and overshoots if an admin purged or re-activated it
lower in between; settling costs a paired minority newcomer about 40% of its
first hour, not half.

**Not changed:** a stream waiting on a leave job's merge holds its permit
without a head, as it held it before (only the head is later); a result that
mentions none of a leave task's forced racks is not refused (today's MAGPIE
forces them; unconfirmed); an export row left `running` when marking it
`failed` also fails waits for a restart (unconfirmed).

### 21.5 Adversarial check of the pass's fixes

**2 medium, fixed.**

- **A busy large job handed a small one its claims, and settling forgave
  them.** 21.2's fix dropped a busy job as a rival, so with a 99% job's lock
  held the 1% job beside it took every claim of the spell unchecked — each
  worth 99 of the other's — and the 99% job's settling forgave the lead: 20 of
  20 claims during the hold, **49** of about 3,000 where 30 is fair (31 on the
  committed code). Kept as an ordinary rival, a busy job stalls the job beside
  it, 21.2 again. **Fix:** a busy job is not tried again in the request but
  stays a rival in every round, the last included, with a ratio unit of slack
  instead of one of its claims — the largest a claim can be, so a 50% job
  beside a busy one takes up to fifty claims and a 1% job one. **Verified:**
  `I-SCHED-3t` fails on the fix before it (5 of 5 during the hold) and passes
  (at most two of five, 33 at most in all); `I-SCHED-3s` still passes.
- **The new batch cap broke the contract-fixture capture.** Its heartbeat
  fixture came from a games job of ten million games, too big to finish; the
  cap refused it, and `scripts/e2e_magpie_native.sh --cases capture` failed
  before capturing it. **Fix:** the largest batch allowed, 10,000 games, with
  a simming player, so it still runs past the thirty-second heartbeat.
  **Verified:** the capture run completes (21.6).

**Held:** the unchecked last round reopened no burst in any shape tried (1%
jobs at exactly 30 with 32 to 128 workers; 1/99, 1/1/98, 1/9/90, 1/33/33/33,
equal thirds, halves and tenths; no claim idle); real MAGPIE results pass the
new checks (tier 6 with capture on games of 20 and pairs of 3: games 0..5
covered, the longest CGP 182 characters, the longest previous play 17, the
longest shipped tile `L·L`, 3); `refuse_nul` right on escapes; SSE through the
changed Nginx arrives live, and complete streams and fixed-length responses
are unchanged. **Lows fixed:** a decline naming a missing file with a NUL was
a `500` that left the claim open (now a `400`,
`a_decline_holding_a_nul_is_refused_and_the_claim_stays_declinable`); PLAN's
claim steps say eight rounds, not three, and when an `Idle` can still come
with work about; the idle figure is 67 to 166 everywhere; the `F-NGINX`
section sits with tier 1F; `check_moves` has its doc comment back.

### 21.6 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **621 of 621** (`I-SCHED-3r`, `3s`, `3t`; `A-WORKER-21` and its decline
  test; three unit tests; four fixtures completed).
- `npm run check`: 0 errors, 0 warnings; `npm test`: **122 of 122**
  (`F-NGINX-1`).
- **Tier 5, natively: 14 of 14** (its Nginx without the chunking line);
  **tier 6, natively, every case**; the contract-fixture capture
  (`--cases capture`) completes, the heartbeat fixture included.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 25 and 19 blocks.
- MAGPIE unchanged this pass.

## Pass 22 — follow-up pass

**Plan.** The diff since the previous pass's base (`3839333..1555879`), one
reviewer per part: the backend (the scheduler's busy handling and last round,
the result checks); the frontend, proxy and scripts (tier 5 natively, the
fixture capture); the docs. MAGPIE is unchanged. Plus one area not examined in
this run: **the audit log and data retention** — what is recorded, whether it
can be lost, what grows, what personal data it holds, and what the retention
loops do.

**Findings: 0 high, 3 medium** from the four reviewers (backend 1 medium, 3
low, 2 unconfirmed; frontend, proxy and scripts none, 5 low, 1 unconfirmed;
docs none, 17 low, 1 unconfirmed; audit log 2 medium, 7 low, 1 unconfirmed).
All fixed.

### 22.1 Medium — credential changes left no audit row, and a revoked key no trace (audit-log reviewer)

PLAN promises every significant account action in the log. Issuing,
suspending or revoking an API key, a password reset and an address
confirmation wrote nothing, and revoking deletes the key's row — so after a
takeover (KL-33's case: keys minted, then the owner resets and revokes)
nothing said which keys had existed or when the password changed. **Shown:**
a failed sign-in, a key issued, suspended and revoked, a reset: no audit rows
for the account, and no key row. **Fix:** `api_key.created`,
`api_key.deactivated`, `api_key.reactivated` and `api_key.revoked` (the label
in `reason`), `user.password_reset` and `user.email_confirmed`, each in the
change's transaction (`audit::log_account`); PLAN's table lists them, and says
sign-in attempts stay unlogged by design — their number is the caller's.
**Verified:** `A-ACCOUNT-8` and `A-AUTH-12` (the committed code wrote none, the
reviewer's run).

### 22.2 Medium — a full restore silently undid security actions, and RUNBOOK §1 did not re-apply them (audit-log reviewer)

A point-in-time restore brings back every credential as it was: a revoked or
suspended key authenticates again, a reset password is the old one, sessions
ended by a reset or "sign out everywhere" are valid again, bans added or
lifted are undone, a deleted account returns. The damaged instance is the only
record, and §1 retired it without a step to carry them over. **Shown** (the
restore simulated by putting the snapshot rows back): a stolen session, a
revoked key and the old password went from `401` to `200`/`204`/`200`.
**Fix:** a §1 block, before repointing, run from the ops shell: every session
generation bumped, key state and bans copied from the damaged instance
(identities the restored copy lacks skipped), the password of every account
reset since the restore point copied, and the accounts deleted since listed
for deletion again. It runs in a subshell, so a refusal stops it without
ending the ops shell. PLAN's deletion text says the nightly dumps keep a
deleted account for `backup_retention_days`, and that a restore brings it
back. **Replayed:** against a "restored" and a "damaged" database from the
template — keys, the reset, the bans and the deletion list all match the
damaged instance, a newer identity's ban is skipped; pasted twice, the same
state; with `RESTORE_TIME` or `DAMAGED_HOST` unset it refuses before touching
anything, and an interactive shell survives the refusal.

### 22.3 Medium — repeated busy spells on a settling job handed the job beside it that job's claims (backend reviewer)

Pass 21 kept a busy job as a rival with a ratio unit of slack, which bounds one
spell. But the busy job's next claim, inside its settling hour, settled it
level with the job that had run ahead — forgiving the lead — and each further
spell added another unit. **Shown:** a 10% job beside a settling 90% one took
400 of 2,400 claims over twenty spells of ten (fair 240; 240 with settling
off); 1% / 99%, 62 of 3,260 where 33 is fair. **Fix** (its second form,
22.5): a job found busy is settled a ratio unit short for ten minutes
(`BUSY_MEMORY`, an in-process map pruned on each entry) — a spell lets the
jobs beside it run a unit ahead at most, and that lead is now paid back.
**Verified:** `I-SCHED-3u` fails with the check switched off (116 of 1,109
where 110 is fair) and passes; 3s and 3t still pass.

### 22.4 Low findings

**Fixed:**
- Audit: a rating-pool removal of a config not in the pool is a `404`, not a
  logged removal and a refit (test added); a ban's reason is at most 1,000
  characters and holds no NUL (a `500` before; test added); a job the server
  completes — its stopping rule, SPRT, its last generation — writes
  `job.completed` with no actor (only the admin path did, and `jobs` keeps no
  completion time; asserted in `I-STATS-9`); KL-32's costs are the measured
  ones (a filtered page ~90 ms and the count ~70 ms at a million rows); PLAN
  says `backups.row_counts` is the one JSONB column, and that a row's
  `created_at` is its transaction's start.
- Scheduler and validation: the `Busy`, rivals and three-attempts comments;
  PLAN's claim steps (a 1% job beside a busy one takes one or two; when an
  `Idle` can still come with work about; the `Busy` bullet); the too-long-tile
  message; 32,768, not 32,767; PLAN's creation rules and KL-2 bound the games
  and pairs batches, and KL-2 says the capturing cap ignores how many plays a
  position records (a result passes 64 MiB at under 900 games recording 50);
  `decode_result`'s list names the every-game rule; `fixture_tests` says what
  it skips; the job form's batch field has a `max`; A-WORKER-21 checks the
  CGP and in-move play caps, and which rule refused each case.
- Proxy and scripts: `F-NGINX-1` refuses `chunked_transfer_encoding` off in any
  case or quoting, and checks HTTP/1.1 and no buffering inside `/api/`; PLAN
  says a cut is a failed transfer over HTTP/1.1 or later; 21.3 says the Nginx
  is compose's and local stacks' (deployed, the load balancer sends `/api/`
  past it); the capture job runs for hours, not minutes.
- Docs: KL-89's gap count; 3i fails with the turn check off about one run in
  four, not always; 3r's figure is 67 to 166; a stray blank line.

**Recorded (KL-90):** the release of an expired unconfirmed account, an
admin's recompute and leave-merge, and the bulk stream write no audit row;
a ban's `target_type` does not say which kind of identity it names. **In
KL-89:** a job whose lock stays held idles the workers behind it once they are
a unit ahead (a synthetic 40 s holder, 187 idle claims; no ordinary holder).

### 22.5 Adversarial check of the pass's fixes

**3 medium, fixed.**

- **RUNBOOK §1's block copied the damage back.** It copied the damaged
  instance's `api_keys` and `worker_bans` whole, and those hold the damage
  too: a bad migration's changes to keys or bans, or an admin account acting
  for an attacker, were redone on the restored instance. **Shown:** with a
  migration that suspended every key and dropped every ban, every key came
  back suspended and every ban gone. **Fix:** redesigned — the block is driven
  by the damaged instance's audit rows since the restore point (a migration
  writes none), exported and printed for review first (a damaging line is
  deleted before the apply), and applied from them: revoked keys removed, a
  key's last suspend or resume, reset passwords, confirmations, each
  identity's last ban or unban, and accounts deleted since shut (no password,
  no keys, not an admin) and listed by name. Every unused reset token is spent.
  Reasons travel base64, so a `\.` line cannot end a file early (it did). Both
  blocks refuse unless `DATABASE_URL` holds nothing newer than the restore
  point, which catches DNS still answering with the damaged instance. Key
  labels are no longer logged: user text, and the log outlives a deletion.
  **Replayed:** legitimate changes with a migration's damage on top (only the
  legitimate ones applied), a pruned deletion (not applied), the `\.` reason,
  `DATABASE_URL` at the damaged instance (both blocks refuse), no export
  (refused), and both pasted twice (the same state).
- **Unsettled, a newcomer found busy once took over a split fleet.** 22.3's
  first form did not settle a recently busy job at all; a newcomer is the job
  that needs settling, and one found busy once in the split of `I-SCHED-3j`
  took the majority's claims (their job's first came 331st). **Fix:** settled
  a ratio unit short instead (22.3). **Verified:** `I-SCHED-3v`, from the
  adversary, fails on the first form and passes; `3u` still passes.
- **One account could grow the audit log at request rate.** The key
  suspend/resume route has no rate limit and logged every call, changes or
  not: 8,000 rows in nine seconds. **Fix:** only a change is written and
  logged; a request that changes nothing is answered `204` with no row.
  **Verified:** in `A-ACCOUNT-8`.

**Held:** no new foreign key or lock order for the audit writes; every server
completion path logs; the busy map is bounded and safe across tests; the nginx
test and the form's `max` are right.

### 22.6 Tests

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included:
  **626 of 626** (`A-ACCOUNT-8`, `A-AUTH-12`, `I-SCHED-3u`, `3v`, a ratings
  test; the ban bound, the server completion and more A-WORKER-21 cases
  asserted in existing tests).
- `npm run check`: 0 errors, 0 warnings; `npm test`: 122 of 122.
- **Tier 5, natively: 14 of 14**; **tier 6, natively, every case**.
- `scripts/runbook-check.sh RUNBOOK.md README.md`: 27 and 19 blocks; RUNBOOK
  §1's new step replayed as in 22.5.
- MAGPIE unchanged this pass.
