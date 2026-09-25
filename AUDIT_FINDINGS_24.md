# AUDIT_FINDINGS_24 — the twenty-eighth audit (2026-09-25, eighteenth pass)

Branch `audit/birdtest-2026-09-24-pass18`, off `audit/birdtest-2026-09-24-pass17`
(`3a0d751`). MAGPIE changes are on `birdtest-contribute` at `8cc0b31c`, on top of
`ed801d83`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _23. Same severity bar as the twenty-fifth:
only real defects count as findings.
- **No real defects:** the auth, accounts and frontend reviewer, and the
  deployment reviewer.
- **Real defects:** four, from three reviewers.
  - Two are in the previous pass's own work: the `?worker=` page (1.1) and
    RUNBOOK §6 (1.3).
  - One is a MAGPIE result bug that predates this series of passes (1.2).

**Corrections to AUDIT_FINDINGS_23:**
- **§1.1:** "costs a page whoever the contributor is" held only when the
  planner chose the identity index, and the two-identity case never had that
  cost (1.1).
- **§3:** the identity indexes grow by more than their 24 bytes an entry
  suggests. The added keys defeat B-tree deduplication, since an identity's
  entries were duplicates before.
  - The storage reviewer measured 7 MB growing to 47–56 MB at two million
    claims.
  - My bench measured 145 MB at three million, against a 482 MB heap.
  - This is still small next to the heap and the record tables.

**Count: 3 code wins (1 birdtest, 2 MAGPIE), 4 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 `?worker=` could still walk: the wrong index, and a sort over both identities — **code updated** (availability; the twenty-seventh audit's fix, corrected)

**The wrong index (dispatch reviewer).** The claim range said
`state = 'completed'`. That lets the planner prove the fleet-wide
`task_claims_completed_idx` usable. The planner multiplies the identity's share
of all claims by the job's share, and for a heavy contributor on a large job
that estimate is large. It then walked every completion in the fleet, filtering
on identity and job:
- 6.2 s warm, 13.7 s cold, to return an empty page;
- 2.1 s for a contributor's own first page once another job had newer
  completions.

The route is public and unmetered, so eight such requests in a loop hold the
display pool.

On my own bench, with the same shape of data, the planner chose the identity
index even with `state`. The plan depends on the statistics.

**Fix:** the range names no `state`. `completed_at IS NOT NULL` already means
completed: only the completion sets it, together with the state, and a
completed claim never changes state. Without `state`, the partial index cannot
be proven usable, so that plan cannot be chosen at all.

**The two-identity sort (dispatch and storage reviewers).** A name that is both
an account and a pseudonym read both identities' claim ranges as one
`UNION ALL` under a single sort. Postgres did not merge the ranges in order.
- **What it did:** an Append, a sort of every claim of both identities in the
  job, and for opening racks a hash join over a sequential scan of the records.
- **Measured:** 0.6–2.5 s.
- **Who can trigger it:** anyone. Usernames can be any 3–32 characters and
  pseudonyms are public, so registering a heavy anonymous worker's pseudonym
  makes every `?worker=<it>` pay this.

**Fix:** each identity's page is built whole (its range, its records, its
order and its `LIMIT`), and the two pages are merged. That is at most twice
the page size, and exact. (The merge by name itself is the recorded design;
refusing pseudonym-shaped usernames is weighed below.)

**Measured** on a new bench:
- job A: two million games claims, 30% by one account and 10% by one
  anonymous worker, interleaved with 997 others;
- job C: a million newer claims, all by one other account;
- an opening-rack job of two million records.

| Case | Warm | Cold |
|---|---|---|
| Heavy contributor, a job they never worked | 0.14 ms | 1.6 ms |
| Heavy contributor, own first page | 1.5 ms | 52 ms |
| Heavy contributor, deep cursor | 1.2 ms | — |
| Contributor heavy elsewhere | 0.05 ms | — |
| Both identities, games | 1.0 ms | 10 ms |
| Both identities, deep cursor | 0.9 ms | — |
| Both identities, opening racks | 0.4 ms | 1.5 ms |

