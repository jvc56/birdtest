# AUDIT_FINDINGS_25 — the twenty-ninth audit (2026-09-25, nineteenth pass)

Branch `audit/birdtest-2026-09-24-pass19`, off `audit/birdtest-2026-09-24-pass18`
(`9b730de`). MAGPIE changes are on `birdtest-contribute` at `913a874b`, on top of
`8cc0b31c`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _24. Same severity bar as the twenty-fifth:
only real defects count as findings.
- **No real defects:** none of the five reviewers came back clean.
- **Real defects:** every reviewer found at least one.
  - Several are in the previous passes' own work:
    - the `?worker=` cursor, the third correction of that page (1.6);
    - RUNBOOK §6's blocks, for the third pass running (1.3);
    - the thread cap (1.4);
    - the recorder rule's texts (1.5).
  - One is older: rolling back a release (1.1).

**Corrections to AUDIT_FINDINGS_24:**
- **§1.2** said the refusal of a `best` recorder "stays, for static players".
  The code refused every player; it now refuses static ones only (1.5).
- **§1.4's cap of 512 threads** did not prevent the exit it was added for
  (1.4).

**Count: 6 code wins (5 birdtest, 1 MAGPIE), 5 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 Rolling back a release left the service down — **code and plan updated** (availability; deployment reviewer)

The backend runs `sqlx::migrate!` at start with sqlx's default: a migration the
database has and the binary does not is an error (`VersionMissing`). So the
previous image, started against a schema a newer release had migrated, exited.
The service keeps no healthy task through a deploy
(`deployment_minimum_healthy_percent = 0`), so a rollback crash-looped with
nothing serving. No document covered rolling back.

**Fix:**
- `db::migrate` sets `ignore_missing`. An applied migration whose file
  *changed* is still refused.
- README's "After a schema change" states the rule that makes this safe:
  after release a schema change is a new, additive migration, and drops and
  renames wait a release.
- RUNBOOK has a new "Rolling back a deploy" section:
  - the three image tags go back together;
  - so does `min_magpie_version` if the release raised it;
  - a non-additive migration means fixing forward, or §1.

**Test:** `A-MIGRATE-1`: a database with a migration the binary lacks
migrates, and one with an edited migration is refused.

**Also:** nothing alarmed when the site was down, whether from a crash loop, a
health check that never passes, or such a rollback. New
`birdtest-{backend,frontend}-down` alarms fire on no healthy target for ten
minutes, which is longer than a deploy's gap and its migrations. Missing data
counts as breaching. They exist only while `desired_count` is above 0.
`terraform validate` and `fmt` pass.

### 1.2 A simulating player's `score` sort meant two things — **code updated** (wrong results; MAGPIE reviewer)

The two job types sort a simmer's candidates differently:
- **Games:** autoplay's simulating player generates its candidates sorted by
  equity (`get_top_simming_move` hard-codes it).
- **Opening racks:** the executor, since 1.2 of the last pass, generates them
  with the player's own sort.

So a `score` simmer ranked the top `num_plays` plays by score in an opening-rack
job, with each move's reported `equity` being its score, and by equity in a
games job. The reviewer reproduced it on rack EEIIOUU:
- equity simulated exchanges and picked one;
- score simulated only plays and reported exchanges at 0.0.

