# AUDIT_FINDINGS_15 — the nineteenth audit (2026-09-25, ninth pass)

Branch `audit/birdtest-2026-09-24-pass9`, off `audit/birdtest-2026-09-24-pass8`
(`49d1117`). MAGPIE changes are on `birdtest-contribute` at `d91dc083`, on top of
`c32ba750`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _14. No finding is critical. It also corrects
two things AUDIT_FINDINGS_14 got wrong (§0).

**Count: 18 code wins (14 birdtest, 4 MAGPIE), 9 plan wins, 18 unresolved
pending feedback** (all carried).

---

## 0. Corrections to AUDIT_FINDINGS_14

1. **§1.4, "Terraform would fail every apply after the first storage autoscaling
   step": wrong.** The AWS provider already suppresses the diff when the
   variable is below an autoscaled size, as long as the ceiling is above it (the
   `allocated_storage` DiffSuppressFunc in `internal/service/rds/instance.go`).
   The `ignore_changes = [password, allocated_storage]` the eighteenth audit added
   only did harm: raising `db_allocated_storage` silently stopped growing the
   volume. **Reverted** to `[password]`. The variable now has a description of
   what it does (§1.5).
2. **§4's "new" unresolved item (usernames accept any Unicode) duplicated an
   existing Known Limit** ("Usernames are unique whatever their case, and
   otherwise free text"). The two are merged, the wording now says characters
   rather than bytes, and the count goes back to 18.

## 1. Findings

### 1.1 A purge waiting on a rating fit held every contributor's row — **code updated** (the eighteenth audit's fix, corrected)

The eighteenth audit took every pool's fit lock inside the purge, after the purge
had given back its contributors' counters. So a purge that met a running fit held
the fit-lock wait with every contributor's row locked. Every submission of
theirs, for any job, then waited on it holding a pool connection; reproduced at
about 3.5 s per waiting submission. **Fix:** the refit mark and the export
deletion come before the give-back, which is now the last statement before the
commit again. `A-ADMIN-20` holds a fit's lock on a side connection and checks
that a submission goes through during the purge.

### 1.2 The API could be run out of memory through rate-limit keys — **code updated** (security)

The login and password-reset limiters are keyed on the username or address the
caller sends. Axum's default body limit is 2 MB, so each key could be a
megabyte, kept until the ten-minute sweep; a few addresses could exhaust the
task's 2 GB. **Fix:** a key longer than 128 bytes is kept as its SHA-256. The
auth and account routes take bodies of at most 16 KiB. Covered by `A-AUTH-4d`.

### 1.3 The job list's `stalled` flag scanned every task, and nothing tested it — **code updated** (performance)

"No result in a day" joined every task of the job to the day's completions:
300–460 ms at a million tasks, on every list view whose page had an active job
with a recent decline. That is now normal on the home page, and it reaches the
display pool's timeout, failing the page, at tens of millions of tasks. **Fix:**
`jobs.last_completed_at`, set by the submission that stores a result, at most once
a minute and without taking the row lock otherwise. A purge clears it, and RUNBOOK
§2.3 recomputes it. `A-PUBLIC-1c` is the first test of the flag at all.

### 1.4 Other code fixes

| Item | Change |
|---|---|
| A purge, then a quick re-issue of as many claims as the finish check saw, could complete the emptied job | A second witness: the per-job purge count, compared under the job's row lock before the completion commits (`I-STATS-9d`) |
| Derived-build leases were set on the builder's clock and judged on the database's | Set in SQL (`now() + interval`), returned as the token |
| `forget` and a build's supersession check took their two locks separately | Both under the payloads' lock, in one order |
| A late load error on a job page could hide stats already streamed in | Shown only while there are none |
| Admin job and player-config forms threw away the per-field reasons | Listed with the message |
| Unknown endpoints and wrong methods got empty 404/405s | JSON `not_found` and `method_not_allowed` (`A-BOUND-12`) |
| Path rejections of the wrong-number-of-parameters and unsupported-type kinds (server bugs) were 404s | 500s |
| An import's `git_ref` went into the GitHub URL as given: `../../../user` reached another endpoint with the server's token | Ref-name characters only, no empty, `.` or `..` segments; the echoed reply truncated |
| RDS changes to instance class and Multi-AZ waited for the maintenance window (up to a week) | `apply_immediately` (a variable, default on) |
| Allocation past 80% of the storage ceiling raised no alert | The event subscription includes `notification` (RDS-EVENT-0225) |
| The ratings page's empty state described a form that does not exist | It names the API |
| MAGPIE: `compat/ctime.h` included `io_util.h` and used nothing from it, which was the cause of the cycle the eighteenth audit worked around | Include removed; `io_util.c` back on the compat clock wrapper |
| MAGPIE: its sanitizer CI shards ran neither `contribute` nor `builderhash` | Both added to the `rest` shard |
| MAGPIE: the claim-body fixture test compared the length only, and a misplaced comment | The body is built from the fixture's version and ids, and every key and value is compared; the comment moved; README says 0.1.1 |

### 1.5 Plan wins

1. **README's alert-path checks could not work.** The backup-failure run passed
   `sh -c "exit 1"` to a `bash -c` entry point, which exits 0. The staleness
   alarm set to ALARM while already in ALARM sends nothing. The commands also
   lacked `--region`. They now set OK and then ALARM, give the backup task's
   override as one string with its cluster and network, check `TriggeredRules`,
   and use the region.
2. **RUNBOOK §5** built the DR stack without `prod.tfvars`, so it came up with a
   micro instance, 20 GiB and the fleet's MAGPIE floor back at 0.1.1. It now
   passes the var file and sizes storage for the restore at creation, and step 7
   runs the alert checks with the `-dr` names.
3. **RUNBOOK §2.3b** is run as a file with `psql -f`. Pasted into interactive
   psql, an error stopped only one statement, and pasting again in the same
   session applied the old snapshot. Tested: stopped by a lock timeout, then run
   again, it leaves 0 disagreements. The last check is now §4's recount from the
   claims, and `temp_buffers` is 64 MB.
4. **RUNBOOK §2.2** has a branch for a PITR scratch instance (its guard refused
   that route), shows the quoted heredoc that writes the script, and its load loop
   exits non-zero.
5. **RUNBOOK §1**'s ceiling is at least 30% above the allocation, since RDS
   warns at 80%.
6. **`infra/backup.tf`** said a failed dump is retried; the scheduler retries
   only a RunTask it could not make.
7. **PLAN**: the backup metric is `Success`, not `SuccessTimestamp`; error codes
   add `method_not_allowed`; the `stalled` Known Limit is closed; the alarms
   paragraph covers `notification`.
8. **`db_allocated_storage`** has a description: it grows the volume and never
   shrinks it (§0).
9. **TESTING.md**: MAGPIE's four contract tests, `A-ADMIN-20`, `A-AUTH-4d`,
   `A-PUBLIC-1c`, `I-STATS-9d`; 494 backend tests.

## 2. Objective 3

Re-traced against `d91dc083`: every outcome-affecting setting is pinned per task
and reset. There are no task-setting changes.

## 3. Objectives 4–6

Most severe first:
1. The purge's lock wait holding contributor rows (1.1).
2. The `stalled` flag's full-history scan (1.3).
3. Rate-limit key memory (1.2).

Nothing else is new on the claim, submit or heartbeat path. Storage: one column,
`jobs.last_completed_at`.

## 4. Unresolved, pending human feedback

The eighteen carried from AUDIT_FINDINGS_13 §4, unchanged (§0.2).

Weighed and left:
- A final-attempt build that outlives its 45-minute lease is failed while still
  running; its later outcome is dropped, and the admin retry recovers it.
- 503s are logged twice.

## 5. `birdtest-contribute` (this pass, `d91dc083`)

See 1.4, last three rows. MAGPIE tests `contribute`, `config`, `rit`, `wmp`,
`autoplay` and `builderhash` pass. `find_circ_deps.py` and `format.py` pass on a
clean archive. Tier 6 passes on the new binary.

## 6. Other

- **Python worker as a production client:** none.
- **Verified:**
  - The purge-count witness handles every interleaving of check, hold, lock and
    commit.
  - `mark_every_pool_for_refit` takes its locks in pool order (EXPLAIN VERBOSE).
  - A fit takes no row lock a purge holds.
  - `SseBroadcaster::close` under a concurrent subscribe.
  - The `?status=` plan.
  - §1's ceiling parsing and arithmetic.
  - §2.1's detached restore quoting.
  - The DR stack's names.
  - The ops image's tools.
