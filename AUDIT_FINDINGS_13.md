# AUDIT_FINDINGS_13 — the seventeenth audit (2026-09-25, seventh pass)

Branch `audit/birdtest-2026-09-24-pass7`, off `audit/birdtest-2026-09-24-pass6`
(`1de113b`). MAGPIE changes are on `birdtest-contribute` at `2a697c69`, on top of
`c447de15`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header): `git ls-remote` shows the remote branch at `47b57aa`.

**Builds on** AUDIT_FINDINGS_7 to _12. No finding this pass is critical. The
most serious one is in the sixteenth audit's own RUNBOOK rewrite (1.1).

**Count: 21 code wins (17 birdtest, 4 MAGPIE), 10 plan wins, 18 unresolved
pending feedback** (all carried; none new).

---

## 1. Code versus PLAN.md, and the previous pass's fixes

### 1.1 RUNBOOK §2.3b, re-run after an interruption, zeroed counters it had already fixed — **plan updated** (the sixteenth audit's rewrite, corrected)

- Step 2 zeroed identities absent from `recount`; step 3 consumed `recount` as it
  applied it. Re-running the block after a lock timeout — as the text said to —
  zeroed every identity step 3 had already applied (reproduced: 45,000 users left
  at 0, and `left_to_apply` said 0). Without `ON_ERROR_STOP`, a timeout in step 2
  ran on into step 3. And the zeroing's `NOT IN` became a per-row subplan once
  `recount` was analysed: 45 s for ten rows.
- **Fix:** step 3 consumes a copy (`pending`); `recount` is kept whole, typed
  `uuid`, indexed and analysed; the zeroing uses `NOT EXISTS` (a hash anti-join,
  half a second a batch); `ON_ERROR_STOP` and `temp_buffers` are set; the final
  query reports both what is left to apply and what is left to zero; and **§4
  gained a contributor-counter check**, so this kind of damage is no longer
  invisible. Re-tested with a lock timeout mid-run and a re-run: 0 disagreements.

### 1.2 The stats cache could still cache a pre-action job, and pushes ignored supersession — **code updated** (the sixteenth audit's fix, finished)

- A viewer's `Job` was read before waiting for the per-job build lock. A build
  that started after an admin action's `forget`, from that older copy, was kept as
  newer than the action, and the admin page put the old allocation back in its
  form. Pushes published payloads the cache had refused as superseded. Admin
  actions never pushed, so open pages kept showing a deactivated job as active.
  A build slower than the TTL was never shared, and every waiter rebuilt in turn.
- **Fix:** a build reads the job row itself after it starts. A waiter accepts any
  build that started after it asked. A push sends only a kept payload (otherwise
  it rebuilds, a bounded number of times). Admin actions push to open pages
  (`push_after_change`). Covered by `I-STATS-11`.

### 1.3 A deferral that ended mid-claim obeyed the shutdown it existed to prevent — **MAGPIE updated** (the sixteenth audit's redesign, finished)

The claim body is sent unchanged on every retry. If a set-aside job's interval
ended during a 429 or a deploy's 503s, the server still saw it and answered
`data_out_of_date`. The clock then said no deferral was active, so the worker
exited with "run download_data.sh" (reproduced). A `magpie_too_old` shutdown was
also napped through. **Fix:** the claim records whether its body named a
set-aside job; only data shutdowns are waited out, and version ones are obeyed at
once; a claim with no job id is not set aside by id. The body builder and the
deferral are now exposed and unit-tested.

### 1.4 Other code fixes