**Fix:** config creation refuses `sort_strategy = 'score'` for a simmer, since
a simmer's candidates are the top plays by equity. A static player may still
sort by score. The following now say so:
- PLAN's parameter table;
- the schema comment (in the migration and in PLAN's copy);
- the form's option.

**Test:** `I-JOB-14b`, two cases in
`a_simming_player_config_is_bounded_by_iterations_not_time`.

### 1.3 RUNBOOK §6: failures still passed as success — **plan updated** (the twenty-eighth audit's §6, corrected again)

The storage reviewer ran every block in `bash -i`, with the real Terraform
1.9.8 and a stubbed `aws`:
- **Staging block 1.** `for … || break; done && echo staged`: `break` exits
  with status 0, so a failed `leaves` sync printed "staged". Block 2 then
  uploaded the partial copy and deleted it. **Fix:** the syncs are chained
  with no loop; a `.staged` marker is written only when all four succeed, and
  block 2 requires it.
- **The teardown's apply** carried `prod.tfvars`' zones for a drill stopped
  before §5 step 4 (the likeliest to fail, at the restore), which pins the
  copy's own. It planned the subnets into zones `$DR_REGION` lacks and
  stopped the chain at its first step, so the copy of production's data
  stayed up. **Fix:** `-var azs=null` whenever `dr.tfvars` has no `azs`, on
  the apply and the destroy.
- **The after-destroy block** ran unchained. A failed snapshot delete or
  `rb` still deleted the workspace and moved `dr.tfvars`, which erased the
  record of the drill while the snapshot, a full copy of production's
  database, remained. That snapshot also stops the next drill's `destroy`.
  **Fix:** chained, with "already gone" counted as done, so a re-paste
  finishes the job:
  - the snapshot: `DBSnapshotNotFound`;
  - the bucket: a 404 from `head-bucket`.
- **Block 2 could not be re-pasted** after a failed upload: `s3 mb` refuses a
  bucket the account owns outside us-east-1. **Fix:** `head-bucket` first.

Also, from the reviewer's nits:
- block 1 reuses `$STAGE` across re-pastes and says when no manifest is listed;
- `empty` uses `mktemp` files;
- the teardown checks the regions and the account before it selects the `dr`
  workspace, so a refused run leaves `default` selected;
- an unreadable state is reported as that, not as "already destroyed".

The same stubbed-shell method covered every case again:
- a failed sync, then a re-paste;
- block 2 without a whole stage, and with the bucket already made;
- the teardown with and without pinned zones;
- an unreadable state;
- a failed listing;
- the wrong account;
- the after-destroy block failing on `rb`, then finishing on a re-paste.

### 1.4 MAGPIE: the thread cap did not prevent the exit — **MAGPIE updated** (the twenty-eighth audit's cap, corrected)

Move generation's pool has `MAX_THREADS` (512) slots, and a task at N threads
holds up to 2N+1 of them:
- a simulated game per thread;
- each game simulating on N more;
- the contribute thread's own slot, which it keeps across tasks.

The reviewer reproduced "movegen pool exhausted" at `threads 512` on an
opening-rack simulation, and with autoplay at 300. The default of cores − 1
reaches it on a large enough machine.

**Fix:** `CONTRIBUTE_MAX_THREADS = (MAX_THREADS - 1) / 2`, which is 255.
**Test:** `test_a_runs_threads_are_capped`, with `threads 5000` in the file.

### 1.5 The recorder rule refused a working config, and its texts said otherwise — **code updated** (auth reviewer; the twenty-eighth audit's §1.2 follow-through)

With 1.2 of the last pass, a `best` simmer ranks as many moves as it records.
The rule still refused it, and five texts said a `best` simmer "has nothing to
choose between":
- the API's field message;
- the player-config form;
- the job-new warning;
- a test's doc comment;
- PLAN.

**Fix:**
- the refusal and the job-new warning apply to static players only;
- the texts say what a `best` recorder does and does not limit.

Refused for any player, static or simulating, is `num_plays` below
`num_plays_recorded`. An opening-rack analysis sizes its move list from
`num_plays`, so every rack would store fewer moves than asked (found while
fixing this).

**Test:** `I-JOB-14c`:
- a `best` simmer asking for ten is accepted;
- a simmer recording twenty of twelve candidates is refused.

### 1.6 `?worker=` with a cursor could read the contributor's whole range, or every position record — **code updated** (availability; dispatch reviewer; the twenty-seventh and twenty-eighth audits' page, corrected)

A page past the first broke the tie at the cursor's time with a row comparison,
`(completed_at, id) < ($4, $5)`. Postgres estimates a row comparison from its
first column alone, so it applied the claim range's own `completed_at <= $4`
selectivity a second time. The estimate was 818 rows where 25,880 were real.
Expecting fewer rows than the page, the planner dropped the ordered scan that
stops at the page:
- for games, it read and sorted the contributor's whole range before the
  cursor;
- for opening racks with few records per claim, it hash-joined a sequential
  scan of **every position record in the fleet**.

The older the cursor, the worse. Cursors are the client's to send, and ordinary
paging makes old ones.

The reviewer measured on about 2.6M claims:

| Case | Time |
|---|---|
| Opening racks, few records per claim | 0.47–2.0 s |
| Games, 200–500 a page | 0.1–0.6 s |
| Two identities | 55–125 ms |

First pages were fine.

**Fix:** inside the range, the row comparison equals
`NOT (completed_at = $4 AND id >= $5)`, which the planner estimates at about
1. The reviewer verified it on the same bench:
- identical pages (same md5) for a merged two-identity page and a cursor
  inside a tie;
- NULL cursor parts behave as before;
- every failing case became the ordered index scan, at 2–70 ms.

The two page templates are now functions (`opening_rack_page`, `game_page`),
so `A-PUBLIC-3c` checks them for the negation and for no row comparison.
A-PUBLIC-3 and 3b still pass.

A sturdier follow-up is weighed below: add `id` to the identity indexes and
make the row comparison the index condition.

## 2. Objective 3

The MAGPIE reviewer re-traced the opening-rack path at `8cc0b31c`:
- a `best` and an `all` simmer give byte-identical results at one thread;
- `config_init_game` before `game_reset` changes nothing: bag, seed, racks
  and scores are untouched;
- the candidate list is bounded by `num_plays`;
- `num_moves` agrees with what the server checks.

The only gap was the sort (1.2). Its fix is on birdtest's side, since no config
that reaches MAGPIE can now carry it.

## 3. Objectives 4–6

Most severe:
1. **The `?worker=` cursor plans (1.6).** Seconds per public request.
2. **The rollback outage (1.1).** Latent until the first release after launch.
3. **The `score` simmer's two meanings (1.2).**

Unchanged elsewhere. The storage reviewer found nothing new that grows without
bound. Dropping the `state` predicate from `?worker=` changes no result, since
`completed_at` is set only with `state = 'completed'`.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_24 §4, unchanged.

Weighed and left (nits the reviewers listed apart):
- **Ratings:** `build_matrix` counts a self-play `game_pairs` job in the pool
  page's "N pairs from M jobs", though the fit drops it.
- **Pentanomial:** the label for bucket 2 is "Split 1-1", but it also holds
  draw+draw pairs.
- **Pool page:** a membership change whose refit fails after the commit shows
  the error and does not reload. The sweep repairs the fit.
- **`admin.rs`:** a field message's literal carried runs of spaces. The new
  messages use `\` continuations; one older one remains.
- **Ties at one `completed_at`:** a contributor with thousands of claims
  sharing one completion time pays for the whole tie group on each page
  (50k tied claims: 0.1–1 s). A claim's time is its submitting transaction's
  own `now()`, so a client cannot create ties, and real ones are rare. Adding
  `id` to the identity indexes, and a claim id to the opening-rack cursor,
  would make the tie an index condition too.
- **Candidate order:** autoplay's simmer simulates candidates in heap order,
  and opening racks sort them first. Both are valid, but tie-breaking can
  differ.

## 5. `birdtest-contribute` (this pass, `913a874b`)

| Change | Why |
|---|---|
| `CONTRIBUTE_MAX_THREADS = (MAX_THREADS - 1) / 2`; `test_a_runs_threads_are_capped` | 1.4 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build.
`format.py` and `find_circ_deps.py` pass on a clean archive. Tier 6
(`M-1`…`M-11`) passes on the release build.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified by the reviewers:**
  - **Statistics:**
    - SPRT bounds and LLRs;
    - the Bradley–Terry fit, its standard errors and residuals;
    - the ETAs and progress numbers;
  - **Every destructive admin action's** confirmation, CSRF check and error
    path;
  - **`Cargo.lock`** is current, and is in the image's build context;
  - **README's first deploy** followed literally;
  - **The derived builder's** schedule and IAM;
  - **The alarm topic's** policy;
  - **§5 and §6's** variable names and step numbers.
