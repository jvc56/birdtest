# AUDIT_FINDINGS_8 — the twelfth audit (2026-09-24, second pass)

Branch `audit/birdtest-2026-09-24-pass2`, off `audit/birdtest-2026-09-24`
(the eleventh audit, `e71f4f8`) — branched from it rather than from `main`
because `main` does not have the eleventh audit's fixes yet and this pass
re-audits them. MAGPIE changes are on `birdtest-contribute` at `dc6c4426`, on top of
the eleventh audit's `68a74611`; `docker/Dockerfile` pins `dc6c4426`. **Neither
MAGPIE commit is pushed**: the image build fetches the pin from GitHub, and CI's
e2e and nightly jobs build `birdtest-contribute`'s remote head, which reports
0.1.0 — below this branch's floor of 0.1.1 — so both fail until
`birdtest-contribute` is pushed. That is the one action this branch needs from
a person before it can merge.

**Builds on** `AUDIT_FINDINGS_7.md` (the eleventh audit, same day) and, through
it, the ten before (folded into PLAN.md at `d5ed19a`). It revisits three of the
eleventh's fixes that were incomplete (1.1, 1.2, 1.3 below) and one of its
decisions that was wrong in part (1.9: the per-(username, IP) login bucket).

**Count: 21 code wins, 5 plan wins, 7 unresolved pending feedback** (4 carried
from AUDIT_FINDINGS_7 — claim idempotency, the purge redesign, scheduling-history
retention, rating-pool edit/delete — plus 3 new).

Run the same way: five parallel read-only reviews, told to treat the previous
pass's diff with suspicion, every finding re-verified by hand before a change.

---

## 1. Code versus PLAN.md, and the previous pass's fixes

### 1.1 A purge still stalled the fleet, through contributor rows — **code updated** (race, high)

- **Code:** `release_contributions` updated every contributor's `users` /
  `anonymous_workers` row at the *start* of a purge or delete and held those
  locks through minutes of deletes. Every worker request runs an anonymous
  identity's `last_seen_at` touch (a blocking `UPDATE` once a minute), and every
  submission, for any job, bumps its contributor's counter — so the eleventh
  audit's claim-lock fix (AUDIT_FINDINGS_7 1.7) left the same outage open
  through a different table.
- **PLAN.md (as the eleventh audit left it):** "nothing else waits on the
  claims it holds"; Known Limits: "it no longer stalls the rest of the fleet".
