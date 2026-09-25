# AUDIT_FINDINGS_18 — the twenty-second audit (2026-09-25, twelfth pass)

Branch `audit/birdtest-2026-09-24-pass12`, off `audit/birdtest-2026-09-24-pass11`
(`d67d846`). MAGPIE changes are on `birdtest-contribute` at `0a69b625`, on top of
`c2eadc65`; `docker/Dockerfile` pins it. **Still unpushed** (AUDIT_FINDINGS_8,
header).

**Builds on** AUDIT_FINDINGS_7 to _17. No finding is critical. Two are
corrections of the twenty-first audit's own work, and both matter:
- Its worker credential gate could lock working machines out (1.1).
- Its zone default could plan to replace the subnets during an AZ event (1.2).

**Count: 14 code wins (10 birdtest, 4 MAGPIE), 10 plan wins, 19 unresolved
pending feedback** (all carried).

---

## 1. Findings

### 1.1 The credential gate refused working machines — **code updated** (the twenty-first audit's gate, redesigned)

The twenty-first audit's `MissGate` refused every worker request from an
address once 30 credentials from it had matched nothing in a minute. Valid
keys, valid UUIDs and first claims were refused along with the rest.

Its refusal lasted about 2 s and was renewed by the next miss, and refused
requests were not charged. So about 10 bogus requests a second kept an address
refused ~95% of the time.

The effect on MAGPIE's side:
- Heartbeats are sent once, so they lapsed.
- Submissions retry a 429 about 20 times and were then thrown away.

Three ways to get there:
- An attacker behind the same NAT.
- A fleet still running a key that was deactivated or revoked.
- An account an admin deleted.

It also left two holes:
- A valid identity's lookups were still unmetered, because its bucket was
  charged after the query.
- A burst of misses sent at once all reached the database before any was
  counted.

**Fix: `ratelimit::CredentialGate`.**
- Every presented key or UUID is charged its own bucket (1 a second, burst 5)
  before the lookup. The credential already names the bucket.
- A credential that has not resolved in the last ten minutes also pays a cell
  of its address's bucket (5 a second, burst 100) before its lookup, match or
  not.
- A credential that resolved recently skips the address's bucket, so a bad
  neighbour does not lock out working machines.
- The burst admits a hundred machines behind one address at once after a
  restart.

Tests:
- The router test (`A-WORKER-16`) floods an address with made-up keys until the
  429s start, and checks that a real worker at the same address is still
  served. The old gate fails it by design.
- `authz`'s dead-key test now gives each round a fresh app. A dead key is
  charged too, so a revoked key gets 429s after five requests in a burst
  instead of 401s. MAGPIE retries a 429, so a stale machine sees its 401 later.
  This is recorded as a trade-off.

### 1.2 The zone default could replace the subnets during an AZ event — **code updated** (the twenty-first audit's default, corrected)

The data source was filtered on `state = "available"`. A zone reported
`impaired`, `unavailable` or `constrained` would change the chosen pair. A
subnet's zone forces replacement, so the next plan would destroy the four
subnets under the ALB, the tasks and RDS. That apply fails on in-use network
interfaces, and every later apply stays blocked. This happens exactly when
RUNBOOK §1's import and closing apply run.

A mocked `terraform test` confirmed that a changed zone list replaces all four
subnets. The reviewers also confirmed that an existing us-east-1 stack left
unset resolves to a/b with no diff.

**Fix:**
- No state filter, only the opt-in one.
- An `azs` output.
- README step 4 pins it into `prod.tfvars` after the first apply, and RUNBOOK
  §5 passes it on the DR copy's later applies.

### 1.3 `dr_region` equal to `region` failed the first apply half-way — **code updated** (deployment blocker)

`dr_region` defaults to us-west-2 and nothing checked it. For a stack in
us-west-2, which README's first-deploy steps allow by setting `region` alone,
both providers made `alias/birdtest-backups` in the same region. KMS aliases
are unique per region, so the apply failed after most of the stack existed,
including the globally named `*-dr` buckets. Had the aliases differed, backups
would have been replicated into their own region, which protects against
nothing.

**Fix:**
- A validation that `dr_region != region`. A cross-variable validation needs
  Terraform 1.9, which is what `required_version` now says (CI and README
  already use 1.9).
- README step 4 names `dr_region`.

### 1.4 MAGPIE: a distribution past 50 letters overran every per-letter array — **MAGPIE updated** (memory safety)

