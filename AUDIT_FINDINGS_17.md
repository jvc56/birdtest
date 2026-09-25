# AUDIT_FINDINGS_17 — the twenty-first audit (2026-09-25, eleventh pass)

Branch `audit/birdtest-2026-09-24-pass11`, off `audit/birdtest-2026-09-24-pass10`
(`4ec27b5`). MAGPIE changes are on `birdtest-contribute` at `c2eadc65`, on top
of `1dc9151f`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header): the remote branch is at `47b57aa`, so CI's `magpie-contract`, `images`
and `e2e` jobs, and the nightly's backend build, cannot fetch the pin until it
is pushed.

**Builds on** AUDIT_FINDINGS_7 to _16. No finding is critical. The most serious
is in MAGPIE: a malformed opening rack from the server crashed the worker, in
the release build too (1.1). Four others are in the twentieth audit's own work.

**Count: 13 code wins (11 birdtest, 2 MAGPIE), 11 plan wins, 19 unresolved
pending feedback** (18 carried, 1 new: §4).

---

## 1. Findings

### 1.1 MAGPIE: a lower-case opening rack crashed the worker — **MAGPIE updated** (memory safety; the twentieth audit's fix, finished)

The twentieth audit made undrawn racks refuse designated blanks in two callers.
It did not change `rack_set_to_string`, and the opening-rack executor draws each
rack the server sends through `draw_rack_string_from_bag`, which still used it.
A lower-case letter there counted a machine letter ≥ 128 about 260 bytes past a
stack `Rack`:
- The dev build died with an ASan SEGV that clobbered the executor's `player`
  pointer.
- The release worker died with SIGSEGV (rc 139).
- Every worker that later claimed the reissued task would crash the same way.

birdtest's own racks are upper case, so it takes a malformed or compromised
server to trigger this, which is the same threat model as AUDIT_FINDINGS_16
§1.2. The same write sits behind GCG racks, `commit`'s pass-out rack, the
autoplay leaves-count file and `convert csv2klv`.

**Fix:** `rack_set_to_string` itself answers `-1`, leaving the rack empty, for
any machine letter outside the distribution. A `Rack` cannot hold a designated
blank, so no caller wanted the old behaviour; there are no lower-case rack
literals in `src/` or `test/`. The twentieth audit's
`rack_set_to_string_undesignated` is folded back into it. Every caller already
handled `-1` except `klv_csv.c`: `csv2klv` indexed its arrays with
`KLV_UNFOUND_INDEX` for a leave that did not parse or was not in the KLV, a
SEGV. It now refuses the row with `ERROR_STATUS_KLV_INVALID_LEAVE`.
`test_a_designated_letter_is_not_a_drawable_rack` checks the opening-rack
refusal: nothing is drawn and the bag is unchanged.

### 1.2 One host could take every live stream — **code updated** (security; the twentieth audit's cap, finished)

The 2,000-stream cap was global and the route unauthenticated. One laptop (or a
few dozen HTTP/2 connections through the ALB) could hold every place, idle, and
every public dashboard got 503s for as long as it stayed connected.

**Fix:** at most 32 streams per client address, counted in a table whose guard
is held by the stream alongside the global permit. A refusal of either kind
holds nothing.

There are two tests:
- `A-PUBLIC-6c` covers the counting.
- A router test opens 32 streams from one address, is refused the 33rd, is
  served from another address, and gets a place back after dropping one. It
  fails if the handler stops holding its place for the stream's life (checked
  by mutation: the reviewer showed that `let _permit = …` in place of the
  closure capture would still have passed `A-PUBLIC-6b`).

### 1.3 Refused pages came back every five seconds — **code updated** (performance)

On any failure, `sse.ts` waited a fixed 5 s, probed `GET /api/jobs/:id` (the
full stats payload) and reopened. EventSource cannot read the 503's
`Retry-After`. At the cap, 2,000 refused pages made about 400 probes a second.
With `JOB_STATS_CACHE_SECONDS=0`, an accepted setting that skips the build
lock, each probe was an uncoalesced payload build, enough to saturate the
display pool. **Fix:** the wait doubles from 5 s to a minute, jittered down by
up to half, and an event resets it (`F-SSE-4`).

### 1.4 Unmetered main-pool lookups from anyone — **code updated** (security)

- **Worker identity:** the `WorkerIdentity` extractor looks up the API key or
  anonymous UUID on the main pool before any worker bucket can be charged,
  because the bucket belongs to the identity. A made-up key or UUID therefore
  cost one query and a 401, on the 20 connections that claims and submissions
  need. This is the failure `db.rs` says the display pool exists to prevent.
  **Fix:** `ratelimit::MissGate`. A real worker never misses; an address that
  misses 30 times in a minute is refused with a 429 before the lookup until its
  bucket refills (`A-WORKER-16`).
