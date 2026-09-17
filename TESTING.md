# birdtest — Testing

[PLAN.md](PLAN.md) says what birdtest is and why it is built that way.
[README.md](README.md) says how to run it. This document says **what we
guarantee and how we check it**: seven tiers, what each one is allowed to touch,
the shared machinery underneath them, and an enumerated list of every test worth
writing.

The lists below are meant to be **worked through**, not read for flavour. There
are 250 entries. Each has an id (`U-RACK-3`, `I-SCHED-13`, …) so progress can be
tracked, and is phrased as a claim a test either proves or fails to prove. An
entry says what to set up and what to assert; it does not say how to write Rust.

Two pieces of infrastructure have to exist before most of them can be written.
The [tier-2 harness](#2-integration) (155 entries depend on it) now does:
`backend/tests/common/mod.rs` clones a template database per test and provides
the SQL builders described below. The [file mail
backend](#reading-confirmation-codes) (2 depend on it) does not yet.

The organising idea is that **the development environment and the automated
tiers above it are one code path**. A tier brings up the stack, seeds it, runs
workers, asserts, and tears down; the dev environment does all of that except
the last two steps. Sharing the bring-up means the dev environment cannot rot —
every tier-5 and tier-6 run exercises it.

The one thing the dev environment does *not* share is the worker. **A real
`magpie contribute` is the only worker outside tier 5.** `fake_worker.py` is a
tier-5 instrument — it makes browser journeys deterministic without a C
toolchain — and tier 6 and `dev.py` both refuse it, so nothing you develop
against or ship on is validated by synthetic results.

---

## The tiers

| # | Tier | Touches | Workers | Cadence |
|---|---|---|---|---|
| 1 | **Unit** | Nothing. Pure logic. | — | Every run |
| 1F | **Frontend unit** | Nothing. Pure TS. | — | Every run |
| 2 | **Integration** | Real Postgres. No HTTP. | — | Every run |
| 3 | **API** | Real router in-process + Postgres. No browser. | — | Every run |
| 4 | **Contract** | Committed fixtures. Nothing live. | — | Every run |
| 5 | **End-to-end** | Full stack in Docker + a real browser. | `fake_worker.py` | Every run |
| 6 | **MAGPIE smoke** | Full stack + a real MAGPIE build. | `magpie contribute` | Nightly, opt-in |

Tiers 1–4 need nothing but a Postgres (1 and 1F need nothing at all). Tier 5
needs Docker and Playwright. Tier 6 additionally needs a MAGPIE checkout with
real MAGPIE-DATA, and is the only tier that is not run by default.

**Pick the lowest tier that can prove the claim.** A rule about arithmetic goes
in tier 1 even if it is reachable over HTTP; a rule about SQL goes in tier 2
even if a browser could see it. A failure at tier 1 names a function, a failure
at tier 5 names a symptom.

### Status today

| Tier | Tests | Where |
|---|---|---|
| 1 Unit | 88 | `#[cfg(test)]` in `inputdata`, `backups`, `derived`, `magpie`, `jobs::racks`, `jobs::plausibility`, `version`, `compat`, `config`, `clientip`, `sse`, `routes::admin`, `stats::sprt`, `stats::bradley_terry` |
| 1F Frontend unit | **0** | — (no runner yet) |
| 2 Integration | 2 | `backend/tests/leave_gen.rs` — the claim decisions that never reach HTTP |
| 3 API | 26 | `backend/tests/worker_api.rs` (12), `admin_api.rs` (4), `leave_gen.rs` (7), `auth_api.rs` (3) — each names the bug or decision it pins |
| 4 Contract | 6 | `routes::worker::contract_fixtures`; MAGPIE checks its half in `test/contribute_test.c` |
| 5 End-to-end | **0** | — |
| 6 MAGPIE smoke | — | Retired: the server now *runs* MAGPIE for every KLV, so there is no second implementation to round-trip against. MAGPIE's own `builderhash` test is what holds the derived-file builders still; see below |

Tier 2 is the largest gap and the highest value. Every SQL string in the
codebase is currently unverified: `sqlx::query` is checked at runtime, so the
compiler sees opaque text. Two bugs of exactly this shape have already shipped
and been found by hand — a three-column `INSERT` into a table with a fourth
`NOT NULL` column, and a `SELECT` of `win_pct_model` after that column became
`winpct_id`, which broke *every* two-config games job. Neither was caught by
`cargo check` or by 63 passing tests. One tier-2 test per SQL-bearing function
retires that whole class.

---

## Coverage map

Every source file, the tier that owns its behaviour, and where it stands. "Owns"
means the tier a regression should be caught by; most files are also touched
incidentally by higher tiers.

### Backend

| File | Owning tier | Status |
|---|---|---|
| `stats/sprt.rs` | 1 | Covered |
| `stats/bradley_terry.rs` | 1 | Covered |
| `version.rs` | 1 | Covered |
| `compat.rs` | 1 | Covered |
| `jobs/racks.rs` | 1 | Partial — see `U-RACK-*` |
| `derived.rs` | 1 + 2 | Partial — naming and the dispatch gate at tier 1; the build itself needs MAGPIE |
| `magpie.rs` | 1 | Partial — the `builders` JSON contract and error bounding; the subprocess needs MAGPIE |
| `jobs/plausibility.rs` | 1 | Covered |
| `inputdata.rs` (archive walk) | 1 | Covered |
| `inputdata.rs` (download, staging, confirm) | 2 | **Gap** |
| `backups.rs` | 1 | Covered |
| `error.rs` | 1 | **Gap** |
| `auth/api_key.rs`, `auth/session.rs`, `auth/csrf.rs` | 1 | **Gap** |
| `config.rs` | 1 | **Gap** |
| `jobs/handler.rs` (wire types) | 1 + 4 | Partial |
| `models/job.rs` | 1 | **Gap** |
| `jobs/mod.rs` (`expected_data`, inserts) | 2 | **Gap** |
| `jobs/game.rs`, `game_pair.rs`, `opening_rack.rs`, `leave_gen.rs` | 1 (validation) + 2 (SQL) | Partial |
| `jobs/registry.rs` | 2 | **Gap** |
| `scheduler.rs` | 2 | **Gap** |
| `ratings.rs` | 2 | **Gap** |
| `jobstats.rs` | 2 | **Gap** |
| `audit.rs` | 2 | **Gap** |
| `artifacts.rs` | 2 | **Gap** |
| `sse.rs` | 3 | **Gap** |
| `ratelimit.rs` | 3 | **Gap** |
| `auth/mod.rs` (extractors) | 3 | **Gap** |
| `routes/auth.rs` | 3 | **Gap** |
| `routes/account.rs` | 3 | **Gap** |
| `routes/worker.rs` | 3 + 4 | Partial |
| `routes/admin.rs` | 3 | **Gap** |
| `routes/public.rs` | 3 | **Gap** |
| `routes/ratings.rs` | 3 | **Gap** |
| `main.rs` (wiring, sweep task) | 3 | **Gap** |

### Frontend

| Area | Owning tier | Status |
|---|---|---|
| `lib/format.ts` | 1F | **Gap** |
| `lib/api.ts` (error mapping, CSRF header) | 1F | **Gap** |
| `lib/sse.ts` | 1F | **Gap** |
| `lib/auth.ts` | 1F | **Gap** |
| `lib/components/*.svelte` (chart maths) | 1F | **Gap** |
| Every page under `routes/` | 5 | **Gap** |

### Scripts and cross-repo

| Area | Owning tier | Status |
|---|---|---|
| `worker/fake_worker.py` output shapes | 1 | Partial — `opening_rack` pinned |
| `scripts/seed.py` | 5 (used by it) | **Gap** |
| `scripts/dev.py` | Manual | Deliberate — see [Not tested](#what-is-deliberately-not-tested) |
| `scripts/backup.sh`, `restore-*.sh` | Nightly | Partial |
| birdtest ↔ MAGPIE wire | 4 + 6 | Partial |

---

## 1. Unit

Pure logic, no I/O. Rust idiom: `#[cfg(test)] mod tests` in the same file as the
code.

**May not**: open a socket, connect to a database, or read a file. Fixture bytes
come from `include_bytes!`; anything that needs a path uses a `tempdir`.

### Already covered

`stats::sprt` (8), `stats::bradley_terry` (12), `version` (3), `compat` (3),
`jobs::racks` (3), `jobs::plausibility` (12),
`inputdata` archive walk (10), `backups` (5), `derived` (2), `magpie` (3). Do not rewrite these; the entries
below are what is missing.

Two of them are **tables to maintain rather than tests to leave alone**.
`compat.rs`'s known-good/known-bad lexicon/distribution table is the artifact
worth keeping current — an uncovered combination must be rejected, not guessed —
and `jobs::plausibility`'s cases must keep asserting both directions: each rule
rejects its impossibility *and* ordinary results still pass. These run at
submission time with no false-positive budget, so a rule that clips real play is
worse than no rule.

### `U-RACK-*` — rack space (`jobs/racks.rs`)

The rack index is a permanent identifier: opening-rack results are stored
against it, so a change in ordering silently re-points existing rows.

- `U-RACK-1` `LetterDistribution::parse` accepts a real `english.csv` and a
  minimal two-letter one, and reports the right tile counts and machine letters.
- `U-RACK-2` `parse` rejects each malformed shape distinctly: a short row, a
  non-numeric count, a duplicate letter, an empty file. The error names the
  origin string it was given.
- `U-RACK-3` `enumerate_racks(k)` returns exactly `total()` racks for that size,
  each sorted, with no duplicates.
- `U-RACK-4` `enumerate_leaves(max)` covers sizes 1..=max and matches the sum of
  per-size counts.
- `U-RACK-5` `rack_at(i)` agrees with `enumerate_racks` at every index for a
  small distribution — the existing `unranking_matches_full_enumeration`, extend
  to a distribution with a tile count above 2 so multiset repetition is
  exercised.
- `U-RACK-6` `racks_in_range(start, count)` equals the slice of
  `enumerate_racks`, including a range that runs off the end.
- `U-RACK-7` `total()` for real English is exactly 3,199,724 for size 7 and
  914,624 for leaves up to 6 — the two numbers PLAN.md quotes and that job
  creation cost depends on.
- `U-RACK-8` A distribution with a blank tile enumerates blanks in the documented
  position, and a rack containing one round-trips through `rack_at`.

### `U-ERR-*` — error mapping (`error.rs`)

Every handler returns `AppResult`, so this type decides what a caller sees.

- `U-ERR-1` Each `AppError` constructor maps to its documented HTTP status:
  `bad_request` → 400, `unauthorized` → 401, `forbidden` → 403, `not_found` →
  404, `conflict` → 409, `rate_limited` → 429, `internal` → 500.
- `U-ERR-2` The serialized body always carries `code` and `message`, and
  `with_field` errors appear under `fields`.
- `U-ERR-3` `rate_limited(n)` sets `Retry-After: n`, and never below 1.
- `U-ERR-4` A `sqlx::Error` converted into `AppError` becomes a 500 whose public
  message does **not** contain the SQL string or the database URL. A leaked
  query in an error body is the failure this test exists for.

### `U-AUTH-*` — credentials (`auth/api_key.rs`, `auth/session.rs`, `auth/csrf.rs`)

- `U-AUTH-1` An API key hashes deterministically: the same raw key gives the
  same hash across calls, and two generated keys differ. (Determinism is
  load-bearing — lookup is an exact hash match, not a per-row verify.)
- `U-AUTH-2` A generated raw key is URL-safe and at least 32 bytes of entropy.
- `U-AUTH-3` A password verifies against its own Argon2 hash and fails against
  another's; two hashes of the same password differ (per-password salt).
- `U-AUTH-4` A session token issued by `session::issue` round-trips its subject,
  username and admin flag through `session::verify`, and fails verification
  after any byte is altered.
- `U-AUTH-5` A session signed with a different key fails verification.
- `U-AUTH-6` A session past `session_ttl` is rejected. Drive it by issuing with
  a near-zero TTL rather than by waiting.
- `U-AUTH-7` `csrf::verify` passes GET/HEAD/OPTIONS with no token at all;
  requires both cookie and header on POST/PATCH/DELETE; rejects a mismatch, a
  missing cookie, and a missing header with distinct messages.

### `U-CFG-*` — configuration (`config.rs`)

- `U-CFG-1` Every variable with a default takes it when unset, and the value
  when set. Table-driven; the point is that a rename cannot silently fall back.
- `U-CFG-2` A missing variable with no default is a startup error naming it, not
  a panic or an empty string.
- `U-CFG-3` `MIN_MAGPIE_VERSION` parses through `Version`, so a malformed value
  fails at startup rather than silently becoming `0.0.0` and admitting every
  client. (This is the setting that made every local task decline.)

### `U-WIRE-*` — wire types (`jobs/handler.rs`, `models/job.rs`)

- `U-WIRE-1` `seed` serializes as a **decimal string** and round-trips a value
  above 2^53 without loss. A JSON number would lose it silently.
- `U-WIRE-2` `TaskRequest` tags with `job_type` in snake_case, and each variant
  round-trips.
- `U-WIRE-3` Every `#[serde(default)]` field on a response type is genuinely
  optional: a payload omitting all of them deserializes.
- `U-WIRE-4` An unknown field in a response is ignored rather than rejected, so
  a newer client can add one. Assert the current behaviour deliberately, either
  way, because it is a compatibility decision.
- `U-WIRE-5` `GameAggregate::is_consistent` accepts a valid tally and rejects
  each way of breaking it (negatives, sum mismatch).
- `U-WIRE-6` `JobType` round-trips through its serde representation and its
  Postgres enum representation with the same strings.
- `U-WIRE-7` `SprtParams::from` reads the same alpha/beta/elo values out of a
  `GameConfig` and a `GamePairConfig`, so the two job types cannot diverge.

### `U-PLAUS-*` — plausibility gaps (`jobs/plausibility.rs`)

Twelve rules are covered. What is missing:

- `U-PLAUS-1` `check_against_task` doubles the dispatched count for
  `game_pairs` and does not for `games` — the pairs-versus-games unit confusion
  that the `min_pairs`/`max_pairs` naming exists to keep straight.
- `U-PLAUS-2` A batch reporting one game more, and one fewer, than dispatched is
  rejected; the exact count passes.

### `U-FAKE-*` — fake worker shapes (`worker/fake_worker.py`)

The fake worker is the only client below tier 6, so shape drift is invisible
until a job of that type is run — which is how its opening-rack submission came
to answer with a field the server had never accepted.

- `U-FAKE-1` A captured `games` submission deserializes into
  `GameResultsResponse` and passes `process_response`.
- `U-FAKE-2` A captured `game_pairs` submission does the same, **including** the
  pentanomial cross-checks.
- `U-FAKE-3` A captured `opening_rack` submission deserializes into
  `PositionAnalysisResponse`. *(Covered.)*
- `U-FAKE-4` A captured `leave_generation` submission deserializes into
  `LeaveResponse` and passes `check_rack_occurrences`.
- `U-FAKE-5` Each `--mode` (`malformed`, `stale`, `abandon`) produces something
  the server's validation **rejects**, so the adversarial modes cannot silently
  become valid.

Fixtures live in `backend/src/jobs/testdata/` and are regenerated by a documented
command, not hand-edited.

---

## 1F. Frontend unit

Pure TypeScript, no browser, no network. **New dependency**: Vitest, plus
`@testing-library/svelte` for the component maths. Run by `npm test` in
`frontend/`, and by CI alongside `npm run check`.

This tier exists because the charts contain real arithmetic — a scale, a bucket
index, a threshold — and getting those wrong produces a plausible-looking
picture rather than an error.

### `F-FMT-*` — `lib/format.ts`

- `F-FMT-1` `workerLabel` renders a username when present, "Anonymous" plus a
  short UUID prefix when not, and never leaks a full UUID.
- `F-FMT-2` `duration` renders seconds, minutes, hours and days at the right
  boundaries, and `null` as a dash rather than "null".
- `F-FMT-3` `datetime` renders `null` as a dash and a valid ISO string as a
  local time; an unparseable string does not throw.
- `F-FMT-4` `jobTypeLabel` covers all four job types, and an unknown type falls
  back to the raw string rather than "undefined".
- `F-FMT-5` `sprtLabel` covers all four statuses.

### `F-API-*` — `lib/api.ts`

- `F-API-1` A non-GET request sends `x-csrf-token` read from the cookie; a GET
  does not.
- `F-API-2` A 204 resolves to `undefined` rather than throwing on an empty body.
- `F-API-3` A 4xx with a JSON error body rejects with an `ApiError` carrying
  `status`, `code`, `message` and `fields`.
- `F-API-4` A 4xx with an empty or non-JSON body still rejects with an
  `ApiError`, not a `SyntaxError`.
- `F-API-5` Every request sets `credentials: 'include'`.

### `F-SSE-*` — `lib/sse.ts`

- `F-SSE-1` `subscribeToJob` parses an event and calls back with the decoded
  payload.
- `F-SSE-2` A malformed event is skipped without tearing down the subscription.
- `F-SSE-3` The returned function closes the `EventSource`, and calling it twice
  is safe.

### `F-CHART-*` — chart maths

Test the pure functions; do not snapshot the SVG.

- `F-CHART-1` `RatingDotPlot` places the anchor's dot at the anchor rating, and
  a config one standard error away at the expected offset.
- `F-CHART-2` It clamps a runaway error bar rather than letting one
  barely-measured config flatten the scale, and still reports the true number in
  the table.
- `F-CHART-3` A config with `connected_to_anchor: false` is listed as unrated
  and **not** drawn at a position.
- `F-CHART-4` `RatingHistoryChart` caps at six series, picks them by latest
  rating, and reports how many it omitted.
- `F-CHART-5` It assigns colour by config identity, so filtering the list does
  not repaint the survivors.
- `F-CHART-6` `ResidualMatrix` sorts by absolute residual descending, and flags
  the non-transitive case only when at least three head-to-heads exceed the
  threshold.
- `F-CHART-7` The job page maps pentanomial buckets to the right labels — index
  0 is "P1 lost both", index 4 "won both". An off-by-one here inverts the
  reading of every paired job.
- `F-CHART-8` Percentages are computed against pairs, not games, and a zero
  denominator renders 0.0% rather than `NaN`.

### `F-AUTH-*` — `lib/auth.ts`

- `F-AUTH-1` `refreshSession` sets the store to the user on 200 and to `null` on
  401, distinguishing "signed out" from "not yet known" (`undefined`).
- `F-AUTH-2` `signOut` clears the store even if the request fails, so the UI
  cannot be left showing a session that is gone.

---

## 2. Integration

Real Postgres, migrations applied, **no HTTP layer**. This is where birdtest's
genuinely hard logic lives, because most of it is SQL and concurrency rather
than Rust.

**Build the harness first.** It is the dependency for tiers 2 and 3 — 155 of the
entries in this document cannot be written without it — and it is not yet
written. Three decisions, settled below, because each has an obvious-looking
answer that is wrong here.

### Isolation: `CREATE DATABASE … TEMPLATE`

Each test gets its own database, cloned from a single pre-migrated template:

```rust
let db = TestDb::new().await;   // CREATE DATABASE <unique> TEMPLATE birdtest_test_template
                                // ... test body ...
                                // Drop on Drop
```

Measured against the compose Postgres (35 tables, an 877-line migration):

| | Per test | Supports concurrency tests? |
|---|---|---|
| Transaction per test, rolled back | ~1 ms | **No** |
| `CREATE DATABASE` + run migrations | ~360 ms | Yes |
| **`CREATE DATABASE … TEMPLATE`** | **~125 ms** | **Yes** |

**Transaction-per-test is the usual advice and it is wrong for this suite.** A
rolled-back transaction is invisible to a second connection, so the tests that
matter most cannot be written in it at all: `I-SCHED-13` (concurrent claimers
racing the `(job_id, seed)` unique index), `I-LEAVE-2` (concurrent upserts
summing occurrences), and `I-SUBMIT-3` (counter updates under contention). And
`I-AUDIT-2` asserts that a log written inside a rolled-back transaction does not
persist, which is incoherent if the test is itself that transaction.

The template is built once, before any test runs, behind a `OnceCell`. Two
constraints that will otherwise cost an afternoon:

- **Nothing may hold a connection to the template.** Postgres refuses
  `CREATE DATABASE … TEMPLATE` while one is open, so the pool used to build it
  must be closed before any test clones it.
- **Template creation must be serialized.** `cargo test` runs in parallel
  threads; two racing to build it will collide.

Not `testcontainers`: the compose Postgres is already running for development,
and a container per test costs seconds where a clone costs 125 ms. Revisit if
isolation starts to bite.

Do **not** add a second, faster isolation mode for the read-only tests. Two
modes is a decision to re-make at every new test, in exchange for 100 ms.

### State: builders that are deliberately dumber than the application

```rust
let admin = db.user().admin().create().await;
let ld    = db.input_data().letterdist("english").create().await;
let cfg   = db.player_config().static_equity().create().await;
let job   = db.job().game_pairs(cfg_a, cfg_b).active(100).create().await;
```

Typed builders with defaults and per-field overrides, for about **ten
entities** — user, api_key, input_data, player_config, job plus its four config
rows, task, task_claim, game_result, rating_pool. Not all 35 tables; anything
else a test needs, it inserts inline. Raw helper functions would work but put
six required columns at every call site and make a schema change touch every
test.

**The rule that matters: a builder writes SQL directly and validates nothing.**
Roughly half the entries below need a state the application would refuse to
create, and a builder that enforces invariants makes exactly those unwritable:

- `I-SCHED-14` — a task already at `redundancy` active claims
- `I-SCHED-12` — a claim exactly one second past its timeout
- `I-JOB-7` — an allocation outside 0–100
- `I-SUBMIT-2` — a `game_results` row whose pentanomial contradicts its counts

This is the same boundary as [What tiers 2 and 3 must not
share](#what-tiers-2-and-3-must-not-share), one level down: the fixture must be
able to express what the system forbids, or the tests that prove the system
forbids it cannot exist.

Two approaches that look tempting and are not: building state by calling the
application's own functions is circular — it uses the thing under test to set up
the test — and building it over the HTTP API makes precise states unreachable,
which is the whole point of not sharing the seed.

### Time: an explicit column, not an injected clock

Many entries need "N seconds ago". Timeouts are computed from `TIMESTAMPTZ`
columns, so the builder takes an explicit `claimed_at` and the test passes
`now() - interval '10 minutes'`. Cheaper than a clock abstraction, and it
exercises the real SQL rather than a test double of it.

### Order to build in

Ninety tier-2 entries cannot land at once. Harness, then `I-SCHED-*` — the largest
group, the hardest logic, and the part that breaks silently. Then `I-JOB-*`,
where the `win_pct_model` bug lived. Then the rest by group, cheapest first.

### `I-SCHED-*` — scheduler (`scheduler.rs`)

The single most important group. Every entry is about a decision made in SQL.

- `I-SCHED-1` A claim against one active job returns a task, inserts a
  `task_claims` row, and increments `active_claim_count`.
- `I-SCHED-2` Deficit selection: two active jobs with allocations 75/25
  converge on that ratio over many claims.
- `I-SCHED-3` A job at 0% is offered to nobody, exactly as an inactive one is:
  every claim goes to the other active job, and with every active job at 0%
  the answer is `204`, not a shutdown — including when a parked job is too new
  for the worker or in its unsupported set, which shuts nobody down until the
  job is raised above 0% (`a_parked_job_shuts_nobody_down`). There is no
  priority.
- `I-SCHED-3a` **A job joins at parity.** A job activated beside one with a
  long claim history splits the next claims by allocation rather than taking
  all of them, a changed allocation holds from the moment it is set, and a
  purged job rejoins level (`claims_baseline`, `scheduler::join_at_parity`).
  *(Covered: `admin_api::a_newly_activated_job_joins_at_parity_instead_of_taking_everything`,
  `admin_api::a_purged_job_rejoins_at_parity`.)*
- `I-SCHED-4` `tasks_dispatched` counts abandoned claims. Abandon many claims on
  one job and confirm its share does **not** grow — excluding them would let a
  job with flaky workers accumulate more than its share.
- `I-SCHED-5` Ties break on `created_at ASC`.
- `I-SCHED-6` **Both capability filters are part of candidate selection.** A
  worker whose `unsupported_jobs` covers the job furthest behind its share is
  offered the next one in deficit order, not shut down.
- `I-SCHED-7` Version filtering: a worker on `1.9.0` is offered a job requiring
  `1.9.0` and not one requiring `1.10.0`. Include the `1.9.0` vs `1.10.0` pair
  specifically — lexical comparison passes every other case.
- `I-SCHED-8` `ClaimOutcome::Idle` when work exists but none is available, vs
  `NoWorkExists` when no job is active. These mean opposite things to a client.
- `I-SCHED-9` `Shutdown` with reason `magpie_too_old`, `data_out_of_date`, and
  `both` — and `both` leads on the version, because updating MAGPIE is the
  action that also fixes the data.
- `I-SCHED-10` A shutdown directive names the required tarball dates and
  version actually derived from the active jobs, not a hardcoded string.
- `I-SCHED-11` **Reclamation**: a claim past the heartbeat timeout flips to
  `abandoned`, decrements `active_claim_count`, and returns an at-capacity task
  to `available`.
- `I-SCHED-12` Reclamation is lazy — it happens on the next claim for that job,
  and a claim one second *inside* the timeout is not reclaimed.
- `I-SCHED-13` **Concurrent claimers**: N simultaneous claims against one job
  produce no duplicate seeds (the `(job_id, seed)` unique index resolves the
  race and the loser retries) and no lost counter updates.
- `I-SCHED-14` A task at `redundancy` active claims is not handed to an
  additional worker.
- `I-SCHED-15` **The `declined` partial-index trap.** `task_claims_user_unique_idx`
  and `task_claims_anon_unique_idx` are partial on
  `WHERE state NOT IN ('abandoned','declined')`. Drop `'declined'` from either
  and a worker that declines a task is permanently barred from claiming it again
  after fixing its data. Nothing else catches this.
- `I-SCHED-16` One worker cannot hold two simultaneous claims on the same task,
  by either identity type.
- `I-SCHED-17` A banned worker's claim is refused, by user id and by anon UUID.
- `I-SCHED-18` `release_claim` returns the task to `available` and decrements
  the counter.
- `I-SCHED-19` An inactive or completed job is never selected.

### `I-EXPECT-*` — capability negotiation (`jobs/mod.rs::expected_data`)

- `I-EXPECT-1` Two players on different lexicons yield two `kwg` and two `klv`
  entries.
- `I-EXPECT-2` The same config on both sides yields one of each, not two
  identical rows.
- `I-EXPECT-3` A static player contributes no `winpct` entry.
- `I-EXPECT-4` A `leave_generation` job yields exactly `kwg`, `letterdist` and
  `layout`, and never a `klv` — its leaves come from the server-built artifact.
- `I-EXPECT-5` Every entry carries the SHA-256 from `input_data`, not a name.
- `I-EXPECT-6` An `opening_rack` job yields its single player's files plus the
  job's distribution and layout.

### `I-JOB-*` — job creation and lifecycle (`routes/admin.rs` SQL, `jobs/registry.rs`)

These are the functions the `win_pct_model` bug lived in. Every SQL string that
job creation touches needs one caller here.

- `I-JOB-1` Creating each of the four job types inserts its config row with
  every column populated, and reads back identical.
- `I-JOB-2` **`validate_shared_player_options` runs against real rows.** Two
  configs with different `winpct_id` are rejected; two with the same are
  accepted; two with different `movegen_margin` are rejected. The regression
  guard for the renamed column.
- `I-JOB-3` `validate_player_compatibility` rejects an incompatible
  lexicon/distribution pair and accepts a compatible one, using real
  `input_data` rows.
- `I-JOB-4` A job referencing a nonexistent `input_data` id fails cleanly with a
  400-shaped error, not a foreign-key 500.
- `I-JOB-5` Creating a job writes no rows up front; a leave-generation job's
  first claim starts the seeding of its generation-1 universe.
- `I-JOB-6` Activation sets `allocation` and `activated_at`; deactivation clears
  the schedule without destroying tasks; completion is terminal.
- `I-JOB-7` An allocation outside 0–100 is rejected.
- `I-JOB-8` `purge_job` deletes tasks and claims, leaves the job row, and lets
  task generation resume cleanly from the right seed.
- `I-JOB-9` `delete_job` cascades to tasks, claims, results and configs, and
  leaves no orphans in any table.
- `I-JOB-10` Both write their census to `audit_log` **before** destroying, so
  the record survives the thing it describes.
- `I-JOB-11` A player config referenced by any job cannot be deleted.
- `I-JOB-12` `delete_user` leaves their contributions attributed but anonymised,
  per the design, and does not cascade away results.

### `I-SUBMIT-*` — result submission (`jobs/mod.rs`, `jobs/*.rs`)

- `I-SUBMIT-1` A `games` result inserts one `game_results` row with NULL
  pentanomial and NULL divergent columns.
- `I-SUBMIT-2` A `game_pairs` result inserts the pentanomial, and the database
  CHECK rejects a row whose buckets disagree with the counts. Assert **both**
  the pair-count and half-point constraints fire.
- `I-SUBMIT-3` Accepting a result increments `accepted_count`, decrements
  `active_claim_count`, and completes the task at `redundancy`.
- `I-SUBMIT-4` A submission against a stale claim token is ignored, not
  accepted, and does not move any counter.
- `I-SUBMIT-5` **Position capture deduplication**: under `redundancy = 2`,
  identical replayed games produce one set of `position_analysis_records`, not
  two.
- `I-SUBMIT-6` Captured positions are truncated to the player config's
  `num_plays_recorded`, and `num_moves` records the pre-truncation count.
- `I-SUBMIT-7` `position_analysis_moves` rank 1 is the best move, and the
  partial index on rank 1 is used by the dashboard aggregate.
- `I-SUBMIT-8` An opening-rack result stores one record per requested rack, and
  a result naming a rack the task did not dispatch is rejected.

### `I-LEAVE-*` — leave generation (`jobs/leave_gen.rs`)

- `I-LEAVE-1` `seed_generation` inserts one `leave_rack_progress` row per full
  7-tile rack for the pinned distribution, the count matches
  `enumerate_racks(7)`, and a claim's forced racks are full racks.
  *(Covered: `leave_gen::the_universe_and_the_forced_racks_are_full_racks`.)*
- `I-LEAVE-2` A submission is **staged** — one row, the generation's live
  counters bumped, no per-rack row touched — and a merge **sums** what is
  staged into `leave_rack_progress`, exactly once, with a rack outside the
  universe creating no row. Two submissions open at once, naming the same racks
  in opposite orders, neither deadlock nor lose an occurrence. *(Covered:
  `leave_gen::a_result_folds_into_the_generation_and_creates_no_rows`,
  `leave_gen::overlapping_leave_submissions_do_not_wait_on_each_other`.)*
- `I-LEAVE-2a` The racks a staged result's task forced are held out of
  selection until the merge, and a generation does not close with anything
  staged: the claim asks for a merge and the next one decides on exact figures.
  A purge discards what is staged. *(Covered:
  `leave_gen::racks_of_a_staged_result_are_not_handed_out_again_before_the_merge`,
  `leave_gen::a_generation_does_not_close_with_results_still_staged`,
  `leave_gen::a_purge_discards_staged_results`.)*
- `I-LEAVE-3` Rack selection picks the racks furthest below target, skips racks
  an open claim is already forcing, and returns nothing once all are at target
  with no claim in flight. *(Skipping covered:
  `leave_gen::racks_out_with_an_open_claim_are_not_handed_out_again`.)*
- `I-LEAVE-4` Generation transition folds progress into a KLV, uploads it,
  records the digest, and marks the generation complete.
- `I-LEAVE-5` `ON CONFLICT DO NOTHING` on the artifact row keeps the **first**
  digest, so a racing transition cannot rewrite history.
- `I-LEAVE-6` Generation 0's zeroed KLV exists at job creation and sums to
  exactly zero.
- `I-LEAVE-7` Every dispatched generation carries a non-null
  `previous_artifact_key`, including generation 1.
- `I-LEAVE-8` **`rebuild_artifacts` reproduces bytes.** `run_transition` and
  `rebuild_artifacts` share `generation_means` precisely so a rebuild cannot
  drift; fold, rebuild, compare digests.
- `I-LEAVE-9` A job with `generation_count > 1` advances to the next generation
  and finishes after the last.
- `I-LEAVE-10` The task's `num_games` is the only termination condition — the
  rack target is not sent to the worker.
- `I-LEAVE-11` **One transition per generation.** With every rack at target and
  no claim in flight, the first claim decision starts the transition and every
  later one is told there is no work yet, rather than starting a second fold of
  millions of rows. *(Covered:
  `leave_gen::only_one_claim_starts_a_generations_transition`.)*
- `I-LEAVE-12` **A transition that never finished is taken over**, once past the
  takeover timeout, and the takeover is recorded in `attempts`; a *completed*
  transition is never restarted however old it is. *(Covered:
  `leave_gen::a_transition_that_never_finished_is_taken_over`.)*
- `I-LEAVE-13` **The transition owner's row is committed** before the transition
  runs -- the claim transaction that decides a generation is complete commits
  rather than rolls back, or the row that stops a second transition would be
  discarded -- and a transition that *fails* hands ownership back immediately
  instead of waiting out the takeover timeout. *(Covered:
  `leave_gen::the_transition_owner_is_committed_before_the_transition_runs`.)*
- `I-LEAVE-14` **A result for a closed generation is credited but not folded**:
  the claim completes and the `leave_records` row is written, and the closed
  generation's `occurrence_count` does not move — so a rebuild of that
  generation still reproduces the artifact's digest. *(Covered:
  `leave_gen::a_result_for_a_closed_generation_is_credited_but_not_folded`.)*
- `I-LEAVE-15` **A reopened task is reissued only while its generation is
  current.** A task whose claim timed out is handed to the next worker (same
  racks, not a new task beside it) while its generation is open; once that
  generation has closed, the next claim gets a task for the new generation
  instead. *(Covered:
  `leave_gen::a_reclaimed_task_is_reissued_only_while_its_generation_is_open`.)*

### `I-RATE-*` — rating pools (`ratings.rs`)

`build_matrix` has been checked by hand against a live database; these make it
permanent.

- `I-RATE-1` A matching `game_pairs` job's results enter the matrix.
- `I-RATE-2` Excluded: wrong `variant`, wrong `letterdist_id`, wrong `layout_id`,
  a job whose player is not a pool member, and a plain `games` job. One test per
  exclusion, because each is a separate clause.
- `I-RATE-3` The pair is the unit: a head-to-head's `games` equals pairs, not
  games, and its score is the half-point total over four.
- `I-RATE-4` `recompute` writes one `rating_runs` row and one
  `player_config_ratings` row per member, with `is_anchor` set on exactly one.
- `I-RATE-5` The anchor's stored rating equals `anchor_rating` exactly.
- `I-RATE-6` A member with no path to the anchor is stored with
  `connected_to_anchor = false`.
- `I-RATE-7` Adding a member changes other members' ratings, and removing one
  changes them back — the property that makes a refit necessary.
- `I-RATE-8` Removing the anchor is refused.
- `I-RATE-9` `recompute_stale` refits a pool whose evidence grew and skips one
  whose `pairs_used` is unchanged.
- `I-RATE-10` Two pools with different scopes over the same jobs produce
  different, internally consistent fits.
- `I-RATE-11` A pool with one member (the anchor) and no games produces a run
  rather than an error.

### `I-STATS-*` — dashboard aggregates (`jobstats.rs`)

- `I-STATS-1` A `games` job's stats sum every result and compute SPRT over
  games.
- `I-STATS-2` A `game_pairs` job's stats sum the pentanomial, and
  `units_completed` equals the pair count — **not** the divergent count.
- `I-STATS-3` `divergent_pairs` is reported and is not what SPRT consumed.
- `I-STATS-4` A job with no results reports zeros and an LLR of 0, not an error
  or a NaN.
- `I-STATS-5` Opening-rack stats count analysed racks against `total_racks`, from
  the running `jobs.racks_analyzed` total, and count a task's racks **once** even
  when two redundant claims of it are accepted. *(Covered:
  `worker_api::analysed_racks_are_counted_once_per_task_as_they_arrive`.)*
- `I-STATS-5b` The job list's `units_completed` reads the same kind of running
  total and agrees with `game_stats` on a redundancy-2 job. *(Covered:
  `worker_api::redundant_results_for_one_task_count_once`.)*
- `I-STATS-5d` **Concurrent submissions for one task count once.** Two
  redundant claims of a task submitting at the same moment (each blocked,
  before commit, on a lock the other holds) still add the task's games to the
  running total once. *(Covered:
  `worker_api::concurrent_redundant_results_count_once`.)*
- `I-STATS-5c` A purge zeroes both running totals. *(Covered:
  `admin_api::a_job_can_be_purged_and_its_dispatch_counter_resets`.)*
- `I-STATS-6` Leave-generation stats report racks at target against the
  universe, and the current generation.
- `I-STATS-7` `worker_contributions` attributes tasks to the right identity and
  totals correctly across both identity types.
- `I-STATS-8` ETA is `None` without recent throughput rather than infinity.
- `I-STATS-9` `finish_if_done` completes a job at SPRT significance and at the
  hard cap, and does **not** complete below `min_units` even with a crossed LLR.

### `I-INPUT-*` — input data import (`inputdata.rs`)

The archive walk is unit-tested; the database half is not.

- `I-INPUT-1` A staged import inserts `input_data_import_rows` and no
  `input_data` rows until confirmed.
- `I-INPUT-2` Confirming inserts `input_data` rows, dedupes by `(path, sha256)`,
  and reports what was new versus already present.
- `I-INPUT-3` Only server-read roles (`letterdist`, `layout`) keep their bytes
  in the row; `kwg`/`klv`/`winpct` store a digest and NULL content. Enforced by
  the CHECK — assert the CHECK fires, not just that the code does it.
- `I-INPUT-8` `kwg` and `klv` rows carry an `object_key` and their bytes are in
  the object store, because the server builds wordmaps and rack info tables
  from them; `winpct` rows carry neither, because nothing server-side builds
  anything from a win% model. The key is the digest, so re-importing a tarball
  whose lexica have not changed uploads nothing.
- `I-INPUT-4` A second import of the same tarball is a no-op.
- `I-INPUT-5` `fail_orphaned_imports` fails a row left `running` by a restart
  and leaves `staged` and `confirmed` rows alone.
- `I-INPUT-6` Deleting an `input_data` row referenced by a player config or job
  is refused.
- `I-INPUT-7` A failed import records its error and stages nothing.

### `I-AUDIT-*` — audit log (`audit.rs`)

- `I-AUDIT-1` Each helper writes the actor, target type, target id and job id it
  was given.
- `I-AUDIT-2` A log written inside a rolled-back transaction does not persist —
  the audit trail cannot claim something that did not happen.
- `I-AUDIT-3` Every destructive admin action writes exactly one row.

### `I-ART-*` — object store (`artifacts.rs`)

- `I-ART-1` Put then get returns identical bytes, against MinIO.
- `I-ART-2` Getting an absent key is a clean error, not a panic.
- `I-ART-3` A key is namespaced per job and generation, so two jobs cannot
  collide.

### `I-DATA-*` — the pinned-row invariant

- `I-DATA-1` **Server-side reads use the pinned row.** Two `letterdist` rows
  with the same name and different bytes produce different `total_racks` for
  otherwise identical jobs. This is the one test that would catch the server and
  the worker disagreeing about the alphabet.
- `I-DATA-2` `seed_generation` and every MAGPIE conversion read the job's pinned
  distribution, not a filesystem path or a default. For the conversions this is
  structural — each runs in a throwaway directory written from
  `input_data.content` and the object store, and the distribution is stated on
  the command line rather than inferred from the lexicon's name — but a test
  that two distributions with one name produce two different derived hashes is
  what proves it.

### `I-DERIVED-*` — wordmaps and rack info tables (`derived.rs`)

- `I-DERIVED-1` A job whose players ask for a wordmap queues exactly one
  `derived_data` row per (lexicon, distribution), however many players share
  the lexicon; one asking for a rack info table queues a `rit` row **and** the
  `wmp` row it is built from.
- `I-DERIVED-2` A leave-generation job queues a wordmap and never a table.
- `I-DERIVED-3` A job with any unbuilt derived file is not dispatched, and the
  same job dispatches once the row says `built`. The single most important test
  here: without it, dispatching early sends a worker no `derived` entry, which
  it reads as a server that checks nothing.
- `I-DERIVED-4` A `failed` row blocks dispatch exactly as a `pending` one does.
  "Give up and send it anyway" is the wrong recovery and must be impossible to
  reach by accident.
- `I-DERIVED-5` Two builders cannot take the same row: the lease and
  `SKIP LOCKED` together.
- `I-DERIVED-6` A row queued under a builder version this binary does not have
  is left alone, not built. Recording a hash against a builder that did not
  produce it is the failure this whole design exists to prevent.
- `I-DERIVED-7` A build whose `kwg` row predates `object_key` fails with a
  message naming the remedy, and the row is left `failed` rather than retried
  forever.
- `I-DERIVED-8` A claim's `derived` entries name the table by
  `<lexicon>.<leaves>`, and two jobs on one lexicon with different leaves get
  two different names and two different hashes.
- `I-DERIVED-9` Each player's derived files come from that player's own rows:
  two players on one lexicon share a wordmap, two on different lexicons get one
  each. Nothing is shared between players, so this ought to fall out of the
  query — but comparing bots on two lexicons is a supported configuration that
  a wordmap keyed on the wrong player would silently break.

---

## 3. API

The real Axum router in-process via `tower::ServiceExt::oneshot`, against a real
Postgres. No browser, no network, no server process. `tower` is already a direct
dependency, so this tier costs no new ones.

Shares tier 2's database harness; the only addition is building an `AppState`
pointed at the test database. Add a helper that logs in and carries the session
cookie plus CSRF token, since most routes need one.

**Every route in the table below needs at least an authorization test and a
happy path.** Where a route has interesting failure modes they are enumerated.

### `A-AUTHZ-*` — authorization, applied to every route

Write these as one table-driven test each rather than 50 separate functions.

- `A-AUTHZ-1` Every `/api/admin/*` route returns 403 for an authenticated
  non-admin. Enumerate the routes from the router so a new one cannot be
  forgotten.
- `A-AUTHZ-2` Every `/api/admin/*` and `/api/me/*` route returns 401 for an
  anonymous caller.
- `A-AUTHZ-3` Every mutating cookie-backed route rejects a request with no CSRF
  header, a mismatched one, and a missing cookie.
- `A-AUTHZ-4` `/api/worker/*` is CSRF-exempt by design — a worker sends a bearer
  token or `X-Worker-UUID`, neither of which a browser attaches automatically.
- `A-AUTHZ-5` A session for a deleted user is rejected; a session for a demoted
  admin loses admin access without needing to expire.
- `A-AUTHZ-6` A revoked or deactivated API key is rejected.

### `A-AUTH-*` — `routes/auth.rs`

- `A-AUTH-1` Register → confirm → login succeeds and sets a session cookie.
- `A-AUTH-2` An unconfirmed login is 403 with a message naming the fix.
- `A-AUTH-3` **A wrong password and an unknown username return an identical
  401** — body and status both, so the endpoint cannot enumerate accounts.
- `A-AUTH-4` **Registering a taken address returns the same body as a fresh
  registration.** Assert the bodies are byte-identical.
- `A-AUTH-5` Registration validates password strength, and rejects a password
  containing the username or email.
- `A-AUTH-6` A confirmation code is single-use; replaying it fails.
- `A-AUTH-7` An expired confirmation code fails.
- `A-AUTH-8` **A password-reset request returns the same body for a known and an
  unknown address**, and mail is sent off the request path so timing does not
  disclose either.
- `A-AUTH-9` A reset token is single-use, expires, and is invalidated by a
  successful reset.
- `A-AUTH-10` Logout clears the cookie and the session no longer authenticates.
- `A-AUTH-11` Rate limits: 11 registrations from one IP hits 429 with
  `Retry-After`; 6 reset requests for one **address** hits 429 even from
  different IPs — the half that matters, since IPs are cheap.

### `A-WORKER-*` — `routes/worker.rs`

- `A-WORKER-1` A claim with no body is rejected with a message naming the fix,
  not a bare 422.
- `A-WORKER-2` A claim with a malformed `magpie_version` is rejected rather than
  assumed.
- `A-WORKER-3` An `unsupported_jobs` list over 200 is **truncated, not
  rejected** — a truncated list costs at most a wasted claim.
- `A-WORKER-4` A first claim with no identity mints an anon UUID and returns it
  in the body; the client reusing it is recognised.
- `A-WORKER-5` A client-invented UUID that is not in `anonymous_workers` is
  rejected with 401 and a message naming the fix.
- `A-WORKER-6` 204 for idle, and a `shutdown` body for each reason. These are
  different answers to different questions.
- `A-WORKER-7` A claim returns `expected_data` with digests, and the client
  declining with `missing_data` records `worker_data_gaps` and releases the
  claim immediately.
- `A-WORKER-8` A decline with an unknown reason is rejected; the three known
  reasons are accepted.
- `A-WORKER-9` Heartbeat extends the claim and is rejected for a stale token.
- `A-WORKER-10` Result submission with a valid token is accepted and publishes
  an SSE event.
- `A-WORKER-11` Result submission with a stale token is silently ignored with a
  success status — the client cannot distinguish, deliberately.
- `A-WORKER-12` Each validation failure from tier 1 surfaces as a 400 with a
  message, not a 500.
- `A-WORKER-13` **Artifact fetch resolves only keys the server minted**; an
  arbitrary key is 404. Otherwise this is a read primitive for the whole bucket.
- `A-WORKER-14` Worker endpoints are rate limited per identity: the 6th request
  in a second is 429, and a different identity is unaffected.
- `A-WORKER-15` `client-version` reports the configured floor and a download
  URL.

### `A-ADMIN-*` — `routes/admin.rs`

- `A-ADMIN-1` Player config create/get/list/delete round-trips, and a config in
  use cannot be deleted.
- `A-ADMIN-2` Creating a job of each type returns `{job}` and the
  job is inactive with no allocation.
- `A-ADMIN-3` Job creation rejects: mismatched win% models, incompatible
  lexicon/distribution, an unknown `input_data` id, a simming player with no
  win% model, a static player with one.
- `A-ADMIN-4` Activate / deactivate / complete / purge / delete each return the
  documented shape and are reflected in a subsequent read.
- `A-ADMIN-5` Import start → poll → confirm over HTTP, including that polling
  reports progress while running.
- `A-ADMIN-6` Confirming an import that is not `staged` is rejected.
- `A-ADMIN-7` `input-data` list and delete, including the in-use refusal.
- `A-ADMIN-8` `job/:id/data-gaps` reports what workers declined for.
- `A-ADMIN-9` `fleet` reports connected workers and their versions.
- `A-ADMIN-10` `backups` reports staleness from the `backups` table.
- `A-ADMIN-11` `rebuild-artifacts` returns what it rebuilt and is idempotent.
- `A-ADMIN-12` Ban and unban by user id and by anon UUID; a banned worker's next
  claim is refused; unban restores it.
- `A-ADMIN-13` `audit-log` paginates and filters by job.
- `A-ADMIN-14` `delete_user` over HTTP.

### `A-RATE-*` — `routes/ratings.rs`

- `A-RATE-1` Pool list and detail render the latest run, with residuals.
- `A-RATE-2` A pool with no run renders empty rather than erroring.
- `A-RATE-3` Creating a pool adds the anchor as a member automatically.
- `A-RATE-4` Adding and removing a member each trigger a refit and return a new
  `run_id`.
- `A-RATE-5` Removing the anchor is refused with a message naming the fix.
- `A-RATE-6` History returns points in time order and excludes unrated configs.
- `A-RATE-7` Recompute is admin-only and returns a new run.

### `A-PUBLIC-*` — `routes/public.rs`

- `A-PUBLIC-1` Job list paginates, and `per_page` is clamped to the maximum.
- `A-PUBLIC-2` Job detail returns the right stats block for each job type, and
  404s for an unknown id.
- `A-PUBLIC-3` `job_results` paginates and filters, and returns `total = -1`
  where an exact count is deliberately not computed.
- `A-PUBLIC-4` `rack_lookup` finds an analysed rack and 404s otherwise.
- `A-PUBLIC-5` The SSE stream emits an event after a result is accepted, and the
  event body is byte-identical to what a page reload would fetch.
- `A-PUBLIC-6` The SSE stream ends cleanly when the client disconnects.
- `A-PUBLIC-7` User and worker lists paginate and do not leak email addresses or
  key hashes.

### `A-ACCOUNT-*` — `routes/account.rs`

- `A-ACCOUNT-1` `me` returns the current user and never a password hash.
- `A-ACCOUNT-2` A created API key is returned in full **exactly once** and never
  again by the list endpoint.
- `A-ACCOUNT-3` The 100-key cap is enforced.
- `A-ACCOUNT-4` Deactivating a key stops it authenticating; reactivating
  restores it; revoking is permanent.
- `A-ACCOUNT-5` One user cannot see or modify another's keys.

---

## 4. Contract

[`contract-fixtures/`](contract-fixtures/) holds one committed example of every
message crossing the birdtest↔MAGPIE boundary. `routes::worker::contract_fixtures`
parses each against the real wire types.

Client→server fixtures are deserialized into the types that actually handle the
request. Server→client fixtures are compared by **field structure, not bytes**,
so fields stay free to move before the first release while a renamed or dropped
field still fails.

Five fixtures exist. The set is complete when every message has one:

- `C-1` `assignment-opening-rack.json` — **missing**.
- `C-2` `assignment-game-pairs.json`, carrying `game_pairs: true` — **missing**.
- `C-3` `result-games.json` — **missing**.
- `C-4` `result-game-pairs.json`, carrying the pentanomial — **missing**, and
  the most important one: it is the newest message and the one MAGPIE and
  birdtest most recently disagreed about.
- `C-5` `result-opening-rack.json` — **missing**.
- `C-6` `result-leave-generation.json` — **missing**.
- `C-7` `heartbeat.json` — **missing**.
- `C-8` `expected-data.json`, the digest list on an assignment — **missing**.
- `C-9` `anon-uuid-assignment.json`, a first claim that mints a UUID —
  **missing**.

Each fixture should be **captured from a real exchange**, not hand-written, and
the capture command documented next to it. A hand-written fixture tests only
that the author and the parser agree.

---

## 5. End-to-end

Playwright against the full stack in Docker, with `fake_worker.py` supplying
contributions — **the only tier that uses it**, and the reason it exists.
A browser journey needs contributions to arrive on cue and land at predictable
values; a real MAGPIE would supply neither, and would put a C build in the way
of a suite that runs on every pull request. Journeys, not assertions per field —
anything that can be checked at tier 3 belongs at tier 3, because a failure
there names the cause and a failure here names a symptom.

Runs against the **built** frontend served by Nginx, not the Vite dev server,
because the built artifact is what ships.

- `E-1` An anonymous visitor browses the landing page, job list, a job detail
  page and the contributor leaderboard.
- `E-2` Register → confirm the email → log in → generate an API key → see it
  exactly once → deactivate it.
- `E-3` An admin imports input data, reviews the staged diff, and confirms it.
- `E-4` An admin creates two player configs and a game-pairs job, activates it
  with an allocation, and watches the dashboard update live over SSE as fake
  workers contribute. **The journey that justifies the tier**: the only place
  SSE, the built Svelte app, the scheduler and a worker are exercised together.
- `E-5` An admin bans a worker and that worker can no longer claim.
- `E-6` A non-admin is redirected away from `/admin`, and an anonymous visitor
  from `/account`.
- `E-7` The ratings page: an admin creates a pool, adds a config, sees the fit
  appear, removes it, and sees the ratings change. Covers the one flow where a
  write is expected to move numbers elsewhere on the page.
- `E-8` A job detail page renders the pentanomial table with the five buckets
  labelled, and the SPRT status text.
- `E-9` The password reset flow end to end.
- `E-10` A page renders correctly at phone width — one journey, not all of them.

### Reading confirmation codes

Two journeys need to read an emailed code. This is the one piece of tier-5
infrastructure that does not exist yet, and it should be built before the tier
is written.

**First, shrink the problem.** Only `E-2` (register → confirm → log in) and
`E-9` (password reset) need a code at all. The other eight need a *confirmed
admin*, which `scripts/seed.py` already produces — so the Playwright fixture
seeds that user and those journeys start at login. That turns "how does the
browser read mail" into a question about two tests rather than ten.

**Then add `MAIL_BACKEND=file`.** One message per file in a bind-mounted
directory, named by timestamp and sanitised recipient:

```
$MAIL_OUTBOX_DIR/20260910-191500-e2e-<uuid>-at-example-invalid.txt
```

A journey registers `e2e-<uuid>@example.invalid` and reads the file matching its
own address. **Naming by recipient is the point**, not a convenience: it is what
makes parallel journeys safe, and it is exactly what the log-scraping approach
cannot do, since the log is one stream with no key tying a code to the
registration that caused it.

`MAIL_BACKEND` is already a config enum with a `Console` arm of about six lines,
so this is one more arm rather than a new concept — and once it exists,
`scripts/seed.py` should use it too and drop its `docker compose logs` scraping.
The hack disappears rather than being reimplemented in a second place.

What was considered and rejected:

| Approach | Why not |
|---|---|
| **Mailpit / MailHog** container with an HTTP API | The one that looks best and is not. birdtest sends through the **SES SDK, not SMTP** ([email.rs](backend/src/email.rs)), so this needs an SMTP backend that production never executes — the E2E tier would be exercising a path that does not ship, which is backwards for the tier whose job is testing what does. |
| A **test-only endpoint** returning the latest code, env-gated | A permanent auth-bypass endpoint. One misconfiguration and anyone can confirm any account. |
| **Scraping `docker compose logs`** from Playwright | Zero code change, and what `seed.py` does today. Cannot tell which code belongs to which registration when journeys run in parallel, and needs Docker daemon access from wherever Playwright runs. |
| **Reading the database** | Impossible, deliberately: `email_confirmations` stores only a hash, so a leaked dump cannot hand out working confirmation links. |
| A **fixed code** under a test flag | Weakens a real security property in a way that can leak into another environment. |

The cost to accept: a third mail backend is a third thing to keep working. It is
small, but it belongs in `docker-compose.yml` and `.env.example` so it does not
become folklore.

---

## 6. MAGPIE smoke

The only tier that runs a real MAGPIE. It catches what fixtures structurally
cannot: not whether the *shape* of a message is agreed, but whether MAGPIE's
actual behaviour matches the contract.

**Opt-in, then fail loudly.** Excluded from a default run. When you ask for it
and MAGPIE is missing, that is a hard error, not a skip — a green run must never
silently mean nothing was exercised.

The backend itself is no longer opt-in about this: it runs a pinned MAGPIE for
every derived file and every leave-generation KLV, reads the builder versions
out of the binary at startup, and refuses to bind without one. `MAGPIE_BIN`
names it (`/usr/local/bin/magpie` in the image, a local checkout's `bin/magpie`
in development). That is a stronger version of what the old `#[ignore]`d
round-trip tests bought: there is no longer a second implementation to check
against MAGPIE, because there is no second implementation.

**Correctness is established by version and capability probe.**
`birdtest-contribute` reports `0.1.0`, the shipped `MIN_MAGPIE_VERSION` default
and the branch's pre-release version; a checkout reporting anything lower is refused. The
probe additionally asks the binary what it can do: that `contribute` is a
registered command, and that it accepts the current required claim body.

**This tier cannot use the synthetic fixture lexica** — nor can the dev
environment, for the same reason. The fixture's `NWL23.kwg` is a stub; a real
MAGPIE would load it and fail, or worse, not fail. Tier 6 seeds from a real
MAGPIE-DATA install (`scripts/seed.py`, whose `--tarball-date` defaults to the
`DATA_VERSION` the checkout installed), which is also what makes it a genuine
check that birdtest's pinned digests match what `download_data.sh` actually
installs. If they diverge the client declines every task and the tier fails —
surfacing the mismatch as a red build rather than as a dead job in production.

- `M-1` One `games` task runs through `magpie contribute` and the result lands
  and is credited.
- `M-2` One `game_pairs` task does the same, and **both pentanomial invariants
  hold on real games**: `sum(buckets) * 2 == games`, and
  `sum(i * bucket[i]) == 2 * wins + ties`. This is the check that proves
  MAGPIE's pentanomial and birdtest's validation agree; it has been run by hand
  and must not stay manual.
- `M-3` One `opening_rack` task lands, with one analysis per requested rack.
- `M-4` One `leave_generation` task lands, is staged, moves the generation's
  live counters, and a merge folds it into `leave_rack_progress`.
- `M-5` A worker whose data digests do not match declines with `missing_data`
  rather than contributing unverified results.
- `M-6` A worker below the job's version floor declines with `magpie_version`.
- `M-7` `capture_positions` on a real game produces positions whose CGP parses
  back through MAGPIE.
- `M-8` The KLV a generation transition builds loads in a real MAGPIE.
  *(Now structural: the transition **is** a real MAGPIE writing it. What is
  worth a case instead is that the CSV the server streams is one MAGPIE
  accepts — a rack it names differently, or a generation missing a rack, is
  refused rather than silently valued at zero.)*
- `M-9` Two contributors run concurrently without duplicate seeds — the
  concurrency check from `I-SCHED-13`, against the real client.
- `M-10` A job with `use_rit` dispatches only once its table is built, and a
  real `magpie contribute` builds a table whose hash matches the server's and
  plays with it. The expensive one in this list: a table is 1.9 GB and takes
  minutes, so it belongs in the nightly run and wants the small fixture
  distribution if `M-10` is ever to be quick.
- `M-11` A worker whose derived file does not match declines with
  `derived_mismatch` and both digests reach `worker_data_gaps`. Forcing the
  mismatch is the work here: the honest way is a server whose recorded hash was
  produced by a different builder version.

`scripts/e2e_magpie.py` implements `M-1` to `M-4` (with `M-3` run for a static
and a simming player, asserting the simulated statistics are stored) and checks
that leave generation writes nothing into MAGPIE's data directory. CI runs it
nightly (`.github/workflows/nightly.yml`); locally, bring up the stack and run it
with `--magpie` and `--magpie-root`. `M-2`'s invariants are enforced by the
server's plausibility checks on every accepted pair result, so a clean run
covers them.

That makes a leave-generation smoke expensive here: real English means the
3,199,724-rack universe above at job creation. Two ways out, in preference order:
keep tier 6's leave-generation case to a single generation and accept a slow
nightly job, or place the tiny fixture distribution on MAGPIE's own `-path`
search list so both sides load the same small bag. The server already does the
second thing for its own conversions — every one runs in a throwaway directory
holding exactly the pinned bytes — so the machinery exists. The second is better if it works; **verify that a
real MAGPIE actually plays with a real `NWL23.kwg` against a five-letter bag
before relying on it**, because that combination has never been run.

---

## What is deliberately not tested

Not gaps. Each is a decision, with the reason, so it is not silently
re-litigated or mistaken for an oversight.

**`scripts/dev.py`.** It is a developer convenience whose failure is immediate
and obvious: it either brings up a stack and starts contributors or it does not,
and the person running it is watching. An automated test would have to stand up
Docker, MAGPIE and a browser to assert something a human sees in ten seconds.
Its *seeding* half is worth testing, and is — tiers 5 and 6 both call
`scripts/seed.py`, so a break there fails a build.

**Third-party behaviour.** That Argon2 hashes correctly, that `governor` counts
tokens, that `sqlx` maps types, that S3 stores bytes. We test our *use* of them
(`U-AUTH-3`, `A-AUTH-11`, `I-ART-1`) and not the libraries.

**Terraform.** `infra/` is checked by `terraform validate` and by applying it.
Unit-testing HCL tests the plan, not the deployment, and the failure mode that
matters — an apply that breaks production — is not reachable from a test suite.
Backup *restores* are covered by the monthly drill, which is the real check.

**Generated and vendored code.** Vendored cJSON in MAGPIE, SvelteKit's
`.svelte-kit` output.

**Exhaustive rack enumeration for real English.** 3,199,724 racks and 914,624
leaves. `U-RACK-7` asserts the counts, and `U-RACK-5` proves the unranking
algorithm against full enumeration on a *small* distribution. Running the full
enumeration in a unit test would add minutes to every run to re-prove the same
property.

**Visual appearance.** No screenshot diffing. It is the highest-maintenance,
lowest-signal test there is on a UI still changing shape; `F-CHART-*` covers the
arithmetic that would actually be wrong, and `E-10` covers layout at one narrow
width. Revisit once the design settles.

**Every combination of job type × capability × scheduler state.** The state
space is combinatorial. The tests above pick the boundaries where behaviour
changes (a version exactly at the floor, an allocation of 0, a redundancy of 1
versus 2, an empty top tier) rather than sampling the interior.

**Load and performance.** No throughput or latency assertions. Nothing in
birdtest has a performance requirement anyone has stated, and a threshold nobody
chose fails on a busy CI runner instead of on a real regression. The two places
where cost is structural — leave-universe enumeration at job creation, and the
rating fit — are bounded by design and documented in PLAN.md.

**The 100% ambition, stated honestly.** Line coverage is not the goal and is not
measured. The goal is that **every behaviour PLAN.md promises has a test that
fails when it stops being true.** Some lines — a `Display` impl, a
`#[derive]`, an error branch that only fires on a database that has already
failed — will never be executed by this suite, and forcing them to be would add
tests that assert nothing anyone depends on.

---

## The shared substrate

Tiers 5 and 6 and the dev environment all need the same thing: an empty database
turned into a state where work can flow. They diverge only on which data seeds
it — the fixture tarball for tier 5, a real MAGPIE-DATA install for tier 6 and
`dev.py`, because a real worker cannot be fed stubs. That chain is six steps and
three of them did not exist a month ago, which is why it belongs in code rather
than in prose.

### The fixture tarball

`fixtures/data-<date>.tgz`, built by `fixtures/build.sh` exactly the way
MAGPIE-DATA builds the real ones (`cp -RL`, `tar -czf`, `split`), so import's
chunk-walking and extraction are exercised for real rather than bypassed.

Under 2 KB, because of a useful asymmetry: the server only ever *parses*
`letterdist` and `layout` bytes — that is what `input_data.content` is for. It
does now hand `kwg` and `klv` bytes to MAGPIE, but only for a job whose players
ask for a wordmap or a rack info table, which below tier 6 none do. The `winpct`
rows stay digest-only. So the fixture carries:

| Path | Contents |
|---|---|
| `letterdistributions/english_fixture.csv` | A **deliberately tiny bag**, not real English. The server parses this. |
| `layouts/standard15.txt` | The real 244-byte file. |
| `lexica/NWL23.kwg` | A stub. Never read below tier 6. |
| `lexica/NWL23.klv2` | A stub. |
| `strategy/winpct.csv` | A stub. |

Two naming constraints, both enforced by [`compat.rs`](backend/src/compat.rs),
and both of which look arbitrary until you hit them:

**The lexicon stubs must be named `NWL23`**, not something honest like
`TESTLEX`. Lexicon names are validated by prefix and unrecognised ones are
rejected outright — `MADEUP` is one of the known-bad cases in the compat table —
so a job pinning a made-up lexicon cannot be created at all.

**The distribution must be named with an `english` prefix.**
`ld_type_from_distribution` matches on prefix, so `english_fixture` resolves to
`LdType::English` and is compatible with `NWL23`, exactly as the real
`english_super` is. A name like `testdist` resolves to `Unknown`, and `Unknown`
matches nothing — deliberately, since MAGPIE reaches the same outcome by raising
an error. So the fixture gets an English-prefixed name while being nothing like
English inside.

**And it must be tiny, which is the whole reason not to ship the real
`english.csv`.** The bag size drives two things that happen synchronously at job
creation:

| Distribution | Leaves of size 1–6 | Distinct 7-tile racks |
|---|---|---|
| Real `english` | **914,624** | 3,199,724 |
| A 6-tile fixture bag | **431** | 149 |

`seed_generation` inserts one `leave_rack_progress` row per full rack and the
generation-0 KLV holds a value for every leave, before the job is usable. On
real English that is 3.2 million rows per leave-generation job created —
fine in production, where a job is created once and runs for weeks, and
completely unusable as a per-test fixture. The tiny bag makes the same code path
run in milliseconds while exercising every part of it.

**State this loudly wherever the fixture is used: `NWL23.kwg` is not NWL23.**
Any code that actually loads it is broken by construction. That is why the two
contexts with a real worker in them — tier 6 and `dev.py` — seed from real data
instead, and why the fixture is confined to tier 5 and below.

Serving it needs one small change to the backend: [`inputdata.rs`](backend/src/inputdata.rs)
hardcodes `api.github.com` and `raw.githubusercontent.com`, and
`MAGPIE_DATA_REPO` only substitutes the `owner/repo` segment. Two config
overrides defaulting to the real hosts, plus a static-file container in compose,
make import work offline.

### `scripts/seed.py`

Empty database → work flowing. Drives the **real HTTP API** rather than writing
SQL, so seeding is itself a smoke test of registration, confirmation, validation
and job creation. Two things have no endpoint and are done directly: promoting a
user to admin, and reading the confirmation code.

```
scripts/seed.py [--api URL] [--job-type TYPE] [--tarball-date YYYYMMDD]
                [--magpie-root PATH] [--min-magpie-version V] ...
```

1. Register a user, read the confirmation code, confirm it. The code comes
   from the backend's log, not the database: `email_confirmations` stores only
   a hash, which is the point — a leaked dump must not hand out working
   confirmation links. (Log scraping is a stopgap. Once `MAIL_BACKEND=file`
   exists for tier 5, this reads the outbox file for its own address instead —
   see [Reading confirmation codes](#reading-confirmation-codes).)
2. Promote to admin (SQL — `is_admin` is settable through no endpoint).
3. Import input data and confirm the staged diff. The date defaults to the
   `DATA_VERSION` in the caller's MAGPIE checkout, so the digests the server
   pins are the bytes its workers actually have.
4. Create player configs pinning those rows. Two static players that sort
   differently, which is the cheapest way to get a job with real signal: they
   choose different moves nearly every turn, so pairs diverge instead of
   playing out identically.
5. Create a job of the requested type, with an explicit version floor — a job
   records the floor it was created under, so leaving it implicit lets one
   created earlier keep declining an unreleased local build for ever.
6. Activate it with an allocation.

Re-running is safe: an unconfirmed account is confirmed, an imported tarball is
skipped, and an active job of the same type is reused rather than duplicated.

### What tiers 2 and 3 must *not* share

They do not call the seed. They construct exactly the state each test needs.

The moment integration tests depend on a realistic fixture, every test is
coupled to its contents and the cases that matter become unreachable: zero
active jobs, every active job at 0%, a worker locked out of every job, a job
at capacity, a claim one second past its timeout. Those need precise state, not
plausible state. Tiers 2 and 3 share the migration and the builders, and nothing
above them.

This is the boundary that erodes first, because reusing the seed is always
easier in the moment.

---

## The development environment

```
scripts/dev.py [-w N] [--threads N] [--job-type TYPE] [--no-browser] ...
```

Brings up the stack, waits for health, seeds it, starts `N` workers, and opens a
browser. It is tier 6's setup with the assertions and the teardown removed, and
it calls the same `seed.py`. `--help` lists the rest; README has the table.

**Both scripts exist and are exercised.** `dev.py` has been run end to end
against real MAGPIE contributors: results land, the pentanomial's two
invariants hold on real games, and SPRT reads all pairs rather than a filtered
subset.

**Workers are always real `magpie contribute` clients.** There is no fake-worker
mode. `fake_worker.py` belongs to tier 5 and nowhere else: it exists so a browser
journey can have contributions arriving under it without a C toolchain in the
loop, and that is a property of an assertion harness, not of a place you develop.
Developing against synthetic results means the behaviour you watch in the UI is
one nobody's MAGPIE will ever produce — the numbers move, the dashboard fills,
and none of it is evidence. The cost is real and accepted: `dev.py` requires a
built MAGPIE (`MAGPIE_BIN`) and a real MAGPIE-DATA install (`MAGPIE_DATA_PATH`),
so bringing up birdtest is no longer a Docker-only operation, and it fails with a
message naming both when either is missing.

**Which means `dev.py` seeds from real data, not the fixture.** A real MAGPIE
cannot be fed the fixture's stub lexica, for exactly the reason tier 6 cannot —
so the dev environment inherits tier 6's data requirements wholesale, including
the leave-generation cost noted above. `scripts/dev.py --job-type` passes through
to the seed; prefer a `games` or `game_pairs` job for day-to-day work and reach
for `leave_generation` deliberately.

`--no-browser` for SSH sessions and CI.

---

## CI

GitHub Actions.

**Per pull request**, in order, so the cheap thing fails first:

1. `cargo clippy --all-targets -- -D warnings`, `cargo test` (tiers 1 and 4),
   `npm run check` and `npm test` (tier 1F). No services needed.
2. Tiers 2 and 3 against a Postgres service container.
3. Tier 5: compose up, seed, Playwright.
4. `terraform fmt -check` and `terraform validate` (no AWS credentials).
5. MAGPIE's half of the contract: check out MAGPIE `birdtest-contribute`, copy
   this branch's `contract-fixtures/` over its `test/birdtest_contract/`, and run
   `magpie_test contribute`. A fixture changed here and not in MAGPIE fails
   here.

Implemented in `.github/workflows/ci.yml`: 1 (without `npm test`, which has no
tests yet), 2, 4, 5, and the image builds. Tier 5 is not.

**Nightly**:

- Tier 6, with a built MAGPIE and a real `download_data.sh` install
  (`.github/workflows/nightly.yml`, running `scripts/e2e_magpie.py`).
- A migration replay from an empty database.
- `scripts/restore-roundtrip.sh` — dump, drop, restore, verify.

Nightly failures are an alert rather than a blocked merge, because they are
slower and more environment-sensitive than a pull request should wait on.

---

## Conventions

| Tier | Lives in | Run with |
|---|---|---|
| 1 | `#[cfg(test)] mod tests`, in-file | `cargo test` |
| 1F | `*.test.ts` beside the source | `npm test` |
| 2, 3 | `backend/tests/` | `cargo test --test '*'` |
| 4 | `routes::worker::contract_fixtures` | `cargo test` |
| 5 | `e2e/` | `npx playwright test` |
| 6 | `#[ignore]`, marked with a reason | `cargo test -- --ignored` |

Environment variables: `TEST_DATABASE_URL` (tiers 2–3), `MAGPIE_BIN` and
`MAGPIE_DATA_PATH` (tier 6 and `scripts/dev.py`, required by both).

**Name a test after the claim it proves, not the function it calls.**
`a_negative_standard_deviation_is_rejected`, not `test_check_game_aggregate`. A
failing name should tell you what broke without opening the file.

**Assert the reason, not just the status.** A test that accepts any 400 passes
when the handler rejects the request for the wrong reason. Match on the error
code or a distinctive part of the message.

**A test that needs a service it cannot find fails; it does not skip.** The one
exception is tier 6, which is excluded from the default run by `#[ignore]` — but
once selected, it fails loudly like everything else.

**Where a bug has been found by hand, the fix lands with the test that would
have caught it**, and the test names the bug. Three exist to be written from
this session alone: `I-JOB-2` (the renamed `winpct_id` column), `U-FAKE-3` (the
fake worker's opening-rack shape, already written), and `M-2` (the pentanomial
invariants, currently proven only by a manual run).
