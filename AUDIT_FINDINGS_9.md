# AUDIT_FINDINGS_9 — the thirteenth audit (2026-09-24, third pass)

Branch `audit/birdtest-2026-09-24-pass3`, off `audit/birdtest-2026-09-24-pass2`
(`b43d8b7`) for the same reason pass 2 branched off pass 1: `main` has neither
earlier pass's fixes, and this pass re-audits them. MAGPIE changes are on
`birdtest-contribute`, at `465c2e1f`, on top of `dc6c4426`; `docker/Dockerfile` pins the
result. **Still unpushed** — the one human action this branch needs before it
can merge (AUDIT_FINDINGS_8, header).

**Builds on** AUDIT_FINDINGS_7 and _8 (same day) and, through them, the ten
before. Two of the twelfth audit's fixes had defects of their own (1.1, 1.2)
and one was misplaced (1.3); this pass finishes them.

**Count: 19 code wins, 6 plan wins, 7 unresolved pending feedback** (all
carried from AUDIT_FINDINGS_7 and _8; none new).

---

## 1. Code versus PLAN.md, and the previous pass's fixes

### 1.1 A rebuilt KLV was refused by every worker — **code updated** (the twelfth audit's fix, finished)

- The twelfth audit had workers verify the leave KLV against
  `leave_generation_artifacts.sha256`, which keeps the *first* hash on purpose.
  `rebuild_artifacts` rewrites the object (missing, or `force`) with bytes a
  changed builder can legitimately change — so after the runbook's own
  missing-object procedure, every task of the next generation failed its check
  on every worker, each counting a failure toward the five that end a run.
- **Fix:** `served_sha256` (NULL while the object holds the first bytes), set by
  a rebuild that rewrites the object; workers are sent
  `COALESCE(served_sha256, sha256)`. MAGPIE now declines a mismatch as
  `derived_mismatch` (role `klv`) — the job set aside for the run — instead of
  failing the task. Tested in the real-MAGPIE rebuild test (`I-LEAVE-8`).

### 1.2 A dropped purge released its hold while its locks were held — **code updated** (race)

- The ALB dropping a purge dropped its transaction (its rollback then waited for
  the running statement, minutes) and its `DispatchHold` at once: claims and
  submissions went back to waiting on the job's locks, and the reclaim grace
  could run out before the locks were released, so the job's claims were
  reclaimed anyway. **Fix:** purge and delete run on a task of their own
  (`run_to_completion`), which owns the hold and the transaction: a dropped
  request no longer cancels the operation, and the hold lasts exactly as long
  as the locks. (Consequence recorded in Known Limits: a large purge now
  finishes past the load balancer's timeout, the admin told nothing.)

### 1.3 The large-result bound was waited on inside the transaction — **code updated** (critical path)

- Each waiter held a pool connection and its claim and task rows for up to 30 s,
  and the permit ended before the insert. **Fix:** `large_result_turn`, taken
  before `begin()` and held to commit. The remaining gap — waiters keep their
  bodies resident — is recorded in Known Limits.

### 1.4 Two purges sharing contributors could deadlock at their ends — **code updated**

`Contributions::count` reads in id order, so `give_back` locks rows in a fixed
order.

### 1.5 Other code fixes

| Item | Change |
|---|---|
| A squattable tombstone blocked deleting an account (`<id>@deleted.invalid`, ids public, address unique) | Random tombstone address (`A-ADMIN-18`) |
| Bans could not be listed or lifted from the UI; a UUID's kind was guessed from the first page of contributors, sending most anonymous UUIDs as user ids (refused) | `GET /api/admin/workers/bans`; the page states the kind and lists bans with Unban (`A-ADMIN-18`) |
| The admin users page said deleting removes results and rolls counters back | Text says what happens (anonymized; results kept; ban or purge to discard) |
| No link to a job's admin page | "Manage" for admins on the job page |
| Per-key worker limit untested; key churn unbounded | Per-account bucket (10/s, burst 50) beside the per-key one; test `A-WORKER-14b` |
| `/api/workers` hashed every anonymous contributor per page view | Hashed after the LIMIT |
| Case-variant usernames coexisted | Unique index on `lower(username)`; registration checks case-insensitively (`A-AUTH-4c`) |
| Pages swallowed failures (reset request, account key actions, users, audit log, input data, player configs; an export poll outliving its page) | Errors shown; poll stops on unmount |
| Exports unbounded | One running per job (partial unique index, 409, `I-EXPORT-8`); at most two build at once |
| Contributor list: nondeterministic ties, a second grouped scan for the count | Tie-broken order; `count(*) OVER ()` from the same scan |
| Progress and ETA counted tasks for on-demand jobs (a 1%-done opening-rack job read "3 minutes left", ~99% on the list) | Opening racks: racks analysed of the rack space, ETA in racks ÷ (batch ÷ redundancy); leave: generations closed, no ETA |
| `tasks_job_idx` had no reader that needed `state` | Dropped |
| Token tables looked up without indexes | Indexes on code/token hashes and user ids |
| DR artifacts bucket had no lifecycle | Noncurrent-version expiry and multipart abort, as the primary |
| MAGPIE: the RIT/WMP writers wrote in place; the KLV temp name was per task, not per process; a permanent 429 loop printed nothing; the contract test did not list `previous_artifact_sha256` | Temp sibling + rename (`temporary_sibling`, `rename_into_place`); per-process temp name; "the server is rate-limiting this worker" after five; key added to the test |

### 1.6 Plan wins (documents changed to match the code)

1. **Ops scripts ignored the stack's region** (a region-loss run went to the
   lost region): a `region` output; both scripts export it and print it.
