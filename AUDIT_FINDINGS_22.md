# AUDIT_FINDINGS_22 — the twenty-sixth audit (2026-09-25, sixteenth pass)

Branch `audit/birdtest-2026-09-24-pass16`, off `audit/birdtest-2026-09-24-pass15`
(`960e3b4`). MAGPIE changes are on `birdtest-contribute` at `d4162f8e`, on top of
`e7d04183`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _21. Same severity bar as the twenty-fifth:
only real defects count as findings.
- **No real defects:** the auth, accounts and frontend reviewer.
- **One each:** the other four areas. One of them, MAGPIE's settings restore,
  is the third correction running of the same mechanism. This pass replaces
  the mechanism rather than patching it (1.1).

**Count: 6 code wins (5 birdtest, 1 MAGPIE), 7 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 MAGPIE: contribute's tasks now run in a config of their own — **MAGPIE updated** (the twenty-fourth and twenty-fifth audits' restore, replaced)

The tasks ran in the caller's config and then tried to undo themselves: a
snapshot of `settings.txt`, replayed afterwards, a fallback when the replay
failed, and a skipped save. The reviewer reproduced two more holes:
- **The file.** A user with no lexicon, after one `contribute` against a job
  with no wordmap for its lexicon, had the fallback's state in the session. The
  next save wrote it into `settings.txt`: the task's lexicon with wordmaps off,
  or, after a leave-generation task, `-k1 <lexicon>_birdtest_<sha>`, a file
  nothing cleans up.
- **The session.** The fallback did not turn off word info tables, so a user
  with `-wit` on was left with a session that failed every command.

**Fix:** `impl_contribute` creates a config for its tasks
(`config_create_for_contribute`):
- the same data paths;
- the caller's thread control, so `stop` and the output reach it;
- settings never saved.

The caller's session and settings file are never touched, so there is nothing
to snapshot, replay or undo. The snapshot and restore functions and the
save-skip flag are gone, which removes 239 lines and adds 76.

`test_contribute_runs_in_a_config_of_its_own` checks:
- the task config shares the thread control and saves nothing;
- a task's lexicon, with every derived file asked for, leaves the caller's
  settings string unchanged;
- the caller's session still loads.

### 1.2 `?worker=` walked the whole job for someone who worked elsewhere — **code updated** (availability; a documented cost that did not hold)

PLAN said a filtered results feed costs 1–3 ms. The filtered query is planned
for the identity asked about, but the planner estimates that identity's share
of *this* job from their share of *all* claims. So a contributor who worked
hard in job A and never in job B, looked up on job B, got the heavy
contributor's plan: walk B's feed index newest first, probing each record's
claim, to return nothing. The reviewer measured 5.8 s cold (1.4 s warm) over
two million records, on a public, unmetered route with an eight-connection
display pool.

