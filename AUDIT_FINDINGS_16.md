# AUDIT_FINDINGS_16 — the twentieth audit (2026-09-25, tenth pass)

Branch `audit/birdtest-2026-09-24-pass10`, off `audit/birdtest-2026-09-24-pass9`
(`1874900`). MAGPIE changes are on `birdtest-contribute` at `1dc9151f`, on top
of `d91dc083`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _15. No finding is critical. The most serious
is a documentation one: RUNBOOK §5's DR stack could not start its tasks (1.1).
One is a memory-safety bug in MAGPIE that only a sanitizer build shows (1.2).

**Count: 10 code wins (8 birdtest, 2 MAGPIE), 13 plan wins, 18 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 RUNBOOK §5's DR stack could not start a task — **plan updated** (high)

The eighteenth audit had §5 pass `prod.tfvars`, and with it
`github_token_parameter_arn`. SSM parameter ARNs are regional, so the DR task
definitions named a secret in the lost region, and ECS cannot start a task
whose secret it cannot read. **Fix:** the apply overrides it as empty, with a
note that every ARN `prod.tfvars` names in the lost region needs the same
treatment. `GITHUB_TOKEN` is optional; if needed, it can be recreated in
`$DR_REGION`.

Two more §5 problems:
- **DR storage was sized from the dump.** A restore of a `pg_dump -Fd
  --compress=6` dump came to 4.1 times the dump (188 MB → 779 MB), so "three
  times the dump" left `pg_restore` to fill the disk part-way. Autoscaling
  cannot keep up with a bulk load. The allocation is now computed from the
  replicated manifest's `database_bytes` (`pg_database_size` at dump time),
  plus 30% and room for WAL. The placeholder also spanned two lines without a
  continuation.
- **Step 7 ran the alert checks with the old `REGION`.** In the lost region
  they test nothing that exists. It now sets `REGION=$DR_REGION`.

### 1.2 MAGPIE: a lower-case rack wrote outside the rack — **MAGPIE updated** (memory safety)

`rack_set_to_string` reads a lower-case letter as a designated blank. Its
machine letter (≥ 128) indexes `Rack.array[MAX_ALPHABET_SIZE]`, so the letter
was counted outside the rack, on the stack. The KLV lookup then read past the
array too. Two callers take racks that must be undrawn tiles:
- `convert rackequity2klv`, on the server's rack equity CSV.
- The forced racks a leave-generation task sends the worker
  (`rack_list_restrict_to_forced_racks`).

birdtest's `magpie_leave.rs` has a case for this exact input (a misnamed rack
is refused). It passed on the release build and is a `stack-buffer-overflow`
under AddressSanitizer. It was found because this pass ran tier 6 against the
sanitizer build. The server writes upper case, so a healthy deployment never
sends such a rack; this is about malformed input, not normal use.

**Fix:** `rack_set_to_string_undesignated` answers `-1`, with the rack left
empty, for any designated letter. Both callers use it and refuse the row or
rack with their existing messages. `test_an_undrawn_rack_refuses_designated_blanks`
covers it in MAGPIE. All tier-6 tests pass on the sanitizer build. TESTING.md
now says to run tier 6 against both builds when MAGPIE's side changes.

### 1.3 The public job stream had no bound — **code updated** (security)

`GET /api/jobs/:id/stream` is unauthenticated. Each open stream holds a
connection, a task and a broadcast receiver. Every push was also a `String`,
cloned in full per subscriber. One host could open enough streams to exhaust
the task's memory. **Fix:**
- At most 2,000 live streams across all jobs. Past that, a `503` with
  `Retry-After`; the page already retries a failed stream every few seconds.
- Payloads are `Arc<str>`, shared rather than copied.

Covered by `A-PUBLIC-6b`.

### 1.4 One account could have many login buckets — **code updated** (security)

The per-account login bucket was keyed on Rust's `to_lowercase` of the typed
name, but the lookup matches with Postgres's `lower`, and the two disagree:
- Postgres (`en_US.utf8`, verified on the test database) lowers `TİM` to `tim`.
- Rust lowers it to `ti̇m`.

