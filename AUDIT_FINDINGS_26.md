# AUDIT_FINDINGS_26 — the thirtieth audit (2026-09-25, twentieth pass)

Branch `audit/birdtest-2026-09-24-pass20`, off `audit/birdtest-2026-09-24-pass19`
(`ed83493`). MAGPIE is on `birdtest-contribute` at `7400184f`, a comment-only
commit on top of `913a874b`; `docker/Dockerfile` pins it. **Still unpushed**
(AUDIT_FINDINGS_8, header).

**Builds on** AUDIT_FINDINGS_7 to _25. Same severity bar as the twenty-fifth:
only real defects count as findings.
- **No real defects:**
  - the dispatch reviewer: `?worker=` at every cursor age, the unfiltered
    feeds, `set_ignore_missing`, and the claim path under concurrency all hold;
  - the MAGPIE reviewer: the thread cap held at 511 of 512 generator slots
    under its worst mix.
- **Low severity only:** the deployment reviewer and the auth reviewer.
- **Medium:** one of the storage reviewer's two, in the rollback rule written
  last pass.

The code this pass changes is small. Most of the work is in the docs.

**Corrections to AUDIT_FINDINGS_25:** §1.4's reason for the thread cap was
wrong, though the cap itself is right. A task's N autoplay workers each simulate
on one thread at a time, not on N more. With the contribute thread and possibly
the calling thread, that is at most 2N+2 generators (1.5).

