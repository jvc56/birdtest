# AUDIT_FINDINGS_12 — the sixteenth audit (2026-09-24, sixth pass)

Branch `audit/birdtest-2026-09-24-pass6`, off `audit/birdtest-2026-09-24-pass5`
(`ef21df5`), for the reason the earlier passes gave. MAGPIE changes are on
`birdtest-contribute` at `c447de15`, on top of `8be92854`; `docker/Dockerfile`
pins it. **Still unpushed** — the one human action this branch needs before it
can merge (AUDIT_FINDINGS_8, header).

**Builds on** AUDIT_FINDINGS_7 to _11 (same day). As before, the largest finding
is in the previous pass's own work (1.1).

**Count: 26 code wins (20 birdtest, 6 MAGPIE), 10 plan wins, 18 unresolved
pending feedback** (14 carried, 4 new: §4).

---

## 1. Code versus PLAN.md, and the previous pass's fixes

### 1.1 The worker's run state was never initialised — **MAGPIE updated** (critical; the fourteenth and fifteenth audits' fields)

`ContributeState` was `malloc`'d, and the fields added in `ac64d754` and
`8be92854` (`server_artifact_mismatch`, the known-bad KLV) were set nowhere:
freed as garbage at the end of every run (valgrind: conditional jumps on
uninitialised values in `free`; with a perturbed allocator, a segfault), read on
the first leave task, and a stray nonzero flag turned a local table mismatch into
a server-KLV one. Tier 6 passed only because fresh pages are zero. **Fix:**
`calloc` (and for `HttpClient`); a test creates and destroys the state under the
sanitizer build, and fails without the fix (checked).

### 1.2 The known-bad KLV outlived its repair and idled the whole worker — **MAGPIE updated** (the fifteenth audit's fix, redesigned)

- Every RUNBOOK §3 repair leaves the (key, hash) pair a claim sends unchanged,
  so a worker that had seen the KLV missing or wrong declined the job without
  fetching for the rest of the run — the job stalled fleet-wide after the admin
  fixed it. The doubling wait napped the *worker* (up to ten minutes per draw of
  the job, starving every other job), and one slot let two bad jobs defeat it.
- **Fix:** such a job is set aside for a doubling interval (sent as unsupported
  until then), then claimed and its KLV fetched afresh, so a repair is noticed;
  other jobs go on meanwhile; a shutdown answered only because of set-aside jobs
  is waited out. PLAN's decline section updated.

### 1.3 Other MAGPIE fixes

| Item | Change |
|---|---|
| `stop` was ignored while a request waited to retry (a claim retries without limit when the server is down) | The HTTP client takes an abort check, asked each second of a wait |
| Temporaries named by pid and a per-process counter collide across PID-1 containers; a failed exclusive open deleted the other's file | Nanosecond clock in the name; only a file this process created is removed |
| The distribution/layout identity was recorded even when the reread failed | Recorded only on success |
| An opening-rack request with a `previous_play` would be analysed without it | Refused |

### 1.4 The job stats cache ignored admin actions and serialized every job — **code updated** (the fifteenth audit's cache)

After Activate/Deactivate/Complete/Purge/Merge the admin page reloaded a payload
up to ten seconds old — and reset the allocation field to the old value, so a
second click reverted the admin's change. One global lock made every job's page
wait on any job's build (and could be driven from unauthenticated routes), and a
slower older build could overwrite a newer push. **Fix:** `jobstats::forget` on
every admin action, auto-completion, and generation close; one build lock per
job; freshness from the build's *start*, never replacing a newer build; the
stream subscribes before reading its first payload (a push in between was lost).

### 1.5 Other code fixes

| Item | Change |
|---|---|
| Each submission took a pool connection just to consult the template cache (and looked its token up twice) | Cache first; the hold check reuses the one lookup |
| An export started beside a purge could build the emptied job and become the re-run's download | Status read `FOR SHARE` in the insert's transaction; export, merge and rebuild refused while a purge runs |
| KLV cache misses went through one global lock with no S3 timeout | One fetch per key; a 30 s limit, answered 503 |
| The rating sweep's cheap check missed a purge and re-run to the same count | Purge and delete mark every pool's newest fit for a refit |
| A rebuild's served-hash write could land on a re-run's row | Compare-and-set on the row it read |
| A purge that committed answered 500 if its export cleanup or generation-0 rebuild then failed (inviting a second purge) | Logged; the purge answers success (generation 0 heals on the next claim) |
| The generation-0 build marker stayed set if its build panicked | Drop guard |
| Login's `next` check passed `/\evil.com` | Resolved as a URL, same origin only |
| A config `MIN_MAGPIE_VERSION` like `0.2.0-rc1` passed startup and then failed every job from the form | Strict at startup too (`U-CFG-3`) |
| "Force a rebuild" had no button | **Force rebuild**, behind a confirm |
| A deleted job's page re-requested its stream every five seconds forever | Stops on 404 (`F-SSE-3`) |
| The API sent no `nosniff` (the Nginx comment said it did) | Set on every API answer (`A-BOUND-11`) |
| The games ETA's redundancy fix was untested | `I-STATS-8b` |

