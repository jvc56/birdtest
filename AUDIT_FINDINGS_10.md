# AUDIT_FINDINGS_10 — the fourteenth audit (2026-09-24, fourth pass)

Branch `audit/birdtest-2026-09-24-pass4`, off `audit/birdtest-2026-09-24-pass3`
(`86e5ac4`), for the reason the earlier passes gave: `main` has none of this
day's fixes, and this pass re-audits them. MAGPIE changes are on
`birdtest-contribute` at `ac64d754`, on top of `465c2e1f`; `docker/Dockerfile`
pins it, and CI now checks out that pin. **Still unpushed** — the one human
action this branch needs before it can merge (AUDIT_FINDINGS_8, header).

**Builds on** AUDIT_FINDINGS_7, _8 and _9 (same day). As in pass 3, most of what
this pass found was in the previous pass's own fixes.

**Count: 22 code wins, 6 plan wins, 7 unresolved pending feedback** (all
carried; none new).

---

## 1. Code versus PLAN.md, and the previous pass's fixes

### 1.1 The served hash was set only by a rebuild that rewrote — **code updated** (the thirteenth audit's fix, finished)

- `served_sha256` was written only when a rebuild rewrote the object. A rebuild
  whose upload landed but whose `UPDATE` did not, an operator copying back an
  older object version (RUNBOOK §3), and a restore whose rows describe other
  bytes all left workers sent a hash the object did not have — and running
  **Check artifacts** again did nothing, since nothing was rewritten.
- **Fix:** every rebuild sets `served_sha256` from the bytes it wrote or, when it
  leaves the object alone, the bytes it read back (`NULL` when they are the
  first). The report carries `served_sha256`. RUNBOOK §1, §2.4, §3 and §5 say to
  run **Check artifacts** after any restore or version copy. Tested in `I-LEAVE-8`.

### 1.2 A KLV mismatch shut contributors down with the wrong advice — **MAGPIE updated**

- The thirteenth audit declined a mismatched KLV as `derived_mismatch` and, like
  every decline, remembered the job as unsupported. For a worker whose only
  active job was that leave job, the server then answered "every active job
  needs input data you do not have" (`data_out_of_date`) and the run ended —
  sending contributors to re-download data that was never the problem.
- **Fix (MAGPIE `ac64d754`):** a KLV mismatch is declined without setting the job
  aside, and the worker waits its idle interval before claiming again; it is
  not recorded among the local data gaps. PLAN's decline section says so.

### 1.3 A second purge click stacked behind the first — **code updated** (race)

Purge and delete run to completion on a task of their own (thirteenth audit), so
a re-click started a second one queued on the first's locks, and whichever
finished first ended the hold the other relied on. **Fix:** `409` while the job's
claims are held (`A-ADMIN-19`).

### 1.4 Purge ordering — **code updated**

- The rejoin at parity (`claims_baseline`) was taken before the cascades, so the
  minutes they ran were counted against the job; it now runs just before the
  counters are given back.
- `give_back` relied on the array order of its `UPDATE ... FROM unnest` for its
  lock order, which Postgres does not promise; it now locks the rows with
  `SELECT ... ORDER BY id FOR NO KEY UPDATE` first.

### 1.5 An export could stay `running` forever — **code updated**

A panic or hang in a build left its row `running`, and with one export per job
that refused every later export of it until a restart. **Fix:** the build runs
on a task of its own under a six-hour limit (aborted past it), a panic or
timeout marks the row failed, and a build whose row an admin cleared while it
waited for its turn does not start.

### 1.6 The per-account worker bucket was too tight — **code updated** (the thirteenth audit's fix, replaced)

Ten requests a second, burst fifty, per account: fifty idle machines under one
account (it may hold a hundred keys) filled it, and heartbeats, which are not
retried, lapsed. **Fix:** removed; key churn is bounded at creation instead,
ten keys an hour per account (`A-ACCOUNT-6`).

### 1.7 Other code fixes