Each spelling that signs in to the same account got a fresh bucket of 100
guesses a minute. **Fix:**
- A known account's bucket is keyed by its id, after the lookup and still
  before any Argon2 verify.
- An unknown name keeps a bucket of its own, so a 429 says nothing about which
  names exist.

`A-AUTH-11b` fails without the fix.

### 1.5 The submit path's purge witness was read too late — **code updated** (the nineteenth audit's witness, corrected)

`submit_result` checked for a purge's claims hold and then, in
`after_submission`, read the per-job purge count that the finish check
compares under the row lock. A purge that took and released its hold between
the two reads was invisible to both. The count is now read before the hold
check and passed down. The `run_to_completion` comment also now says that the
hold lasts at least as long as the purge's locks.

### 1.6 Other code fixes

| Item | Change |
|---|---|
| Rating-pool writes naming a config that does not exist answered `409` "that is still referenced by other records" (the generic foreign-key mapping), which says the opposite | Unknown anchor or member: `400` on the field. Unknown pool: `404` (`A-RATE-4b`) |
| The import form sent `git_ref: ""` when the field was cleared, and showed the server's message without the field reasons (as the job forms did before the nineteenth audit) | Trimmed; empty means the server's default. One `errorText` helper for all three forms (`F-API-6`) |
| birdtest CI's check that the server's conversions exist grepped the help text. It passed as long as a name appeared anywhere, and never covered `createdata klv`, which every leave job's generation 0 uses | Each command is run the way the server runs it, on MAGPIE's two-letter test data, and must exit 0, report no error and write its file. A rack missing from the CSV makes it fail (checked) |
| `terraform fmt -check` has failed in CI since the sixteenth audit (a comment inside a block in `backup.tf`) | Moved. `fmt` and `validate` pass (the `hashicorp/terraform:1.9.8` image) |
| MAGPIE: the KLV builders had a version and no hash pin. Both go through `klv_create_empty` and so through the KWG maker, which is still changing on main | `builderhash` pins `createdata klv` and `rackequity2klv` too, run through the same commands the server runs |

### 1.7 Plan wins

1. **RUNBOOK §5** (1.1): the token ARN, storage from the manifest, and
   `REGION` in step 7.
2. **RUNBOOK §2.3** read the claims twice, once for `claims_issued` and once
   for `last_completed_at` (1.08 s → 2.05 s at a million claims). It now reads
   them once; tested on 2 million claims in a rolled-back transaction.
3. **RUNBOOK §2.2**'s PITR branch had the operator type the master password
   into a URL and a single-quoted file. A password with `'` or URL-reserved
   characters broke both. The URL is now derived from `$DATABASE_URL` (a PITR
   restore keeps the password) and written with `%q`. Tested with `a'b@c`.
4. **RUNBOOK §2.3b** shows the quoted heredoc that writes the file (unquoted,
   the shell expands each `$$`).
5. **RUNBOOK §5** said Terraform "does not grow" storage afterwards. Raising
   the variable does grow it; the comment now says why sizing at creation
   still matters.
6. **README "Deploying"** starts with the first deployment in order: tools,
   images (MAGPIE pushed first), `init`, an apply at zero tasks, then the
   password and parameters, then SES and DNS.
