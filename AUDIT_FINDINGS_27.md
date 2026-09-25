# AUDIT_FINDINGS_27 — the thirty-first audit (2026-09-25), a loop of passes

Branch `audit/birdtest-2026-09-25`, off `audit/birdtest-2026-09-24-pass21`
(`7d51e3d`), the tip of the thirtieth audit's chain plus the audit prompts and
PLAN.md's numbered Known Limits. `main` (`5d35525`) holds none of the
eleventh-to-thirtieth audits' commits, so a branch off `main` would have audited
code that has since changed; this one is taken off their tip instead, as every
earlier pass branch was. MAGPIE is on `birdtest-contribute` at `7400184f`,
which `docker/Dockerfile` pins. **Still unpushed** (AUDIT_FINDINGS_8, header).

**Builds on** AUDIT_FINDINGS_7 to _26, and on PLAN.md's Known Limits
(KL-1 to KL-53), which are not re-flagged unless the code under them changed.

This is the first audit run under `BIRDTEST_AUDIT_LOOP_PROMPT.md`: at most four
passes, pass 1 full, later passes the diff plus one named area, stopping on a
pass with no high or medium finding (then one confirmation full pass).

## Summary (kept current)

- **Pass 1 (full):** 2 high and 7 medium from the reviewers, and 1 high and
  4 medium in the pass's own fixes from its adversarial checks; all fixed and
  verified within the pass. 31 low: 12 fixed, the rest KL-54 to KL-69. The
  loop continues (pass 1 found high and medium).

---

## Pass 1 — full pass

**Plan.** Every objective across the whole repository, split between five
reviewers who report only:
1. dispatch, races and public-route cost;
2. auth, statistics and the frontend;
3. storage, performance, RUNBOOK and README;
4. deployment, CI and security;
5. MAGPIE (`birdtest-contribute`) and objectives 3 and 8.

**Findings: 2 high, 7 medium, 31 low** from the five reviewers. One medium
(1.9) was reported as low and is medium by this audit's bar (a documented
guarantee that does not hold). One reported medium, the storage reviewer's
"multi-step procedures are still pasted Markdown", is low by the same bar
outside §2.2, which 1.2 fixes: no failure was shown in the rest, and it is
KL-66. The pass's adversarial checks then found 1 high and 4 medium in the
pass's own fixes (1.12, 1.13). **Every high and medium is fixed and
verified.** Of the lows, twelve are fixed (small, local and safe), and the rest
are recorded as KL-54 to KL-69, with KL-37 extended.

### 1.1 High — anyone, with no credentials, could run the web task out of memory through `POST /api/worker/result` (deployment reviewer)

**Code updated.** The route reads up to 64 MiB, and the ALB streams `/api` to
the backend unbuffered. The identity check ran inside the handler, after the
body had been read; `WorkerIdentity` returns `Unregistered` for a header-less
request without charging anything; and the three-slot bound on large results
was taken after all of it. Nothing bounded how many bodies were held at once.
Reproduced by the reviewer: twelve header-less 60 MiB uploads held open took
the backend from 38 MB to 778 MB resident, none answered before its body
completed. KL-6 had accepted "large results waiting their turn" on the grounds
that only capture jobs reach them; that did not hold.

**Fix**, as it stands after this pass's adversarial check (1.12) broke the
first version:
- bodies are read in two tiers (`extract::read_body`). A *small* body —
  declaring at most 1 MiB, or nothing — shares no budget, so nothing another
  caller does makes it wait; it has 30 seconds to arrive, and one declaring
  nothing is cut off at 1 MiB. A *large* body — only a result, from an
  identity already checked — reserves its declared length whole from a
  192 MiB budget (`extract::LARGE_BODIES`) before a byte is read, or is
  refused at once with `503` and `Retry-After` (MAGPIE retries 5xx for about
  fifteen minutes); one identity may hold 64 MiB of it at once; results over
  1 MiB are decoded three at a time; each has 30 s plus its
  size at 64 KiB/s and may not stall 30 s; the reservation is held until the
  handler returns (`ChargedJson`), so bodies waiting for a store turn count;
