# AUDIT_FINDINGS_11 — the fifteenth audit (2026-09-24, fifth pass)

Branch `audit/birdtest-2026-09-24-pass5`, off `audit/birdtest-2026-09-24-pass4`
(`1420a62`), for the reason the earlier passes gave: `main` has none of this
day's fixes, and this pass re-audits them. MAGPIE changes are on
`birdtest-contribute` at `8be92854`, on top of `ac64d754`; `docker/Dockerfile`
pins it (now declared once), and CI checks out that pin. **Still unpushed** — the
one human action this branch needs before it can merge (AUDIT_FINDINGS_8,
header): CI's image, e2e and contract jobs and every image build fetch the pin
from GitHub.

**Builds on** AUDIT_FINDINGS_7 to _10 (same day). One of the fourteenth audit's
fixes had a defect of its own (1.1), and one was too tight (1.12).

**Count: 33 code wins (26 birdtest, 7 MAGPIE), 9 plan wins, 14 unresolved
pending feedback** (7 carried, 7 new: §4).

---

## 1. Code versus PLAN.md, and the previous pass's fixes

### 1.1 "Check artifacts" approved any object's bytes — **code updated** (the fourteenth audit's fix, corrected)

- The fourteenth audit made every rebuild set `served_sha256` from the object it
  found. After a mistaken purge, a re-run that closed a generation, and RUNBOOK
  §2's copy-back of the old rows, the object held the *re-run's* KLV; the check
  reported "matches" (the rows rebuild the recorded bytes) and served the re-run's
  hash, so workers played leaves from a run whose results were not in the
  database. Before that fix, workers refused them, which was at least visible.
- **Fix:** the object's hash is served only when it is the first build, this
  rebuild, or what was already served. An object nothing accounts for is
  replaced with the recorded build when the rows reproduce it exactly (the
  bucket is versioned; the old one stays as a noncurrent version); otherwise it
  is left, reported (`object_sha256`, `object_accounted_for`) and shown in the
  admin page's new **Served** column, and workers go on refusing it. RUNBOOK §3
  says what to do. `I-LEAVE-8` covers both branches.

### 1.2 `stop` did not stop `contribute`, and submitted truncated results — **MAGPIE updated** (objective 3)

The REPL's `stop` and the API's stop set the user-interrupt status, which cuts
autoplay and simulation short without an error; the contribute loop never
looked at it. A stopped task's short batch of games, cut-off simulations or a
leave task's partial counts were submitted and — at the last wave of a batch —
accepted; later claims kept running with the status set. **Fix:** after each
executor the loop checks for a stop, hands the claim back (`task_failed`, not
counted) and ends; it checks before each claim, and its waits return on a stop.
PLAN's "stopping is cooperative" promise, which the code never kept, is
replaced by what it now does.

### 1.3 A missing leave KLV ended every leave worker's run — **code and MAGPIE updated**

`artifacts.get` mapped every S3 error to 404, and MAGPIE counted a non-200
artifact fetch as a task failure: in RUNBOOK §3's "missing object" window (or
on one S3 hiccup) every leave worker hit five failures in seconds. **Fix:**
404 only for `NoSuchKey`, `503` + `Retry-After` otherwise (MAGPIE retries it);
MAGPIE declines a 404 like a hash mismatch — the server's to fix — without
counting it.

### 1.4 The KLV-mismatch retry (fourteenth audit) amplified load — **MAGPIE updated**

Every worker re-claimed, re-downloaded and re-declined every five seconds until
an admin acted: at 200 workers, ~40 claims/s, ~140 MB/s through the one task and
~145k gap and audit rows an hour. **Fix:** a (key, hash) found bad is declined
without fetching again, and the wait doubles to ten minutes, resetting when the
hash changes.

### 1.5 A purge or delete could still be started twice; lifecycle actions waited on it — **code updated** (race)

The fourteenth audit's 409 was a check in the handler and a hold taken later in
the spawned task: a double click passed both. **Fix:** `try_hold_claims` checks
and takes the hold under one lock, in the handler (unit test). Activate,
deactivate and complete are refused while one runs: each waited minutes on the
job's row with a connection, and completing then finished a job the purge had
just emptied (`A-ADMIN-19`).

### 1.6 Result decoding ran with the claim and task locked — **code updated** (critical path)

Decoding a result (tens of ms; about a second at 64 MiB) ran after the claim and
task rows were locked, with a pool connection held. **Fix:**
`registry::decode_result`, which depends only on the job's template, runs
before the transaction (the claim's job read unlocked — a task never changes
job); `store_result` stores what it decoded. A stale claim now costs a decode
before its `accepted: false`.

### 1.7 The job stats payload grew with history and was rebuilt every second — **code updated** (performance, most severe)