- **Link redemption:** `confirm-email` and `reset-password/confirm` are
  unauthenticated writes, and the reset scores a password first. Neither was
  limited. **Fix:** 20 a minute per address (`A-AUTH-11c`).

### 1.5 Other code fixes

| Item | Change |
|---|---|
| The import page stopped polling on any error, a deploy's 503 included. With no list of imports and no id shown, a staged import could not be found again; the admin re-downloaded ~94 MB | Keeps polling through 5xx, 408, 429 and network errors. Shows the import's id. Remembers the in-flight import in the browser (`localStorage`, in `try`) and resumes it on load |
| `create_player_config` never checked `cloned_from_id`: an unknown one was the generic 409 "still referenced by other records". `unknown_config` blamed the anchor for any foreign-key failure of `create_pool`'s insert | `AppError` carries the violated constraint (`db_constraint`, boxed with `db_code` to keep the error small). Each handler maps only its own key: `player_configs_cloned_from_id_fkey`, `rating_pools_anchor_player_config_id_fkey`, `rating_pool_members_player_config_id_fkey` |
| The ratings chart drew the anchor with an error bar; with no games its stored error is `f64::MAX`, drawn as ±400, which stretched the scale | No bar for the anchor; its tooltip says "fixed" |
| `azs` defaulted to us-east-1's zones, so a stack in another region that set `region` alone failed at its first subnet. RUNBOOK §5's `"<region>a","<region>b"` is wrong where accounts get other letters (ap-northeast-1 gives newer accounts a, c and d) | Unset, it is the region's first two zones that need no opt-in (`data.aws_availability_zones`). In us-east-1 that is still a and b, so existing stacks do not change |
| `db_allocated_storage` below 20 failed at instance creation, after the rest of the stack was built | Validated at plan |
| In a DR copy, the derived builder, backup and restore drill ran against the database while it was being restored. The builder failed rows whose inputs were not yet synced, and a 03:00 backup would have dumped the half-restored database as the newest | `scheduled_tasks_enabled`, used by all three schedules; §5 turns it off until step 4 |
| The CI conversion step used MAGPIE's own layout, not the one the server writes, and its `magpie builders` line checked nothing (MAGPIE exits 0 on error) | Uses `backend/src/magpie_standard15.txt`; the builder JSON is required. The temporaries check matches `.tmp` names only (`wbx` is a file mode, not a name) |

### 1.6 Plan wins

1. **RUNBOOK §5 step 1:**
   - The computed allocation was under RDS's 20 GiB for any database under
     about 12 GiB, which is the normal case for the twice-yearly drill. It now
     has a floor of 20 (and the variable validates it).
   - The Python was indented inside the list and failed with
     `IndentationError` when pasted from the raw file. It is now one line.
   - The S3 calls had no `--region`, so in a real outage the CLI asked the lost
     region first. They now take the region the replica is in.
   - AZ names now come from `describe-availability-zones`, or `azs=null`.
   - Scheduled tasks are off until step 4.
2. **RUNBOOK §5 step 3** restores the dump step 1 sized for, checked against its
   manifest's checksum. Replication does not keep order, so just after a backup
   the newest manifest can arrive before all of its dump's files.
3. **RUNBOOK §2.1** now verifies the dump's checksum the way the drill does.
   Before, a partial download restored part of a database and said nothing.
4. **RUNBOOK §2.2:**
   - The PITR branch used `python3`, which the ops image (`postgres:16`) does
     not have: it wrote an empty `SCRATCH_URL` and the ready marker anyway. It
     now uses `sed`, refuses a URL it could not rewrite, and writes nothing
     then. Tested with an encoded `'`/`@` password, a URL without a port, and
     one without credentials.
   - It now says to check the free space before copying back a large job.
5. **README's first deploy:**
   - The ACM certificate is requested and validated before the first apply.
     The HTTPS listener refuses a pending one, which left a half-built stack.
   - The eight variables without a default are listed.
   - The SES DNS records and the production-access request come right after
     the first apply; the check lasts 72 hours and access can take a day.
   - The sandbox's effect on the first admin's confirmation mail is stated.
6. **README's alert checks:** the helper is now `tfout`, not `tf`. A common
   `alias tf=terraform` in bash turned `tf() {…}` into a `terraform` function
   that called itself (the reviewer reproduced it).
7. **PLAN, rate limits:**
   - The security section said public endpoints are limited "at the Axum
     middleware layer". They are limited in handlers and extractors, and the
     read pages are not metered but isolated on the display pool.
   - The table said login is keyed on "the username tried". It now shows the
     account key, the miss gate, the link limit and the stream caps.
8. **PLAN, purge hold:** it said the purge hold lasts "exactly as long as its
   locks". Like the code comment since the twentieth audit, it now says "at
   least".