No plan touched `task_claims_completed_idx`.

**Tests:**
- `A-PUBLIC-3c`: the range names no `state`, and two identities are two pages
  merged under one `LIMIT`.
- `A-PUBLIC-3b` already covers the merged pages' order and paging, and still
  passes.

PLAN's cost paragraph is updated.

### 1.2 MAGPIE: a simulating opening-rack player with a `best` recorder reported the static top play — **MAGPIE updated** (wrong results)

The opening-rack executor generated each rack's candidates with the player's
own recorder. Autoplay's simulating player ignores it
(`get_top_simming_move` generates with `MOVE_RECORD_ALL`). So a `best`
simmer had one candidate. birdtest accepts that with `num_plays_recorded = 1`,
as "the best opening play for every rack". The "simulation" then ran on that
one move and reported it as the best play: `num_plies` and `num_plays` did
nothing, and nothing downstream noticed.

The reviewer ran two tasks that differed only in the recorder, on the same
racks:

| Rack | `best` reported | `all` (the simulation's pick) |
|---|---|---|
| AEGINRV | 8D VINEGAR | 8F REAVING |

**Fix:** the rack's move generation (now `config_contribute_generate_for_rack`)
uses `MOVE_RECORD_ALL` for a simulating player, bounded by `num_plays` as
before. `num_plays_recorded` still caps what is reported.

**Test:** `test_a_simulating_rack_analysis_ranks_every_play`:
- a `-r1 best` player: one move when static, eight (`num_plays`) when
  simulating;
- an empty rack is refused.

It fails with the old generation (checked by reverting it).

PLAN's recorder paragraph and `validate_opening_rack_player`'s doc comment no
longer say a `best` simmer has nothing to choose between. The refusal of `best`
with more than one recorded play stays, for static players.

### 1.3 RUNBOOK §6: the teardown could run against production's state, and failures passed as success — **plan updated** (the twenty-seventh audit's §6, corrected)

- **The workspace.** §5 step 9 leaves the `default` workspace selected, and
  the teardown never selected `dr`. Pasted as written, its apply would plan a
  whole `-dr` stack into production's state, using the scratch account's
  credentials. **Fix:** the block selects `dr`, and runs only if:
  - `workspace show` says `dr`;
  - the caller is `SCRATCH_ACCOUNT`;
  - `DR_REGION`, `THIRD_REGION` and `PROD_REPLICA_REGION` are all set;
  - the state still holds the instance. Past `destroy`, re-pasting the block
    would build the copy again.
- **Failures.** A failed listing counted as an empty bucket: an unset region
  or `NoSuchBucket` gave rc 0, and `destroy` ran. The unnamed
  `PROD_REPLICA_REGION` left `rb` with a usage error, which left a copy of
  users' emails and password hashes in the scratch account. **Fix:**
  - `empty` fails on a failed listing or delete;
  - every step up to `destroy` is chained with `&&`;
  - what `destroy` leaves (the final snapshot, the staging bucket, the
    workspace, `dr.tfvars`) is a separate guarded block.
- **Two accounts in one block.** Pasted whole with production's credentials,
  the staging block created the scratch bucket in production's account. The
  name is global, so the scratch account could then never have it. **Fix:**
  two blocks, each refusing unless `sts get-caller-identity` is the account
  it names. The temporary directory is made only when block 1 runs.

All seven teardown cases and five staging cases were run in an interactive
bash with stubbed `aws` and `terraform`: each guard, a failed listing, a good
run, a re-paste after `destroy`, and the after-destroy block. The `empty` loop
was run for real by the storage reviewer against MinIO: versioned and
GOVERNANCE-locked buckets, with and without the bypass.

### 1.4 Other changes

| Item | Change |
|---|---|
| `contribute.txt`'s `threads` was taken as given. Past 512, move generation's pool of per-thread generators runs out and magpie exits on its first task (the reviewer's 5,000 was OOM-killed first) | Capped at `MAX_THREADS`, as `-threads` is |
| A contributor's `-ritmmap true` stopped reaching tasks when they moved to a config of their own, so a 1.9 GB table was read into memory | The task config takes the caller's `use_mmap_for_rit`, a property of the machine rather than of a result |
| `docker/Dockerfile` built the backend without `--locked`, which CI's steps use | `--locked` |
| TESTING's tier-3 count missed `A-PUBLIC-3b`; `ci.yml`'s header said "both images" | 169 and `public_api.rs` (12); "the three images" |

## 2. Objective 3

Re-traced by the MAGPIE reviewer at `ed801d83`. Every setting that affects a
result is set from the request or reset by the executors, except the one in
1.2, now fixed. The remaining defaults are unused on these paths or only affect
printing:
- leave generation inherits a stale `use_game_pairs`, but its recorder is
  games-only and ignores it;
- the opening-rack path inherits stale display caps, which only affect
  printing.

Static games are identical at 1 and 3 threads (tallies, pentanomial,
positions). Contract fixtures are byte-identical on both sides.

## 3. Objectives 4–6

Most severe:
1. **The `?worker=` plans (1.1).** Seconds per public request.
2. **The opening-rack simulation reporting static plays (1.2).** Silently
   wrong results for any `best` simmer.

Every `task_claims` insert sets `job_id`. RUNBOOK §2.2's copy-back uses
`LIKE`/`SELECT *`, so it carries the column unchanged. The HOT claim in the
migration comment was verified.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_23 §4, unchanged.

Weighed and left (nits the reviewers listed apart):
- **Pseudonym-shaped usernames.** A username of 16 hex characters is merged
  under `?worker=` with the anonymous worker it names, so someone can have
  that worker's records listed beside theirs in the API. The merge is the
  recorded design, and 1.1 removes its cost. Refusing such names at
  registration would end it, and sits beside the carried "usernames accept
  any Unicode" item.
- **Queries that still join `tasks`:**
  - `worker_contributions`, `reclaim_expired_for` and the
    claim-holding checks could read `task_claims.job_id` directly (cleanup).
- **The leave feed** still resolves `?worker=` before ignoring it.
- **Nginx cache headers** are verified by hand each pass; tier 5 does not
  check them.
- **`magpie contribute` exits 0** after giving up on five consecutive task
  failures. That is MAGPIE-wide, since `main` never returns an error.
- **Games tasks** still print autoplay's summary line to the terminal.
- **A running ops task or the service's drain** can re-dirty a bucket during
  teardown. The note says to stop them, and rerunning recovers.

## 5. `birdtest-contribute` (this pass, `8cc0b31c`)

| Change | Why |
|---|---|
| `config_contribute_generate_for_rack`: a simulating player's candidates are every play up to `num_plays`; test | 1.2 |
| `contribute.txt` threads capped at `MAX_THREADS` | 1.4 |
| The task config takes the caller's `-ritmmap` | 1.4 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build.
`format.py` and `find_circ_deps.py` pass on a clean archive. Tier 6
(`M-1`…`M-11`) passes on the release build.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified by the reviewers:**
  - **Account flows end to end:**
    - register, confirm, login and logout;
    - password reset;
    - session revocation;
    - API keys;
    - every admin guard and CSRF check;
    - the frontend's error handling.
  - **The Nginx headers** on every path, including 304 revalidation.
  - **Terraform** `validate` and `fmt`.
  - **The ECS task definitions** against the backend's environment.
  - **The editing of `0001` in place** is right while nothing is deployed
    (README says so).
  - **The claim, submit, decline and heartbeat paths** under their lock
    order.
  - **MAGPIE's whole contribute loop** under ASan and LSan against a fake
    server.