2. **`prod-sql.sh` gave up after ten minutes** (the waiter's cap) while the SQL
   ran on: an uncapped poll. `prod-shell.sh` gained the start-failure handling.
3. **RUNBOOK §1** `state rm`/`import` now use `-var-file=prod.tfvars`; README
   documents keeping the variables there (`*.tfvars` ignored); §5 step 2 names
   the DR region and instance; §2.3's restore SQL handles NULL verdicts with
   `NULLIF`; §2.1's scratch server turns parallel query off; `SHELL_HOURS`
   documented; §2.3's rack recount filters in-game positions.
4. **The drill's disk message** named a knob already at its maximum; it now
   names the alternative, and the ceiling is in Known Limits.
5. **PLAN.md** still described the fixed KLV name in two places and said the
   nightly round-trip "does not exist yet"; corrected.
6. **TESTING.md** cited the wrong test for `U-ARCHIVE-7`; corrected; counts
   updated (474 backend tests).

## 2. Objective 3

Re-traced from scratch against `dc6c4426` and this pass: no setting that can
change a result is taken from local config or a previous task. The leave KLV is
verified, content-named and read afresh each task; a player's lexicon and
leaves are required; wordmap and table use are set before the load; everything
else is required and reset first. Threads remain the contributor's own
(accepted).

## 3. Objectives 4–6

Performance, most severe first: (1) the large-result wait inside the
transaction — fixed (1.3); (2) a dropped purge's early release — fixed (1.2);
(3) `/api/workers` hashing every contributor — fixed; (4) the contributor
list's second scan — fixed; (5) token lookups unindexed — fixed; (6) exports
unbounded — fixed. Storage: `tasks_job_idx` dropped; the DR bucket's
noncurrent versions now expire; fillfactor's costs recorded.

## 4. Unresolved, pending human feedback

All carried, unchanged: claim idempotency (AUDIT_FINDINGS_7 4.1); the
purge/delete redesign (7 4.2 — now finishing past the ALB, see 1.2, but still
one long transaction); scheduling-history retention (7 4.3); rating pools
cannot be edited or deleted (7 4.4); the registration state oracle (8 4.5);
residual retention (8 4.6); plies-as-arrays and the moves key (8 4.7).

## 5. `birdtest-contribute` (this pass)

| Change | Why |
|---|---|
| KLV hash mismatch declined as `derived_mismatch`, not failed | 1.1 |
| RIT/WMP written beside the name and renamed; KLV temp per process | Two processes sharing a data directory |
| A sustained 429 is reported once | A claim now waits them out without limit |
| Contract test requires `previous_artifact_sha256` | The key MAGPIE now requires |

MAGPIE tests run: `contribute`, `config`, `rit`, `wmp`, `wmpmaker`,
`builderhash`, `ap_wmp` pass. `ap_rit` needs a prebuilt 1.9 GB `TWL98.rit` (its
CI shard builds one first) and was not run here.

## 6. Other

- **Python worker as a production client:** none (the dev-stack profile was
  moved out in the twelfth audit, and nothing refers to it now).
- **Verified and unchanged:** the pentanomial CHECK against every insert path
  and fixture; `WITH (fillfactor)` placement and dump behaviour; the drill
  preflight's tools; the replication rules' HCL; `Contributions` exactness; the
  `SKIP LOCKED` touches; `close_generation`'s merge lock; SPRT, pentanomial and
  Bradley–Terry maths; CSRF on every mutating route.
