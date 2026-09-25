# AUDIT_FINDINGS_19 — the twenty-third audit (2026-09-25, thirteenth pass)

Branch `audit/birdtest-2026-09-24-pass13`, off `audit/birdtest-2026-09-24-pass12`
(`a86d2ec`). MAGPIE changes are on `birdtest-contribute` at `50ee826f`, on top of
`0a69b625`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _18. No finding is critical. The three most
serious are regressions from the twenty-second audit's own work, found and fixed
here:
1. **Opening-rack jobs with racks under 7 tiles** could not be worked (1.1).
2. **The ETA query** no longer used its index (1.2).
3. **The credential gate** let a refused flood grow memory (1.3).

A further correction to AUDIT_FINDINGS_18: its §1.6 and §5 said MAGPIE requires
full opening racks. That was the regression (1.1); it now requires at least one
letter.

**Count: 10 code wins (7 birdtest, 3 MAGPIE), 9 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 Opening-rack jobs with racks under 7 tiles could not be worked — **MAGPIE updated** (high; the twenty-second audit's change, corrected)

The twenty-second audit made the opening-rack executor refuse any rack that was
not exactly `RACK_SIZE` letters, to stop an empty rack being analysed as a pass.
But birdtest's opening-rack jobs choose their rack size, from 1 to 7:
- `create_job` allows that range;
- the schema's CHECK allows it;
- `seed.py --rack-size` uses it;
- the request does not restate the size.

So every task of a job with smaller racks was refused ("server sent an unusable
rack"), and each worker stopped after five failures. That is one job stopping
every worker that claimed it.

This was found three ways: by me while the reviewers ran, and independently by
the MAGPIE and the auth reviewers. It was reproduced in tier 6 on `0a69b625`:
`YYZ` refused five times, then the run ended.

**Fix:**
- **MAGPIE `34bf6f0c`:** refuses only an empty rack. Over-long and malformed
  racks are the draw's to refuse, as before.
- **Tier 6:** `M-3` now runs a 3-tile job. It passes on the fix and failed on
  `0a69b625`.

Stating `rack_size` in the request, so MAGPIE could check it exactly, is
possible hardening but is not done here.

### 1.2 The ETA query stopped using its index — **code updated** (performance; the twenty-second audit's rewrite, corrected)

The twenty-second audit bounded the scan by
`greatest(now() − 1 h, activated_at)` through a CTE. The planner sees that as an
InitPlan parameter it cannot estimate, assumes a third of the rows, and drops
`task_claims_completed_idx`. It then seq-scanned both tables on every live push
and detail view. Two reviewers measured it:
- 133–260 ms against 10–26 ms, at about 1–2M claims;
- 60 ms against 1.8 ms on a small, recently activated job.

**Fix:**
- The hour is again a constant bound, `now() − interval '1 hour'`.
- The activation window is a second condition, with its bound and length
  computed in Rust.
- EXPLAIN on 2M claims shows an index scan on the completed-claims index,
  0.5 ms.
- The migration's and PLAN's comments now say the bound must stay one the
  planner can read.

### 1.3 A refused credential flood still grew memory — **code updated** (security; the twenty-second audit's gate, corrected)

`CredentialGate::admit` charged the credential's own bucket before its
address's. Every made-up key or UUID therefore became a new entry in the
`worker` limiter, kept until the ten-minute sweep, even when its address then
refused it. A 429 costs no database work, so one address could send them as
fast as the network allowed. Measured: a million refused requests left a million
entries, about 150 MB, and the task has 2 GB.

**Fix:** an unknown credential pays its address first, and only then its own
bucket. `A-WORKER-16`'s unit test asserts that a refused flood of 1,000 leaves
the limiter's size unchanged.

### 1.4 birdtest read letter distributions more loosely than MAGPIE — **code updated**

`LetterDistribution::parse` accepted files that MAGPIE refuses:
- it trimmed every line and field;
- it skipped `#` lines;
- it asked for three or more columns.

MAGPIE skips only empty lines and does not trim. It takes 5 or 7 columns (empty
fields dropped), integer counts and scores, and a vowel flag of 0 or 1. The
reviewer ran the real `magpie`: a `# upper,lower,…` header and a line of two
spaces both fail it.

So job creation's new check, from the twenty-second audit, passed files that
every worker and builder then refused. Worse, a `#` row is a letter to MAGPIE
but was skipped here, which shifted every machine letter after it: `klv.rs`
bakes those numbers into node bytes.

**Fix:** the parser mirrors MAGPIE's rules. `U-RACK-10` checks each case. Every
shipped distribution (MAGPIE's seven, its three test files, birdtest's fixtures)
reads the same as before.

### 1.5 Other code fixes

| Item | Change |
|---|---|
| The import page forgot a staged import on a 401 or 403 (a lapsed session, "sign out everywhere"), which is the re-download the twenty-first audit meant to prevent | Only a 404 or 400 forgets it; a 401 or 403 stops polling and keeps it |
| The page's first read, answering after a quicker poll had seen the import stage, put `running` back with nothing left polling | The poll's first tick is the first read, and only the newest answer is applied |
| The restore drill, run on a bucket with no backups yet, exited 1 and mailed "backups may not be restorable": a new stack's schedules come on before its first backup, and the drill runs on the 1st at 05:00 | Exits 0 with a log line; missing backups are the backup-stale alarm's to report (it is `breaching` on missing data). `backup-drill-check.sh` and `restore-roundtrip.sh` pass |
| The zone list relied on the opt-in filter alone to leave out Local and Wavelength Zones, which sort first | A `zone-type = availability-zone` filter too |
| MAGPIE: `rackequity2klv`'s message named a missing or lower-case letter, but a rack also fails to parse when it is too long or has a malformed `[..]` letter | "does not parse as a rack of its letters", keeping the phrase tier 6 matches |
| MAGPIE: the oversized-distribution test left its file in `testdata` if an assert failed | Removed before asserting |

### 1.6 Plan wins

1. **RUNBOOK §5 step 3's restore could not be done as written.** The ops task
   it runs in can read only its own stack's backups bucket, not the replica,
   and the step's last sentence contradicted the rest. Now:
   - the dump and its manifest are copied across from the operator's machine
     (`--source-region`);
   - §2.1's first block reads them from the stack's own bucket;
   - the fallback to the previous stamp is a real command (it printed a whole
     `ls` line).
2. **The DR overrides were never persisted.** README now pins the primary's
   zones into `prod.tfvars`, which every DR apply loads, so a DR apply without
   the right override planned the lost region's zones into the new VPC. The
   overrides now live in `infra/dr.tfvars`, and every DR command takes
   `-var-file=prod.tfvars -var-file=dr.tfvars`. The first apply passes
   `azs=null`, and step 4 appends the copy's own zones. The step-1 and step-4
   instructions no longer disagree.
3. **§2.2's space guidance under-counted.** Measured: the loaded rows and
   indexes came to 1.5× the dumped text, and the WAL to 3.2×. It now says to
   allow twice the total plus `db_max_wal_size_mb`. The copy-back also removes
   `/tmp/dump` once it is restored, so it does not share the ops task's disk
   with the job's rows.
4. **README step 4's zone pin** is a command (`echo … >> infra/prod.tfvars`),
   not a placeholder.
5. **README step 3** says the certificate's validation record can read `null`
   for a few seconds.
6. **PLAN's ETA paragraph** covers the activation window and why the hour stays
   a constant.
7. **PLAN's copy of the schema** had kept the pre-twentieth-audit index comment
   (the stalled flag reading the index). It is synced with the migration.
8. **TESTING.md:**
   - `U-RACK-10`;
   - the `A-WORKER-16` order;
   - the 3-tile `M-3` job;
   - the counts: 165 unit tests, 505 backend tests in all.
9. **AUDIT_FINDINGS_18's "full racks"** is corrected above, and **PLAN's list
   of findings files** gains `_18` (the twenty-second audit did not add it)
   and `_19`.

## 2. Objective 3

The MAGPIE reviewer re-traced it at `0a69b625`, and nothing changed:
- all three executors reset shared and per-player settings (the PlayChooser
  included) before applying the request's;
- they require every outcome key, and the simulation keys whenever
  `num_plies > 0`;
- they take threads and seed from the request.

Opening racks copy the player's settings into the run-wide ones with inference
off. The one setting left alone is `print_on_finish`, which is cosmetic.

## 3. Objectives 4–6

Most severe first:
1. **The ETA seq scan (1.2).** Every live push and detail view grew with the
   whole claims table.
2. **The refused flood's memory (1.3).**
3. **§2.2's WAL under-count (1.6).** A full volume stops writes.

The submit path's inline work is unchanged (the finish check). No storage
change.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_18 §4, unchanged.

Weighed and left:
- **The known-credential map can be filled.** Anonymous UUIDs minted at the
  unregistered rate can fill it (17 addresses for ten minutes). Once it is full,
  new credentials pay their address's bucket. That hurts only someone sharing an
  address with the flooder, and remembered workers keep their places.
- **A banned identity is remembered** before its ban check. It still pays its
  own bucket, and MAGPIE ends its run on the 403.
- **The ETA's window uses the app's clock** and the hour the database's. The
  skew is negligible.
- **`rack_size` is not stated in the request** (1.1). This is hardening, not a
  bug.
- **MAGPIE trusts the server** on batch and response size, per the worker's
  trust model.
- **The player-config form** does not say that a blank simulation field means
  MAGPIE's default.

## 5. `birdtest-contribute` (this pass, `50ee826f`)

| Change | Why |
|---|---|
| `34bf6f0c`: an opening rack is at least one letter, not a full rack | 1.1 |
| `50ee826f`: `rackequity2klv`'s message; the `ld` test cleans up first | 1.5 |

MAGPIE's `contribute`, `gameplay` and `ld` suites pass on the sanitizer build.
`format.py` and `find_circ_deps.py` pass on a clean archive. Tier 6 passes on
the release build: the 14 Rust tests, and `M-1`…`M-11` with `M-3`'s new 3-tile
job.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - **§2.1's blocks** were run in `postgres:16` with awscli against MinIO:
    - `S3_REGION` expands correctly and is absent when unset;
    - a deleted file mismatches, and a re-run after fixing it matches;
    - the restore gives `pg_restore exit 0`, and the documented clean restart
      works.
  - **§2.2:**
    - `COPYBACK_DUMP_ONLY` leaves production untouched;
    - the URL rewrite behaves the same in bash and dash on seven URL shapes.
  - **Terraform:**
    - `required_version >= 1.9` matches CI's 1.9.8;
    - `dr_region`'s validation fires for a us-west-2 stack and passes §5's
      overrides;
    - `scheduled_tasks_enabled` gates all three schedules;
    - `terraform validate` and `fmt` pass.
  - **MAGPIE's traffic** under the per-credential bucket: one artifact per task
    at most, heartbeats 30 s apart and joined before a submit, 429s retried with
    `Retry-After`.
  - **The already-confirmed path** rolls back cleanly and sees a concurrent
    confirmation.