- the heartbeat, decline, result and artifact routes take `RegisteredWorker`,
  which resolves and rate-limits the identity and refuses an unregistered one
  from the headers, before the body;
- an identity-less claim is rate-limited and held to 16 KiB before its body;
  the other worker routes take 1 MiB rather than axum's 2 MB (see 1.10).

**Verified.** Failing first: `a_caller_the_route_refuses_is_answered_before_its_body`
timed out waiting for a body (5 s), `a_claim_body_is_small` got `204` for an
oversized claim. After: both pass, with `A-WORKER-18`'s two router tests and
four unit tests (`U-ERR-6`). Measured on a native backend: twelve header-less
60 MiB uploads — resident stays at 37 MiB, all twelve answered `401` at once;
twelve workers each holding a 60 MiB result one megabyte short — three
admitted, nine refused at once, 217 MiB resident, a heartbeat answered in 4 ms.
KL-6 is closed; what is left is KL-54.

### 1.2 High — RUNBOOK §2.2's copy-back loaded each table in one statement (storage reviewer)

**Code updated (procedure moved into a script).** Postgres queues one
after-trigger event per row for a foreign-key check and holds the queue in the
backend until the statement ends: measured by the reviewer at 12 bytes a row,
so a 20-generation leave job's 64 M progress rows would need about 780 MB in
one backend, more than the default `db.t4g.micro` has, failing the same way on
every re-run. Its `ON CONFLICT DO NOTHING` also dropped conflicting rows without
a word (the reviewer's low, fixed with it).

**Fix:** `scripts/restore-job.sh`, delivered to the ops shell through the ops
task definition (`RESTORE_JOB_SH`, written to `/tmp/restore-job.sh` by
`prod-shell.sh`). Batches of 200,000 rows, each its own transaction; each batch
checks that every row is now in production under its primary key *with the same
contents*, and raises (rolling that batch back) otherwise — a row re-seeded
since the purge stops the run with the reason, a row already there identically
is a re-run. It moves the three sequences forward. RUNBOOK §2.2 now runs it;
§2.5 (reactivate the job) was added, which the section never said.

**Verified.** Measured on 1.5 M progress rows: the RUNBOOK's statement peaked at
18.9 MB of `AfterTriggerEvents` (linear in the job), the script at 3.2 MB
(constant). `scripts/restore-job-check.sh`, new and in the nightly workflow,
runs it through: refused before the scratch restore finished; dump only;
a re-seeded row stops it with that batch not loaded and another job untouched;
the same run finishes after §2.0; five batches come back row for row; a re-run
changes nothing; the sequences are ahead. The ops container's command string
was run in `postgres:16` under `bash -c` and writes the script byte-identical.

### 1.3 Medium — the account page told contributors to pass `--api-key`, which MAGPIE refuses (auth reviewer)

**Code updated.** `magpie contribute` reads a key only from an `apikey` line in
`contribute.txt` (on purpose: a key on the command line is in shell history and
`ps`). Reproduced: `magpie contribute --api-key …` → "unrecognized command or
argument". The page now says so, shows the ready line under a fresh key, and
says to use one key per machine (1.6). `F-DOCS-1` (Vitest) reads the pages'
source: failed on both assertions first, passes now.

### 1.4 Medium — a builder-version bump stalled every job needing a wordmap or table (dispatch reviewer)

**Code updated.** `derived_data` rows were requested only at job creation and
activation; a deployment whose MAGPIE reports `wmp-2` finds every running job's
files built under `wmp-1`, reads them as pending, and nothing queues them: `204`
for good, with nothing on `/admin/derived-data`. MAGPIE_DEPENDENCY.md said "the
server rebuilds on a bump". **Fix:** `ready_for_job` queues any file with no row
under this binary's builder (`request_for_job`, idempotent; once queued it is no
longer a miss). Test `I-DERIVED-10` failed first (only the `wmp-0` row), passes
now. The harness's `derived_ready` now marks a claim-queued row built, as a
build would. RUNBOOK §2.4, MAGPIE_DEPENDENCY.md and PLAN updated.