**Fix:** the contributor's claims in this job are found first, through the
identity indexes and the tasks' key:
- **None:** the answer is an empty page.
- **Up to 1,000:** the page is read through them (`task_claim_id = ANY(...)`,
  served by `position_analysis_records_claim_idx` and `game_results`' key).
- **More:** they are dense enough in the job for the walk.

`A-PUBLIC-3` covers a contributor with claims only in another job. PLAN's cost
paragraph is corrected.

### 1.3 A deploy could leave returning visitors on a blank page — **code updated** (runtime)

Nginx served `index.html` with no `Cache-Control`, so browsers cached it by
heuristic: about a tenth of the build's age, over a day for a two-week-old
build. After a deploy the cached page asked for hashed chunks the new image does
not have. `try_files` answered those with `index.html`, a 200 of HTML that
`nosniff` refuses to run as a module, and the app never started.

**Fix:**
- the pages are `Cache-Control: no-cache`;
- `/_app/immutable/` is served `public, max-age=31536000, immutable`;
- a missing asset there is a 404.

Checked in a real `nginx:1.27-alpine` serving the local build: `/` and SPA
routes `no-cache`, a real chunk cached for good, a missing chunk 404.

### 1.4 RUNBOOK §6's drill would block a real §5 — **plan updated**

§6 said to run §5 "into a scratch account or region". §5's `-dr` names are
global (buckets) or account-wide (IAM roles), so a drill in another region of
the same account holds them. The copy cannot be torn down as written:
- deletion protection on RDS;
- 30-day GOVERNANCE-locked backups;
- versioned buckets.

A real region loss then fails at step 1 on the first name. The "kept" branch
would also reuse a same-region drill's `dr.tfvars`, with its smaller storage.

**Fix:**
- Drills go into a scratch account.
- §6 has the teardown: deletion protection off, every version deleted with
  `--bypass-governance-retention`, `destroy`, the workspace deleted and
  `dr.tfvars` moved aside.
- `dr.tfvars` is kept only when both the region and the storage size match.

### 1.5 Other changes

| Item | Change |
|---|---|
| §5 step 4's apply ran even when the zone pin had failed, so `prod.tfvars`' lost-region zones applied and the subnets were replaced | Applied only with `azs` in `dr.tfvars` |
| §5 step 1's apply did not check that `dr.tfvars` was for `$DR_REGION` | It does |
| §4 check 1 used `$STAMP`, which after §1's PITR is the instance name's suffix | Said to be for a dump restore only |
| "§8's return" pointed to a section that does not exist | Step 9 |
| PLAN's performance table said the rating fit ran on every public read of a pool | Only the sweep fits; reads serve the stored run |
| CI read the MAGPIE pin without checking it; renamed, it would check out MAGPIE's default branch silently | Refused when empty (ci.yml twice, nightly) |
| Neither workflow set `permissions`; the nightly passes the token into a container | `contents: read` on both |
| `MAGPIE_THREADS` for the derived builder was `"0.5"` at 512 CPU units | Whole vCPUs, at least one |

## 2. Objective 3

Re-traced by the MAGPIE reviewer at `e7d04183`, unchanged. No task outcome
depends on local settings:
- per-player and shared resets precede every task;
- the opening-rack copy has inference off;
- leave generation's static reset, forced racks and digest-checked KLV;
- leftover options are either overridden or unused.

This pass's MAGPIE change moves where tasks run, not what they set. The task
config starts from `config_create`'s defaults, with no user lexicon or options
at all, which removes even the leftovers.

## 3. Objectives 4–6

Most severe first:
1. **The `?worker=` walk (1.2).** Seconds per request on the display pool.
2. **The blank page after a deploy (1.3).** Every returning visitor until the
   cache expired.

All of RUNBOOK §0–§4's SQL was run against a scratch database with the current
schema, and every foreign key into the copied tables is covered. No storage
change.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_21 §4, unchanged.

Weighed and left (nits the reviewers listed apart):
- **Leave seeding after a purge.** It can seed a generation a purge has
  superseded, in a sub-millisecond window, and corrupts nothing.
- **A leave cursor with an unparsable generation** keeps its rack.
- **The leave feed** resolves `?worker=` and then ignores it.
- **Frontend nits:**
  - the rating history chart draws configs removed from the pool;
  - the admin export poll stops on one failed request;
  - the audit log's Next and Previous read the unapplied filter;
  - the session store is not cleared after a password reset;
  - a failed logout leaves an unhandled rejection;
  - a stale comment in `user_census`.
- **CI hardening:**
  - `ubuntu-latest` becomes Ubuntu 26 on 2026-10-19; Playwright 1.60's
    `--with-deps` may not support it;
  - the ECS service's `depends_on` could name the listener rule.

## 5. `birdtest-contribute` (this pass, `d4162f8e`)

| Change | Why |
|---|---|
| Tasks run in `config_create_for_contribute`'s config; snapshot, replay, fallback and save-skip removed; `test_contribute_runs_in_a_config_of_its_own` | 1.1 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build.
`format.py` and `find_circ_deps.py` pass on a clean archive. Tier 6 (14 Rust
tests, `M-1`…`M-11`) passes on the release build, whose CLI `contribute` now
runs every task in its own config.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - **The leave feed's generation stepping:**
    - a page that exactly exhausts a generation;
    - an `after` rack that is its generation's last;
    - the oldest generation exhausted;
    - zero, negative and `i32::MAX` generations;
    - at most `limit + 1` round trips.
  - **The other public cursors** are single bounded keyset queries.
  - **Every dispatch and receipt race** rechecked.
  - **§5's blocks** in an interactive bash with stubs, every case.
  - **Deployment:**
    - ALB redirect and TLS policy;
    - health checks and grace periods;
    - security groups;
    - Nginx SSE settings;
    - `prod-shell.sh` and `prod-sql.sh`;
    - the base images;
    - compose `--wait`.
  - **The MAGPIE CLI path** (`contribute cfg`) with games, leave generation
    and opening racks in one run, clean under ASan with leak detection, and
    the user's `settings.txt` byte-identical.