### 1.6 Infrastructure (objective 9)

| Item | Change |
|---|---|
| The fifteenth audit's `FreeStorageSpace` alarm was in ALARM from the first apply and never cleared (threshold = the first allocation) | An RDS event subscription (`low storage`, `failure`) to the alerts topic, which now admits `events.rds`; the CPU-credit alarm (wrong for unlimited mode, empty at launch) replaced by CPU over 80% for fifteen minutes |
| `prod-shell.sh` stopped its task when the ECS Exec session ended (twenty idle minutes), killing any restore running in it | The task outlives the session; `--attach <task>`; asks before stopping. RUNBOOK runs `pg_restore` detached |
| `nginx:1.27-alpine` (unmaintained branch) served the site; Node 20 past end of life | `nginx:1.28-alpine`, Node 22 (not built here: image builds need approval) |
| `prod-sql.sh` read the logs the moment the task stopped, missing psql's final ERROR | Waits for ingestion |

### 1.7 Plan wins (documents changed to match reality)

1. **RUNBOOK §2.3b** rescanned the recount on every batch (seconds per batch at a
   million identities, ~2,000 hand-run statements): now two `DO` loops that
   consume the recount as they apply it and commit per batch — tested on scratch
   data with wrong counters.
2. **§2.0 + §2.3b left phantom counts** (claims deleted in §2.0 raised counters
   nothing gave back); the zeroing is now a required step.
3. **§2.2's loops carried on after an error**, burying it under foreign-key
   failures; they stop. A deleted job's restore also needs its config rows and
   any `player_configs` deleted since.
4. **The `setval` statements** could still move a sequence back by a race; they
   now set nothing unless the table is ahead.
5. **§4's counter check** runs serial (in parallel each worker built the whole
   aggregate).
6. **§2.0 and §2.3** read `:'job'` with nothing setting it; `psql -v job=`.
7. **§1** hard-coded the instance class; now the source's.
8. **§5**: the MAIL FROM MX must point at the DR region; README's first deploy
   sets the region on every command.
9. **TESTING.md** said CI used `cargo test`; it uses nextest plus doctests.
10. **PLAN**: worker deferral and stop, the stats cache's invalidation, the
    alarms, `prod-shell.sh --attach`, and four Known Limits (§4).

## 2. Objective 3

Re-traced against `c447de15`; the reviewer's table finds no outcome-affecting
setting unpinned. The new gaps were behavioural (1.1–1.3), not settings.

## 3. Objectives 4–6

Most severe first: (1) the stats build lock serializing every job — per job;
(2) a connection per submission for the template cache — gone; (3) KLV misses
behind one lock with no timeout — per key, bounded; (4) §2.3b's rescanning
batches — consuming loops; (5) §4's parallel counter check — serial. Storage: no
new growth; the partial indexes and dropped username index verified in use
(EXPLAIN on 600k tasks / 720k claims).

## 4. Unresolved, pending human feedback

Carried: the fourteen of AUDIT_FINDINGS_11 §4 (the seven from _7/_8, and the
per-job contributor total, Terraform remote state, one `users` contribution
index, `leave_rack_progress` autovacuum, builder-mismatch shutdown advice, local
write failures counted, full-disk writer exit).

New:

1. **Recovering an account does not revoke its API keys** — revoking on reset
   stops every one of the contributor's machines with it.
2. **Confirmation on page load** — a scanner that runs script confirms; a button
   costs every registrant a click.
3. **ASCII-only addresses** — matches SES; the form could send IDN domains as
   punycode.
4. **`test_before_acquire`** — a ping per pool acquire, against a failover's
   broken connection failing one request instead of being replaced silently.

All four are recorded in PLAN's Known Limits.

## 5. `birdtest-contribute` (this pass, `c447de15`)

| Change | Why |
|---|---|
| State (and `HttpClient`) zeroed; create/destroy test | 1.1 |
| Per-job deferral replacing the known-bad KLV | 1.2 |
| Abort check in HTTP retries | 1.3 |
| Nanosecond temp names; no removal of another's file | 1.3 |
| Identity recorded on success; `previous_play` refused | 1.3 |

MAGPIE tests: `contribute` (with `test_a_runs_state_starts_clean`), `config`,
`rit`, `wmp`, `autoplay` pass. Tier 6 passes on the new binary.

## 6. Other

- **Python worker as a production client:** none found.
- **Verified and unchanged:** the finish witness with the job read before the
  submit transaction; lock order across all paths; no double dispatch; the
  partial unique indexes still enforce one live slot per identity; the backup
  KMS statement's conditions; the Dockerfile's single ARG; CSRF on all 22
  mutating admin and ratings routes; RUNBOOK §2.0 → §2.2 (twice) → setval → §2.3
  → §2.3b end to end, counters identical to the source.