`ld_create_internal` loaded any number of rows. Every array sized
`MAX_ALPHABET_SIZE` (50) was written past:
- a `Rack`'s counts;
- the machine-letter tables;
- the blanked forms at `ml | 0x80`.

The reviewer showed UBSan and ASan errors from `createdata klv` on a 55-row
file. birdtest's server runs `createdata klv` on a job's pinned distribution,
and every worker holding the file loads it too. Nothing on the server limited
the row count. It takes an admin or tarball input, so admin-trusted, but it
fails deep in a builder subprocess.

**Fix:**
- MAGPIE refuses fewer than 1 or more than 50 rows before reading any, with a
  test.
- birdtest's `LetterDistribution::parse` refuses more than 50 (`U-RACK-9`).
- Job creation parses the pinned distribution and answers a 400 on
  `letterdist_id`. Before, the first claim found the problem, as a 500
  (`A-ADMIN-3`).

### 1.5 The player-config form wrote 0 into cleared fields — **code updated**

Svelte 4 binds a cleared `type="number"` box as `null`, not `''`, so the form's
`x === '' ? null : Number(x)` sent `Number(null)`, which is 0. Configs cannot be
edited, so the mistake was permanent. Clearing any of these after typing a
value saved 0 instead of MAGPIE's default:
- movegen margin;
- inference margin;
- either utility weight;
- the spread scale;
- minimum play iterations.

A `movegen_margin` of 0 also makes the config impossible to pair with any
config on the default margin.

The job form and the job page have the same kind of problem. A cleared
required number was sent as `null`, and the answer named no field: "did not
match any variant of untagged enum".

**Fix:**
- `optionalNumber` for optional fields.
- `blankFields` names blank required fields before the request is sent.
- The allocation box is checked the same way.

Covered by `F-FMT-6`.

### 1.6 Other code fixes

| Item | Change |
|---|---|
| The ETA divided the last hour's completions by 3,600 s even for a job activated minutes ago: 6× too long at ten minutes, 30× at two | Measured since `greatest(now − 1 h, activated_at)`, at least a minute (`I-STATS-8c`) |
| The import page's resume forgot a staged import on a 5xx (the re-download the twenty-first audit meant to prevent). A failed first read left nothing polling. A resumed cancelled import showed nothing. After a confirm whose follow-up read failed, the button stayed live and a second click was a 409 | Forgets only on a final 4xx; watches before the first read; shows the cancellation's reason; marks the import confirmed once the confirm succeeds |
| Opening a confirmation link again (a double click, a mail scanner) said "invalid or has expired" and offered to register again, which then said the name was taken | Answered "email already confirmed" when the spent code's account is confirmed; the code is a secret, so this tells no one else anything (`A-AUTH-6`) |
| MAGPIE: a server-sent opening rack shorter than a full rack was drawn and analysed (an empty one as pass-only) | Must be exactly `RACK_SIZE` letters. Not unit-tested: the executor is static; the refusal it reports is the existing "unusable rack" |
| MAGPIE: `rackequity2klv` refused an unparsable row only as "must name full racks" once the rack was emptied | Names the row and says a letter is not in the distribution (keeps the phrase birdtest's tier-6 case matches) |
| MAGPIE: the `commit` pass-out rack error printed a literal `%s` | Formatted |

### 1.7 Plan wins

1. **README's first deploy:**
   - Step 3 used `$REGION` before anything set it. It now sets it and shows the
     certificate request and validation record commands.
   - The first apply runs with `scheduled_tasks_enabled=false`, so the builder
     and a 03:00 backup do not fail against the placeholder parameters.
   - `dr_region` is named, and the zones are pinned.
2. **RUNBOOK §2.1** has two blocks:
   - A fetch-and-verify block that starts from an empty `/tmp/dump`, so it can
     be run again, and says not to restore on a mismatch.
   - A restore block, with how to start clean after a partial attempt.
   Before, a mismatch printed a line and the paste went on into `initdb` and a
   detached restore, and a re-run failed in three places.
3. **RUNBOOK §5 step 3:**
   - It used `${MANIFEST…}` in the ops shell, where step 1's variables do not
     exist. It now uses §2.1's first block with the replica's bucket and
     region.
   - If the newest dump never completes (its region is gone), it falls back to
     the previous stamp.
4. **RUNBOOK §2.2's space check** measured whole tables in the scratch copy,
   every job's rows. The script now has `COPYBACK_DUMP_ONLY=1`: it dumps the
   job's rows, prints their sizes and stops before loading.
5. **RUNBOOK §2.2's URL rewrite:**
   - An `@` in a query string was taken for the end of the password. The
     userinfo is now bounded by the first `/`, `?` or `#` after the scheme.
   - A pasted `:5432` is stripped.
   Tested with an encoded password, a query with `@`, no port, and no
   credentials.
6. **RUNBOOK §5 and PLAN:** the DR copy's zones are pinned after its first
   apply.
7. **PLAN, rate limits:**
   - The table covers the credential gate.
   - It adds the registration notice's own limit (5 an hour per address,
     previously missing).
   - The closed Known Limit describes the redesign.