7. **README's alert checks:**
   - They named `birdtest-backup` literally, which is wrong for a suffixed
     stack. They now read the `backup_task_definition` output and take
     `SUFFIX`.
   - They kept the command in `$TF`, which zsh (macOS's default shell) does not
     word-split. It is now a `tf()` function.
8. **`db_apply_immediately`** has a description: what it applies at once (the
   instance class restarts the instance), and to check `PendingModifiedValues`
   on an existing stack first.
9. **The Bradley–Terry standard error** was documented, in code and in PLAN,
   as a lower bound on the uncertainty. It is not: counting a pair (two games,
   scored in quarters) as one trial widens it by at least √2, while ignoring
   the off-diagonal terms narrows it. Both now say so.
10. **The `stalled` flag** was still described as reading
    `task_claims_completed_idx` (the migration's comment and PLAN). It now
    reads `jobs.last_completed_at`; the ETA is the index's only reader.
11. **The submit path's comment** said a leave job's submissions skip the
    job-row lock. That holds only above redundancy 1; at redundancy 1, leave
    generation included, every submission takes it.
12. **PLAN**: the login bucket (1.4), the stream cap (1.3), and the
    foreign-key exceptions in the error mapping.
13. **TESTING.md**:
    - The per-tier counts are regenerated from `cargo nextest list` and
      vitest. They had drifted by up to 17: 161 unit, 143 integration,
      165 API, 14 contract and 14 tier-6 tests make 497 backend tests; 98
      frontend.
    - The new IDs (`A-PUBLIC-6b`, `A-AUTH-11b`, `A-RATE-4b`, `F-API-6`).
    - The KLV pins and the conversion runs in CI.
    - Tier 6 on the sanitizer build.

## 2. Objective 3

Re-traced by the MAGPIE reviewer at `d91dc083`: every outcome-affecting setting
is pinned per task and reset before it, and nothing is new. This pass's MAGPIE
changes touch rack parsing for undrawn racks and a test, not task settings.

## 3. Objectives 4–6

Most severe first:
1. The unbounded stream route (1.3). Its memory scaled with open connections
   times payload size.
2. §2.3's second pass over the claims (1.7.2).
3. The DR allocation (1.1).

Measured and unchanged:
- **The submit path's conditional `UPDATE jobs`:** 0.6 ms with no wait when it
  matches nothing.
- **HOT updates:** 4,022 of 4,027 were HOT.
- **The `stalled` flag:** 148 ms over 1 million gap rows. Its remaining cost is
  the older "no open claims" check, bounded by the fleet's claims in flight.
- **§2.3b:** at 200,000 users it completes, survives a SIGINT and a re-run, and
  §4 check 3b reads 0.

No storage change this pass.

## 4. Unresolved, pending human feedback

The eighteen carried from AUDIT_FINDINGS_15 §4, unchanged.

Weighed and left:
- **Submit round trip:** the `last_completed_at` update is one extra round trip
  per submission on a job whose counters did not change.
- **§2.3b's zeroing loop** is quadratic when very many identities need zeroing
  (66 s at 150,000). After §2.0 it is normally a handful.
- **A job can appear twice in the worker's `unsupported_jobs`.** It is
  harmless: the server filters with `= ANY`.
- **Shutdown advice for a builder mismatch.** When every gap is a wmp/rit
  derived mismatch, the worker's shutdown still advises `download_data.sh`.
  This is the carried "builder-mismatch shutdown advice" item.

## 5. `birdtest-contribute` (this pass, `1dc9151f`)

| Change | Why |
|---|---|
| `builderhash` pins both KLV builders (`89ae0299`) | 1.6 |
| `rack_set_to_string_undesignated`, used by `rackequity2klv` and forced racks (`1dc9151f`) | 1.2 |

MAGPIE tests `rack`, `contribute`, `config`, `autoplay`, `rit`, `wmp`,
`builderhash` and `klv` pass. `find_circ_deps.py` and `format.py`
pass on a clean archive. Tier 6 passes on both the sanitizer and the release
binary.

**For the merge notes:** `compat/ctime.h` no longer includes `io_util.h` on
this branch (the nineteenth audit), but upstream main still does. An upstream
file that gets `io_util` declarations through `ctime.h` would fail to compile
after a merge. CI would catch it.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - The 16 KiB auth body limit is well above any real body, and a `413` reaches
    the pages as JSON.
  - The rate-limit key digests cannot collide with short keys.
  - The 404/405 fallbacks reach nested routes.
  - The fixtures in `contract-fixtures/` are byte-identical to MAGPIE's.
  - The MAGPIE sources compile with `-Werror` under gcc-10, clang-10 and
    clang-18 (`gnu2x`), at `BOARD_DIM` 15 and 21, and in the wasm branch.
  - The README alert-check arguments come out as intended in bash (checked
    with stubs).
  - `-var` after `-var-file` wins.
