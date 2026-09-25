# AUDIT_FINDINGS_21 — the twenty-fifth audit (2026-09-25, fifteenth pass)

Branch `audit/birdtest-2026-09-24-pass15`, off `audit/birdtest-2026-09-24-pass14`
(`a7d2eba`). MAGPIE changes are on `birdtest-contribute` at `e7d04183`, on top of
`f87d136c`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _20. This pass's reviewers were asked to report
as findings only real defects: wrong results, data loss, security,
availability, material performance, races, or a documented guarantee that does
not hold. Nits were to be listed apart.
- **No real defects:** the dispatch-and-races and the deployment reviewers
  found none.
- **Real defects:** four in all, one from each of the other three areas
  (public API, RUNBOOK, MAGPIE) and one found by me. Two are in the previous
  pass's own work (1.2, 1.3).

**Count: 9 code wins (5 birdtest, 4 MAGPIE), 7 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 A made-up cursor on a public feed could hold the display pool — **code updated** (availability)

The leave-generation results feed reads a generation at a time, newest first,
and stepped to `current - 1` after each. Nothing checked the cursor's
generation against the job's real ones.
- **Effect:** a cursor naming generation 2,147,483,647 stepped down one empty,
  fast read at a time, so the statement timeout never fired. Each request held
  a display-pool connection for as long as the client waited.
- **Exposure:** the route is public and unmetered, leave job ids are listed by
  `/api/jobs`, and the display pool has eight connections. A handful of such
  requests starved every public page.
- **Fix:** after a short page, one probe of the primary key
  (`MAX(generation) … AND generation < $2`) finds the next generation that has
  rows. A forged cursor costs one empty read and one probe.
- **Test:** `I-LEAVE-16b`.

### 1.2 RUNBOOK §5's guards did not guard — **plan updated** (the twenty-fourth audit's guards, corrected)

The twenty-fourth audit guarded §5's dangerous blocks with `: "${X:?}"` lines.
Pasted into an interactive shell, a failed expansion abandons only its own line,
and the pasted lines after it run (reproduced).
- **Worst case, step 3's cross-region copy:** with `REPLICA` and `MANIFEST`
  unset and the regions set, the copy's source was the local root. With
  `--dryrun` it listed the machine's files for upload into the Object-Locked
  bucket, where they would sit for 30 days.
- **Fix:** each block is now one `if` that runs only when every variable is
  set (tested in an interactive bash with stubbed `aws` and `terraform`).

Also changed in §5 and §6:
- **Step 1** keeps an existing `dr.tfvars` only when it names the same region;
  one left by a drill is not reused. §6 says to move it aside after a drill.
- **The workspace step** uses `workspace select -or-create`.
- **The zone pin** captures `terraform output` first, so a failed output no
  longer writes a bare `azs =` that the idempotence check then took for done.
  README's pin is the same.

### 1.3 MAGPIE: the settings restore could still break magpie, and the session — **MAGPIE updated** (the twenty-fourth audit's restore, corrected)

The restore replayed `settings.txt` from disk, and two things went wrong.

**The file (reproduced).** With `-savesettings false` there is no snapshot. The
older file on disk turned saving back on when replayed, the replay failed part
way, and the REPL then saved the task's lexicon with `-w1 true`. magpie failed
at every start, as before the twenty-fourth audit's fix.
- **Fix:** the in-memory snapshot is replayed, and nothing when there is none.

**The session (reproduced three ways).** A snapshot taken before any lexicon
was loaded names none but says `-w1 true`. Replayed over a task's lexicon whose
wordmap the worker lacks (the normal case: a wordmap exists only for jobs that
pin one), it left the session holding that lexicon with the flag on. Every later
command, another `contribute` included, then failed.
- **Fix:** when the replay fails, or there is no snapshot, wordmaps and rack
  info tables are turned off so the session loads. The file is not saved from
  it.

**Tests:**
- `test_contribute_leaves_an_unsaved_session_unsaved`;
- `test_a_session_loads_after_contribute`. It dies with the reported
  error 153 when the fallback is removed (checked).

### 1.4 MAGPIE: a failed command could swallow the next command's save — **MAGPIE updated** (found by me)

The twenty-fourth audit's skip of the REPL's after-command save was cleared only
by that save, which runs only after a command that succeeds. A command that
ended in an error left it set for the next command.

The MAGPIE reviewer showed that today's REPL always resets the error before
that point, so it cannot trigger. It is still cleared explicitly on an error
(defensive), and the test covers it.