8. **PLAN, letter distributions:** job creation refuses a distribution MAGPIE
   cannot hold.
9. **`dr_region`'s description:** it must differ from `region`, and why.
10. **TESTING.md:** `A-WORKER-16` rewritten, `A-AUTH-6`, `U-RACK-9`,
    `I-STATS-8c`, `F-FMT-6`, `A-ADMIN-3`, and the counts.

## 2. Objective 3

The MAGPIE reviewer re-traced it at `c2eadc65`, and nothing changed:
- games, opening racks and leave generation are unchanged;
- the decline and shutdown reasons match on both sides.

Every caller of the stricter `rack_set_to_string` handles a refusal. No caller
passes designated letters on purpose:
- the `rack`, infer and sim commands use the unblanking form;
- GCG and CGP racks are upper case.

A KLV round trip is byte-identical for english, french, german, catalan
(bracketed letters included), polish and dutch. This pass's MAGPIE changes are
to distribution loading and the executor's rack check.

## 3. Objectives 4–6

No large performance problem:
- the public job list with the `stalled` flag took 32 ms over 1M tasks and 1M
  claims;
- the submit path's inline work is the finish check alone.

Most severe first:
1. The credential gate's collateral refusals (1.1). An availability problem for
   whole NATs, which is also a throughput one.
2. The ETA's first hour (1.6). Display only.

No storage change.

## 4. Unresolved, pending human feedback

The nineteen carried from AUDIT_FINDINGS_17 §4, unchanged.

Weighed and left:
- A dead key is charged its own bucket, so a revoked fleet sees 429s before its
  401 (1.1).
- After a restart, more than a hundred machines behind one address are
  admitted five a second (1.1).
- Thirty-two live pages per address: a tournament hall behind one address loses
  live updates on its thirty-third tab.
- A region with fewer than two opt-in-free zones fails `slice` with an index
  error rather than a message. `azs = ["a","a"]` passes validation.
- The redeem limiter runs after the body is parsed; a malformed body costs no
  database work.
- `csv2klv` accepts a CSV that leaves leaves out (they stay zero); birdtest
  does not use it.

## 5. `birdtest-contribute` (this pass, `0a69b625`)

| Change | Why |
|---|---|
| Distributions of 1–50 rows only, refused before any row is read; `test_a_distribution_past_the_alphabet_limit_is_refused` | 1.4 |
| Opening racks must be full racks | 1.6 |
| `rackequity2klv`'s message for an unparsable rack; `commit`'s `%s` | 1.6 |

All 70 suites in MAGPIE's default test table pass on the sanitizer build, the
new `ld` case included. `find_circ_deps.py` and `format.py` pass on a clean
archive. Tier 6 (14 Rust tests, `M-1`…`M-11`) passes on the release build, and
the 14 Rust tests on the sanitizer build.

## 6. Other

- **Python worker as a production client:** none.
- **Tier 5 not run:** it needs image builds.
- **Verified:**
  - All three dump-digest implementations agree (a real `pg_dump -Fd`, recopied
    with new mtimes, in `postgres:16`).
  - The ops image has every tool the RUNBOOK blocks use.
  - The `-var azs=null` and `scheduled_tasks_enabled` overrides work.
  - The JMESPath backticks inside single quotes survive the shell.
  - `aws acm wait certificate-validated` exists.
  - Every `terraform output` the docs name exists.
  - The per-address stream table's locking and drop order.
  - `db_code` and `db_constraint` consumers.
  - The frontend backoff against every way a stream ends.
  - CSRF, SameSite, the `next` guard and `session_generation`.
  - zxcvbn's cost on a 16 KiB password.
  - Every `api.ts` type against its serde struct.
