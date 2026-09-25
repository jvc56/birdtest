# AUDIT_FINDINGS_23 — the twenty-seventh audit (2026-09-25, seventeenth pass)

Branch `audit/birdtest-2026-09-24-pass17`, off `audit/birdtest-2026-09-24-pass16`
(`d73e67f`). MAGPIE changes are on `birdtest-contribute` at `ed801d83`, on top of
`d4162f8e`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _22. Same severity bar as the twenty-fifth:
only real defects count as findings.
- **No real defects:** the MAGPIE reviewer. Its one nit, the thread count of a
  task's derived builds, is fixed anyway (1.4).
- **Real defects:** four. The worst, from two reviewers, is in the previous
  pass's own fix: the `?worker=` claims lookup (1.1). AUDIT_FINDINGS_22 §1.2
  said that fix read the contributor's claims "through the identity indexes and
  the tasks' key", as if that were one cheap range. It was not: with no job on
  a claim, the lookup read every claim the contributor had ever made.

**Count: 3 code wins (2 birdtest, 1 MAGPIE), 5 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 `?worker=` still walked for seconds — **code updated** (availability; the twenty-sixth audit's fix, replaced)

The twenty-sixth audit found a named contributor's claims in the job first. If
there were none, the page was empty. Up to 1,000, the page was read through
them. Past that, the job's feed was walked. The dispatch and storage reviewers
measured three defects in it:
- **The lookup.** `task_claims` had no job column. The lookup joined tasks and
  filtered on their job, so it read every claim the contributor ever made:
  3–5 s for a heavy contributor, on a public route with an eight-connection
  display pool.
- **The threshold.** Past 1,000 claims the old walk came back. It costs the
  distance from the newest record to the contributor's fiftieth. 1,100 claims
  in a 1.5-million-claim job walked all of it: 12.5 s.
- **The opening-rack page.** The page query joined every one of the
  contributor's records to its best move before the sort chose fifty. That
  took about 1 s a page at half a million records.

A budget counted in records rather than claims was written first and dropped.
Any threshold between "read through a list of claims" and "walk the feed"
fails the contributors just past it. The worst case is a contributor whose
work is all early in a long job.

**Fix:**
- **The claim's job.** `task_claims.job_id` is copied from the task at claim
  time.
- **The indexes.** The identity indexes are keyed `(identity, job_id,
  completed_at)`. Only completed claims have a `completed_at`, so a
  contributor's completed claims in a job, newest first, are one backward
  index range.
- **The page.** It is read through that range, joined to each claim's records
  until full. A claim's records share its completion time, since they are
  written in the same transaction. So this is the feed's own order.
- **The cursor.** It carries the claim's time, so paging stays exact even
  where a games result's `submitted_at` falls moments after its claim's
  `completed_at`.
- **Both identities.** A name that is both an account and a pseudonym reads
  both ranges and merges them (`UNION ALL`). An `OR` would sort every claim of
  both.
- **Choosing the page.** Both opening-rack paths choose the page before
  joining moves and accounts.

The completion time adds no write cost. Completing a claim changes `state`,
which the open-claims index's predicate reads, so that update was never HOT.
A heartbeat touches neither.

**Measured** on a scratch database: 2 million games claims and results, and
3 million opening-rack records in 10,000 claims.

| Case | Before | Now, warm | Now, cold |
|---|---|---|---|
| 150,000 claims, all the oldest in a 1.5-million-claim job | 7.4 s | 1.4 ms | 30 ms |
| No claims in the job, heavy elsewhere | — | 0.06 ms | — |
| Mid-job cursor | — | 1.1 ms | — |
| Opening racks, first page and deep cursor | — | 0.4 ms | — |

The walk is the previous query shape.

**Test:** `A-PUBLIC-3b`, `the_filtered_feed_pages_through_a_contributors_claims`:
- pages of two break inside a batch of three;
- the concatenation equals the unfiltered feed restricted to that
  contributor, in order;
- an account named with another worker's pseudonym gets both workers'
  records, merged.

Every claim insert (`issue_claim`, the tests, `restore-roundtrip.sh`) sets
`job_id`. PLAN's schema copy and cost paragraph are updated, and so are the
comments that said `task_claims` has no job column.

### 1.2 A missing chunk's 404 was cached for a year — **code updated** (runtime; the twenty-sixth audit's fix, corrected)

`/_app/immutable/`'s `Cache-Control: public, max-age=31536000, immutable` was
added with `always`, so it went on the 404 for a missing chunk as well. A
browser or CDN that asked for a chunk during a deploy, before the new image
served it, kept the 404 for a year. Nginx's `add_header` without `always`
covers 200, 201, 204, 206, 301, 302, 303, 304, 307 and 308 only.

**Fix:** `always` dropped on that line. Checked in `nginx:1.27-alpine`:
- a chunk is a 200 with the immutable header;
- a missing chunk is a 404 with no `Cache-Control`;
- `/` is still `no-cache`.

### 1.3 RUNBOOK §6's drill could not be done, or torn down, as written — **plan updated** (the twenty-sixth audit's §6, corrected)

- **The scratch account could not read the source.** §5 restores from
  production's replicas. A scratch account has no read on them, and granting
  it would change production's Object-Locked buckets. **Fix:** §6 stages the
  newest dump, its manifest, and `leaves/` and `inputs/` through the
  operator's machine. They are read with production's credentials and written
  to a staging bucket with the scratch account's. The local copy is removed,
  since it holds users' emails and password hashes. §5 then runs with
  `REPLICA` pointing at the staging bucket. §5 now says `<account>` is this
  account's id. Step 6 (DNS) is skipped in a drill.
- **The listing merged every page.** `list-object-versions` paginates
  automatically, so past 1,000 versions it built one request that
  `delete-objects` refuses. Its errors were not checked. **Fix:** an `empty`
  function lists one page at a time (`--no-paginate`), stops when the listing
  is empty, and stops on any `Errors` entry, printing it. Tested against a
  stub: 2,500 versions deleted 1,000/1,000/500, an empty bucket, and a refused
  delete.
- **What destroy was left to do.** Nothing stopped the service or the 03:00
  backup from writing while the buckets were emptied. The two artifact
  buckets, which are versioned and have no `force_destroy`, were never
  emptied, so `destroy` failed on them. The final snapshot `birdtest-dr-final`
  was left behind. **Fix:**
  - first `apply` with `desired_count=0 scheduled_tasks_enabled=false`;
  - all four buckets emptied, sources before replicas, with
    `--bypass-governance-retention` only on the locked backups buckets;
  - the snapshot and the staging bucket deleted after `destroy`.

### 1.4 MAGPIE: a task's derived files were built with the wrong thread count — **MAGPIE updated** (a nit, fixed)

The task config from `config_create_for_contribute` starts at
`config_create`'s default of every core. The executors set `contribute.txt`'s
thread count, but only after a task's derived files (KLV, wordmap) were built.
So the first task's builds used every core, and later ones used the previous
task's count. **Fix:** `impl_contribute` sets the task config's thread count
before each executor.

### 1.5 Nits fixed on the way

- `MAX_CLAIMS_READ_DIRECTLY` sat between `worker_predicate`'s doc comment and
  the function. Now moot: the constant is gone.
- The claims lookup counted declined and abandoned claims. Now moot: the range
  holds completed claims only.

## 2. Objective 3

Unchanged since AUDIT_FINDINGS_22 §2. The MAGPIE change sets the thread
count, which no task outcome depends on.

## 3. Objectives 4–6

Most severe: **the `?worker=` walk (1.1)**, seconds per request on the display
pool, for a contributor anyone can name.

Storage:
- the identity indexes grow by 16 bytes an entry (`job_id`) and 8
  (`completed_at`);
- `task_claims` rows grow by 16 bytes.

`restore-roundtrip.sh` passes with the new column (41 tables, counts,
references, bytea and doubles).

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_22 §4, unchanged.

Weighed and left (nits the reviewers listed apart):
- **Nginx:**
  - `robots.txt` and `favicon` fall through to the SPA;
  - no `gzip`;
  - `absolute_redirect` is on.
- **Public stats:**
  - SPRT's label pairs;
  - the LLR's "within" wording;
  - `pool_detail`'s tiebreak;
  - `divergent_pairs` is optional in the response.
- **MAGPIE:**
  - the caller's lexicon stays loaded, in memory, while contribute runs;
  - a board-layout nit that predates this pass's change.
- **RUNBOOK:** §5 step 1 invites raising `db_allocated_storage` by hand, and
  pasted again, it then calls that `dr.tfvars` "another region or size". It
  says to move the file aside, and nothing is applied.

## 5. `birdtest-contribute` (this pass, `ed801d83`)

| Change | Why |
|---|---|
| `impl_contribute` sets the task config's thread count before each executor | 1.4 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build.
`format.py` and `find_circ_deps.py` pass on a clean archive. Tier 6
(`M-1`…`M-11`) passes on the release build.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - the unfiltered feed's page is chosen before its joins, and pages as
    before (`A-PUBLIC-3`, `the_results_feed_walks_every_row_exactly_once`);
  - the claim-walk plans at scale: a backward index range, incremental sort,
    and `LIMIT`, never the job's feed index;
  - the nginx headers in a real container;
  - §6's `empty` loop against a stub.
