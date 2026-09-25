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
- **Pass 2 (follow-up: pass 1's diff, and input-data import):** 1 high and 6
  medium from the reviewers — four of the mediums in pass 1's own fixes — and 3
  medium in the pass's own fixes from its adversarial check; all fixed and
  verified within the pass. Lows: most fixed, the rest KL-44, KL-54, KL-69 to
  KL-71. The loop continues.
- **Pass 3 (follow-up: pass 2's diff, and job exports):** 1 high and 4 medium
  from the reviewers (the high and one medium in pass 2's archive walk, which
  was on its third correction and was redesigned), and 1 medium in the pass's
  own fixes from its adversarial check; all fixed and verified. Lows: most
  fixed, the rest KL-70 to KL-73. The loop continues to pass 4, the last the
  budget allows.
- **Pass 4 (follow-up: pass 3's diff, and rating pools):** 1 high and 4 medium
  from the reviewers; the high and three mediums fixed and verified, **one
  medium left open** (the rating fit's bias and understated errors, KL-74,
  which needs a statistical decision). The adversarial check found no high or
  medium. The budget of four passes is spent, so **the loop stops here** with
  KL-74 open.
- **Run total:** 5 high and 21 medium found by reviewers across four passes,
  plus 1 high and 8 medium in the run's own fixes found by its adversarial
  checks; all fixed and verified but one (KL-74). Known Limits KL-54 to KL-77
  added, KL-6 closed, KL-37, KL-44, KL-62 and others updated. Final green run
  below; tier 5 not run (image builds).

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

---

## Pass 2 — follow-up pass

**Plan.** The diff since pass 1's starting point (`7d51e3d..7ca481e`; MAGPIE
unchanged), reviewed against every objective, one reviewer per part it
touches: backend; frontend; docs and procedures (RUNBOOK, README, PLAN,
TESTING, MAGPIE_DEPENDENCY, `scripts/`); infra (`infra/`, `docker/`, CI). Plus
one area not examined in recent passes: **input-data import** — the GitHub
fetch and resolve, the tarball walk and staging, confirm and its collision
rule, expiry of staged imports, the import page's polling, and how imported
rows reach jobs and derived builds.

**Findings: 1 high, 6 medium, 26 low** from the five reviewers (backend 1
medium and 4 low; frontend 1 medium and 2 low; docs and procedures 2 medium
and 12 low; infra 2 low; input-data import 1 high, 2 medium and 7 low). Every
high and medium is fixed and verified; the fixes' adversarial check is 2.9.

### 2.1 High — a PAX `size` record walked past every archive cap (input-data import reviewer)

**Code updated.** `walk_archive` judged an entry by `entry.header().size()`, the
ustar field; the tar reader follows a PAX `size` record instead. A header
saying 0 and a PAX record saying N passed the per-entry cap, was read whole,
and only then counted: reproduced by the reviewer, a 4 MiB gzip took the walk
to 927 MiB resident before "expands more than 20x", and a 300 MiB entry was
accepted against the 128 MiB cap. About 12 MiB compressed would OOM the web
task. **Fix:** the size is `entry.size()`, and the per-entry cap and the ratio
judge it before the entry is read, pinned or not; reads are bounded and must
equal it. `U-ARCHIVE-8` fails against the committed walk ("unexpected EOF during
skip" — it read the entry) and passes now.

### 2.2 Medium — a tarball naming one path twice staged and confirmed two rows (import reviewer)

**Code updated.** Reproduced: two `lexica/NWL23.kwg` rows confirmed, of which a
worker extracting the tarball holds only the last, so a job pinned to the other
is declined by every worker. **Fix:** a pinned path (file or alias) seen twice
refuses the archive. `U-ARCHIVE-9` failed against the committed walk, passes now.

### 2.3 Medium — PLAN's 30-minute import limit was not enforced (import reviewer)

**Code updated.** The client's timeouts are per connection and per read;
reproduced, a download trickling a piece every 20 s was still `running` at
130 s, and one at 1 KB/s would hold its import, and up to 512 MiB, for days.
**Fix:** `run_import_within` bounds download-to-staged at 30 minutes and fails
the row with the reason. `I-INPUT-10` (a 3 s limit, a piece every 2 s) passes;
the failure first is the reviewer's run.

### 2.4 Medium — an identity-less claim sent without a length was not held to 16 KiB (backend reviewer; pass 1's own fix)

**Code updated.** Pass 1's check read only `Content-Length`; a chunked claim
went to the small tier's 1 MiB. Reproduced natively: 300 such claims from 12
addresses took the backend from 37 to 494 MiB. **Fix:** the identity-less
claim's body itself is bounded (`http_body_util::Limited`, 16 KiB). The new test
`a_chunked_first_claim_is_held_to_its_bound` timed out (5 s) against the
committed handler and passes now.

### 2.5 Medium — pass 1's account page broke end-to-end journey E-2 (frontend reviewer)

**Code updated.** The fresh-key box's second `<code class="break-all">` made
E-2's `locator('code.break-all')` match two elements, a strict-mode failure, so
tier 5 and CI's `e2e` job would fail. Reproduced by the reviewer with e2e's own
Playwright on the rendered markup. **Fix:** both elements have test ids, and E-2
reads the key by id and checks its `apikey` line. Verified the same way (the
old locator fails, the new one passes on the new markup) and by
`playwright test --list`, which compiles every spec; `F-DOCS-1` pins the ids.
**Tier 5 itself was not run:** it builds the Docker images, which this machine
builds only with the user's go-ahead (see the final output).

### 2.6 Medium — the copy-back restored a copy taken after the mistake (docs and procedures reviewer)

**Code updated.** Pass 1's refusal caught only an empty copy. A purge leaves an
active job dispatching (§2.0 says so), so a nightly dump taken after it holds
the job's new rows; reproduced, the script loaded 3 tasks and printed
`restored`. **Fix:** the script reads production's last `job.purged` or
`job.deleted` audit row for the job and refuses a scratch copy holding any task
of the job made at or after it. The check's new case failed against the
committed script ("restored from a copy taken after the purge") and passes now.

### 2.7 Medium — restoring a deleted job had no working procedure (docs and procedures reviewer)

**Code updated.** The RUNBOOK said to re-insert the `jobs` and config rows "the
same way", which was the pasted loop pass 1 removed; the script stopped on the
first foreign key. **Fix:** with no `jobs` row in production the script
restores, first, the `input_data` and `player_configs` rows the job names (only
if missing; checked by key, since they are shared), the `jobs` row forced
`inactive`, and its config row; references to accounts deleted since are
cleared. RUNBOOK §2.0 and §2.2 say so; the check's new case restores a deleted
job with a player config and a leaves row deleted with it, row for row.

### 2.8 Low findings

**Fixed:**
- The ops shell no longer stops when its task definition predates
  `RESTORE_JOB_SH`: `/tmp/restore-job.sh` is then a stub that says to apply
  `infra/` (pass 1's `${…:?}` took §2.1, §2.3 and §5 down with §2.2). Run in
  `postgres:16` both ways.
- The copy-back: the merge-lock psql released the script's `flock` and waited
  on the lock without limit (fd 9 closed, `lock_timeout` 60 s); `split -C` broke
  a line longer than `BATCH_BYTES` (a batch is now at least the longest line);
  "about 400,000 rows" was 170,000; the check leaves no lock file behind.
- Contributor instructions put `contribute.txt` "beside" MAGPIE; MAGPIE reads it,
  and `data/`, from its working directory (home page, account page, README).
- The job form's duplicate-file refusal advised renaming a file, which the UI
  cannot do; it says to choose players from one data release.
- The derived build's remedy text promised re-importing fills in a missing
  object key; it does not (I-DERIVED-7 updated).
- Stale text: comments on the one-at-a-time rule and a `BODY_BUDGET` that no
  longer exist; "fixed by activating it again"; PLAN's `/admin` rows and tree;
  the 8 MiB threshold (now 1 MiB); KL-58's, KL-64's and KL-65's claims; "every
  table a result touches" in README, PLAN and `restore-roundtrip.sh`; the
  RUNBOOK §6 and TESTING coverage and nightly lists now name
  `restore-job-check.sh` and `scrub.sql`; `S-SCRUB-1` moved out of the nightly
  scripts' list; `leave_gen.rs` has 30 tests, not the 31 pass 1 wrote.

**Recorded:** KL-44 (an import holds its lexica too, some 300 MB), KL-54
(opening-rack results over 1 MiB; a slow link's large claim), KL-69 (a
re-imported input row under a deleted job; a refused run's files), KL-70
(small import items: progress, audit rows, concurrent collision labels,
expiry), KL-71 (the claim's "update MAGPIE" message for other 400s).

### 2.9 Adversarial check of the pass's fixes

A reviewer who wrote none of it was given the diff (`7ca481e..` working tree).
**Held:** `entry.size()` follows PAX and GNU sparse sizes; truncated entries are
refused; the real release tarballs (built `cp -RL`) have no aliases or
duplicates, and `classify` maps one to one, so nothing correct is newly
refused; the claim bound gives `413` and MAGPIE's first claim is tiny; bash 5.2
and mawk in `postgres:16` handle the script; the E-2 test ids match.

**Medium, fixed — a copy taken after a leave job's purge still passed.** A purge
writes the job's generation-0 artifact row back, so such a copy holds one row
and no tasks, and the task-timestamp check never fired: reproduced, the script
"restored" one artifact row and said so. The check now asks whether the scratch
copy holds the audit row of production's last purge or delete; that row is
written in the purge's own transaction, so its presence is exact — and it also
ends a false refusal the timestamps allowed (a PITR point between a purge's
start and its commit) and any dependence on DateStyle. The check covers a games
job that went on running and a leave job with only its artifact back.

**Medium, fixed — a deleted job stopped after its `jobs` row never got its
config.** The prelude ran only while production had no `jobs` row, so a re-run
skipped it and reported success: reproduced with a one-shot trigger. The
prelude now runs every time (the `jobs` row checked by key only, since a purge
changes its counters); the check stops a first run on the config and resumes.

**Medium, fixed — tar extension headers were read whole, outside every cap.**
The tar reader reads a PAX header's or a GNU long name's data in full before the
walk sees an entry: reproduced, a 400 KB gzip with a 400 MB PAX header held
465 MB and was accepted, and a 4 GB directory entry was inflated and discarded
in 8 s. Now a first pass over the raw entries refuses an extension header over
64 KiB before the walk reads it, and a counting reader bounds every
decompressed byte at 1 GiB (and 20 times the download, past a 64 MiB floor).
`U-ARCHIVE-10`'s two tests fail against the committed walk and pass now.

**Lows fixed:** the split test now uses batches smaller than a line; the
deleted-completed job's status step (RUNBOOK §2.3) no longer says §2.0 brought
the row back; the time-limit message. **Recorded:** a reused player-config name,
clone lineage, and an old task definition's script (KL-69); an expired import's
walk runs on (KL-70).

---

## Pass 3 — follow-up pass

**Plan.** The diff since pass 1's commit (`7ca481e..b9b4ec5`; MAGPIE and infra
unchanged), reviewed against every objective, one reviewer per part it touches:
backend; frontend and the end-to-end spec; docs and procedures (RUNBOOK,
README, PLAN, TESTING, `scripts/`). Plus one area not examined in recent passes:
**job exports** — the build task and its time limit, the object-store uploads,
the export rows' states and expiry, the download route, what a purge or delete
does to an export in flight, and the admin UI for them.

**Findings: 1 high, 4 medium** from the four reviewers (backend 1 high, 1
medium, 3 low; frontend 1 medium outside the diff, 2 low; docs and procedures 1
medium, 5 low; exports 1 medium, 6 low). All fixed and verified; the fixes'
adversarial check is 3.6.

### 3.1 High — GNU sparse extension blocks were expanded inside the tar reader, and a PAX `size` desynchronised the first pass (backend reviewer): the walk redesigned

**Code updated.** Both came from the tar reader interpreting extensions inside
its own `next()`, where no cap of the walk's reaches. A `GNUSparse` header with
extension blocks made the reader build 64 bytes of bookkeeping per 24-byte slot
before the walk could refuse the entry: reproduced, 1.1 GB peak from a 44 MB
gzip, about 2 GiB from 78 MB (high). And a PAX `size` record ahead of an
ordinary entry made pass 2's raw first pass and the interpreting walk disagree
on where every later header starts, so a large PAX header slipped past the
64 KiB refusal: reproduced, 508 MB peak, archive accepted (medium). This was
the archive walk's **third correction** in the audit (2.1, 2.9, now), so it was
redesigned rather than patched again: the walk reads the archive **raw**, in
one pass, and reads extension headers itself — each bounded at 64 KiB, taking
only a path or link target for the next entry — refusing a PAX `size` or sparse
record and a sparse entry, none of which a release needs. Every entry's size is
the header's own, which is what the raw reader follows, so nothing can
desynchronise, and the first pass (a second full decompression) is gone.
**Verified:** `U-ARCHIVE-8` (a PAX size refused, and one that cannot move the
next header) and `U-ARCHIVE-11` (a sparse entry refused unexpanded, under
16 MiB held) fail against the pass-2 walk ("no recognisable data files";
85 MiB held) and pass; `U-ARCHIVE-12` (PAX `path` with `mtime`, GNU long
names) passes on both. By hand: 51 release files (133 MB) from `~/MAGPIE/data`
packed by GNU tar and, with a PAX header on every member, by Python's
`tarfile`, both walk to the same 37 pinned files.

### 3.2 Medium — a reader that stopped early left its corpus query running (exports reviewer)

**Code updated.** sqlx drains a pool connection dropped mid-result before
returning it, so each hung-up spot check of the results stream, and each export
whose upload failed, left Postgres building the whole corpus on a connection
neither cap counted: reproduced, ten spot checks held a ten-connection pool,
`pool.acquire()` timed out at 30 s, and the scans ran on 90 s. PLAN's "at most
two streams" did not hold. **Fix:** the stream's and the export's connection is
acquired explicitly and closed on drop rather than drained. `I-EXPORT-9` failed
before (a scan still running two seconds after the hang-up) and passes; the
other export and stream tests pass.

### 3.3 Medium — a purged job that completed again ended §2 completed, with a verdict from deleted results (docs and procedures reviewer)

**Code updated.** A purge leaves an active job active; a small job or a
force-complete can reach `completed` again before anyone notices, and then
§2.0's deactivate and §2.5's activate are both refused and §2.3's status step
does not apply: reproduced end to end with the RUNBOOK's own blocks.
**Fix:** §2.0's transaction returns a job completed since the purge to
`inactive` with its verdict cleared, and `restore-job.sh` refuses a `completed`
job (a purge leaves a completed job inactive, so one completed now completed
again). The block was run through psql on such a job; the check has the case.

### 3.4 Medium — the site at phone width scrolled sideways, and E-10 could not see it (frontend reviewer, outside the diff)

**Code updated.** The header's links ran to 533 px on a 393 px Pixel 5, "Sign
in" and "Register" off screen, and a phone's browser widened its layout
viewport to fit, so E-10's `scrollWidth <= innerWidth` always passed. TESTING
guarantees E-10 "renders correctly at phone width". **Fix:** the header wraps
(and the page padding is smaller on a phone); E-10 compares with the device's
width. Checked with e2e's Playwright against a build of the pages, API mocked:
the old layout 533 px (fails the new check), the new one 393 px with every
header link wholly on screen. Tier 5 itself was not run (image builds).

### 3.5 Low findings

**Fixed:** the audit-row refusal also matches the row's action and job; RUNBOOK
§2.2 says only the last purge or delete is checked; "two databases" (three now)
in RUNBOOK §6 and the check's header; PLAN's "one transaction" description of
the selective restore; the release's size (190 MB in five chunks, 1.3x, not
94 MB and 3-4x) in `inputdata.rs`, PLAN and KL-44; the contributor
instructions' "a directory of its own" (a `contribute.txt` of its own, most
simply in a directory of its own).

**Recorded:** KL-70 (an import timing out mid-commit), KL-71 (a chunked first
claim's generic 413), KL-72 (exports: objects left by a delete, duplicate rows
above redundancy 1, PITR and ready rows, a hidden older download, presigned
link lifetime, delete markers and old rows), KL-73 (the instruction test's
single phrasing).

### 3.6 Adversarial check of the pass's fixes

**Held:** the redesigned walk against GNU tar's gnu, oldgnu, posix, pax and
ustar formats (a ustar prefix path, long names, a 120-character name, long link
targets by `K` and PAX `linkpath`) and Python's gnu and pax archives — right
names and digests; `..` and absolute paths, hard links and escaping links via
PAX, sparse and `size` records, an over-long long name, truncated entries — all
refused; `close_on_drop` discriminates (without it three scans ran on 15.7 s,
with it none) and costs nothing that matters; the check script, the purge's own
reset of a completed job, and E-10's assertion on the old and new header.

**Medium, fixed — §2.0 left a since-completed job's export.** An export of the
post-purge results (possible once the job completed again) survived §2.0, and
once §2.3 completed the job again it would be served as the restored job's
corpus, by the stream's redirect and the admin page — the reason a purge
deletes exports. Reproduced with the block as written. **Fix:** §2.0 deletes the
job's export rows, and the script refuses while any exist; both run (the block
through psql, the refusal in the check).

**Lows fixed:** a global PAX `path`, a second PAX header, and a long name beside
a PAX path each named an entry differently from GNU tar — now refused (a
comment-only global header, as `git archive` writes, still walks;
`an_entry_given_two_names_is_refused`); a long username or the link column
could still widen or squeeze the phone header (it truncates, and the links take
their own row; E-10's check re-run on the build); the export held its
connection through the tail upload; KLV generation's read had the same drain on
an early exit (closed on drop too); POSIX-format extension headers counted
toward the 5,000-entry cap (now only entries do; their bytes are counted).
**Left:** E-10 was checked against a build with the API mocked, not run end to
end (tier 5 builds images).

---

## Pass 4 — follow-up pass (the last the budget allows)

**Plan.** The diff since pass 2's commit (`b9b4ec5..d03837d`; MAGPIE and infra
unchanged), reviewed against every objective, one reviewer per part it
touches: backend (the raw archive walk above all); frontend and the phone-width
journey; docs and procedures. Plus one area not examined in recent passes: **the
rating sweep and rating pools** — pool creation and membership, the fit and its
prior, refits and their triggers, history thinning and residuals, the ratings
routes and pages.

**Findings: 1 high, 4 medium** from the four reviewers (backend 1 medium and
10 low; frontend 1 medium outside the diff and 4 low; docs and procedures 7
low; rating pools 1 high, 2 medium and 5 low). Three mediums and the high are
fixed and verified; **one medium is left open** (4.3, KL-74). The pass's
adversarial check found no high or medium (4.6).

### 4.1 High — the public rating-history route scanned every pool's ratings (rating-pools reviewer)

**Code updated.** `pool_history` joined its kept runs to `player_config_ratings`;
the planner guessed thousands of kept runs (there are at most 501) and hash-
joined a sequential scan of every pool's rows: on a month of 2-minute runs in 11
pools (4.75 M rows) 510–590 ms an anonymous request here, 0.76–1.1 s for the
reviewer, and growing with every pool, against a display pool of eight
connections. **Fix:** each kept run's ratings are read by the primary key
(`CROSS JOIN LATERAL … OFFSET 0`): 125–240 ms on the same data, independent of
the other pools, and the plan shows the key's index. The adversarial check
confirmed identical rows and order over 11 pools and three limits; the history
tests pass.

### 4.2 Medium — a restored deleted job never re-entered the ratings (rating-pools reviewer)

**Code updated.** The copy-back puts the `jobs` row back first, carrying its
counters; a sweep during the load recorded the new total as seen, fitted part
of the rows, and nothing moved the total again: reproduced, no refit during or
after the restore, a rival stored at 2000 against 2462.5 on the full evidence.
**Fix:** `restore-job.sh` ends by marking every pool for a refit
(`mark_every_pool_for_refit`'s statement and locks), and RUNBOOK §2.4 says so.
The check asserts no pool's newest run counts its evidence as seen afterwards.

### 4.3 Medium, left open — the rating fit biases large or thinly linked groups and understates their errors (rating-pools reviewer)

**Plan updated, code left pending a decision.** The prior (two virtual draws per
member against the anchor) pulls a thinly linked group together, MM converges
very slowly on such a group and stops at its 10,000-iteration cap, and the
diagonal standard errors leave out the link's uncertainty. Measured with
noiseless evidence against the real fit: a 12-member cluster 42 Elo low, shown
±1.9 against about ±28; a 20-config chain's top 36.5 low; a 30-member cluster
about 200 low. PLAN's claims ("lands in microseconds for any plausible size",
errors "more than good enough") are corrected; the fix — a Newton solve with
the full information, a weaker or targeted prior, full-covariance errors —
changes every published rating and the prior's strength is a statistical
choice, so it is **KL-74, open**, and the first thing to take up next.

### 4.4 Medium — the raw walk refused a correct archive whose PAX values hold a newline (backend reviewer; pass 3's redesign)

**Code updated.** The tar crate's `PaxExtensions` splits on newlines rather than
record lengths, so an extended-attribute value with a 0x0a — which POSIX allows
and GNU tar (`--xattrs`) and macOS write — refused the archive; and it read a
keyword after two blanks as another keyword, letting a `size` record past its
refusal. **Fix:** the walk's own parser splits records by their stated lengths.
With it, the reviewer's lows where the walk and GNU tar disagreed were closed
too: a second PAX header before one entry, a base-64 size field, a directory or
link with data, a ustar version other than `00`, a NUL in a PAX path, `./` and
`//` spellings of one file (normalised as GNU tar does), and a tarball gzipped
in several members (all read, as `tar -xzf` does). `U-ARCHIVE-13`, `-14`, `-15`
and the spelling test fail against the pass-3 walk and pass; the realistic GNU
and Python PAX tarballs still walk to the same 37 files, and the reviewer walked
the real 190 MB release to its 75.

### 4.5 Medium — the job page scrolled sideways on a phone with a long registered name (frontend reviewer, outside the diff)

**Code updated.** The contributors table had no scrolling box, so a name of 21
to 31 characters (by width) widened the page past the screen, which E-10's
anonymous-only data could not show. **Fix:** it scrolls in its own box, as do
the account page's key table, the pool list, a pool's config table and the job
page's two other tables; the account page breaks long values; the admin tabs
wrap. The reviewer's sweep (12 widths × 5 sessions × 16 routes) had no page
wider than the screen afterwards, and the extra spec over the pool and job
pages is clean at every phone width.

### 4.6 Adversarial check of the pass's fixes

**Held:** every correct archive tried (GNU tar 1.30 in five formats with
`--xattrs` and `./` prefixes; Python gnu and pax with newline values; a 128 MB
posix tarball plain, split, in two members, with trailing zeros or garbage)
walks and names files as `tar -t` does; the PAX parser's edge cases; the ustar
check spares GNU and v7 headers; the history query's rows and order; the refit
mark (no pools, no deadlock); the frontend wrappers. **Lows fixed:** a file
named with a trailing `/` (which GNU tar makes a directory, reading on into its
data) — refused; a base-256 size is accepted only as a positive eight-byte one;
the pool page's and job page's remaining tables; the refit mark's output and
its failure message. **Recorded:** KL-77 (a tarball with no end blocks and
zero padding after its member; lexical link resolution) — both fail safe.

### 4.7 Low findings

**Fixed** (docs and procedures reviewer): PLAN's archive paragraph and
TESTING's `U-ARCHIVE-10` described the reader interpreting extensions;
`I-EXPORT-9` claimed test coverage of the export half; a re-run after §2.3's
status step got advice that would undo the restore (the message now says the
restore is done); the new refusals are listed in RUNBOOK §2.2 and both scripts'
headers; §2.4's exports bullet; two stale size comments. **Recorded:** KL-75
(history thinned by count; `bingo_bonus`/`sim_cutoff` outside a pool's scope;
no-op membership changes), KL-76 (header tap targets; E-10's anonymous-only
contributors), KL-77.

---

## Final green run (after pass 4)

- `cargo clippy --all-targets -- -D warnings`: clean.
- Full backend suite with `TEST_DATABASE_URL`, tier 6's opt-in tests included
  (`MAGPIE_BIN` a `portable_release` build of `7400184f`): **539 of 539
  passed**.
- `npm run check`: 0 errors, 0 warnings; `npm test`: 115 of 115.
- `terraform fmt -check -recursive` and `terraform validate` (the
  `hashicorp/terraform:1.9` image, on a copy of `infra/`): clean.
- `scripts/restore-roundtrip.sh` and `scripts/backup-drill-check.sh` on an
  isolated compose stack with the current schema: both passed.
  `scripts/restore-job-check.sh`: passed.
- MAGPIE did not change during the run, so its test table and tier 6's native
  run were not required; tier 6's Rust half ran in the suite above.
- **Not run:** tier 5 (`e2e/run.sh`), which builds Docker images. E-2's and
  E-10's changes were checked with e2e's Playwright against builds of the pages
  with the API mocked.
