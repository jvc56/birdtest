# AUDIT_FINDINGS_20 — the twenty-fourth audit (2026-09-25, fourteenth pass)

Branch `audit/birdtest-2026-09-24-pass14`, off `audit/birdtest-2026-09-24-pass13`
(`2ff986e`). MAGPIE changes are on `birdtest-contribute` at `f87d136c`, on top of
`50ee826f`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _19. No finding is critical. The most
serious is again in the previous pass's own work: its empty-bucket fix to the
restore drill could never run (1.1).

**Count: 12 code wins (9 birdtest, 3 MAGPIE), 8 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 The drill's empty-bucket fix could never run — **code updated** (the twenty-third audit's fix, corrected)

`aws s3 ls` exits 1 when a listing is empty. Under the drill's
`set -Eeuo pipefail`, the assignment that listed the manifests ended the
script, silently, before the check the twenty-third audit had added. The
drill of a new stack still failed and mailed "backups may not be
restorable", now with nothing in its log. Two reviewers reproduced this: one
against MinIO (rc 1, no output), and one from awscli's source and a stub.
`backup-drill-check.sh` had never drilled an empty bucket, so it passed.

**Fix:**
- The drill lists with `s3api list-objects-v2`. It exits 0 on an empty listing
  and still fails on a missing bucket or refused access.
- It passes only a bucket with nothing in it at all, saying so. A bucket with
  objects but no manifest under the prefix fails with a message: passing it
  would pass every drill of a misplaced prefix.
- `backup-drill-check.sh` covers both cases (`S-BACKUP-2b`), run here.

### 1.2 MAGPIE: after a REPL `contribute`, magpie would not start — **MAGPIE updated** (medium)

`impl_contribute` snapshotted `settings.txt`. At the end it replayed the
snapshot into the session and let the REPL's after-command save rebuild the
file from that session.