9. **Standard errors (PLAN and `bradley_terry.rs`):** "at least √2 wide" is not
   a guarantee. The factor is `√(2/(1+ρ))` for the correlation ρ between a
   pair's two games: never below 1, and below √2 when ρ > 0. The prior's
   virtual games are in the fit but not the information. Both texts now say so.
10. **PLAN's Known Limits:**
    - "A worker's identity is resolved before its rate limit is checked" is
      closed (1.4).
    - "`birdtest-contribute` has seven include cycles" was stale: there are
      none on a clean archive (the cycles are in a working tree's untracked
      `cppcheck_dir/`). It now records the one real divergence from upstream,
      `ctime.h`.
11. **PLAN and TESTING.md:**
    - PLAN: the DR rebuild's settings.
    - TESTING.md: `F-SSE-4`, `A-AUTH-11c`, `A-WORKER-16`, `A-PUBLIC-6c`, the
      anchor in `F-CHART-2`, the clone check in `A-BOUND-3`, and the counts.

## 2. Objective 3

The MAGPIE reviewer re-traced it at `1dc9151f`, and nothing changed. Every job
type resets the shared settings and applies the run's; each player's settings
are reset and then applied from the required-key lists. Opening racks copy the
player's settings into the run-wide ones with inference off. Leave generation
takes its seed and KLV from the request. This pass's MAGPIE change is to rack
parsing, not task settings.

## 3. Objectives 4–6

Most severe first:
1. **Refused pages' fixed 5 s retry (1.3).** It was a load problem of its own
   exactly when the stream cap was full, and a display-pool saturation with the
   stats cache off.
2. **Bogus-credential lookups on the main pool (1.4).** They competed with
   claims and submissions.
3. **RUNBOOK §2.3** (verified, unchanged): the single pass is 1.28 s at 500,000
   tasks against 2.13 s before, and a job with no claims still gets its row.

No storage change. Periodic jobs were re-checked; none grows without bound
beyond the recorded Known Limits.

## 4. Unresolved, pending human feedback

The eighteen carried from AUDIT_FINDINGS_16 §4, and one new:

1. **Should the backup image be mirrored for disaster recovery?** The backup,
   ops and drill tasks run `public.ecr.aws/…/postgres:16`. Public ECR is
   anchored in us-east-1; whether it can still be pulled in another region
   while us-east-1 is down is not verified here. Mirroring it with the three
   app images removes the question, at the cost of one more image to keep
   current. Recorded in PLAN's Known Limits.

Weighed and left:
- **Streams have no maximum age.** The per-address share bounds what one host
  can hold, and tabs left open on finished jobs hold at most their address's
  share.
- **Whether the ALB closes its backend connection when a browser goes away** is
  worth one check in staging. If it does not, abandoned tabs hold places until
  the keep-alive write fails, which is at most 15 s after the ALB drops them.
- **Name-spelling buckets:** a name that matches no account keeps a bucket per
  Rust-lowered spelling. `/api/users` publishes every name, so it reveals
  nothing.
- **A failed purge still bumps the witness count,** so a completion check in
  that window is wasted. The claim path completes an exhausted job anyway.
- **README step 6's `aws rds wait`** can return before the password reset
  starts. The service crash-loops for a minute and recovers on its own.
- **An image registry outside AWS** would need `repositoryCredentials`, or must
  be public.
- **`arn:aws:` is hard-coded,** so the stack does not work in GovCloud or China.
- **`builderhash` leaves its files behind on failure;** they are gitignored and
  overwritten by the next run.

## 5. `birdtest-contribute` (this pass, `c2eadc65`)

| Change | Why |
|---|---|
| `rack_set_to_string` refuses designated letters (the helper folded back into it); `klv_csv` refuses unknown leaves | 1.1 |
| `test_a_designated_letter_is_not_a_drawable_rack`; `rack` test on the core function; `test_a_leave_with_no_index_is_refused` | 1.1 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build
(`rack`, `gameplay`, `klv`, `gcg`, `cgp`, `autoplay`, `contribute`, `config` and
`builderhash` among them), including the new `klv` case. `find_circ_deps.py`
and `format.py` pass on a clean archive. Tier 6 (14 Rust tests, `M-1`…`M-11`)
passes on the release build, and the 14 Rust tests on the sanitizer build too.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - The purge witness in every interleaving of hold, witness read, claim lock,
    commit and `complete_unless_purged`.
  - The stream permit's release on disconnect, on shutdown, on a job delete's
    `close`, and on a 404 before the stream starts.
  - `add_member`'s lock order.
  - No other limiter lowers names differently from Postgres.
  - The CI step on ubuntu-latest's tools, with the block scalar stripping its
    indent.
  - The replica manifest's naming and `tail -1`.
  - §5's overrides against every region-bound value in `prod.tfvars`.
  - The KLV pins are deterministic across platforms and unaffected by
    `BOARD_DIM`.