### 1.5 Medium — leave selection turned to its tail mid-lap and hashed every sweep claim's racks (dispatch reviewer)

**Code updated.** The switch read only the summary row, so a half-hourly merge
that put it under the threshold mid-lap made every later claim hash the racks of
every sweep claim still out, inside the job's dispatch lock: measured by the
reviewer at 438–466 ms a claim with 1,000 claims of 500 racks out. The
reviewer's proposed fix (sweep while a cursor exists) was not enough: the lap's
end deletes the cursor with the lap's stragglers still out. **Fix:** the turn is
made only at a lap boundary, where nothing is in flight or staged
(`leave_gen::at_boundary`), and remembered as a cursor row with no rack
(`cursor_rack` is now nullable); a lap in progress runs to its end. Test
`I-LEAVE-19` failed first (mid-lap it handed out the tail's rack), passes now,
with the other 56 leave tests. Measured: with the tail's own worst case, all
50,000 below-target racks out at 400,000 racks, the selection is 67 ms warm
(96 ms cold). PLAN's schema copy, its selection section and the performance
table updated.

### 1.6 Medium — machines sharing a credential starved each other's heartbeats (MAGPIE reviewer)

**Code updated.** Every worker request was charged to one bucket per credential
(1/s, burst 5). Idle machines claim every five seconds and retry a `429` without
limit; a heartbeat is sent once and not retried. Reproduced by the reviewer with
eight real `magpie contribute` processes on one UUID against a mock with the
same limiter: 11 of 11 heartbeats from the busy one refused over 5.5 minutes —
its claim would have lapsed and its result been refused. **Fix:** a credential's
claims and its work in hand (heartbeat, decline, result, artifact) have separate
buckets (`ratelimit::WorkerBucket`); which one is decided by the extractor type
(`WorkerIdentity` for the claim route, `RegisteredWorker` for the rest), not by
path. The account page says one key per machine. Test `A-WORKER-14c` failed with
the buckets merged back, passes now; `A-WORKER-14`'s "the same bucket covers the
claim" assertion was changed to each bucket's own burst.

### 1.7 Medium — an admin could create a job no worker can run (MAGPIE reviewer)

**Code updated.** Two players on files of one role and name from different data
releases (the natural old-vs-new comparison after a MAGPIE-DATA update) made a
job whose claims state both digests; MAGPIE resolves a file by name, so one
always mismatches and every worker declines every task, and if the job is the
only one active, every worker is told to download data that cannot help.
Reproduced by the reviewer with the real binary against a mock. **Fix:** job
creation refuses, after writing the config in the same transaction, a job whose
`expected_data` has two entries of one role and name with different digests
(`refuse_one_name_for_two_files`); the same bytes twice are allowed. Test
`A-ADMIN-20` failed with the check disabled, passes now.

### 1.8 Medium — `scrub.sql` left every anonymous worker's credential in a scrubbed dump (storage reviewer)

**Code updated.** `X-Worker-UUID` alone authenticates an anonymous worker, and
scrubbing left every UUID (reproduced: the digest of 1,000 UUIDs unchanged).
**Fix:** each is replaced by a fresh UUID; claims, bans and audit rows
(`actor_anon_uuid`, and `target_id` of `worker` targets) follow it; open claims'
tokens are replaced. New rows first, old ones deleted last, so no foreign key is
switched off and no superuser is needed. Test `S-SCRUB-1` failed first (the UUID
unchanged), passes now, run twice, and pins the table's columns to the ones the
script copies. Run through psql exactly as `dev-restore.sh` does: 1,000 UUIDs,
digest changed, count kept. README and PLAN updated.

### 1.9 Medium (reclassified) — a leave transition that failed before it started kept ownership for half an hour (dispatch reviewer, reported as low)

**Code updated.** PLAN says a failure the server survives hands a transition
back at once; the config and distribution reads ran before the guard, so a pool
timeout there left the owner row for the 30-minute takeover. A documented
guarantee that does not hold is medium by this audit's bar. **Fix:** both reads
moved inside the guarded block. Test (under `I-LEAVE-13`) failed against the
committed `scheduler.rs` (not released within the wait), passes now.

### 1.10 A regression in this pass's own fix, caught by the full suite

With the other worker routes at 64 KiB, `boundaries::a_decline_list_is_cut_to_its_cap…`
got `413`. Looking at why showed a worse case: MAGPIE keeps its
`unsupported_jobs` list for the whole run without a cap (about 40 bytes a job),
and a claim refused for its size ends the run. The limit is 1 MiB (about 26,000
job ids); the budget, not the per-route limit, is what bounds memory.

### 1.11 Low findings fixed

- RUNBOOK §3 used `$ARTIFACTS_BUCKET`, `$JOB` and `$VERSION` without defining
  them, and no region: defined from the Terraform outputs, with the region; run
  in `bash -i` with `terraform` and `aws` stubbed.
- §2 never reactivated the restored job: §2.5.
- The copy-back's silent loss of conflicting rows: the script's row check (1.2).
- `docker/Dockerfile` and `infra/derived.tf` described a mounted volume that
  does not exist; the Dockerfile's header did not say the fake worker is
  test-only (objective 7's Python-worker rule): both corrected.
- A stray `contribute.txt` at the repository root (a local run's, committed in
  the twenty-fifth audit): removed and ignored.
- README said leave jobs write their universe at creation; it is seeded by the
  first claim of each generation. TESTING's `S-BACKUP-4` claimed a row in every
  table a result touches; it names the seven it seeds.
- PLAN disagreed with the UI in six places (plan updated): `/admin` redirects to
  `/jobs`; `/account` shows no confirmation status; the audit log filters by
  action and target type, not actor; a reset lands on `/login` with no message;
  a key's `last_used_at` is stamped at most once a minute; clone names and
  `cloned_from_id` are the caller's.

### 1.12 Adversarial checks

Two reviewers who did not write the fixes were given only the diffs.

**The code reviewer broke the first body budget (high) — redesigned.** As
first written, every body shared one 192 MiB budget, charged as bytes arrived
and waited for (ten seconds, then `503`) when spent, with a deadline computed
from the declared length even on small routes. Reproduced: 192 identity-less
claims each declaring 64 MiB and sending a megabyte filled it in 52 ms, and a
victim's heartbeat got `503` after ten seconds; still so at 70 seconds. A
heartbeat is never retried, so held past five minutes every claim in the fleet
lapses, and logins and bans got `503` too. Also (medium): four honest 60 MiB
uploads held-and-waited each other, and a heartbeat queued behind a large
waiter although it fit. This was the mechanism's second correction in the pass
(the first was 1.10's limit), so it was redesigned rather than patched: the two
tiers of 1.1, no waiting anywhere, reservations whole or not at all. Lows fixed
with it: the claim handler's rewrite of every rejection into "update MAGPIE"
dropped `Retry-After` (now only parse errors are rewritten); the buffer
reservation used the declared length even on small routes (now the tier's
bound). The bucket split's doubling of a credential's lookups went into KL-54.
The redesign was given to a further adversarial check (1.13).

**The scripts reviewer found two mediums in `restore-job.sh` — fixed.**
- *An empty restore reported success*: a mistyped id, `SCRATCH_URL` pointing at
  production, or a dump taken after the purge printed "nothing to restore" for
  every table and then the success line, and the operator would go on to reset
  the counters and reactivate an empty job. Now refused: the same database
  (server start time and database name), a job production has active, and a
  job the scratch copy holds no row of. `restore-job-check.sh` covers all three.
- *The half-hourly merge could fold restored staged rows mid-run*, and a re-run
  then stopped with the wrong diagnosis. Now the script holds the job's merge
  lock (the sweep skips a job whose lock is taken) from a psql coprocess for
  the whole load; the check asserts it is held mid-load. A re-run after a merge
  between two runs is documented (the hint and RUNBOOK say: §2.0, then from the
  start) and recorded in KL-69.
- Lows fixed: a missing dump file read as zero rows (now an error, and a
  `flock` on the work directory); the row check could hash-join the whole table
  per batch, 1.38 s and 243 MB of temp at 3.2 M rows (index probes forced);
  `split` doubled the largest table on disk (each file removed once split);
  batches by rows made a staged table one batch (now 16 MiB of text,
  `split -C`); an old task definition without `RESTORE_JOB_SH` wrote an empty
  script that did nothing (now `${RESTORE_JOB_SH:?}` stops the task; run in
  `postgres:16` both ways); `COPYBACK_DUMP_ONLY=0` meant dump-only; the error
  text for a secondary-index conflict; the nightly check now covers position
  analyses and all three sequences. Recorded: copying by column position, and
  UUIDs typed into free-text reasons (KL-69).
- **Found while fixing, not by the reviewer:** the first version of the merge
  lock released it with `kill "$HOLD_PID"` in an EXIT trap; running the check
  showed Postgres logging "server process … terminated by signal 15" and
  reinitialising. Inside the container the pid was a server process's. In the
  ops shell that would be the §2.1 scratch Postgres. The kill was removed: the
  coprocess ends, and the lock with it, when the script's end closes its input;
  the check was re-run clean, with no advisory lock or psql left behind.

### 1.13 The further adversarial check, on the redesign

A third reviewer, given the redesign and the script's changes, ran a native
backend against them. **Held:** identity-less uploads to the result, heartbeat
and decline routes answered `401` at once with memory flat; three stalled
60 MiB reservations and a fourth refused within five seconds, 317 MB peak with
three full uploads; a client that disconnects mid-body gives its reservation
back (the same credential's next upload admitted); claims keep every behaviour
MAGPIE relies on; small bodies never wait; the script's refusals, lock (the
`hashtext` keys match the Rust merge's), resume and sequences.

**Medium, fixed:** the rule of one large body per identity made a fleet sharing
one key, or one copied UUID, send its large results one at a time — reproduced
with two 8 MiB uploads on one key, the second refused. Each identity now has a
64 MiB share of the budget (`LARGE_BODY_SHARE_KIB`), so several smaller ones go
at once and no identity takes more than a third. Test
`a_worker_may_send_a_share_of_large_results_at_once` (two 8 MiB admitted, a
60 MiB beside them refused, reservations back as they end).

**Lows fixed:** the merge-lock session now turns off `idle_session_timeout` and
`idle_in_transaction_session_timeout`, which a parameter group could set;
results of 1 to 8 MiB took a large reservation but no store turn, so their
decodes were bounded by nothing (the turn's threshold is now 1 MiB); RUNBOOK
says a deleted job's `job_exports` rows are not restored. **Low left:** a
refused "no row of the job" run leaves its dumped files in the work directory
until the next run clears them.

### 1.14 Low findings recorded (Known Limits)

KL-54 (the large-result budget can be held by identities), KL-55 (redundancy above 1:
reclaim deadlock, an unregistered second slot), KL-56 (identity-less claims
mint identities), KL-57 (a waiting job's derived status on every claim), KL-58
(`worker_contributions` scales with the job), KL-59 (a failed sign-out), KL-60
(the job form's missing fields, the blank password), KL-61 (a refit skipped by
coincidence), KL-62 (Terraform cross-variable checks), KL-63 (secrets in Nginx
logs and MAGPIE's environment; unverified database TLS), KL-64 (infrastructure
hardening), KL-65 (alarm gaps), KL-66 (procedures still pasted), KL-67 (§4's
first check), KL-68 (small MAGPIE contribute items), KL-69 (what the copy-back
script does not handle). KL-37 now also names the
racing registration's `409` and `/api/users` listing unconfirmed accounts.

**Examined and sound** (from the reviewers): claim ordering, reclaim and submit
under `SKIP LOCKED`, first-result-per-task, the finish check, purge/delete lock
order; SPRT, pentanomial and Bradley–Terry (checked against scipy); every admin
route's `AdminUser` and CSRF; objective 3 in full — every outcome-affecting
MAGPIE setting is set by the server and applied by contribute, and a hostile
`settings.txt` gave a byte-identical result; the contract fixtures match
byte for byte; the version floor; the backup and drill scripts.