- **Decision:** code updated to make the plan true. `Contributions::count`
  reads the per-identity totals right after `lock_open_claims` (exact: the
  dispatch lock and the locked open claims freeze the job's completed claims);
  `Contributions::give_back` applies them as the transaction's last statement.
  The anonymous and API-key touches now `SKIP LOCKED`. Test `A-ADMIN-17`.

### 1.2 Submissions during a purge still held a connection each — **code updated** (critical path)

- Each retry of a submission for a held claim waited the full 5 s
  `lock_timeout` on a connection; MAGPIE's backoff starts at a second, so a few
  hundred workers on the job held the pool. **Fix:** `DispatchHolds` gained
  kinds (`DispatchOnly` for a seeding, `Claims` for a purge/delete); a
  submission or decline looks up its job (unlocked) only while some job's
  claims are held, and is answered `503` at once. Test `A-ADMIN-16`.

### 1.3 A purge that rolled back cost the job its claims — **code updated**

- Heartbeats now skip a locked claim, so a purge that ran past the ALB's
  300 s and rolled back left every claim of the job looking lapsed, and the next
  claim request reclaimed them. **Fix:** a `Claims` hold dropped without
  `committed()` records the job in a `reclaim_not_before` map for a heartbeat
  timeout; `reclaim_lapsed` skips it. Test `A-ADMIN-16`.

### 1.4 A generation closing during a purge deadlocked with it — **code updated** (race)

- `close_generation` locked its transition row, then waited on the job row
  (the artifact insert's FK) while the purge held the job row and waited on the
  transition row; Postgres aborted the purge after its full length.
  **Fix:** `close_generation` takes the merge lock first (purge and delete
  hold it for their whole transaction). Its final completion is now guarded on
  `status = 'active'`, as every other automatic completion is.

### 1.5 The leave KLV was still shared between processes and unverified (MAGPIE) — **code updated; wire extended**

- A fixed file name per lexicon meant two `contribute` processes on one data
  directory could overwrite each other's KLV between write and load, and the
  fetched bytes were never checked. **Fix:** `LeaveRequest.previous_artifact_sha256`
  (joined from `leave_generation_artifacts`); MAGPIE requires it, verifies the
  bytes, and writes them under `<lexicon>_birdtest_<16 hex>` via a temporary
  file and `rename` — a name always means the same bytes. Contract fixtures
  (both repos) carry the field.

### 1.6 A `429` still ended runs and discarded results (MAGPIE + birdtest) — **code updated**

- MAGPIE retried a `429` five times, then failed the claim (ending the run) or
  the submission (declining the finished task). The worker limiter was keyed
  per account, so six idle machines on one account exhausted one request a
  second. **Fix:** MAGPIE waits out a `429` on the transient budget (without
  limit for a claim, twenty for a submission, none for the heartbeat);
  birdtest keys the worker limiter per API key (`k:<key-id>`).

### 1.7 Names that pass import but fail on the worker — **code updated**

- A dotted or spaced data name was importable, dispatched, then refused by the
  worker as a counted failure — every worker the job reached stopped after
  five. **Fix:** `inputdata::classify` skips any name outside
  `[A-Za-z0-9_-]`. Test `U-ARCHIVE-7`.

### 1.8 MAGPIE did not require a player's lexicon and leaves — **code updated**

- Absent, the load kept whatever leaves were loaded. Both are now required
  keys; `previous_artifact_key` is required too. Also: the contribute run
  clears its rack-info-table name overrides on exit (they leaked into a later
  CLI `-rit true`), and a table being rebuilt is evicted *before* the build
  (the old 1.9 GB and the build's 2.4 GB were resident together).

### 1.9 The per-(username, IP) login bucket could never trip — **code and PLAN.md updated**

- The eleventh audit added it at the per-IP rate, after the per-IP check, so
  it only ever saw requests the IP bucket had let through. Removed; the limits
  are per IP (10/min) and per username from anywhere (100/min). PLAN.md's flow
  and rate-limit table corrected.

### 1.10 Registration awaited its mail, except when it did not — **code updated**

- The taken-address notice, when skipped by its per-address limit, answered
  faster than a fresh registration awaiting its send — a timing oracle. Both
  sends are now spawned, as password reset's is. (Consequence: a mailer outage
  no longer fails registration with a 500; the failure is logged, and an
  unconfirmed account frees its name and address after a day.)

### 1.11 Other code fixes

| Item | Change |
|---|---|
| Pentanomial draw identity had no schema backstop | Added to `game_results_pentanomial_all_or_nothing`; tested at both layers (`I-SUBMIT-2`) |
| Large results decoded without bound | Results ≥ 8 MiB decode under a 3-permit semaphore, 30 s wait then `503` |
| Contributor list hashed every completed claim | Grouped by identity, limited, then hashed |
| `task_claims` heartbeats not HOT | `fillfactor = 85` |
| Imported input data not replicated to DR | Second replication rule for `inputs/` |
| Ratings page swallowed a failed history load | Pool and history fetched together |
| The drill's disk sized for a dump alone | `restore_ephemeral_storage_gib` (200) for drill and ops tasks; the drill checks free space against the manifest's `database_bytes` first; parallel query off in the drill's server |
| `backup-drill-check.sh`'s leftover check could not fail | Asserts the local drill cleaned up, and runs the drill a second time with `DRILL_TARGET=server` |
| `prod-sql.sh` error paths | Start failure prints ECS's reasons; no stream prints the stopped reason; logs paginated; both scripts print the Terraform workspace |
| A fake-worker profile on the development stack | Moved into `docker-compose.e2e.yml`, the only place it is used; its knobs removed from `.env.example` (the task's rule: no fake-worker mode outside the e2e suite) |
| `tasks_job_idx`'s comment named a reader that no longer exists | Comment corrected |

### 1.12 Plan wins (documents changed to match the code)

1. **RUNBOOK §1:** the provider (AWS v5) tracks `aws_db_instance` by resource
   id, so after the rename swap the state followed the damaged instance.
   Added `state rm` + `import`; each rename is polled for before its `wait`
   (the waiter fails at once on a name that does not exist yet).
2. **RUNBOOK §5:** apply with `desired_count=0` (the backend would migrate the
   empty instance and stop the restore at its first object); set the
   placeholder master password; sync both `leaves/` and `inputs/`; drop the
   `S3_BUCKET` option (nothing can set it); select the default workspace after.
3. **RUNBOOK §2.3:** a purged completed job's status and verdict are restored
   from the scratch copy.
4. **`variables.tf`:** `restore_drill_enabled` still described the old drill.
5. **TESTING.md:** duplicate `I-STATS-9b` renumbered (`I-STATS-9e`) and given
   the in-flight test it claimed; counts updated (470 backend tests).

---

## 2. Objective 3 — settings that change a task's outcome

Re-traced from scratch; the eleventh audit's table holds, with two corrections
now fixed: the leave KLV is verified and content-named (1.5), and a player's
lexicon and leaves are required rather than inherited (1.8). Outputs-only
settings (`print_boards`, human-readable output, `-ritmmap`) are not reset and
need not be.

## 3. Objectives 4–6 — critical path, performance, storage

Performance, most severe first: (1) purge stalling the fleet through
contributor rows — fixed (1.1); (2) submissions parked during a purge — fixed
(1.2); (3) unbounded concurrent large decodes (~7 × 64 MiB results → OOM) —
fixed (semaphore); (4) the contributor list's per-claim SHA-256 (tens to 100+
ms per detail view and push) — fixed; (5) non-HOT heartbeats (~13M index
tuples a day at 1,000 claims in flight) — fixed (fillfactor); (6) `?worker=`
pseudonym scan (100–200 ms at 100,000 contributors) — recorded in Known
Limits. Considered and not moved: the per-generation progress upsert (it saves
milliseconds; recorded).

Storage: see section 4 for the three schema/retention items; the eleventh
audit's plies/records/index changes were re-verified against every reader,
script, test and RUNBOOK step.

## 4. Unresolved, pending human feedback

Carried from AUDIT_FINDINGS_7 §4, unchanged: **4.1** claim idempotency;
**4.2** the purge/delete redesign (batched, or partitioned tables);
**4.3** scheduling-history retention; **4.4** rating pools cannot be edited or
deleted. New:

### 4.5 Registration leaves a state that answers what its response hides

A fresh address creates the account, a taken one does not, so signing in with
the username and password just used (`403` vs `401`) says whether the address
had an account — deterministically. **Options:** (a) pending registrations in
their own table, the `users` row created at confirmation, a taken address
recorded as a pending row too; (b) accept and correct PLAN.md's "learns
nothing". **Trade-off:** (a) is a schema and flow change touching confirmation,
login's 403 path and the stale-account release. Recorded in Known Limits;
PLAN.md's Account Creation Flow left as it was (the unresolved rule).

### 4.6 Residuals kept for every run of the month

Only the newest run's residuals are read, but every two-minute run keeps its
own for the month runs are kept in full: ~4.5M rows for a pool of twenty, 3–6
GB for a dense pool of forty, dumped nightly. **Options:** keep them for the
newest run only (delete the superseded run's in the fit), or only for runs the
thinning keeps. It is a retention decision PLAN.md made deliberately for runs
in general, so it is left for a person. Recorded in Known Limits.

### 4.7 Plies as rows, and the move's surrogate key

Plies as two `float8[]` columns on the move (~2.4 GB saved per simming
opening-rack job, and no ply inserts on the submit path), and
`(record_id, rank)` as the moves' key (~0.9 GB per full opening-rack job).
Schema and insert-path rewrites touching restore order and every reader; worth
doing together before release if simming opening-rack jobs are planned.
(The second was noted by the eleventh audit's reviewer and omitted from
AUDIT_FINDINGS_7; recorded here.) Recorded in Known Limits.

Also recorded as limits rather than open decisions: unconfirmed accounts,
spent tokens and anonymous identities are never reaped; `records.task_id` is
stored for opening racks (~50 MB per job); the Windows (WinHTTP) path of
MAGPIE's HTTP client, which is written but not compiled, lacks the stall
semantics and returns a truncated body on a mid-read failure.

## 5. `birdtest-contribute` changes (this pass)

| Change | Why |
|---|---|
| `previous_artifact_sha256` required and verified; KLV content-named, written via rename | 1.5 |
| `429` on the transient budget | 1.6 |
| Player `lexicon`/`leaves` and `previous_artifact_key` required | 1.8 |
| Rack-info-table overrides cleared after a run; a table evicted before it is rebuilt | 1.8 |
| Contract fixture carries `previous_artifact_sha256` | 1.5 |

`MAGPIE_VERSION` stays 0.1.1: the eleventh audit's 0.1.1 was never pushed, so
both passes' changes ship as one version. MAGPIE tests run: `contribute` and
the suite in the final run (see the summary).

## 6. Other

- **Python worker as a production client:** the development compose file's
  `fake-worker` profile was the one remaining fake-worker mode outside the e2e
  suite — moved (1.11).
- **Verified correct and not changed:** curl constants against a real
  `curl.h`; eviction's memory safety; the identity macros on WASM/macOS; every
  name birdtest sends passes MAGPIE's rules; the nginx template's envsubst
  filter; ops-task IAM and ECS Exec; Terraform validations (statically);
  TESTING.md's counts and named tests; the plies/records/index changes against
  every reader.