Its contributor list groups every completed claim of the job (with no `job_id`
on `task_claims`, the planner scans the fleet's claims), and its game statistics
read every result: 200 ms at 14k tasks, ~1 s at 400k, on every view, every
stream connect, and every second a busy job was watched. **Fix:** a per-job
payload cache (`JOB_STATS_CACHE_SECONDS`, default 10), filled by live pushes,
now spaced by the same interval; misses built one at a time. The durable fix —
a per-job contributor running total — is a schema change left for a decision
(§4).

### 1.8 Other code fixes

| Item | Change |
|---|---|
| A finished leave job read one generation short (`current_generation` is capped) on both detail pages | `generations_closed` in the payload |
| Games/pairs ETA ignored redundancy (half the time left at redundancy 2) | Units per claim from the batch size ÷ redundancy |
| A key created while the account was being deleted survived it and authenticated | `deleted_at` rechecked under the lock; the key lookup requires a live account |
| A malformed per-job MAGPIE floor ("v1.6.0", "1") became 0.0.0 — the most permissive floor | `400` (`Version::parse_strict`; `I-JOB-13`) |
| Email accepted `x <victim@...>` and lists — each variant bypassed taken-address detection and the per-address notice limit | One bare address only (unit test) |
| Login ignored a bad `?next=`, leaving a signed-in user on /login | Only same-site paths |
| Admin bans page listed only the first page of workers | Paginated |
| Player configs MAGPIE refuses (plies > 25) or truncates (recorded plies > 10) were accepted | `400` (`I-JOB-14`) |
| A leave job whose generation-0 build failed after commit never recovered | The next claim builds it, one build per job (`I-LEAVE-18`) |
| An export finishing after its row was purged or reaped left its objects | Removed (`I-EXPORT-5`) |
| `/api/worker/artifact` buffered each KLV per request (GBs at a generation boundary) | The last few served KLVs kept in memory, only if they match the served hash; misses fetched one at a time |
| Decline gaps inserted a row per statement under the claim lock | One `UNNEST` insert |
| The rating sweep built every pool's matrix every two minutes to find nothing changed | Compares the eligible jobs' `games_completed` sum (`rating_runs.evidence_games`) and membership first |
| Key creation at ten an hour held back a first setup of many machines | Burst of 100 (the cap), then ten an hour (`A-ACCOUNT-6`) |
| Per-identity `task_claims` indexes also indexed the other identity kind under NULL | Partial (`IS NOT NULL`): ~38 bytes and one index write less per claim |
| `users.username UNIQUE` duplicated `users_username_lower_idx` | Dropped; the user page's lookup uses the lower index |
| MAGPIE: letter distribution and layout reloaded only on a name change | Reread when the file changed (identity), stale game dropped |
| MAGPIE: leave results never cleared between runs | Cleared before each run |
| MAGPIE: temporaries named by PID alone (containers sharing a volume) | `<name>.<pid>-<n>.tmp`, opened exclusively |
| MAGPIE: a dead `fatal` path citing a function that does not exist | Removed |

### 1.9 Infrastructure and deployment (objective 9)

| Item | Change |
|---|---|
| **The nightly backup failed for any table file over 8 MB** (high): a multipart upload under SSE-KMS needs `kms:Decrypt`, which the key policy denied the backup task | Decrypt granted only via S3 and only for the backups bucket; the task still holds no `s3:GetObject`. PLAN's wording corrected |
| No `health_check_grace_period_seconds`: a migration over ~30 s was killed and retried forever with nothing serving | 600 s; the backend container's `stopTimeout` 120 s |
| `prod-sql.sh` (fourteenth audit's poll) could read `None` right after `run-task` and report failure while the SQL ran — inviting a second run | `None` counts only once the task was seen, or after five minutes; an unknown exit status says so |
| `MAGPIE_COMMIT` declared twice in the Dockerfile | Once, before the first stage |
| No RDS alarms against the 5× storage ceiling or CPU credits | `FreeStorageSpace` and (burstable classes) `CPUCreditBalance` alarms |
| `alert_email` unvalidated (a typo leaves alarms nowhere) | Validation |
| SES custom MAIL FROM needed MX/SPF records nothing output | `ses_mail_from_records` output; README |

### 1.10 Plan wins (documents changed to match reality)

1. **RUNBOOK §2.2** could not run: `COPY … ON CONFLICT` is not SQL. Now a
   file per table, loaded through a temporary table with `INSERT … ON CONFLICT
   DO NOTHING`; run twice against scratch databases (the second inserted
   nothing). `worker_data_gaps` added.
2. **§2.2's `setval(…, max(id))`** could move a sequence back below the live
   fleet's ids; now `GREATEST`, and `leave_rack_staging` included.
3. **§2.3** rewrote every task row; now only rows that differ. **§2.3b**'s
   zeroing statement is batched like the rest.
4. **The restore checks' counter query** (drill, round trip, RUNBOOK §4) was a
   per-task lateral probe; now one grouped pass.
5. **README "Deploying"**: first apply at `desired_count=0`; Terraform state is
   local and must be kept elsewhere. **RUNBOOK §5**: images must be pullable
   from the DR region.
6. **TESTING.md** CI items 4 and 6 and the Dockerfile's comment still said CI
   used `birdtest-contribute`'s head.
7. **PLAN** "Stopping" promised behaviour the code never had (now accurate);
   the decline reasons, artifact serving, ETA, sweep, rate-limit table,
   email rule, player-config and floor validation, settings table, and
   `JOB_STATS_CACHE_SECONDS` updated.
8. **Known Limits**: player-config delete FK scans; the MAGPIE-side items in §4.
9. **PLAN schema block** regenerated; TESTING counts (482 backend tests).

## 2. Objective 3

Re-traced against `8be92854` (reviewer's table: seeds, lexicon and leaves per
player, distribution, layout, variant, wordmap and table, recorder and every
simulation setting, bingo bonus, cutoff, movegen margin, win% model, reset
flags). Two gaps found and closed: the user-interrupt status truncating outcomes
(1.2), and the distribution and layout cached by name (1.8). Threads remain the
contributor's own (accepted).

## 3. Objectives 4–6

Performance, most severe first: (1) stats payload per view/push, ~1 s/s on a
large watched job — cached (1.7), durable fix flagged; (2) the rating sweep's
matrix every two minutes, seconds per pool at millions of results — skipped when
unchanged; (3) KLV fetches at a generation boundary, GBs of buffers — cached;
(4) the bad-KLV retry storm — backed off (1.4); (5) decode under locks — moved
(1.6); (6) restore counter check, hours at tens of millions of tasks — grouped;
(7) player-config delete FK scans — recorded. Storage: partial identity indexes,
redundant username index dropped, RDS alarms; retention items carried.

## 4. Unresolved, pending human feedback

Carried, unchanged: claim idempotency (7 4.1); purge/delete redesign (7 4.2);
scheduling-history retention (7 4.3); rating pools edit/delete (7 4.4);
registration state oracle (8 4.5 — this audit notes a second channel:
`/api/users` lists unconfirmed accounts, so registering a name with a target
address and looking for it answers the same question); residual retention
(8 4.6); plies-as-arrays and the moves key (8 4.7).

New:

1. **Per-job contributor running total** (`job_contributors`, upserted in the
   submit transaction, given back by purge/delete, recounted by RUNBOOK §2.3b):
   makes the stats payload's contributor list an index read. A schema change and
   a write per submission; the cache (1.7) bounds the cost meanwhile.
2. **Terraform remote state.** Local state is lost with the machine; an S3
   backend in a second region needs a bootstrap bucket and a choice of account
   and region. README documents keeping the state safe meanwhile.
3. **One `users` contribution index** if `/api/users` broke ties by id (a
   visible order change).
4. **`leave_rack_progress` autovacuum scale factor** — a tuning choice for when
   production bloat is measured.
5. **A builder mismatch ends in a `data_out_of_date` shutdown** with the wrong
   advice; a distinct shutdown reason needs a server-side change.
6. **Local write failures are counted, not remembered** (a read-only data
   directory keeps claiming leave tasks between others).
7. **A full disk during a table write exits the worker** (MAGPIE's shared CLI
   writers exit rather than return an error).

Also weighed and left: a submission that exhausts its ~15-minute retry budget
ends the run (recorded since the eleventh audit); no ECS deployment circuit
breaker (single-instance design); drill staleness has no alarm.

## 5. `birdtest-contribute` (this pass, `8be92854`)

| Change | Why |
|---|---|
| Stop honoured; truncated task handed back | 1.2 |
| 404 KLV declined, not failed | 1.3 |
| Known-bad KLV not refetched; doubling wait | 1.4 |
| Distribution and layout reread when changed | objective 3 |
| Leave results cleared; exclusive `<pid>-<n>` temporaries; dead path removed | 1.8 |

MAGPIE tests run: `contribute` (with the updated temporaries test), `config`,
`rit`, `wmp`, `wmpmaker`, `builderhash`, `autoplay`, `layout`, `ld`, `klv`,
`cmdapi`, `command` pass. `ap_rit` needs a prebuilt 1.9 GB `TWL98.rit` and was
not run. Tier 6 (all ten cases) passes on the new binary.

## 6. Other

- **Python worker as a production client:** none found.
- **Verified and unchanged:** `/api/workers` rewrite ordering and pagination;
  CSRF on every mutating route; session invalidation; SPRT LLR (recomputed:
  1.125953543 and 0.207499670 against `U-STATS-2`) and Bradley–Terry; the lock
  order across claim, submit, decline, purge, delete, reclaim and close; no
  double dispatch; the fourteenth audit's DR replication fix; every env var the
  tasks read is set; IAM for the web, builder, ops and drill roles; the new
  partial indexes' use by `next_available` and the user lookups (EXPLAIN).