The failure: a snapshot taken before any lexicon was loaded names none but
still says `-w1 true`. Replayed over a task's lexicon whose wordmap the worker
lacks (the local data's `OSPS49` ships without one), the replay failed part
way, and the REPL saved the task's lexicon and `-w1 true`. From then on every
start of `magpie` failed loading that wordmap before running anything,
`contribute` included, and exited 0 without contacting the server.

**Fix:** the file goes back byte for byte, and the REPL's next save is skipped.
The replay into the session stays best effort.
`test_contribute_puts_the_settings_file_back` covers the restore and the
single skipped save.

### 1.3 The ETA's window, the credential gate and a failing job template — **code updated**

- **The ETA's window was measured in Rust before the query.** That left out
  the wait for a connection and any clock skew. Now:
  - it is read in the same statement, from the database's `now()`;
  - the hour stays a constant bound;
  - a forced generic plan uses `task_claims_completed_idx` (0.8 ms on 2M
    claims).
- **A job whose template fails to load was retried on every claim.**
  - Such a job never issues a claim, so it heads every candidate list.
  - Every claim of every worker then paid a pool connection, the read and
    parse, and an error line. It needs only a hand-edited row or a restore
    missing a config row.
  - **Fix:** it is passed over for a minute, without a connection or a log
    line, then tried (and logged) again (`U-DISPATCH-2`).

### 1.4 The distribution parser still disagreed with MAGPIE in four places — **code updated** (both sides)

| Case | MAGPIE | birdtest before | Now |
|---|---|---|---|
| A CRLF blank line (a lone `\r`) | refused (no columns) | skipped | refused |
| A field that is only `\r` | a (then empty) column: refused | dropped | refused |
| A count above 255 | kept in a byte: 256 read as none; a negative or past-`INT_MAX` one sized the bag wrong and wrote past it (ASan heap overflow) | accepted | refused on both sides |
| A letter of 6+ bytes | copied without its terminator into the next row's | accepted | MAGPIE refuses ≥ 6; birdtest refuses > 4, MAGPIE's shipped maximum, which every one of its buffers holds |

Every shipped distribution passes on both sides. `U-RACK-10` covers the
birdtest side, and MAGPIE's `ld` tests cover MAGPIE's.

### 1.5 The import page, yet again — **code updated**, with tests at last

The twenty-third audit's newest-answer check ordered reads within one watch
only:
- **A tick from an earlier watch could land in a newer one.** When an earlier
  import's tick answered after a new import started, it put that import on the
  page, cleared the new one's poll and forgot its id. Insert then confirmed the
  wrong import.
- **A stale error was not ignored.** An older read's error after a newer
  success still showed, and could stop the poll.

This is the third pass running in which this logic was wrong. It now lives in
`lib/importWatch.ts`, where each watch has a generation that makes every older
watch's answers stale. `F-IMPORT-1` (8 tests) covers each race.

Also:
- a 401 refreshes the session, so the admin layout sends the admin to sign in
  and back (`/login?next=`);
- the confirm shows the server's `inserted` count, not the staged one.

### 1.6 Other code fixes

| Item | Change |
|---|---|
| An empty or mistyped `region` or `dr_region` passed plan. An empty `region` falls back to the CLI's region, likely the lost one in RUNBOOK §5, and §5 never set the variables it used | Validated as region names; §5 sets them and guards them |
| Nothing ran the pinned MAGPIE through a real task: the PR checks do not, and the nightly tested the branch head. The pin that refused short opening racks would have passed everything | The nightly's tier-6 job is a matrix: the pin and the head |

### 1.7 Plan wins

1. **RUNBOOK §5 step 1:**
   - It sets `DR_REGION` and `THIRD_REGION`, and guards everything it uses.
   - It writes `dr.tfvars` with `printf`: copied from the raw file, the
     heredoc's indented `EOF` never ended.
   - The apply runs only once the placeholders are filled in.
2. **RUNBOOK §5 step 3:**
   - It guards its variables. An empty `REPLICA` made the copy's source the
     local root, and `--recursive` would have copied the operator's files into
     the Object-Locked bucket for 30 days.
   - It says what KMS access the copy needs, and that the copies take the new
     bucket's retention.
3. **The `azs` appends:** RUNBOOK step 4 and README step 4 append once, on a
   line of their own. A second paste redefined the attribute; a file without a
   trailing newline glued it onto the last line.
4. **README:** keep `prod.tfvars` (and `dr.tfvars`) with the state copy. The
   state holds no input variables, and every later apply reads them.
5. **RUNBOOK §2.1's comment** no longer says §5 sets `BUCKET` and
   `S3_REGION`.
6. **PLAN, drill:** the drill's empty-bucket behaviour.
7. **PLAN, short racks:** what a short opening rack means (the whole rack, a
   full bag), and why MAGPIE requires only one letter.
8. **TESTING.md:**
   - new entries `S-BACKUP-2b`, `U-DISPATCH-*`, `F-IMPORT-1`;
   - extended: `U-RACK-10`, and the nightly matrix;
   - the counts: 166 unit and 506 backend tests, 111 frontend.

## 2. Objective 3

The MAGPIE reviewer re-traced it at `50ee826f`, and nothing changed:
- All three executors reset per-player and shared settings before applying the
  request's.
- The challenge bonus and the endgame options are left alone, and nothing in a
  task reads them: the PlayChooser clock that would is reset to −1.

Short opening racks were run through tasks at 1, 2, 3, 6 and 7 tiles:
- static and simulated, with inference asked for (it is forced off);
- wordmap on and off, 1 and 3 threads.

All were well-formed, byte-identical across runs with the same seed, and clean
under ASan.

## 3. Objectives 4–6

Most severe first:
1. The failed-template retry on every claim (1.3). It cost a connection per
   claim fleet-wide while one job was broken.
2. The ETA window's skew (1.3). Display only.

The ETA plan is verified in custom and generic plans at about 1M claims. A job
holding most of `tasks` still hash-joins its own tasks (about 100 ms): the
payload is cached, and this is not a large pitfall. No storage change.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_19 §4, unchanged.

Weighed and left:
- **The drill's empty-bucket pass relies on `backup_stale`** to report a stack
  that should have backups; there is no drill-success alarm, as already
  weighed.
- **Distribution files are not parsed at import staging.** A file MAGPIE
  refuses imports, and is refused at job creation with the parser's reason.
- **Tearing down a DR copy within 30 days** needs `BypassGovernanceRetention`
  on the copied dump.
- **Terraform CI checks syntax only.** A `terraform test` with mocks would
  exercise the var-file flow; it needs `outputs.tf`'s DKIM output wrapped in
  `try`.
- **Password reset for a known address** commits a token synchronously, a
  millisecond-scale timing difference. It is no worse than the carried
  registration oracle.

## 5. `birdtest-contribute` (this pass, `f87d136c`)

| Change | Why |
|---|---|
| `contribute` restores `settings.txt` byte for byte and skips the REPL's next save; `test_contribute_puts_the_settings_file_back` | 1.2 |
| Distribution counts 0–255 and letters under 6 bytes; `ld_create` passes `true` as `ignore_empty`; `test_a_row_past_what_magpie_holds_is_refused` | 1.4 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build.
`format.py` and `find_circ_deps.py` pass on a clean archive. Tier 6 (14 Rust
tests, `M-1`…`M-11`) passes on the release build.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - **§5 mock plans.** `terraform test` with mock providers covers
    `prod.tfvars` plus `dr.tfvars` plus `-var azs=null`, and then the appended
    zones: every validation passes and the copy's zones are the DR region's.
  - **The zone-type filter** is valid.
  - **The copy's names:** none collides with the lost stack's.
  - **The cross-region copy** re-encrypts under the destination key, and the
    ops task can read it.
  - **§2.2's `rm -rf /tmp/dump`** is safe: nothing later reads it.
  - **The auth surface:** statistics payload types for every job type, and
    authentication, CSRF and pagination.
