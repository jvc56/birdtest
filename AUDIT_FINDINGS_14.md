# AUDIT_FINDINGS_14 — the eighteenth audit (2026-09-25, eighth pass)

Branch `audit/birdtest-2026-09-24-pass8`, off `audit/birdtest-2026-09-24-pass7`
(`ac1e989`). MAGPIE changes are on `birdtest-contribute` at `c32ba750`, on top of
`2a697c69`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header). The one finding that would have blocked the push is fixed (1.1).

**Builds on** AUDIT_FINDINGS_7 to _13. Nothing this pass is critical. The
findings are medium or low, mostly in the previous pass's own work.

**Count: 17 code wins (13 birdtest, 4 MAGPIE), 5 plan wins, 19 unresolved
pending feedback** (18 carried, 1 new: §4).

---

## 1. Findings

### 1.1 MAGPIE: the previous commit made an include cycle its CI refuses — **MAGPIE updated**

`io_util.c` began including `compat/ctime.h`, which includes `io_util.h`.
`find_circ_deps.py`, a required step of MAGPIE's `lint-fast` job, fails on a clean
archive of `2a697c69`. **Fix:** `io_util.c` calls `clock_gettime` directly. I
re-ran the lint on a clean archive of `c32ba750`: no cycles, and `format.py`
passes.

### 1.2 A derived build killed on its last attempt stayed `building` for good — **code updated**

Past the attempt cap nothing retakes a lapsed lease, and the admin retry reopens
only `failed` rows. Such a build is one OOM-killed three times, which is likely
for the 2.4 GB table. It left its jobs undispatchable with no error shown.
**Fix:** before taking work, the builder fails lapsed rows that are at the cap,
with the reason it can only guess at.

### 1.3 The purge recheck under the row lock was a coin flip for most jobs — **code updated** (the seventeenth audit's fix, corrected)

The check looked for a purge's hold, but a games job's purge releases its hold
microseconds after committing. A Complete waiting on the row woke into a race it
lost half the time. **Fix:** `DispatchHolds` counts the claims holds taken per
job. The first check reads that count, and the check under the lock compares it,
so a purge that came and went in between is still seen.

### 1.4 Other code fixes

| Item | Change |
|---|---|
| Automatic completions (a leave job's last generation; `JobFinished` at claim time) never reached open pages | They push (`push_after_change`) |
| A `forget` marker was pruned after the cache age, so a build slower than that, begun before an admin action, was kept and pushed | Markers kept apart, for an hour; supersession checked before pruning |
| A rating fit overlapping a purge could write its pre-purge sum back over the purge's refit mark | The mark is taken under every pool's fit lock, in pool order (`ratings::mark_every_pool_for_refit`) |
| The error mapping wrote the database's message over every 5xx, so after the seventeenth audit's scrub-only-500s change it reached users on 503s ("database error: canceling statement…") | Our own 503 message is kept; the database's goes to the log (`error::tests::a_busy_databases_503_keeps_its_own_message`) |
| A first-load error on a job page never cleared once live stats arrived | Cleared by the stream |
| Usernames were measured in bytes, not characters | Characters |
| The home page's "active jobs" filtered only the newest page | `GET /api/jobs?status=` (`A-PUBLIC-1b`) |
| A deleted job's open streams never ended | `SseBroadcaster::close`; pages reconnect, get a 404, and stop |
| Path rejections showed the UUID parser's words, and turned server bugs into 404s | A fixed message; missing path parameters are a 500 |
| Frontend typed admin job actions as list items | `JobRow` |
| **Terraform would fail every apply after the first storage autoscaling step** (it set `allocated_storage` back to the variable, a shrink RDS refuses). Found while fixing 1.5 | `ignore_changes = [password, allocated_storage]` |
| MAGPIE: the shutdown decision was untested, and a server-KLV decline without a job id claimed the same task straight back | `contribute_shutdown_waits_for_deferral`, asserted against birdtest's three shutdown fixtures; the claim body is checked against `claim-request.json`; a decline without a job id waits |

### 1.5 Plan wins

1. **RUNBOOK §1** passed the source's storage ceiling straight to the restore.
   RDS refuses a ceiling less than 10% above the allocation, which is exactly
   where autoscaling leaves an instance near its top. The ceiling is now derived
   to be at least a quarter above the allocation, and the section says when to
   raise `db_allocated_storage` before the closing apply.
2. **RUNBOOK §2.2** is to be saved and run as a script: pasted, its `exit` ended
   the operator's shell.
3. **README** has a post-apply check that every alert path delivers (a test
   alarm, the event subscription's status, a failing backup run). Nothing proves
   the three paths work under the new `aws:SourceAccount` condition. That
   condition is documented for the CloudWatch and RDS paths but not stated for
   the EventBridge-to-SNS path.
4. **PLAN**: `?status=` on the job list, the username length rule, and a new
   Known Limit (§4).
5. **TESTING.md**: `A-PUBLIC-1b`, the new error and SSE unit tests, and 490
   backend tests.

## 2. Objective 3

No change to any task setting. The reset functions and leak tests are unchanged
and pass.

## 3. Objectives 4–6

Nothing new on the claim, submit or heartbeat path. The fit-lock marking adds a
short wait to a purge when a fit is running, which is bounded by one fit. There is
nothing new on storage.

## 4. Unresolved, pending human feedback

All eighteen carried. One new item:

1. **Usernames accept any Unicode.** Zero-width and bidi characters and lookalike
   letters from other scripts get past the case-insensitive uniqueness. The
   options are to refuse format and control characters, to NFKC-normalize, or to
   restrict names to one script. This is recorded in Known Limits.

Weighed and left: the Force-rebuild refusal checks status, not open claims. A
`both` shutdown that is waited out prints no version advice. Stale local deferrals
can delay a real data shutdown by up to ten minutes. Admin-action pushes can lag
by up to the cache age.

## 5. `birdtest-contribute` (this pass, `c32ba750`)

| Change | Why |
|---|---|
| No include cycle | 1.1 |
| Tested shutdown decision; claim body against its fixture | the fixed behaviour was untested |
| NULL job id waits; no "0 seconds" line | 1.4 |

MAGPIE tests `contribute`, `config`, `rit`, `wmp` and `autoplay` pass. Its CI
lint steps pass on a clean archive. Tier 6 passes on the new binary.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds, and so does the nginx `1.30-alpine`
  bump. The tag exists and keeps its entrypoint's template behaviour, but it has
  not been run here.
- **Verified:** the lease token's precision; `exports::purge` inside the purge
  transaction has no lock cycle; `ApiPath` leaves the 400s that callers rely on
  as they were; CSRF on every mutating route; login's `goto(next.href)`; the
  scenarios reviewers ran against a fake server (waits, exits, stops, and a
  valgrind-clean 429 straddle).