### 1.5 Other code fixes

| Item | Change |
|---|---|
| Import page: a new fetch left the previous import on screen with a live Insert until the new one's first read. Confirming it forgot the new import's id, and `confirm` read `current` after its awaits | `current` is cleared on start; `confirm` works on the id it started with and forgets the stored id only if it is still that one |
| `watcher.stop()` did not stop a later `watch()`: a start the admin left the page during began a poll nothing could clear | Stopped for good; a watch after stop starts nothing (tested) |
| MAGPIE copied the fullwidth display forms (columns 6–7) without a length check, the bug the twenty-fourth audit fixed for letters | Both sides refuse a form of 6 bytes or more (birdtest: more than 5) |
| The nightly's "Which MAGPIE" step pasted the dispatch input into its script | Passed through `env:` |

### 1.6 Plan wins

1. §5's guards (1.2).
2. `dr.tfvars` is kept only for the same region, and §6 has a post-drill note.
3. `workspace select -or-create dr`.
4. The zone pins (RUNBOOK and README) capture the output first.
5. The Dockerfile's comment names the nightly's pin and head legs.
6. TESTING.md:
   - `I-LEAVE-16b`;
   - `F-IMPORT-1` (stop);
   - `U-RACK-10` (fullwidth);
   - the nightly `backup-drill` line with `S-BACKUP-2b`;
   - the counts (507 backend, 112 frontend).
7. PLAN's list of findings files.

## 2. Objective 3

Re-traced by the MAGPIE reviewer at `f87d136c`: no gap. Leftover `settings.txt`
values (`-cb`, `-otpenalty`, `-ttfraction`, `-eplies`, `-ritmmap`, `-gp`, `-hr`)
are either overridden or unused on the contribute paths, and the derived-file
flags are set per task.

## 3. Objectives 4–6

- **Most severe: the forged-cursor walk (1.1).** It held the display pool for
  as long as a client liked.
- **Measured and verified:**
  - The drill's listing: an empty bucket; objects under another prefix; a
    missing bucket; 1,100 stamps across auto-paginated pages; a page of
    prefixes only. `--max-keys 1` turns off pagination.
  - The ETA's clamp matches its earlier form.
  - The negative cache's map is bounded by the number of jobs.
- No storage change.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_20 §4, unchanged.

Weighed and left (nits the reviewers listed apart):
- **A transient database error while loading a job's template** parks the job
  for 60 s on that process. Recording only lasting failures would need telling
  them apart.
- **A job whose derived build failed** still costs a connection and two
  queries per claim. A pending build is deliberately asked about each time
  (PLAN); a short negative cache for the failed state would match the
  template one.
- **SSE edge cases:**
  - A panic inside a stats push leaves the job's push entry behind until a
    restart.
  - A stream subscribing just after a delete holds its place on keep-alives.
- **SPRT with zero variance** returns 0 even when the mean is not 0.5, so a
  config that wins every game runs to `max_units`.
- **`eusc-de-east-1`** is refused by the region check. It is in its own
  partition, and `arn:aws:` is already a recorded limit.
- **Import confirm on a 409** leaves the button live, and the job pages read
  their id once.

## 5. `birdtest-contribute` (this pass, `e7d04183`)

| Change | Why |
|---|---|
| Replay the snapshot, not the file; fall back to derived files off so the session loads; two tests | 1.3 |
| A failed REPL command drops a pending save skip | 1.4 |
| Fullwidth display forms length-checked | 1.5 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build.
`format.py` and `find_circ_deps.py` pass on a clean archive. Tier 6 (14 Rust
tests, `M-1`…`M-11`) passes on the release build.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - **The nightly matrix tests what it says.** The backend's `MAGPIE_BIN` and
    the workers both use the mounted checkout, so the pin leg runs the pin
    throughout.
  - **The drill role's IAM.** Its `s3:ListBucket` has no prefix condition, so
    both listings are allowed.
  - **The region pattern** accepts every region in the standard, GovCloud,
    China and ISO partitions tried.
  - **Every shipped distribution** loads on both sides with the new checks.
  - **The printf-built `dr.tfvars`** is identical whether pasted from the
    rendered or the raw file.
  - **The idempotent zone pins** leave valid HCL when run twice, with and
    without a trailing newline.
  - **The auth surface:** `importWatch`'s generations, the 401 flow through
    `/login?next=`, and the inserted count.