**Count: 3 code wins (2 birdtest, 1 MAGPIE comment), 4 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 The additive-migration rule allowed a new enum value, which stops a rolled-back fleet — **plan updated** (medium; storage reviewer; the twenty-ninth audit's rule, corrected)

README's rule listed new tables, nullable or defaulted columns and indexes as
additive. Under that rule `ALTER TYPE job_type ADD VALUE` passes, and it is how
a new job type would arrive. But the backend reads `job_type`, `job_status`,
`task_state` and `claim_state` into closed Rust enums. Once one active job of
the new type exists, the previous image fails
`candidate_jobs`' `SELECT j.* … WHERE status = 'active'` on every claim, and the
job pages with it. So a rollback would bring back a fleet that dispatches
nothing.

**Fix:**
- README says a new enum value is not additive: ship the reading of it one
  release before anything writes it.
- The RUNBOOK rollback's first step deactivates any job using such a value.

### 1.2 The rollback's wait did not wait for health — **plan updated** (low; storage reviewer)

`aws ecs wait services-stable` succeeds once the running count equals the
desired count, which a crash-looping task meets between restarts. The service
keeps no healthy task through a deploy and has no circuit breaker.

**Fix:** the section waits on `aws elbv2 wait target-in-service` for both
target groups, with the names for §5's copy given. It says a waiter's ten
minutes equal the health check's grace, so a first timeout means nothing yet.
It also:
- asks for each release's image tags to be kept, since `prod.tfvars` holds
  only the current ones;
- says that rolling back `min_magpie_version` readmits the builds it kept out;
- says the apply keeps the checkout's task definitions.

### 1.3 Seeding an opening-rack job always failed, and the forms disagreed with the rules — **code updated** (low; auth reviewer)

- **`seed.py --job-type opening_rack`, and `dev.py` through it.** The seeded
  player was a static `best` config recording 10 plays, which job creation has
  refused since the rule was written. **Fix:** an opening-rack seed makes a
  static `all` player (`static-equity-all`); games keep `best`.
- **The job-new warning** covered only the static `best` refusal, not
  `num_plays` below `num_plays_recorded`, which is refused after submit.
  **Fix:** it warns for both, with the backend's reasons.
- **The player-config form** sent no `num_plays` for a static player, so it was
  stored as 100. A static config recording more could never be used for an
  opening-rack job, and the refusal named a fix the form could not make.
  **Fix:** a "Plays to generate (-np)" field for static players, defaulting
  to 100, with the rule stated beside "Plays to report".

From the nits: the refusal, the form and PLAN now recommend `all` for ranking,
since a static `equity` recorder keeps only the moves within its margin of the
best. PLAN's recorder paragraph and job-creation list gain the `num_plays` rule
and a static `score` player's equity (its score).

### 1.4 The down alarms page once on the apply that creates them — **plan updated** (low; deployment reviewer)

PLAN said a first apply does not page. That is true of the apply at
`desired_count` 0. The next apply creates the alarms before its task is healthy,
so they start in ALARM with no data and clear minutes later.

**Fix:**
- PLAN says so;
- README's first-deploy step 6 and RUNBOOK §5 step 4 say to expect an
  ALARM mail and then an OK;
- RUNBOOK §1, which scales the service to 0, expects the alarms too.

### 1.5 MAGPIE: the thread cap's reason — **MAGPIE updated** (comment only; MAGPIE reviewer's nit)

The reviewer traced every path and found at most 2N+2 generators:
- N autoplay workers, each with one simulation or inference thread at a
  time;
- the contribute thread;
- a calling REPL or API thread.

It measured a peak of 511 with a 255-thread game-pairs batch run after an
opening-rack task. The comment in `contribute_defs.h` now says this.

## 2. Objective 3

Re-traced by the MAGPIE reviewer at `913a874b`. With a simmer's `score` sort
refused, a player config means the same in games and opening-rack jobs:
- candidates are all plays by equity, up to `num_plays`;
- `movegen_margin` is inert under `all`;
- small plays are off;
- every simulation setting is copied;
- cutoff and bingo bonus come from the request.

The remaining differences are already documented:
- inference is off for opening racks;
- a static `equity` recorder's margin applies to opening-rack lists;
- sampling depends on the thread count;
- capture raises a simmer's plays.

## 3. Objectives 4–6

Most severe: **the enum-value rollback (1.1)**, latent until the first release
that adds a job type.

The dispatch reviewer measured `?worker=` at 2.65M claims across cursor ages
from 1970 to 2030 and page sizes of 50 and 500:
- every plan was the ordered backward scan, at 0.2–20 ms warm;
- the unfiltered feeds' custom plans use the row comparison as an index
  condition;
- `pg_prepared_statements` showed no generic plans after nine executions.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_25 §4, unchanged.

Weighed and left (nits the reviewers listed apart):
- **`reclaim_expired_for`** updates `tasks` rows in no fixed order. Two
  concurrent reclaims could deadlock at redundancy above 1 with several lapsed
  claims per task. That is logged, non-fatal, and repeated by the next claim.
- **The reset-password confirm** clears the cookie but not the session store.
  This is carried as "the session store is not cleared after a password
  reset".
- **A machine with more than 256 cores** is capped at 255 threads, and says
  only the count at start.
- **`test_a_runs_threads_are_capped`** checks the arithmetic and the clamp, not
  the 2N+2 bound itself.

## 5. `birdtest-contribute` (this pass, `7400184f`)

| Change | Why |
|---|---|
| The thread cap's comment | 1.5 |

The comment-only change was followed by `magpie_test contribute` on the
sanitizer build, a release rebuild, and tier 6 (`M-1`…`M-11`). The full
70-suite run is from the previous pass's `913a874b`; the code is unchanged
since.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified by the reviewers:**
  - **RUNBOOK §6's four blocks,** in `bash -i` against:
    - real Terraform 1.9.8;
    - AWS CLI v1 and v2 against MinIO, with GOVERNANCE-locked buckets of over
      1,000 versions;
    - every failure and re-paste case;
    - `-var azs=null` as a real null;
    - the real `DBSnapshotNotFound` and `(404)` error texts.
  - **The down alarms:**
    - their dimensions and metric set;
    - no page on a normal deploy (3–5 minutes against 10);
    - the conditional `for_each` under a mocked-provider plan.
  - **`set_ignore_missing`** skips only `VersionMissing`.
  - **No bypass of the player-config rules,** since configs are immutable
    and clones pass the same checks.
  - **Claim, submit, decline and heartbeat lock order;**
  - **Token spending** on confirm and reset;
  - **zxcvbn's input cap.**