| Item | Change |
|---|---|
| A purge whose post-commit export cleanup failed left the old run's `ready` export to be served as the re-run's | The export rows are deleted in the purge's transaction; the objects after it |
| The derived builder's outcome ignored its lease: a build that outlived the 45-min lease could set a row another builder had just built back to `pending` or `failed` | The lease (as stored) is the token each outcome is recorded against; a stale outcome is dropped with a warning |
| Complete/activate/deactivate could land just after a purge committed | Rechecked under the row lock |
| Purge/delete's "mark each pool's newest fit" walked every run (most of a second, under the purge's locks) | One index seek per pool (`LATERAL`) |
| Over HTTP/2 (the ALB) `statusText` is empty: every error without JSON (a deploy's 502/503) had no message | "The server answered N." (`F-API-4`) |
| Path and query rejections were axum's plain-text 400s; a bad job id left the page on "Loading…" and the stream retrying forever | `ApiPath`/`ApiQuery`: JSON, a malformed id `404` (`A-BOUND-12`); the stream stops on any 4xx but 408/429 |
| API key labels were unbounded (2 MB × 100 keys an account) | 100 characters (`A-ACCOUNT-7`) |
| Login `next=/.//evil.com` resolved to the path `//evil.com`, and a malformed URL threw | Same-origin check on the resolved URL, navigating to its `href`; parse failures fall back |
| Force rebuild's confirm understated it, and it was offered on active jobs (workers mid-task would decline the new bytes) | Accurate text; refused while active (`409`) |
| Busy-server 503s read "internal error" | Only a 500's message is scrubbed |
| MAGPIE: the temp-name clock was read uninitialised if the call failed, and bypassed the compat wrapper | Zero-initialised, through `ctimer_clock_gettime_realtime` |

### 1.5 Infrastructure (objective 9)

| Item | Change |
|---|---|
| `prod-shell.sh` left the task sleeping (with `DATABASE_URL`) when its exec agent never started or the Session Manager plugin was missing | Checked before the trap is dropped; plugin checked first |
| `nginx:1.28-alpine` was itself no longer maintained | `nginx:1.30-alpine` (and the e2e fixtures' server) |
| The RDS event subscription could be created before the topic policy allowing it | `depends_on` |
| The alerts topic admitted three service principals from any account | `aws:SourceAccount` condition |

### 1.6 Plan wins

1. **RUNBOOK §2.3b** (1.1), and **§4**'s contributor check.
2. **§2.1's detached `pg_restore`** wrote nothing on success: it now ends its log
   with `pg_restore exit N`, and §2.2 (and §5) refuse to start until it says 0.
   Taken mid-restore, a copy-back found some tables loaded and others empty and
   reported success.
3. **§2.2** stops the whole procedure on a dump failure, and reads the scratch URL
   from `/tmp/restore.env` (an `--attach`ed shell or the detached script had none).
4. **§2.3b** says it runs in the ops shell's psql, not `prod-sql.sh` (one
   transaction; its `COMMIT`s fail).
5. **§1** passes the source's storage ceiling to the restore.
6. **§3**: a check or forced rebuild that started before a purge goes on writing;
   run Check artifacts again after a copy-back.
7. **README**: both region variables, assigned before exporting (CLI v1, and a
   failed `terraform output` hidden by `export X=$(...)`).
8. **PLAN**: the routine `low storage` mail before each autoscaling step versus the
   ceiling event; path/query errors and the 503 message; the shutdown rule.
9. **TESTING.md**: `I-STATS-11`, `A-BOUND-12`, `A-ACCOUNT-7`, `F-API-4`, `F-SSE-3`;
   487 backend tests.
10. **PLAN schema block** unchanged (no migration change this pass).

## 2. Objective 3

Re-traced against `2a697c69`: no outcome-affecting setting is unpinned. The
changes are in claim handling, not task settings.

## 3. Objectives 4–6

Most severe first: (1) §2.3b's `NOT IN` subplan — minutes to hours when analysed —
fixed; (2) purge's `rating_runs` walk — a seek per pool; (3) stats builds slower
than the TTL rebuilt by every waiter — shared; (4) the derived builder's stale
outcomes — lease-checked. Nothing new on the claim, submit or heartbeat path.
Storage: nothing new.

## 4. Unresolved, pending human feedback

All eighteen carried from AUDIT_FINDINGS_12 §4, unchanged. Weighed and left:
marking every pool (not only the job's scope) for a refit on purge; the
artifact-fetch waiters' serial retries during an S3 hang; surplus-credit charges
below the 80% CPU alarm; `low storage` mail before each autoscaling step.

## 5. `birdtest-contribute` (this pass, `2a697c69`)

| Change | Why |
|---|---|
| `claim_named_deferred`; data-only shutdown wait | 1.3 |
| NULL job id not deferred | 1.3 |
| Temp clock through the compat wrapper | 1.4 |
| `test_a_set_aside_job_is_left_out_of_claims_for_a_while` | the deferral was untested |

MAGPIE tests: `contribute`, `config`, `rit`, `wmp`, `autoplay` pass; tier 6
passes on the new binary.

## 6. Other

- **Python worker as a production client:** none.
- **Verified and unchanged:** the per-job build locks and per-key fetch locks
  cannot double; `exports::start`'s `FOR SHARE` cannot deadlock with a purge or
  delete; the compare-and-set on the served hash; SSE subscribing first; the
  finish witness; lock order across paths; the event-subscription categories and
  principal; Node 22's engine ranges; `setsid`/`pgrep` in the ops image; the
  abort check and `calloc`s in MAGPIE (valgrind-clean runs against a fake server).