| Item | Change |
|---|---|
| Login matched usernames exactly, registration case-insensitively: "Josh" could not sign in as "josh", and the error said the password was wrong | Login matches `lower(username)` (`A-AUTH-4c`) |
| Two registrations of case-variant names racing past the check got a 500 | The unique violation is the same field error as the check (`409`) |
| Job detail and admin job pages showed task progress for on-demand jobs (the thirteenth audit fixed only the list) | Racks analysed / generations closed, as the list |
| The job list labelled every unit "units" | Games, pairs, racks, generations |
| `/api/workers` sorted every contributor of both kinds on every view — the comment claimed an index merge the plan never did | Each arm is an ordered, limited scan of its own index (`users_worker_rank_idx`; the anonymous index gains `uuid`): 35 ms → 3 ms on 250k identities |
| Banning an identity that does not exist succeeded | `404` naming the likely mistake |
| The large-result permit was held past the commit | Released at the commit |
| `api_keys` had no index on `user_id` (key list, cap check, cascades) | `api_keys_user_idx` |
| Admin users page kept a stale error; api.ts had a doc comment on the wrong type | Fixed |
| MAGPIE: a writer killed mid-write left its temporary (1.9 GB for a table) forever | Temporaries of the same name untouched for an hour are removed before a write |
| MAGPIE: a stale top-level `contract-fixtures/` copy | Removed; the tests read `test/birdtest_contract/` |

### 1.8 Plan wins (documents changed to match reality)

1. **RUNBOOK §2.3b** recounted every contributor in one transaction, holding
   every counter row while submissions (which bump them) waited and then 503'd.
   Now a temp-table count and batched, `lock_timeout`-bounded, re-runnable
   updates of only the rows that differ.
2. **RUNBOOK §1** told the operator to `terraform apply` without the var file.
3. **PLAN**'s backup layout used `:` in the stamp; `backup.sh` writes
   `03-00-00Z`.
4. **PLAN** rate-limit table, login flow, purge/delete API rows, the served-hash
   paragraph, exports and the decline reasons updated to match the code; the
   schema block regenerated from the migration.
5. **Known Limits:** the job list's `stalled` flag scans the fleet's last day of
   completions for each active job with a recent decline — recorded, with the
   fix to make if the list slows.
6. **TESTING.md:** new and changed tests; 476 backend tests.

### 1.9 Infrastructure

| Item | Change |
|---|---|
| **The DR backup replica could not receive anything** (high): S3 does not replicate from an Object Lock bucket into one without it, and the role lacked the retention reads | `object_lock_enabled` on `backups_dr`; `s3:GetObjectRetention`, `s3:GetObjectLegalHold` on the role. An existing stack replaces the DR bucket (a copy) on apply |
| `backups_dr` kept noncurrent versions and incomplete uploads forever | Noncurrent expiry (90 days) and multipart abort (7), as the source |
| `prod-sql.sh` polled forever for a task ECS no longer describes (`None` after about an hour) | `None` is stopped; a failed call is asked again |
| CI built and tested MAGPIE's branch head, not the commit the image pins | Both jobs check out the Dockerfile's `MAGPIE_COMMIT`; the nightly still tests the head |

## 2. Objective 3

Re-traced against `ac64d754`: unchanged from AUDIT_FINDINGS_9 §2. The KLV change
alters only what a worker does after refusing a KLV, not what it plays.

## 3. Objectives 4–6

Performance, most severe first: (1) the per-account bucket starving idle fleets
— fixed (1.6); (2) `/api/workers` sorting every contributor — fixed; (3) RUNBOOK
§2.3b's recount stalling submissions — fixed (plan); (4) the large-result permit
past commit — fixed; (5) the `stalled` flag's scan — recorded. Storage:
`api_keys_user_idx` and `users_worker_rank_idx` added; orphan table temporaries
removed by MAGPIE; DR backup bucket's versions now expire.

## 4. Unresolved, pending human feedback

All carried, unchanged: claim idempotency (AUDIT_FINDINGS_7 4.1); the
purge/delete redesign (7 4.2); scheduling-history retention (7 4.3); rating pools
cannot be edited or deleted (7 4.4); the registration state oracle (8 4.5);
residual retention (8 4.6); plies-as-arrays and the moves key (8 4.7).

## 5. `birdtest-contribute` (this pass)

| Change | Why |
|---|---|
| A KLV mismatch waits and leaves the job claimable | 1.2 |
| Stale `<name>.<pid>.tmp` siblings removed before a write | 1.7 |
| Stale `contract-fixtures/` removed | 1.7 |

MAGPIE tests run: `contribute`, `config`, `rit`, `wmp` pass (with the new
`test_an_abandoned_temporary_is_removed`). `ap_rit` needs a prebuilt 1.9 GB
`TWL98.rit` and was not run here.

## 6. Other

- **Python worker as a production client:** none.
- **Verified and unchanged:** the purge hold's lifetime against the spawned
  task; `EXPORT_BUILDS` ordering against the one-running index; the DR bucket's
  KMS and replication filter; the RUNBOOK recount's SQL against a scratch copy
  of the schema.
