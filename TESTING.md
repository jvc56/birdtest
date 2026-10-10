# birdtest — Testing

[PLAN.md](PLAN.md) says what birdtest is and why it is built that way.
[README.md](README.md) says how to run it. This document says **what we
guarantee and how we check it**: seven tiers, what each one is allowed to touch,
the shared machinery underneath them, and an enumerated list of every test worth
writing.

The lists below are meant to be **worked through**, not read for flavour. Each
entry has an id (`U-RACK-3`, `I-SCHED-13`, …) so progress can be tracked, and is
phrased as a claim a test either proves or fails to prove. An entry says what to
set up and what to assert; it does not say how to write Rust. Each carries a
*(Covered: …)* note naming the tests that prove it (`file::test` for
`backend/tests/`, `module::tests::test` for in-file unit tests), and a test
written for an entry starts its doc comment with the id, so `grep -rn
'I-SCHED-6'` finds it from either end. Tests that predate the list, and the
groups a later coverage review added, cite a bug or PLAN.md instead and are
found from this end only. Where the code deliberately does something other
than an entry first said, the entry has been corrected and says why.

The list has been worked through: every id below has a test, with the
exceptions marked **Partial** in place, and a coverage review added groups the
original list did not have (`U-ARCHIVE-*`, `U-STATS-*`, `I-EXPORT-*`,
`A-BOUND-*`, `S-BACKUP-*`). The infrastructure they needed exists: the [tier-2
harness](#2-integration) (`backend/tests/common/mod.rs`), the [file mail
backend](#reading-confirmation-codes), the [fixture
tarball](#the-fixture-tarball) with a static GitHub stand-in, and the contract
[capture proxy](#4-contract).

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

Tiers 1–4 need nothing but a Postgres and a MinIO (1, 1F and 4 need nothing at
all). Tier 5 needs Docker, Playwright and a MAGPIE build (the backend refuses to
start without one, though no journey runs it). Tier 6 additionally needs a
MAGPIE checkout with real MAGPIE-DATA, and is the only tier that is not run by
default.

**Pick the lowest tier that can prove the claim.** A rule about arithmetic goes
in tier 1 even if it is reachable over HTTP; a rule about SQL goes in tier 2
even if a browser could see it. A failure at tier 1 names a function, a failure
at tier 5 names a symptom.

### Status today

| Tier | Tests | Where |
|---|---|---|
| 1 Unit | 250 | `#[cfg(test)]` in `jobs::plausibility` (30), `inputdata` (30), `jobs::racks` (17), `stats::bradley_terry` (30), `stats::match_test` (10), `stats::outcomes` (7), `error` (9), `config` (8), `extract` (7), `routes::admin` (16), `jobs::handler` (6), `backups` (5), `auth::api_key` (6), `auth::session` (4), `clientip` (5), `version` (4), `auth::csrf` (3), `compat` (3), `derived` (3), `jobs::opening_rack` (7), `magpie` (3), `models::job` (3), `routes::public` (3), `sse` (5), `email` (8), `jobs::dispatch` (2), `ratelimit` (2), `routes::auth` (2), `jobs::game` (2), `board`, `exports`, `jobs`, `jobs::leave_gen`, `routes` (1 each); `jobs::game_pair` (3), `artifacts` (2) |
| 1F Frontend unit | 218 | Vitest, `frontend/src/lib/`: `format.test.ts` (46), `jobSettings.test.ts` (13), `matchScore.test.ts` (4), `matchTest.test.ts` (5), `moveList.test.ts` (5), `compare.test.ts` (3), `consensus.test.ts` (8), `cgp.test.ts` (16), `api.test.ts` (18), `auth.test.ts` (17), `accountRules.test.ts` (9), `ratingPool.test.ts` (7), `sse.test.ts` (12), `importWatch.test.ts` (10), `poller.test.ts` (7), `contributeDocs.test.ts` (4), `nginxConfig.test.ts` (3), and `charts/`: `ratingDotPlot.test.ts` (19), `labels.test.ts` (2), `residuals.test.ts` (5), `pentanomial.test.ts` (5) |
| 2 Integration | 186 | `backend/tests/`: `leave_gen.rs` (34), `ratings.rs` (33), `scheduler.rs` (19), `jobs.rs` (20), `stats.rs` (19), `input_data.rs` (14), `derived.rs` (9), `leave_generation.rs` (8), `exports.rs` (16), `submissions.rs` (6), `artifacts.rs` (5), `audit.rs` (3) |
| 3 API | 250 | `backend/tests/`: `worker_api.rs` (52), `admin_api.rs` (59), `auth_routes.rs` (28), `worker_routes.rs` (24), `boundaries.rs` (18), `public_api.rs` (24), `admin_routes.rs` (13), `authz.rs` (7), `account.rs` (10), `auth_api.rs` (5), `finish.rs` (9), `fake_worker.rs` (1) |
| 4 Contract | 15 | `routes::worker::contract_fixtures`, over 18 fixtures; MAGPIE checks its half in `test/contribute_test.c` |
| 5 End-to-end | 22 | Playwright journeys `E-1`..`E-18` (`E-11` in three tests, `E-10` and `E-12` in two each) in `e2e/tests/*.spec.ts`, plus the `admin.setup.ts` sign-in they share; run by `e2e/run.sh` |
| 6 MAGPIE smoke | 16 cases + 16 | `scripts/e2e_magpie.py`'s cases `M-1`..`M-7`, `M-9`..`M-17` against a real `magpie contribute` (natively via `scripts/e2e_magpie_native.sh`, or the nightly compose job); and 16 opt-in `#[ignore]` Rust tests that run the server's own MAGPIE (`MAGPIE_BIN`): `magpie_smoke.rs` (6), `magpie_leave.rs` (7), `magpie_routes.rs` (3) |

The tier-2/3 split is by the ids a file proves; many tier-2 files also drive
the router to reach a state, and several tier-3 files read the database
directly to assert one. With the tier-6 tests selected, `cargo nextest run
--run-ignored all` runs 717 backend tests (the per-tier counts above are
from `cargo nextest list --run-ignored all` and `vitest`, after the thirty-third
audit's fourth pass).

Tier 2 was the largest gap and the highest value.
`sqlx::query` is checked at runtime, so the compiler sees opaque text. Two bugs
of exactly this shape shipped and were found by hand before the harness existed
— a three-column `INSERT` into a table with a fourth `NOT NULL` column, and a
`SELECT` of `win_pct_model` after that column became `winpct_id`, which broke
*every* two-config games job. Neither was caught by `cargo check` or by 63
passing tests. Every module that carries SQL now has tests that execute it
against a real database; the coverage map below tracks it module by module.

---

## Coverage map

Every source file, the tier that owns its behaviour, and where it stands. "Owns"
means the tier a regression should be caught by; most files are also touched
incidentally by higher tiers.

### Backend

| File | Owning tier | Status |
|---|---|---|
| `stats/match_test.rs`, `stats/outcomes.rs` | 1 | Covered — values pinned to independently computed numbers, and the match test's error rate and power simulated (`U-STATS-*`) |
| `stats/bradley_terry.rs` | 1 | Covered |
| `version.rs` | 1 | Covered |
| `compat.rs` | 1 | Covered |
| `jobs/racks.rs` | 1 | Covered (`U-RACK-*`) |
| `derived.rs` | 1 + 2 (+ 6) | Covered — naming and the gate at tier 1, the queue at tier 2 (`I-DERIVED-*`); the build itself at tier 6 (`magpie_smoke.rs`, `M-10`, `M-13`) |
| `magpie.rs` | 1 (+ 6) | Covered — the `builders` JSON contract and error bounding at tier 1; the subprocess in the opt-in `magpie_smoke.rs` |
| `jobs/plausibility.rs` | 1 | Covered |
| `inputdata.rs` (archive walk) | 1 | Covered (`U-ARCHIVE-*`) |
| `inputdata.rs` (download, staging, confirm) | 2 | Covered (`I-INPUT-*`, against a fake GitHub and a real object store) |
| `backups.rs` | 1 | Covered |
| `error.rs`, `extract.rs` | 1 | Covered (`U-ERR-*`) |
| `auth/api_key.rs`, `auth/session.rs`, `auth/csrf.rs` | 1 | Covered (`U-AUTH-*`) |
| `config.rs` | 1 | Covered (`U-CFG-*`, through `Config::from_lookup`, so no test mutates the environment) |
| `email.rs` | 1 | Covered for the file backend; the SES arm is never executed below production (see [Not tested](#what-is-deliberately-not-tested)) |
| `jobs/handler.rs` (wire types) | 1 + 4 | Covered (`U-WIRE-*`, `C-*`) |
| `models/job.rs` | 1 | Covered |
| `jobs/mod.rs` (`expected_data`, inserts) | 2 | Covered (`I-EXPECT-*`, `I-SUBMIT-*`) |
| `jobs/game.rs`, `game_pair.rs`, `opening_rack.rs`, `leave_gen.rs` | 1 (validation) + 2 (SQL) | Covered |
| `jobs/registry.rs` | 2 | Covered (`I-JOB-*`, through job creation and `store_result`) |
| `scheduler.rs` | 2 | Covered (`I-SCHED-*`) |
| `ratings.rs` | 2 | Covered (`I-RATE-*`) |
| `jobstats.rs` | 2 | Covered (`I-STATS-*`) |
| `audit.rs` | 2 | Covered (`I-AUDIT-*`) |
| `artifacts.rs` | 1 + 2 | Covered (`U-ART-1`; `I-ART-*`, needs `TEST_S3_ENDPOINT`) |
| `exports.rs` | 2 | Covered (`I-EXPORT-*`, needs `TEST_S3_ENDPOINT`) |
| `sse.rs` | 3 | Covered (`sse::tests`, `A-PUBLIC-5`, `-6`, `-6a`) |
| `ratelimit.rs` | 3 | Covered (`A-AUTH-11`, `A-WORKER-14`, `A-BOUND-1`, `-2`) |
| `auth/mod.rs` (extractors) | 3 | Covered (`A-AUTHZ-*`) |
| `routes/auth.rs` | 3 | Covered (`A-AUTH-*`) |
| `routes/account.rs` | 3 | Covered (`A-ACCOUNT-*`) |
| `routes/worker.rs` | 3 + 4 | Covered (`A-WORKER-*`, `C-*`, `I-STATS-9`'s finish check) |
| `routes/admin.rs` | 3 | Covered (`A-ADMIN-*`) |
| `routes/public.rs` | 3 | Covered (`A-PUBLIC-*`) |
| `routes/ratings.rs` | 3 | Covered (`A-RATE-*`) |
| `main.rs` (wiring, sweep task) | 3 | Partial — the state `main` builds (`AppState::new`, `A-BOUND-10`) and each startup reaper and sweep body are tested as functions; the loop that schedules them is not |

### Frontend

| Area | Owning tier | Status |
|---|---|---|
| `lib/format.ts` | 1F | Covered (`F-FMT-*`) |
| `lib/api.ts` (error mapping, CSRF header, query strings) | 1F | Covered (`F-API-*`) |
| `lib/sse.ts` | 1F | Covered (`F-SSE-*`) |
| `lib/auth.ts` | 1F | Covered (`F-AUTH-*`) |
| `lib/accountRules.ts` | 1F | Covered (`F-ACCOUNT-*`) |
| `lib/ratingPool.ts` | 1F | Covered (`F-RATE-*`) |
| `lib/importWatch.ts`, `lib/poller.ts` | 1F | Covered (`F-IMPORT-*`, `F-POLL-*`) |
| `lib/roundRobin.ts` | 1F | Covered (`F-RR-*`) |
| `lib/allocation.ts` | 1F | Covered (`F-ALLOC-*`) |
| `lib/movegens.ts` | 1F | Covered (`F-MOVEGENS-*`) |
| Chart maths | 1F | Covered (`F-CHART-*`). The arithmetic moved out of the components into `lib/charts/*.ts` so it could be tested; the `.svelte` files that draw it are exercised only by tier 5 |
| Every page under `routes/` | 5 | Partial — the eighteen journeys (E-10 visits `/users`). `/admin/backups`, `/admin/derived-data`, `/admin/fleet` and `/admin/users` are in none of them (`/admin/allocation` is E-4's); their endpoints are tier 3 |

### Scripts and cross-repo

| Area | Owning tier | Status |
|---|---|---|
| `worker/fake_worker.py` output shapes | 1 | Covered — a captured submission for every job type and every `--mode` (`U-FAKE-*`), kept current by `scripts/fake-worker-fixtures.sh --check` in CI (`U-FAKE-6`) |
| `scripts/seed.py` | 5, 6 (used by both) | Covered by use: `e2e/run.sh` and `e2e_magpie.py` both seed through it |
| `scripts/dev.py` | Manual | Deliberate — see [Not tested](#what-is-deliberately-not-tested) |
| `scripts/backup.sh`, `restore-drill.sh`, `restore-roundtrip.sh`, `restore-job.sh`, RUNBOOK §1's re-apply step | Nightly | Covered (`S-BACKUP-*`) |
| `scripts/scrub.sql` | 3 | Covered (`S-SCRUB-1`, `-2`) |
| `infra/` variable validations, the deploy-failed alert, the split services' deployment settings | CI (`terraform`) | Covered (`S-TF-1`..`-4`: `terraform test` against mock providers) |
| `scripts/dev-restore.sh` | CI (`scripts`) | Covered (`S-BACKUP-6`: its `SCRUB` rule against a stub `COMPOSE`, and that the scrub is the copy's) |
| birdtest ↔ MAGPIE wire | 4 + 6 | Covered — `C-1`..`C-9` on both sides, and tier 6 |

---

## 1. Unit

Pure logic, no I/O. Rust idiom: `#[cfg(test)] mod tests` in the same file as the
code.

**May not**: open a socket, connect to a database, or read a file. Fixture bytes
come from `include_bytes!`, and the repository files a consistency test compares
(`U-CFG-5`, the SES alarm check in `email.rs`) from `include_str!`, so cargo
rebuilds the test when one changes and the test binary carries them; anything
that needs a path uses a `tempdir`, and a test may read back what it wrote
there. Two narrow exceptions: `email.rs`'s stand-in for SES listens on loopback,
and `U-ARCHIVE-11` reads the process's own `/proc/self/status` for its peak memory.

### Covered before the list

`stats::sprt` (since replaced by `stats::match_test`), `stats::bradley_terry`, `version`, `compat`, `backups`,
`derived`, `magpie`, `clientip`, `routes::admin`'s time-limit validation, and the
first rules of `jobs::plausibility` and the `inputdata` archive walk had tests
before the entries below were written; the entries were what was missing, and
each now names its tests. Every group has since grown (see the status table).

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
  minimal two-letter one, and reports the right tile counts and machine letters
  -- read as a row's position in `letters()`, the numbering `canonical_rack`
  and the positions page use (a `machine_letter()` the tests alone read, kept
  for a KLV builder since deleted, went in the thirty-third audit's third pass).
  A zero-count row keeps its machine letter (it used to shift every later
  letter's number). *(Covered:
  `racks::tests::the_real_english_distribution_parses_with_magpies_numbering`,
  `racks::tests::a_minimal_two_letter_distribution_parses`,
  `racks::tests::a_zero_count_row_keeps_its_machine_letter_but_has_no_tiles`.)*
- `U-RACK-2` `parse` rejects each malformed shape distinctly: a short row, a
  non-numeric count, an empty file. The error names the origin string it was
  given. A distribution whose racks this representation cannot spell -- a
  letter listed twice, or a multi-character letter such as Catalan's `L·L`,
  `NY` and `QU` -- still *parses*, because a games job only hands its bytes to
  MAGPIE, and has no rack space: `RackIndex::new` refuses it with the reason,
  so an opening-rack or leave-generation job on it is a `400` at creation.
  *(Covered:
  `racks::tests::each_malformed_distribution_is_rejected_for_its_own_reason`,
  `racks::tests::a_distribution_whose_racks_cannot_be_spelt_parses_but_has_no_rack_space`
  (the real `catalan.csv`), and
  `jobs::a_catalan_games_job_runs_and_a_catalan_rack_job_is_refused_at_creation`.
  A duplicate letter used to be accepted and enumerated twice; refusing it in
  `parse`, the first fix, made every Catalan job -- games included -- fail to
  load, which a review caught before it shipped.)*
- `U-RACK-2b` A distribution keeps every row whole for drawing a tile -- the
  letter as named (`L·L`), its blank's spelling (`l·l`), its score -- in
  machine-letter order, and `canonical_rack` spells a rack typed any way as
  MAGPIE spells a captured position's (`rack_get_string`): machine-letter
  order (Catalan's `Ç` after `C`, not after `Z`), the blank last, a
  multi-character letter bracketed; a rack with a tile the distribution lacks
  is `None`. The positions search used to sort the characters, so a rack with
  a blank -- `?` sorts first, MAGPIE writes it last -- was never found.
  *(Covered: `racks::tests::a_rack_is_spelt_as_magpie_spells_it`.)*
- `U-RACK-3` `enumerate_racks(k)` returns exactly `total()` racks for that size,
  each sorted, with no duplicates. *(Covered:
  `racks::tests::every_enumerated_rack_is_canonical_distinct_and_counted`.)*
- `U-RACK-4` `enumerate_leaves(max)` covers sizes 1..=max and matches the sum of
  per-size counts. *(Covered:
  `racks::tests::the_leave_universe_is_every_size_up_to_the_maximum`.)*
- `U-RACK-5` `rack_at(i)` agrees with `enumerate_racks` at every index for a
  small distribution with a tile count above 2, so multiset repetition is
  exercised. *(Covered: `racks::tests::unranking_matches_full_enumeration`,
  extended; `racks::tests::unranking_is_past_the_end_safe`.)*
- `U-RACK-6` `racks_in_range(start, count)` is `rack_at` over those indices, in
  index order, and a range that runs off the end stops there. *Not* a slice of
  `enumerate_racks`, as this entry first said: `racks_in_range` scatters
  adjacent indices across the space (`rack_at`), so its ranges tile the
  enumeration's set, not its order. The plain slice is
  `racks_in_enumeration_range`, which leave generation walks. *(Covered:
  `racks::tests::a_range_is_the_racks_at_its_indices_and_stops_at_the_end`,
  `racks::tests::scattering_spreads_adjacent_indices`.)*
- `U-RACK-7` `total()` for real English is exactly 3,199,724 for size 7 and
  914,624 for leaves up to 6 — the two numbers PLAN.md quotes and that job
  creation cost depends on. *(Covered:
  `racks::tests::real_english_has_3199724_racks_and_914624_leaves`.)*
- `U-RACK-8` A distribution with a blank tile enumerates blanks in the documented
  position — `?` sorts before every letter, so it leads any canonical rack, and
  every rack with a blank comes after every rack without in the index — and a
  rack containing one round-trips through `rack_at`. *(Covered:
  `racks::tests::blanks_lead_their_racks_and_sit_at_the_end_of_the_index`.)*
- `U-RACK-12` A rack typed any way -- lower case, reversed, spaces anywhere --
  is spelt as the rack index spells it (`RackIndex::spelling`), which is how an
  opening-rack job stores it and the rack lookup searches for it, for a
  distribution whose file order is not character order (German's `A, Ä, B`):
  every rack of a small space comes back as `rack_at` gave it. *(Covered:
  `racks::tests::a_typed_rack_is_spelt_as_the_index_spells_it`.)*
- `U-RACK-9` A distribution with more letters than MAGPIE's `MAX_ALPHABET_SIZE`
  (50) is refused, naming the file; one at the limit parses. MAGPIE loaded a
  longer one and wrote past every per-letter array. *(Covered:
  `racks::tests::a_distribution_past_magpies_alphabet_is_refused`; job creation
  refuses one in `A-ADMIN-3`.)* (Twenty-second audit.)
- `U-RACK-11` A rack's consensus standing: it settles once it has its fewest
  analyses and they agree in the share asked for (4 of 5 at 80%, not 3 of 4),
  not before however much they agree; at its most without agreeing it settles
  without a consensus, on its most common move; a tie names the alphabetically
  first; one analysis per rack settles at the first. *(Covered:
  `opening_rack::tests::a_rack_settles_at_its_minimum_once_its_analyses_agree`,
  `…::a_split_rack_settles_at_its_maximum_without_a_consensus`,
  `…::a_tie_names_the_first_move_alphabetically`,
  `…::one_analysis_per_rack_settles_at_the_first`.)*

### `U-ERR-*` — error mapping (`error.rs`)

Every handler returns `AppResult`, so this type decides what a caller sees.

- `U-RACK-10` The parser reads a distribution as MAGPIE does: a comment line,
  a whitespace-only line, four or six columns, a non-integer score, a vowel
  flag other than 0 or 1 and a letter with a space round it are each refused,
  a CRLF ending and a blank line are accepted, and a `#` row is a letter that
  takes its machine letter. It used to trim, skip `#` lines and want three
  columns, so job creation passed files every worker then refused, and a `#`
  letter shifted the numbering. *(Covered:
  `racks::tests::it_reads_a_distribution_as_magpie_does`,
  `racks::tests::a_minimal_two_letter_distribution_parses`.)* (Twenty-third
  audit; since the twenty-fourth, also refused as MAGPIE refuses them or
  cannot hold them: a CRLF blank line (a lone `\r`), a field that is only
  `\r`, a count above 255 (MAGPIE keeps it in a byte), and a letter longer
  than 4 bytes (MAGPIE's shipped maximum; some of its buffers hold no more),
  and, since the twenty-fifth, a fullwidth display form longer than 5 bytes.)
- `U-ERR-1` Each `AppError` constructor maps to its documented HTTP status:
  `bad_request` → 400, `unauthorized` → 401, `forbidden` → 403, `not_found` →
  404, `conflict` → 409, `rate_limited` → 429, `internal` → 500. *(Covered:
  `error::tests::each_constructor_maps_to_its_documented_status`.)*
- `U-ERR-2` The serialized body always carries `code` and `message`, and
  `with_field` errors appear under `fields`. *(Covered:
  `error::tests::the_body_carries_code_message_and_any_field_errors`.)*
- `U-ERR-3` `rate_limited(n)` sets `Retry-After: n`, and never below 1.
  *(Covered:
  `error::tests::a_rate_limit_sets_retry_after_and_never_below_one_second`;
  `rate_limited(0)` sent `Retry-After: 0`.)*
- `U-ERR-5` **A body that does not parse is an API error too**
  (`extract::ApiJson`): no body, malformed JSON, the wrong shape, a missing
  field and a missing content type are each `400 bad_request` in the
  `{code, message}` shape; a body over the route's limit is `413
  payload_too_large` in the same shape; and a body above the blocking-pool
  threshold parses to the same value as one below it. *(Covered:
  `extract::tests::*`.)*
- `U-ERR-6` **Request bodies are read in two tiers** (`extract::read_body`): a
  large body (declaring over 1 MiB) is reserved whole before a byte is read
  and given back, owner and all, when dropped; past the budget, or past one
  owner's 64 MiB share, it is refused at once with `503` and `Retry-After`, never
  queued holding part of it; a small body never touches the budget, one
  declaring nothing is cut off at 1 MiB, and a large body declared to a route
  without the budget is `413` unread; a stalled large body is let go with its
  reservation. *(Covered: `extract::tests::a_large_body_is_reserved_whole_and_given_back`,
  `…::a_large_body_that_cannot_be_reserved_is_refused_at_once`,
  `…::a_small_body_never_waits_and_an_undeclared_one_is_bounded`,
  `…::a_stalled_large_body_is_let_go_with_its_reservation`.)* (Thirty-first
  audit: a caller with no credentials held a dozen 60 MiB uploads open and took
  the web task from 38 MB to 778 MB. The audit's first fix, one budget for every
  body charged as bytes arrived, let 192 identity-less claims make every
  heartbeat wait ten seconds for a `503`; its adversarial check found that, and
  these tests are the redesign's.)
- `U-ERR-4` A `sqlx::Error` converted into `AppError` becomes a 500 whose public
  message does **not** contain the SQL string or the database URL. A leaked
  query in an error body is the failure this test exists for. *(Covered:
  `error::tests::a_database_error_does_not_leak_the_query_or_the_url`,
  `error::tests::a_driver_error_is_not_shown_to_the_client`.)*
- `U-ERR-7` A constraint violation is a 409 that does not name the constraint,
  and a pool timeout is a 503 with `Retry-After`. *(Covered:
  `error::tests::constraint_violations_are_conflicts_without_the_constraint_text`,
  `error::tests::a_pool_timeout_is_a_503_with_retry_after`.)*
- `U-ERR-8` A NUL the database cannot store (SQLSTATE `22021`, and `22P05`
  for a `\u0000` read from JSON as text) is a `400 bad_request` that does not
  carry the database's words: every text the server binds that could hold one
  is the caller's. *(Covered:
  `error::tests::a_nul_the_database_cannot_store_is_a_bad_request`; through
  the API, `A-PUBLIC-8`.)* (Thirty-third audit, pass 3.)

### `U-AUTH-*` — credentials (`auth/api_key.rs`, `auth/session.rs`, `auth/csrf.rs`)

- `U-AUTH-1` An API key hashes deterministically: the same raw key gives the
  same hash across calls, and two generated keys differ. (Determinism is
  load-bearing — lookup is an exact hash match, not a per-row verify.)
  *(Covered: `api_key::tests::a_key_hashes_the_same_every_time_and_two_keys_differ`.)*
- `U-AUTH-2` A generated raw key is URL-safe and at least 32 bytes of entropy.
  *(Covered: `api_key::tests::a_generated_key_is_url_safe_with_32_bytes_of_entropy`.)*
- `U-AUTH-3` A password verifies against its own Argon2 hash and fails against
  another's; two hashes of the same password differ (per-password salt).
  *(Covered: `api_key::tests::a_password_verifies_against_its_own_salted_hash_only`.)*
- `U-AUTH-4` A session token issued by `session::issue` round-trips its subject,
  username and admin flag through `session::verify`, and fails verification
  after any byte is altered; garbage is not a session. *(Covered:
  `session::tests::a_session_round_trips_and_any_altered_byte_fails`,
  `session::tests::garbage_is_not_a_session`.)*
- `U-AUTH-5` A session signed with a different key fails verification.
  *(Covered: `session::tests::a_session_signed_with_another_key_fails`.)*
- `U-AUTH-6` A session past `session_ttl` is rejected. Drive it by issuing with
  a near-zero TTL rather than by waiting. *(Covered:
  `session::tests::an_expired_session_is_rejected`.)*
- `U-AUTH-7` `csrf::verify` passes GET/HEAD/OPTIONS with no token at all;
  requires both cookie and header on POST/PATCH/DELETE; rejects a mismatch, a
  missing cookie, and a missing header with distinct messages; and tokens are
  unpredictable. *(Covered: `csrf::tests::safe_methods_pass_without_any_token`,
  `csrf::tests::writes_need_a_matching_cookie_and_header`,
  `csrf::tests::tokens_are_unpredictable`.)*
- `U-AUTH-8` A confirmation code is stored only as its hash. *(Covered:
  `api_key::tests::a_confirmation_code_is_stored_only_as_its_hash`.)*
- `U-AUTH-9` Argon2 runs wait for one of four turns: with every turn taken, a
  hash does not start, and it runs once one is given back. Unbounded, a flood
  of sign-ins and registrations from seven addresses took the process to
  2.6 GB; bounded, and with the allocator's mmap threshold pinned, the same
  flood peaked at 129 MB and fell back to 52 MB (measured on a native backend;
  the memory half is not a unit test). *(Covered:
  `api_key::tests::argon2_runs_wait_for_a_turn`.)* (Thirty-second audit.)
- `U-AUTH-9b` A request that goes away does not leave its Argon2 run queued
  past the four turns: the turn goes with the run, and a run whose requester
  has gone is skipped. A thousand aborted requests leave a fresh hash under a
  second (it fails when the request holds the turn: the run took 8.4 s).
  *(Covered:
  `api_key::tests::abandoned_argon2_runs_do_not_queue_past_the_turns`.)*
  (Thirty-second audit.)

### `U-CFG-*` — configuration (`config.rs`)

Tested through `Config::from_lookup`, which reads from a closure instead of the
process environment, so no test mutates `std::env` under another.

- `U-CFG-1` Every variable with a default takes it when unset, and the value
  when set. Table-driven; the point is that a rename cannot silently fall back.
  *(Covered: `config::tests::every_default_applies_when_unset_and_yields_to_a_value`,
  `config::tests::database_url_wins_when_set`,
  `config::tests::database_parts_are_assembled_with_the_password_encoded`.)*
- `U-CFG-2` A missing variable with no default is a startup error naming it, not
  a panic or an empty string. *(Covered:
  `config::tests::a_missing_required_setting_is_an_error_naming_it`,
  `config::tests::a_missing_part_names_itself`.)*
- `U-CFG-3` `MIN_MAGPIE_VERSION` parses through `Version`, so a malformed value
  fails at startup rather than silently becoming `0.0.0` and admitting every
  client. (This is the setting that made every local task decline.) *(Covered:
  `config::tests::a_malformed_version_floor_fails_startup`.)*
- `U-CFG-4` A value that is present but malformed — a duration, a count, a
  boolean, an unknown `MAIL_BACKEND`, `MAIL_BACKEND=file` with no
  `MAIL_OUTBOX_DIR`, `MAIL_BACKEND=ses` with no `MAIL_FROM` or `PUBLIC_URL`
  (whose defaults are a laptop's; pass 24, which also drops a `PUBLIC_URL`'s
  trailing slash, that made links `//confirm-email`), a heartbeat timeout outside 180 s to a day (below
  MAGPIE's cadence a live claim lapsed and was handed on, thirty-second audit),
  a session TTL outside a minute to a year, a `BIND_ADDR` that is not an
  address — fails startup naming the setting
  rather than becoming its default. *(Covered:
  `config::tests::a_malformed_value_is_refused_rather_than_defaulted`.)*
- `U-CFG-5` Every hand-kept copy of the default version floor equals
  `DEFAULT_MIN_MAGPIE_VERSION`: Terraform's `min_magpie_version` default (and
  its description), which `ecs.tf` and `derived.tf` pass through; both compose
  files; `scripts/e2e_magpie_native.sh`; `backend/.env.example`, which a
  backend run on the host starts from; the root `.env.example`, which a
  compose user copies to `.env` (its commented value and the version it says
  the contribute branch reports; thirty-third audit, pass 3); the same
  "`birdtest-contribute` reports" sentence in `backend/.env.example` and
  `docker-compose.yml` (thirty-third audit, pass 4); the `jobs`
  table's three column defaults; and `scripts/dev.py`'s note. A raise that missed Terraform passed
  CI and left production admitting the builds it was meant to keep out
  (thirty-third audit, pass 1). *(Covered:
  `config::tests::every_copy_of_the_version_floor_agrees`.)*

### `U-WIRE-*` — wire types (`jobs/handler.rs`, `models/job.rs`)

- `U-WIRE-1` `seed` serializes as a **decimal string** and round-trips a value
  above 2^53 without loss. A JSON number would lose it silently, so one is
  refused. *(Covered:
  `handler::tests::seeds_cross_the_wire_as_decimal_strings_without_loss`,
  `handler::tests::a_seed_sent_as_a_json_number_is_refused`.)*
- `U-WIRE-2` `TaskRequest` tags with `job_type` in snake_case, and each variant
  round-trips, the game-pairs one read from the captured `C-2` fixture rather
  than derived. *(Covered:
  `handler::tests::task_requests_are_tagged_in_snake_case_and_round_trip`.)*
- `U-WIRE-3` Every `#[serde(default)]` field on a response type is genuinely
  optional: a payload omitting all of them deserializes. *(Covered:
  `handler::tests::every_defaulted_response_field_is_optional`.)*
- `U-WIRE-4` An unknown field in a response is ignored rather than rejected, so
  a newer client can add one. Assert the current behaviour deliberately, either
  way, because it is a compatibility decision. *(Covered — ignored:
  `handler::tests::unknown_response_fields_are_ignored_so_newer_clients_stay_valid`.)*
- `U-WIRE-5` `GameAggregate::is_consistent` accepts a valid tally and rejects
  each way of breaking it (negatives, sum mismatch, and a sum that only agrees
  by overflowing); a pentanomial likewise. *(Covered:
  `handler::tests::a_game_tally_must_be_non_negative_and_sum_to_its_games`,
  `game_pair::tests::a_pentanomial_that_only_agrees_by_overflowing_is_rejected`;
  both sums used to overflow.)*
- `U-WIRE-6` `JobType` round-trips through its serde representation and its
  Postgres enum representation with the same strings. *(Covered:
  `job::tests::job_types_have_one_name_on_the_wire_and_in_postgres`,
  `job::tests::job_statuses_have_one_name_on_the_wire_and_in_postgres`.)*
- `U-WIRE-7` `TestParams::from` reads the same match-test settings (on or
  off, floor, cap, confidence) out of a `GameConfig` and a `GamePairConfig`,
  so the two job types cannot diverge.
  *(Covered: `job::tests::test_params_read_the_same_settings_from_games_and_pairs`.)*

### `U-DISPATCH-*` — the job template cache (`jobs/dispatch.rs`)

- `U-DISPATCH-1` Forgetting a job drops its template, and forgetting an unknown
  one is nothing. *(Covered: `dispatch::tests::a_forgotten_job_is_read_again`.)*
- `U-DISPATCH-2` A job whose template failed to load is passed over for a
  minute, without a connection or a log line, then tried again; forgetting the
  job clears it. Without it, a job that never issues a claim heads every
  candidate list, and every claim of every worker paid a pool connection, the
  read and parse, and an error line for it. *(Covered:
  `dispatch::tests::a_job_whose_template_failed_is_passed_over_for_a_while`.)*
  (Twenty-fourth audit.)

### `U-PLAUS-*` — plausibility gaps (`jobs/plausibility.rs`)

Thirteen rules were covered before these -- the thirteenth, that a leave batch
reports no more rack occurrences than its games could draw, by
`plausibility::tests::a_leave_batch_cannot_report_more_occurrences_than_its_games_drew`.
The two that were missing:

- `U-PLAUS-1` The batch-size rule doubles the dispatched count for
  `game_pairs` and does not for `games` — the pairs-versus-games unit confusion
  that the `min_pairs`/`max_pairs` naming exists to keep straight. The rule is
  `plausibility::check_batch_size` against `games_dispatched`, which
  `registry::decode_result` runs for both job types; there is no
  `check_against_task`, as this entry first named it. *(Covered:
  `plausibility::tests::a_pairs_batch_dispatches_two_games_a_pair_and_a_games_batch_does_not`.)*
- `U-PLAUS-2` A batch reporting one game more, and one fewer, than dispatched is
  rejected; the exact count passes. *(Covered:
  `plausibility::tests::a_batch_one_game_off_in_either_direction_is_rejected`.)*
- `U-PLAUS-3` A rack is counted in tiles, a bracketed multi-character letter
  (`[L·L]`, `[NY]`, `[QU]`) being one: seven Catalan tiles pass and eight do
  not, and an unclosed or empty bracket is malformed. Counted in characters,
  every captured position of a Catalan games job was refused. *(Covered:
  `plausibility::tests::racks_are_bounded_by_what_a_rack_holds`.)* (Eleventh
  audit.)
- `U-PLAUS-5` A position's moves match how it was analysed: a static or
  simulated position's carry no solver spread or depth, a pre-endgame's each
  carry a win percentage, a finite spread within bounds and a depth from 0 to
  40, and an endgame position reports exactly the one move its solve chose,
  with a spread, a depth from 0 to 25 and no simulation statistics. *(Covered:
  `plausibility::tests::a_positions_moves_match_its_analysis`.)*
- `U-PLAUS-4` A negative `num_moves` is refused (cast to `usize` it was larger
  than any list), and per-ply statistics must be numbered from 0 in order, with
  a bingo percentage in [0, 100] and a finite, non-negative average score.
  *(Covered:
  `plausibility::tests::a_worker_cannot_report_more_moves_than_it_generated`,
  `plausibility::tests::per_ply_statistics_are_statistics`.)* (Eleventh audit.)
- `U-PLAUS-6` A position's analysis is one its task's players could have run:
  a simulation or an inference only if a player simulates (and infers), a
  pre-endgame solve only if a player has `endgame_plies` and `peg_max_bag`, an
  endgame only if one has `endgame_plies`, and no deeper than the deepest.
  Checked against either player (a position does not say which seat moved),
  and only that way round: a simmer's turn can be static. An opening rack from
  a static player cannot be a simulation. `registry::decode_result` runs it for
  opening racks, games and pairs. *(Covered:
  `plausibility::tests::an_analysis_is_one_the_tasks_players_could_run`.)*
  (Thirty-third audit, pass 1: nothing compared an analysis with the config
  that produced it, and the fake worker sent simulations for static players.)
- `U-PLAUS-7` Only a simulation's moves carry iterations or per-ply
  statistics: a static, pre-endgame or endgame move with either is refused,
  since MAGPIE writes `iterations: 0` and no plies for a move it did not
  simulate. *(Covered:
  `plausibility::tests::an_unsimulated_move_carries_no_iterations_or_plies`.)*
  (Thirty-third audit, pass 1.)
- `U-PLAUS-8` A pairs result's `divergent_games` agrees with the whole: a pair
  that did not diverge is one game from both seats (a win and a loss for
  player 1, or two draws, in bucket 2), so outside the subset wins equal
  losses and draws are even, the subset has no count the whole lacks, and no
  more pairs sit outside bucket 2 than diverged. *(Covered:
  `game_pair::tests::the_divergent_subset_agrees_with_the_whole`.)*
  (Thirty-third audit, pass 1.)
- `U-PLAUS-9` A pre-endgame is ranked no deeper than its schedule reaches:
  stage `s` of a `peg_stage_top_k` at `s + 1` plies, and MAGPIE's exhaustive
  mode -- the one stage `[2147483647]`, which job creation accepts -- at
  `PEG_EXHAUSTIVE_PLIES` (40), past the endgame's 25. Checked against either
  player; only `i32::MAX` itself is exhaustive. *(Covered:
  `plausibility::tests::a_pre_endgame_is_ranked_no_deeper_than_its_schedule_reaches`.)*
  (Thirty-third audit, pass 2: one 25-ply bound for both solvers refused every
  captured position of an exhaustive player, and the job wedged.)

### `U-STATS-*` — pinned numbers (`stats/match_test.rs`, `stats/outcomes.rs`, `stats/bradley_terry.rs`)

Every other statistics test checks a property (monotone, symmetric, order-free),
which a formula wrong by a constant factor passes. These compare against values
computed outside the code, in 40-digit decimal from PLAN.md's formulas.

- `U-STATS-1` The match test's half-width is the documented boundary: n =
  1,000, σ² = 1/16, α = 0.05, tuned at 1,000 units of variance ¼, is
  0.025806505…, and nothing played is an infinite half-width. A games tally,
  W21 L7 D2 (mean 11/15, variance 161/900), at 95% with a floor of 10 and a
  cap of 1,000 gets the interval 0.476214864… to 0.990451801…, still running;
  a wider one is clipped to 1. *(Covered:
  `match_test::tests::the_half_width_is_the_documented_boundary`,
  `match_test::tests::a_games_tally_gets_the_documented_interval`,
  `outcomes::tests::a_games_tally_has_the_documented_mean_and_variance`.)*
- `U-STATS-2` A pentanomial [1, 3, 7, 3, 2] is 16 pairs scored i/4 (mean
  17/32, variance 71/1024); at 90%, floor and cap 16, its half-width is
  0.181952165…, and it is inconclusive at its cap. *(Covered:
  `match_test::tests::a_pentanomial_gets_the_documented_interval`,
  `outcomes::tests::a_pentanomial_has_the_documented_mean_and_variance`.)*
- `U-STATS-3` Over 1,000 games without a draw at 95% (floor 100, cap 10,000),
  548 wins is the fewest that decide for player 1 (lower bound 0.500229941…)
  and 547 runs on (0.499220820…); the mirror images decide for player 2 and
  run. Every pair split is no evidence either way: 10,000 of them end
  inconclusive, the interval still around ½. The floor holds the decision
  back while the interval is reported, and a cap below the floor ends the job
  inconclusive. *(Covered:
  `match_test::tests::the_fewest_wins_that_decide_and_one_short`,
  `match_test::tests::identical_pairs_alone_never_decide`,
  `match_test::tests::the_floor_holds_until_it_does_not_and_the_cap_applies_below_it`,
  `match_test::tests::nothing_played_is_an_even_score_with_every_score_possible`.)*
  The test carries no Elo or rating-point figure: the `elo` fields its result
  once had are gone, so nothing beside it reads as a pool's rating.
- `U-STATS-3b` **The error rate and the power, simulated.** Pairs drawn from a
  pentanomial, the test checked after every batch of 50 from a floor of 500 to
  a cap of 10,000 at 95%, as the finish check does: between equal players
  (three pairs in five split) at most α + 0.02 of 1,000 runs name a winner;
  a player scoring 53.5% per game is found better in at least
  90% of 200 runs, and never the wrong way round. *(Covered:
  `match_test::tests::equal_players_rarely_get_a_winner_however_often_it_is_checked`,
  `match_test::tests::a_better_player_is_found`.)*
- `U-STATS-4` The ratings are on WESPA's scale, 250 points per logit: a 75%
  score fits to 250·ln 3 ≈ 274.65 points, over 100,000 games as over 100
  pairs, and the residuals predict with the same constant (100 points is
  59.9%). A Bradley-Terry standard error equals the analytic
  250/√(n·p·(1−p)): 70.7106… points for an even 50 games, a tenth of that at
  5,000, and 1.8257… for 75% over 100,000. *(Covered:
  `bradley_terry::tests::a_75_percent_score_is_250_ln_3_points`,
  `the_residuals_predict_on_the_same_scale`,
  `a_well_played_head_to_head_is_its_maximum_likelihood`,
  `more_games_narrow_the_standard_error`.)*
- `U-STATS-5` The fit returns noiseless evidence's own ratings, within a few
  points, for the shapes that defeated the old one-config-at-a-time solver and
  its prior toward the anchor (KL-74): a 12-member group joined to the anchor
  by one 300-pair job (every member's error at least the link's ≈34 points), a
  30-member group, a disconnected island (unrated, its internal gap intact)
  and a hundred-member pool (under a second in a debug build); a 20-config
  chain and a 12-rung ladder within 0.6 of each config's error (KL-79). And for
  the shapes the audit's adversarial checks found: a config over a gauntlet of
  twenty lightly played opponents within half its error; conceding a quarter
  or half point lowers the conceding config's rating in the cases pinned (the
  rare exception, under a point and a half, is KL-79's); twenty baselines
  swept by both the anchor and a config 400 above it leave that config within
  one error; a
  strong tier 1,000 above joined by one job within about half its error; two
  tiers of lightly played configs 600 or 800 points apart, joined by one small
  job, the upper within 1.5 of its shown error, which includes the prior's
  pull (1.7 and more without it); a
  newcomer's sweep shrunk the same in a young pool as a mature one; a field
  that swept the anchor held in place by its virtual games, not floated by
  unrelated young configs; clean
  sweeps contradicting the rest of a pool converge with finite errors; a
  newcomer's clean sweep rates higher the more pairs it swept; a well-played
  head-to-head is its maximum likelihood; and a fit with million-pair
  head-to-heads says it converged. *(Covered:
  `bradley_terry::tests::a_thinly_linked_cluster_is_fitted_where_it_is_with_the_links_error`,
  `a_long_chain_reaches_its_top`, `a_large_cluster_is_not_pulled_toward_the_anchor`,
  `an_island_does_not_stop_the_fit_converging`,
  `a_clean_sweep_of_a_group_stays_finite`, `a_hundred_member_pool_fits_quickly`,
  `contradicting_clean_sweeps_do_not_throw_the_fit_into_saturation`,
  `a_newcomers_clean_sweep_rates_higher_the_more_it_swept`,
  `a_well_played_head_to_head_is_its_maximum_likelihood`,
  `a_gauntlet_of_lightly_played_opponents_does_not_hold_a_config_back`,
  `a_fit_at_the_answer_with_huge_head_to_heads_says_it_converged`,
  `a_clean_sweep_rates_at_least_a_near_sweep`,
  `shared_swept_baselines_do_not_pull_a_config_toward_the_anchor`,
  `a_ladder_is_not_compressed_past_its_errors`,
  `a_strong_tier_joined_thinly_is_not_pulled_to_the_centre`,
  `a_newcomer_is_shrunk_the_same_in_a_young_pool_as_a_mature_one`,
  `a_field_that_swept_the_anchor_does_not_float_on_young_configs`,
  `the_error_shown_covers_what_the_prior_pulls_a_thin_tier`.)*
  (Thirty-second audit.)

### `U-FAKE-*` — fake worker shapes (`worker/fake_worker.py`)

The fake worker is the only client below tier 6, so shape drift is invisible
until a job of that type is run — which is how its opening-rack submission came
to answer with a field the server had never accepted.

- `U-FAKE-1` A captured `games` submission deserializes into
  `GameResultsResponse` and passes `process_response`, and each position is
  shaped by its mover's config as MAGPIE's are: player 1 (a simmer, inferring)
  simulated with inferences, player 2 (static) static -- which the server's
  `check_analyses_against_players` holds it to (`U-PLAUS-6`). *(Covered:
  `plausibility::tests::the_fake_workers_games_submission_passes_validation`;
  that its positions are boards the site can draw, each following from the
  last, is `F-CGP-4`.)* (Thirty-third audit, pass 1: the fake simulated every
  position whatever its player, so a static job's page showed simulation
  columns no real job of it could.)
- `U-FAKE-2` A captured `game_pairs` submission does the same, **including** the
  pentanomial cross-checks. *(Covered:
  `plausibility::tests::the_fake_workers_game_pairs_submission_passes_the_pentanomial_cross_checks`.)*
- `U-FAKE-2b` A `game_pairs` submission keeping first divergences, answering
  the captured pairs assignment (two static players), passes the pentanomial
  cross-checks, the divergent subset against the whole (`U-PLAUS-8`),
  `check_first_divergences`, and holds static positions only. *(Covered:
  `plausibility::tests::the_fake_workers_first_divergences_pass_validation`.)*
  (Thirty-third audit, pass 1: the fake's first divergences were checked only
  through `E-17`.)
- `U-FAKE-3` A captured `opening_rack` submission deserializes into
  `PositionAnalysisResponse`, passes `process_response`, and is a simulation,
  as its simming player's analyses are. *(Covered:
  `plausibility::tests::the_fake_worker_speaks_the_opening_rack_response_shape`.)*
- `U-FAKE-4` A captured `leave_generation` submission deserializes into
  `LeaveResponse` and passes `check_rack_occurrences`. *(Covered:
  `plausibility::tests::the_fake_workers_leave_submission_passes_the_occurrence_rules`.)*
- `U-FAKE-5` Each `--mode` (`malformed`, `stale`, `abandon`) produces something
  the server's validation **rejects**, so the adversarial modes cannot silently
  become valid. *(Covered:
  `plausibility::tests::every_malformed_submission_is_rejected_for_the_rule_it_breaks`,
  `plausibility::tests::a_stale_submission_differs_from_a_valid_one_only_in_its_token`,
  `plausibility::tests::an_abandoning_worker_submits_nothing`; and, since a
  stale submission differs from a valid one only in its token, which only the
  claim lookup can refuse, through the router by
  `fake_worker::a_stale_mode_submission_is_not_accepted_and_changes_nothing`.)*

- `U-FAKE-6` **The fixtures are what the fake sends now.** CI re-emits every
  one and fails on a difference (`scripts/fake-worker-fixtures.sh --check`, the
  `scripts` job). *(Thirty-third audit, pass 1: a change to the fake had left
  two fixtures a shape it no longer sent, so `U-FAKE-1` and `-3` tested
  neither.)*

Fixtures live in `backend/src/jobs/testdata/fake_worker_*.json`, one per job
type and mode, and are regenerated by `scripts/fake-worker-fixtures.sh` (see
`backend/src/jobs/testdata/README.md`), not hand-edited.

### `U-ARCHIVE-*` — tarball walk limits (`inputdata.rs`)

The import walks an archive fetched from the network, so PLAN.md's limits are
enforced during the walk, and each is a claim about what a hostile archive
cannot do. (Before these: a well-formed walk, a symlink entry, a traversing
path, nothing recognisable, and a bomb by compression ratio.)

- `U-ARCHIVE-1` An entry whose header claims more than 128 MiB is refused from
  the header, before a byte of it is read. *(Covered:
  `inputdata::tests::an_entry_whose_header_claims_more_than_the_per_entry_cap_is_refused_unread`.)*
- `U-ARCHIVE-2` The 1 GiB total counts every entry, pinned or not, and holds
  where the ratio check does not catch it. *(Covered:
  `inputdata::tests::an_archive_expanding_past_the_total_cap_is_refused`.)*
- `U-ARCHIVE-3` The 5,000-entry cap counts entries — directories, symlinks and
  ignored paths included — not pinned files, which is all it used to count.
  *(Covered:
  `inputdata::tests::an_archive_of_more_than_the_entry_cap_is_refused_whatever_the_entries_are`.)*
- `U-ARCHIVE-4` Hard links, character and block devices and FIFOs are refused,
  not skipped, even at a path birdtest ignores. *(Covered:
  `inputdata::tests::hard_link_and_device_entries_are_refused`.)*
- `U-ARCHIVE-5` A symlink that aliases a file inside the archive is pinned with
  its target's bytes — what `download_data.sh` leaves a worker reading through
  it — and one that leaves the archive, dangles, loops, or names a file of
  another kind is refused. The current `data-20260925.tgz` carries such aliases (23 of them),
  and refusing them refused the whole release. *(Covered:
  `inputdata::tests::a_symlink_alias_is_pinned_with_its_targets_bytes`,
  `inputdata::tests::a_symlink_that_is_not_an_alias_inside_the_archive_is_refused`.)*
- `U-ARCHIVE-5b` An alias of a letter distribution or layout counts as its
  target's bytes against the archive's total and ratio (a lexicon's is a link
  on the worker and no second copy on the server, so costs nothing), and a letter distribution or layout — kept in its row — is
  at most 64 KiB. As a zero-byte entry, a few hundred aliases of one large
  layout were held, staged and inserted a few hundred times over (150 of a
  4 MiB one: 600 MiB from a 263 KiB gzip). *(Covered:
  `inputdata::tests::aliases_count_as_their_targets_bytes_and_kept_files_are_small`.)*
  (Thirty-second audit.)
- `U-ARCHIVE-6` The limits are PLAN.md's table; changing one is a design change.
  *(Covered: `inputdata::tests::the_walk_limits_are_the_ones_the_design_states`.)*
- `U-ARCHIVE-7` A file whose name MAGPIE would refuse as a path — a `.`, a
  space, an empty name — is not importable, so no job can be pinned to it and
  stop every worker it reaches. *(Covered:
  `inputdata::tests::ignores_what_birdtest_does_not_pin`.)* (Twelfth audit.)
- `U-ARCHIVE-8` A PAX `size` record is refused, pinned entry or not, and so
  cannot make the walk and the reader disagree on where the next header
  starts. *(Covered: `inputdata::tests::a_pax_size_record_is_refused`,
  `inputdata::tests::a_pax_size_record_cannot_move_the_next_header`.)*
  (Thirty-first audit: first a header saying 0 and a record saying 900 MiB
  passed every cap and was read whole, 927 MiB resident from a 4 MiB gzip; then,
  with the size honoured, a record ahead of an ordinary entry slipped an 8 MiB
  PAX header past a first pass that read raw. The walk is raw since, and reads
  extension headers itself.)
- `U-ARCHIVE-9` An archive naming one pinned path twice — two files, or a file
  and an alias — is refused. *(Covered:
  `inputdata::tests::an_archive_naming_a_path_twice_is_refused`.)* (Thirty-first
  audit: both were staged and confirmed as rows, of which a worker extracting
  the tarball holds only the last.)
- `U-ARCHIVE-10` An extension header — a PAX record or a GNU long name, which
  the walk reads itself (an interpreting reader read it whole, before the walk
  saw an entry) — larger than 64 KiB is refused before it is read, and every decompressed byte, headers and
  skipped data included, counts against the 1 GiB cap (and the ratio, past a
  64 MiB floor for archives of many tiny entries). *(Covered:
  `inputdata::tests::a_large_extension_header_is_refused_before_it_is_read`,
  `inputdata::tests::skipped_data_counts_against_the_caps`.)* (Thirty-first
  audit, second pass: a 400 KB gzip with a 400 MB PAX header held 465 MB and
  was accepted.)
- `U-ARCHIVE-13` PAX records are split by their stated lengths: a value with a
  newline (an extended attribute, as GNU tar and macOS write) walks, and a
  keyword after an extra blank is refused rather than read as another.
  *(Covered: `inputdata::tests::pax_records_are_split_by_their_lengths`.)*
  (Thirty-first audit, pass 4: the tar crate's parser split on newlines and
  refused such a release, and read `"  size"` as a keyword other than `size`.)
- `U-ARCHIVE-14` Headers GNU tar reads otherwise than the tar crate are
  refused: a base-64 size, a base-256 size other than a positive eight-byte
  one, a directory or link with data, a file whose name ends in `/` (GNU tar
  makes it a directory and reads on into its data), a ustar header of another
  version, a NUL in a PAX path, a second PAX header before one entry.
  *(Covered: `inputdata::tests::headers_extractors_read_differently_are_refused`,
  and `an_entry_given_two_names_is_refused` for the second header.)*
- `U-ARCHIVE-15` A tarball gzipped in several members walks every member, as
  `tar -xzf` does; and a path spelled `./data/...` or `data//...` is the same
  file as `data/...` for the one-name rule. *(Covered:
  `inputdata::tests::every_gzip_member_is_walked`,
  `inputdata::tests::a_path_named_twice_in_two_spellings_is_refused`.)*
- `U-ARCHIVE-11` A GNU sparse entry is refused as it is met, its extension
  blocks never expanded. *(Covered:
  `inputdata::tests::a_sparse_entry_is_refused_unexpanded`, which also bounds
  what the walk holds.)* (Thirty-first audit, third pass: an interpreting reader
  built 64 bytes for every 24 of them, 1.1 GB from a 44 MB gzip.)
- `U-ARCHIVE-12` What another tool writes still walks: a PAX `path` (with an
  `mtime`, as Python's `tarfile` writes) and a GNU long name each name the entry
  after them — once: a second name for the same entry (another PAX header, a
  long name beside a PAX path) and a global PAX `path` are refused, as GNU tar
  would extract under a name the walk did not choose, while a comment-only
  global header walks. *(Covered:
  `inputdata::tests::pax_paths_and_long_names_name_the_entry_after_them`,
  `inputdata::tests::an_entry_given_two_names_is_refused`.)* Also
  checked by hand in the thirty-first audit: 51 release files (133 MB) packed by
  GNU tar and, with PAX headers on every member, by Python's `tarfile`, both
  walked to the same 37 pinned files.

### `U-ART-*` — download links (`artifacts.rs`)

- `U-ART-1` A presigned download link says no longer than the credentials
  that sign it last, less a minute: signed with credentials 20 minutes from
  expiry, an hour's link says 19 minutes, and carries their session token;
  with static keys (no expiry) it says the whole hour; and the arithmetic
  never asks for a zero or negative expiry. On ECS the task role's temporary
  credentials signed it, and the SDK's cache kept them until moments before
  they expired, so a link minted late in their life stopped working with
  them, minutes or seconds after it was handed out, as S3's `ExpiredToken`;
  links are now signed with credentials asked for at the time. *(Covered:
  `artifacts::tests::a_link_lasts_no_longer_than_its_credentials`,
  `artifacts::tests::presign_ttl_is_the_shorter_less_a_minute_and_never_zero`,
  offline. That ECS hands out credentials with an hour left is not testable
  here.)* (Thirty-third audit, pass 1.)

---

## 1F. Frontend unit

Pure TypeScript, no browser, no network. Vitest, run by `npm test` (`vitest
run`) in `frontend/`, and by CI after `npm run check`. `fetch` and
`EventSource` are stubbed per test; nothing renders a component. Instead the
chart arithmetic was moved out of the `.svelte` files into plain modules under
`lib/charts/` (`ratingDotPlot.ts`, `labels.ts`, `residuals.ts`,
`pentanomial.ts`), which the components import and the tests call directly —
so `@testing-library/svelte`, once planned here, was never needed.

This tier exists because the charts contain real arithmetic — a scale, a bucket
index, a threshold — and getting those wrong produces a plausible-looking
picture rather than an error. Writing it found three: `duration` chose its unit
before rounding (59.6 s read "60s"), `api.ts` rejected a non-JSON body with a
`SyntaxError`, and `RatingHistoryChart` (since removed) coloured by rank, so two lines swapped
colours whenever their ratings crossed.

### `F-FMT-*` — `lib/format.ts`

Each entry's tests are the `describe` block named for its id.

- `F-FMT-1` `workerLabel` renders a username when present, "Anonymous" plus
  the whole sixteen-character pseudonym when not (what `?worker=` takes;
  eight characters were shared by several contributors at a few hundred
  thousand), and never leaks a full UUID passed by mistake. *(Covered:
  `format.test.ts`.)*
- `F-FMT-2` `duration` renders seconds, minutes, hours and days at the right
  boundaries, and `null` as a dash rather than "null". *(Covered:
  `format.test.ts`; the unit is now chosen after rounding.)*
- `F-FMT-3` `datetime` renders `null` as a dash and a valid ISO string as a
  local time; an unparseable string does not throw. *(Covered:
  `format.test.ts`.)*
- `F-FMT-4` `jobTypeLabel` covers all four job types, in Title Case ("Opening
  Rack Analysis", "Games", "Game Pairs", "Leave Generation"), and an unknown type falls
  back to the raw string rather than "undefined" — including a name like
  `toString` that an object literal answers. *(Covered: `format.test.ts`.)*
- `F-FMT-5b` `testState`: a games or pairs job's match test reads `paused`
  while the job is inactive and `undecided` when it was completed without a
  decision, never `running` with nothing being played; a completed job shows
  the decision it stopped on (`player1_better`, `player2_better`,
  `inconclusive`); a job without a test is `off` whatever its status.
  *(Covered: `format.test.ts`, `F-FMT-5b testState`.)*
- `F-FMT-5` `testLabel` covers every status ("decided: player 1 is better",
  "inconclusive: the job reached its cap first", …). *(Covered:
  `format.test.ts`.)*
- `F-FMT-5c` `scorePct` shows a score per game as a percentage to a tenth
  (0.53125 is "53.1%", 0.5 "50.0%"). The `signedElo` this entry once also
  named no longer exists: no page shows an Elo difference. *(Covered:
  `format.test.ts`, `F-FMT-5c scores`.)*
- `F-MATCH-1` The match score table's rows, a column per player: wins,
  losses (player 2's wins and losses are player 1's losses and wins, and
  fewer losses are better), draws, the average scores to a decimal place and
  the spread signed from each player's side ("+13.4" / "-13.4", never
  "-0.0"); before any game, no average rather than 0 or NaN. The same rows
  serve a pairs job's divergent-games table. *(Covered: `matchScore.test.ts`.)*
- `F-CMP-1` A two-player table marks the better value of a row better (green)
  and the other worse (red) -- the higher, or the lower where the row says
  fewer is better (losses) -- and neither when they are equal or one is
  missing. *(Covered: `compare.test.ts`.)*
- `F-SET-1` The settings tables (`lib/jobSettings.ts`), in SETTINGS_COMPARISON.md's
  orders. A job's settings are one ordered list, every row shown (type,
  variant, letter distribution, board, bingo bonus; a games job's target or
  maximum, its Significance Test as one row, "no" or "yes (95%)", and its
  Position Recorder as yes, no or "yes (first divergences)"; an
  opening-rack job's minimum and maximum analyses per rack and its consensus
  share, "—" at a maximum of 1; a leave job's generations and each one's
  target; then sim cutoff, a games or pairs job's Threading, "Intra-game parallelism (all threads on one game)"
  or "Per-game parallelism (one game per thread)", the test's minimum, batch
  sizes, the oldest MAGPIE); a job without a test shows none of the test's settings but its
  "no", and a leave job has no sim cutoff and no lexicon or wordmap row of
  its own. Every label is in Title Case, and every job type capitalised. A
  player's settings are one ordered list too, key rows first (Lexicon,
  Leaves, Sorted By, Move Recorder, Moves Generated, Plies, Uses Inference,
  Uses Preendgame, Uses Endgame), shown for every player whether or not it
  simulates or solves, and every key row also a row of the full table; Uses
  Inference is "—" for a static player; the endgame and pre-endgame read
  "yes (6 plies)", "yes (bag ≤ 2)" or "no"; Plies Recorded is "—" for a
  static player, and Movegen Margin "—" unless the recorder keeps moves
  within an equity margin; players side by side, a row they differ in
  marked; the simulation rows of the full table only when one of them
  simulates. Rows
  are keyed on a stable id, not their label, so the wording can change
  without the rules that mark rows. Marked unused: what a leave job never reads
  (leaves, win% model, moves generated, recorder, plays and plies recorded,
  move-gen margin, the endgame and pre-endgame rows); an opening-rack job's inference and
  inference margin and endgame and pre-endgame rows, and its recorder and
  move-gen margin too when its player simulates (only a static opening-rack
  analysis reads them); a games or pairs job's recorder and move-gen margin
  (autoplay generates with MAGPIE's own record type and a margin of 0), and
  its plays and plies recorded too when it records no positions, and then
  its moves generated as well when no player simulates (a static player's
  list only holds the best move it plays; a capture or a simmer's candidates
  read it); a row marked
  unused is never marked as a difference, since the job cannot tell its players
  apart by it. A player's search in a few words leaves out what the job never reads
  (an opening-rack player's inference, the solving where the job never
  reaches the end of a game). *(Covered: `jobSettings.test.ts`.)*
- `F-SET-2` The player settings table lists, by its mode
  (`differingSettings`, `settingsFor`, `PlayerSettingsTable.svelte`), the
  settings two players differ in -- key or not -- as ordinary rows, with no
  heading, tint or fold; the key rows; or every row. Two players alike, asked
  for their differences, say "These players' settings are identical." rather
  than show an empty table; one player lists its key rows with no colour; each
  of two players is headed by its colour. *(Covered: `jobSettings.test.ts`,
  and `PlayerSettingsTable.test.ts`, which renders the component to HTML.)*
- `F-FMT-12` `completionText` says how a job finished: a match test that
  decided, with the confidence, the units and player 1's interval ("its
  significance test found player 1 better at 95% confidence after 1,200
  pairs: player 1 scored 50.1% to 55.2% per game"); a cap reached before it decided, with the
  interval; an admin's force-complete, "before its test decided" only when
  there was a test; a job without a test playing what it was set to; and the
  other job types' own ends. *(Covered: `format.test.ts`.)*
- `F-FMT-13` `jobTitle` titles a job by the name it was given, or by its
  type ("Game Pairs") for one given none or only spaces: the job page's
  heading and the job lists. *(Covered: `format.test.ts`.)*
- `F-FMT-6` A blank optional number is `null`, never 0 (Svelte binds a cleared
  number box as `null`, and `Number(null)` is 0, which the player-config form
  wrote into configs that cannot be edited), and a request's blank required
  fields are named before it is sent (the job form); so is a letter
  distribution or board left on its empty "Choose…" (the job and rating-pool
  forms, which no longer pick the first imported for the admin). A blank list
  field (the pre-endgame's schedule) is `null` too, and a list with a part that
  is not a whole number is named rather than sent. *(Covered:
  `format.test.ts`.)* (Twenty-second audit.)
- `F-FMT-14` `exportSummary`: a snapshot export is labelled "Snapshot as of
  <time> — job still running" (and, once the job has completed, as read before
  it completed — not its final results); a completed opening-rack job's
  snapshot, which may be a final export its consensus edit demoted, is
  labelled as read before the job completed or before its consensus settings
  last changed, never as taken while it ran; a completed job whose newest
  export is a snapshot, or a failed build, is offered **Build the final
  export**; a running job's button exports a snapshot; a leave job's snapshot
  notes it is as of the last merge.
  *(Covered: `format.test.ts`.)*
- `F-FMT-15` `parseTargetRackCounts` reads the job form's "100, 200, 500" (or
  "100 200 500") as one target per generation, a trailing comma forgiven, and
  names what is wrong with anything else — nothing listed, an empty entry, a
  non-integer, a target outside 1–1,000,000, more than 100 generations, a
  thousands separator ("1,000" reads as 1 and 000) — rather than sending it.
  *(Covered: `format.test.ts`.)*
- `F-FMT-17` `leavePlayerConflict` names what job creation refuses of a leave
  job's player -- simulating, sorting on anything but equity, a rack info
  table, solving endgames -- before the submit, and accepts a static equity
  player. *(Covered: `format.test.ts`.)*
- `F-FMT-16` `computeTime` reads a contributor's compute time to the second
  in every unit that is not zero ("2m 13s", "5h 20m 13s", "3d 4h 5m 6s", "2y
  17d 1h", "0s"), and a dash for anything that is not a time. *(Covered:
  `format.test.ts`.)*
- `F-FMT-18` `targetsText` lists a leave job's targets in generation order
  joined by arrows ("100 → 200 → 1,000"), since a comma list cannot be read
  when the numbers carry thousands separators. *(Covered: `format.test.ts`.)*
- `F-FMT-19` `exactCount` reads a contributor's movegens to the last digit,
  grouped ("1,234,567,890"), and a dash for anything that is not a count.
  *(Covered: `format.test.ts`.)*
- `F-FMT-20` `timeLimitNotice` says how many of a job's tasks hit the job's
  own time limit, names the limit and the cures ("3 tasks hit this job's 30m
  time limit — lower the batch size, or raise the limit"), "1 task" for one,
  and nothing while none has. *(Covered:
  `format.test.ts`.)*
- `F-FMT-21` `throughputText` reads a job's pace an hour in its unit
  ("1,240 games/hour", "8 pairs/hour"): whole units from ten up, a decimal
  below, "1 game/hour" singular, and a dash with no recent work. *(Covered:
  `format.test.ts`.)*

### `F-MOVEGENS-*` — `lib/movegens.ts`

- `F-MOVEGENS-1` `movegensLines` lists a breakdown's four job types in one
  order, labelled as the job list labels them, a type the answer leaves out
  at 0. *(Covered: `movegens.test.ts`.)*
- `F-MOVEGENS-2` `contributorKey` names an account by its id and an anonymous
  worker by its pseudonym, escaped for the path, and nobody for a row with
  neither. *(Covered: `movegens.test.ts`.)*

### `F-RR-*` — `lib/roundRobin.ts`

- `F-RR-1` `matchups` makes one self-play job of one config under the name as
  given, every pairing once for more -- seated in the order given (3 → 3,
  4 → 6) -- and names each "{name}: A vs B", or "A vs B" with no name, as the
  server does (`I-JOB-15`). *(Covered: `roundRobin.test.ts`.)*
- `F-RR-2` `matchupSummary` reads "4 configs → 6 jobs", "1 config → 1
  self-play job", and why none or more than twelve cannot be sent;
  `matchupsAllowed` is one to twelve. *(Covered: `roundRobin.test.ts`.)*

### `F-ALLOC-*` — `lib/allocation.ts`

- `F-ALLOC-1` `equalShares` ("Share equally") splits 100% in whole numbers,
  the remainder a point each to the first, among the jobs above 0% **and the
  jobs just created** -- a round robin at 0% beside a job at 100% gets its
  share, where it once changed nothing -- every other job at 0%, and every job
  when none is chosen. *(Covered: `allocation.test.ts`.)*

### `F-CONS-*` — `lib/consensus.ts`

- `F-CONS-1` `analysesPerRack` states an opening-rack job's consensus as the
  job page's explanation of it reads: "1" for one analysis per rack, otherwise
  the range and the share ("3 to 7, until 80% agree on the best move").
  *(Covered: `consensus.test.ts`.)*
- `F-CONS-2` `rackConsensus` counts each analysis's best move in a rack lookup
  and names the most common (the alphabetically first of a tie), with its
  count and share; nothing for no analyses. *(Covered: `consensus.test.ts`.)*
- `F-CONS-3` `consensusProblem` refuses what the server's
  `consensus_problems` refuses of the fields `consensusFields` sends, before a
  job is created with them (the job form) or they are changed on one (the
  admin page's Consensus card): a fewest outside 1–100 or not whole, a most
  below the fewest or above 100, whatever the most (a fewest of 3 and a most
  of 1 is refused, where the job form once sent it as one analysis per rack),
  and a share at or below 50%, above 100% or not a number. Past one analysis
  per rack it accepts any share above 50 (50.5 too: the share box has no
  `min`, which at 51 blocked it). At one analysis per rack the share is not
  sent, so not checked: the server checks its default 100% at creation, or
  the job's stored share (checked when it was set) on a change. *(Covered:
  `consensus.test.ts`.)*
- `F-CONS-4` `consensusFields` is what both forms send: the fewest and most
  as numbers, and the share only past one analysis per rack, so a share left
  in the disabled box at a most of 1 is never sent for the server to refuse.
  *(Covered: `consensus.test.ts`.)*

### `F-TEST-*` — `lib/matchTest.ts`

- `F-TEST-1` `testBar` scales the match test's bar to the interval and an
  even score, with a margin of a tenth of that span either side (at least a
  point), and keeps a wide interval within the scores that exist. *(Covered:
  `matchTest.test.ts`.)*
- `F-TEST-2` `testSentence` states player 1's score per game with its range,
  named for the players and with no Elo or rating figure ("… scores 53.1% per
  game (95% interval 51.2% to 55.0%)."), then names the better player once it
  is decided, or says neither is when it ended inconclusive.
  *(Covered: `matchTest.test.ts`.)*
- `F-TEST-3` `confidenceProblem` refuses what job creation refuses of a
  test's confidence, and only that: above 50 and below 100, so 50.05 and
  99.995 pass and 50, 100, a non-number and a cleared input do not. The job
  form checks it on submit and under the input, which has no `min` or `max`
  (`min="50.1" max="99.99"` blocked values the server takes; thirty-third
  audit, pass 3). *(Covered: `matchTest.test.ts`.)*

### `F-MOVES-*` — `lib/moveList.ts`

- `F-MOVES-1` A move list shows as many plies of statistics as its moves
  carry, up to two (none for a static player), headed P1-S, P1-BP, P2-S,
  P2-BP, P1 the reply; a ply is found by its number, not its place in the
  list. *(Covered: `moveList.test.ts`.)*
- `F-MOVES-2` An inference reads as a line -- how many leaves it found, from
  which previous move, and their mean equity -- and each leave's draws as a
  share of all it drew. *(Covered: `moveList.test.ts`.)*

### `F-CGP-*` — `lib/cgp.ts`

The saved-positions board is drawn from two strings MAGPIE writes, so these
read them as it writes them (`game_get_cgp_string`, `move_get_string`) and
refuse what it could not have written, rather than draw a wrong board.

- `F-CGP-1` `parseCgp` reads the board row by row (runs of empty squares as
  numbers, a blank in lower case, a bracketed multi-character letter as one
  tile, `Ç` as one), both racks in seat order (`?` a blank), the scores and
  the scoreless turns, ignoring options after them; a board that is not
  square, a rack or score missing, or an unclosed bracket is `null`.
  *(Covered: `cgp.test.ts`.)*
- `F-CGP-2` `parseMove` reads a play across as row then column (`8G HUH`) and
  one down as column then row (`E9 (E)RUVIM`), letters played through in
  parentheses or as `.`, blanks and bracketed letters, an exchange and a pass,
  and refuses anything else. *(Covered: `cgp.test.ts`.)*
- `F-CGP-3` The previous move's highlight is the squares it placed, never
  those it played through, and nothing for a pass, an exchange or text it
  cannot read; the player to move is the seat holding the position's rack, in
  any order. *(Covered: `cgp.test.ts`.)*
- `F-CGP-4` Every position MAGPIE wrote (`contract-fixtures/result-games.json`)
  and the fake worker wrote (`fake_worker_games_captured.json`) parses, its
  mover is found, and its previous move is exactly the difference from the
  turn before: its placed squares were empty and now hold the letters it
  names, every other square is unchanged, and the mover's score rose by its
  score. Each states the move played from it, which the next turn names as
  its previous move, with that score, and whose tiles go down on squares that
  are empty. *(Covered: `cgp.test.ts`.)*
- `F-CGP-5` `placedTiles` gives the tiles a play puts down, each on its square
  -- a blank as its letter, flagged -- skipping the squares it plays through,
  and nothing for a pass, an exchange or text it cannot read: what the board
  draws for the move played from a position. *(Covered: `cgp.test.ts`.)*

### `F-API-*` — `lib/api.ts`

- `F-API-1` A non-GET request sends `x-csrf-token` read from the cookie; a GET
  does not. *(Covered: `api.test.ts`.)*
- `F-API-2` A 204 resolves to `undefined` rather than throwing on an empty body.
  *(Covered: `api.test.ts`.)*
- `F-API-3` A 4xx with a JSON error body rejects with an `ApiError` carrying
  `status`, `code`, `message` and `fields`, and `Retry-After` in seconds when
  the server sent one (the reset page says how long to wait). *(Covered:
  `api.test.ts`.)*
- `F-API-4` A 4xx with an empty or non-JSON body still rejects with an
  `ApiError`, not a `SyntaxError` — as does a 200 whose body is not JSON —
  and its message is never empty, the status text being empty over HTTP/2
  (seventeenth audit).
  *(Covered: `api.test.ts`.)*
- `F-API-5` Every request sets `credentials: 'include'`. *(Covered:
  `api.test.ts`.)*
- `F-API-6` `errorText` lists the fields the server named after the message,
  and is the message alone when there are none; the job, player-config and
  input-data import forms show it. *(Covered: `api.test.ts`.)*
- `F-API-7` A query string leaves out a parameter that is `undefined`: a
  saved-positions search's first page sends no cursor, where it sent
  `cursor=undefined` and worked only because the server reads a cursor it
  cannot decode as "from the start"; job results and the audit log alike.
  *(Covered: `api.test.ts`.)*

### `F-SSE-*` — `lib/sse.ts`

- `F-SSE-1` `subscribeToJob` parses an event and calls back with the decoded
  payload. *(Covered: `sse.test.ts`.)*
- `F-SSE-2` A malformed event is skipped without tearing down the subscription.
  *(Covered: `sse.test.ts`.)*
- `F-SSE-3` The returned function closes the `EventSource`, and calling it twice
  is safe; a stream the browser gave up on is reopened after a pause, and an
  unsubscribe cancels a pending reopen; a job that answers a 4xx other than
  408 or 429 (deleted, or a bad id) is not subscribed to again (sixteenth and
  seventeenth audits). *(Covered: `sse.test.ts`.)*
- `F-SSE-4` A stream refused again and again is asked less often: the wait
  doubles from 5 s to a minute, jittered down by up to half, and an event
  resets it. At a fixed 5 s, every page the server's stream cap refused asked
  again, with a stats read first, every five seconds (twenty-first audit).
  *(Covered: `sse.test.ts`.)*

### `F-IMPORT-*` — `lib/importWatch.ts`

- `F-IMPORT-1` The import page's polling: it reads at once and polls while the
  import runs; it stops once staged (keeping the id) and forgets a failed one,
  or one with nothing new (shown, with nothing to confirm);
  a late `running` cannot undo a newer `staged`; an earlier import's late answer
  cannot touch a newer watch (it put that import on the page, stopped the new
  one's poll and forgot its id, and Insert then confirmed the wrong import); a
  503 or a network failure keeps it polling; a 404 forgets the import, a 401
  keeps it and reports the lapsed session; an older read's error after a newer
  success is ignored; stopping ignores answers in flight, and a watch after
  stop starts nothing (a start the admin left the page during began a poll
  nothing could clear). *(Covered:
  `importWatch.test.ts`.)* (Twenty-fourth audit: this logic was wrong three
  audits running.)

### `F-POLL-*` — `lib/poller.ts`

- `F-POLL-1` The derived data pages' polling: it reads at once, then fast while
  a build is queued or running and slower while idle, and never stops by
  itself, so a build queued later is seen (it stopped once idle, and the page
  never showed one); a failed read keeps it polling, and the caller can stop it
  (a 404: the job is gone); it pauses while the tab is hidden, reads at once
  when shown, and a read in flight at hiding reports but schedules nothing; a
  refresh (after a retry) overtakes a read in flight, whose late answer is
  dropped, leaving one chain of reads; stopping ignores answers in flight,
  stops listening, and cannot be undone. *(Covered: `poller.test.ts`.)*

### `F-CHART-*` — chart maths

Test the pure functions; do not snapshot the SVG.

- `F-CHART-1` `RatingDotPlot` places the anchor's dot at the anchor rating, and
  a config one standard error away at the expected offset. *(Covered:
  `charts/ratingDotPlot.test.ts`.)*
- `F-CHART-2` It clamps a runaway error bar rather than letting one
  barely-measured config flatten the scale, and still reports the true number in
  the table. The anchor has no bar at all, whatever error the fit stored for it
  (with no games, `f64::MAX`, which was drawn at the cap). An error the fit
  could not measure (`f64::MAX`) reads `±∞` in the table (`± ∞` in the
  tooltip), not `±1.8e308`. *(Covered: `charts/ratingDotPlot.test.ts`.)*
- `F-CHART-3` A config with `connected_to_anchor: false` is listed as unrated
  and **not** drawn at a position. *(Covered: `charts/ratingDotPlot.test.ts`.)*
- `F-CHART-4`, `F-CHART-5` Retired with the rating history chart they covered
  (its series cap and colour by config identity). The label shortening it
  shared with the dot plot is covered by `charts/labels.test.ts`.
- `F-CHART-6` The ratings page flags the non-transitive case only when at
  least three head-to-heads exceed the threshold on enough pairs to be at
  least three standard errors out — the same misses on ten pairs each do not
  raise it — and states a residual in signed percentage points in a cross
  table cell's hover. *(Covered: `charts/residuals.test.ts`. The separate
  residual table, `ResidualMatrix`, and its ordering and bars went with the
  cross table, which folds the residual into each cell.)*
- `F-CHART-7` The pair-outcome table reads each player's row from its own side
  — bucket 4 is player 1's "Won both" and player 2's is bucket 0 — and its
  three rows (won both, won one and drew one, even) hold all five buckets. An
  off-by-one here inverts the reading of every paired job. *(Covered:
  `charts/pentanomial.test.ts`.)*
- `F-CHART-8` Percentages are computed against pairs, not games, and a zero
  denominator renders 0.0% rather than `NaN`. *(Covered:
  `charts/pentanomial.test.ts`.)*

### `F-AUTH-*` — `lib/auth.ts`

- `F-AUTH-1` `refreshSession` sets the store to the user on 200 and to `null`
  on a 401 or 403 only, distinguishing "signed out" from "not yet known"
  (`undefined`). Any other failure (a 5xx, no network) leaves the store as it
  was: a signed-in user stays signed in, and an unresolved store stays
  `undefined` and is asked again after 2 s, doubling to 30 s and never sooner
  than the server's `Retry-After`, until the server answers. The entry once
  had every failure resolve to `null`, so that the layout guards waiting on
  `undefined` would not wait forever during an outage; that sent a signed-in
  admin to the login page on a deploy's 503, and the retry now resolves the
  wait instead. *(Covered: `auth.test.ts` — 200, 401, 403, a 503 then a
  network failure asked again on schedule until a 200, `Retry-After`
  honoured, a 502 keeping a signed-in user and asking nothing more, in
  flight, and a later 401 replacing a user.)*
- `F-AUTH-2` `signOut` clears the store when the logout succeeds, and stops a
  refresh waiting to ask again. When it fails (a `503`, a network failure) the
  server never removed the HttpOnly session cookie, so the store is not cleared:
  `signOut` asks `/api/me` again and rethrows, a live session stays signed in,
  and only a re-ask the server answers `401` shows signed out. (Until the
  thirty-third audit's pass 3 it cleared the store in a `finally`, telling a
  user on a shared machine they were signed out while the session lived; KL-59.)
  *(Covered: `auth.test.ts`.)*
- `F-AUTH-3` `resetSession` asks again from unresolved, for the two callers
  whose session has just changed: the login page after a sign-in, and the
  import watcher after a 401. Through `refreshSession` alone, a sign-in that
  met a deploy's 503 kept the `null` the login page was reached with, and the
  guard on the next page sent the now signed-in user back to sign in; an
  admin whose import poll met a 401 and then a 5xx kept the stale user, and
  nothing asked again. *(Covered: `auth.test.ts` — `null`, then a sign-in
  meeting a 503 goes unresolved and then to the user, never `null` again; a
  signed-in user reset into a 502 then a 401 ends `null` and stops asking; a
  reset starts a pending retry's wait over from 2 s.)*

### `F-ACCOUNT-*` — `lib/accountRules.ts`

The register, sign-in and password-reset forms are `novalidate` and check these
before a request, so every error is red text under its field; with the
browser's checks, some were a native popup and the browser took `a@b`, which the
server refuses. Each rule mirrors one in `routes/auth.rs`, with its wording.

- `F-ACCOUNT-1` The email rule is `is_bare_address`: every case of
  `only_a_bare_address_is_an_email` answers the same, `a@b` refused; the 64,
  63 and 254 length limits hold; the address is checked trimmed and
  lower-cased, as the server stores it. *(Covered: `accountRules.test.ts`.)*
- `F-ACCOUNT-2` A username is 3 to 32 characters after trimming, counted in code
  points, trimmed of Rust's white space (U+0085 is; U+FEFF, which JavaScript's
  `trim` takes, is not). *(Covered: `accountRules.test.ts`.)*
- `F-ACCOUNT-3` An empty field is named ("must not be empty"), a password
  untrimmed; only the fields with a problem are reported. *(Covered:
  `accountRules.test.ts`.)*

### `F-RATE-*` — `lib/ratingPool.ts`

- `F-RATE-1` The rating pool page's membership comes from the detail's
  `members` (`A-RATE-10`), not the latest fit's ratings: a never-fitted pool's
  anchor is a member, listed as not yet rated and not offered under "Add"; a
  member the fit has not rated is listed unrated (the page gives it a Remove
  button) and not offered; a config removed since the fit is rated but not a
  member, and is offered again. Built from the ratings, the page offered the
  first two under "Add", where adding changed nothing, and gave the second no
  Remove button. *(Covered: `ratingPool.test.ts`.)* (Thirty-third audit, pass
  4.)
- `F-RATE-2` The cross table orders the latest fit's configs best first and
  then those with no chain to the anchor, finds a cell by (row, column), counts
  each head-to-head once for the residual checks (the API serves both sides),
  shows a cell as "58.8% ±6.2" over a signed spread ("+6.8", never "-0.0"),
  and spells it out in its hover with what the ratings predict and the
  residual. A cell's record (`recordSide`) is a win, a loss or even by the
  figure it shows: 50.1% a win, 49.9% a loss, 50.04% and 49.951% even.
  *(Covered: `ratingPool.test.ts`.)*

### `F-DOCS-*` — contributor instructions in `routes/`

- `F-DOCS-1` No page tells a contributor to pass an API key on the command line
  (`--api-key` is no flag of MAGPIE's; `magpie contribute` reads
  a key only from an `apikey` line in `contribute.txt`), and the account page
  shows that line for a freshly created key, under the test ids E-2 reads it
  by; and none puts `contribute.txt` beside the binary (MAGPIE reads it from its
  working directory); and none sends a second process "to a directory of its
  own", where MAGPIE cannot load its board (the README covers running one, on
  a settings file of its own in the same directory). The pages' text, read as
  source. *(Covered: `contributeDocs.test.ts`.)* (Thirty-first audit: the
  account page said `--api-key`, which MAGPIE rejects; thirty-second: the home
  page's second-process advice could not start.)

---

### `F-NGINX-*` — the proxy in front of the app

- `F-NGINX-1` The Nginx template (compose and local stacks; deployed, the load
  balancer sends `/api/` past it) turns chunked transfer off nowhere, in any
  case or quoting, and proxies `/api/` over HTTP/1.1, unbuffered for SSE: with
  chunking off, a results stream cut off part-way closed like a finished
  download through the proxy — curl exited 0 after 15,641 of 150,000 lines,
  and with the line removed exited 18. *(Covered: `nginxConfig.test.ts`.)*
  (Thirty-second audit, passes 21 and 22.)
- `F-NGINX-2` Nginx keeps an idle connection longer than the load balancer
  keeps one to it: the template's one `keepalive_timeout` exceeds
  `aws_lb.main`'s `idle_timeout` in `infra/ecs.tf`, both read from the files.
  At the image's 65 s Nginx closed connections the ALB (300 s) still pooled,
  and a page request sent on one as it closed was answered 502. *(Covered:
  `nginxConfig.test.ts`; that it fails at 65 was checked by hand, and the
  template passes `nginx -t` in the image.)* (Thirty-third audit, pass 1.)
- `F-NGINX-3` Nginx gzips what it serves -- the build's JavaScript, CSS, SVG
  and JSON, and its HTML -- with `gzip on` once, for the whole server, and
  never `text/event-stream` or every type; and `gzip off` once, in `location
  /api/`, whose answers the backend compresses itself (`A-PUBLIC-9`): deployed,
  the ALB sends `/api/` straight to the backend, so Nginx's compression of it
  reached the local stacks only. *(Covered: `nginxConfig.test.ts`; the
  template passes `nginx -t` in the image, and a script and the page came back
  `Content-Encoding: gzip` through it.)*

## 2. Integration

Real Postgres, migrations applied, **no HTTP layer**. This is where birdtest's
genuinely hard logic lives, because most of it is SQL and concurrency rather
than Rust.

**The harness** (`backend/tests/common/mod.rs`) is the dependency for tiers 2
and 3. Three decisions shaped it, settled below, because each has an
obvious-looking answer that is wrong here.

### Isolation: `CREATE DATABASE … TEMPLATE`

Each test gets its own database, cloned from a single pre-migrated template:

```rust
let db = TestDb::new().await;   // CREATE DATABASE <unique> TEMPLATE birdtest_tpl_<hash>
                                // ... test body ...
                                // DROP DATABASE … WITH (FORCE) on Drop
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

The template is built once per schema, behind a `OnceCell` in each process, and
named after a hash of the migration (`birdtest_tpl_<sha256 prefix>`), so an
edited migration gets a fresh template rather than a stale one and concurrent
runs on one schema share it. Two constraints that will otherwise cost an
afternoon:

- **Nothing may hold a connection to the template.** Postgres refuses
  `CREATE DATABASE … TEMPLATE` while one is open, so the pool used to build it
  is closed before it is renamed into place, and a clone that collides with
  another clone's brief attachment is retried.
- **Template creation must be serialized — across processes, not just
  threads.** `cargo nextest` runs every test in its own process, so a
  `OnceCell` alone serializes nothing; the build takes a Postgres advisory lock.

Not `testcontainers`: the compose Postgres is already running for development,
and a container per test costs seconds where a clone costs 125 ms. Revisit if
isolation starts to bite.

Do **not** add a second, faster isolation mode for the read-only tests. Two
modes is a decision to re-make at every new test, in exchange for 100 ms.

### State: builders that are deliberately dumber than the application

```rust
let db     = TestDb::new().await;
let admin  = db.user("root", true).await;
let ld     = db.input_data("letterdist", "english").await;
let player = db.static_player("equity", admin).await;
let job    = db.games_job(/* games per batch */ 2).await;
db.derived_ready(job).await;          // satisfy the dispatch gate by hand
let app    = birdtest::app(db.state().await);
```

A handful of `TestDb` methods for what nearly every test needs — `user`,
`input_data`, `static_player`, `games_job`, `bare_job` — each filling every
required column with a default, plus `derived_ready`. Anything else a test
needs, it inserts inline with plain SQL, which is most of the precise states
below. The request side has `send`, `post_json`, `get_request`,
`admin_headers` (a signed session cookie and CSRF pair) and `claim_body`. Not
all 35 tables; the fluent per-entity builders first sketched here turned out to
be more machinery than the tests wanted.

Three ways to build the `AppState` a test runs against:

- `db.state()` — the test database, a closed object-store endpoint and a
  nonexistent `MAGPIE_BIN`, so an accidental upload or conversion fails rather
  than reaching AWS or whatever MAGPIE is installed. Builder versions are fixed
  (`test_builders()`), not read from a binary.
- `db.state_with(cfg)` — the same from a `Config` the test changed, for a test
  about a setting: a mail outbox, a heartbeat timeout, a version floor.
- `db.state_with_object_store()` — a real object store: a fresh bucket on the
  MinIO at `TEST_S3_ENDPOINT` (its root credentials default to the compose
  MinIO's), returned with a `TestBucket` guard that empties and removes it on
  drop (`artifacts::a_test_bucket_is_removed_with_everything_in_it`). Only the
  tests that are *about* the object store use it — `artifacts.rs`,
  `exports.rs`, the imports in `input_data.rs`, and the leave-job tests that
  create or read a KLV (in `jobs.rs`, `worker_routes.rs`, `magpie_*.rs`) —
  and a missing `TEST_S3_ENDPOINT` fails them rather than skipping.

**The rule that matters: a builder writes SQL directly and validates nothing.**
Roughly half the entries below need a state the application would refuse to
create, and a builder that enforces invariants makes exactly those unwritable:

- `I-SCHED-14` — a task already claimed, or completed, written directly
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
columns, so a test moves the column itself — `UPDATE task_claims SET
claimed_at = now() - interval '1 hour'`. Cheaper than a clock abstraction, and
it exercises the real SQL rather than a test double of it. The one
process-relative time, the restart grace, is a field on `AppState`
(`reclaim_from`) that a test sets rather than waits out.

### Order they were built in

Harness, then `I-SCHED-*` — the largest group, the hardest logic, and the part
that breaks silently. Then `I-JOB-*`, where the `win_pct_model` bug lived. Then
the rest by group, cheapest first.

### `I-SCHED-*` — scheduler (`scheduler.rs`)

The single most important group. Every entry is about a decision made in SQL.

- `I-SCHED-1` A claim against one active job returns a task, inserts a
  `task_claims` row, and increments `active_claim_count`. *(Covered:
  `scheduler::a_claim_writes_its_row_and_counts_itself_on_the_task`.)*
- `I-SCHED-2` Deficit selection: two active jobs with allocations 75/25
  converge on that ratio over many claims. *(Covered:
  `admin_api::a_newly_activated_job_joins_at_parity_instead_of_taking_everything`,
  which re-shares two jobs to 75/25 and asserts exactly 9/3 of the next twelve
  claims; `worker_api::every_claim_advances_the_dispatch_counter`.)*
- `I-SCHED-3` A job at 0% is inactive (`jobs_allocation_is_status`) and
  offered to nobody: every claim goes to the other active job, and with every
  job at 0% the answer is `204`, not a shutdown — including when a parked job is
  too new for the worker or in its unsupported set, which shuts nobody down
  until the job is raised above 0% (`a_parked_job_shuts_nobody_down`; an active
  job at 0% once could, and the schema no longer has one). There is no
  priority. *(Covered: `worker_api::a_job_at_zero_allocation_is_offered_to_nobody`,
  `worker_api::a_parked_job_shuts_nobody_down`.)*
- `I-SCHED-3a` **A job joins at parity.** A job activated beside one with a
  long claim history splits the next claims by allocation rather than taking
  all of them, a changed allocation holds from the moment it is set, and a
  purged job rejoins level (`claims_baseline`, `scheduler::join_at_parity`).
  *(Covered: `admin_api::a_newly_activated_job_joins_at_parity_instead_of_taking_everything`,
  `admin_api::a_purged_job_rejoins_at_parity`.)*
- `I-SCHED-3b` **Parity is with the jobs being served.** Beside a veteran and a
  job on offer that this fleet cannot run (and so has issued nothing), a
  newcomer splits the next twelve claims 6/6 with the veteran rather than
  taking all twelve; and issuing a claim stamps `jobs.last_claimed_at`, which
  is what "served" reads. *(Covered:
  `admin_api::a_job_nobody_is_being_served_from_does_not_set_a_newcomers_parity`.)*
- `I-SCHED-3c` **A job with nothing to hand out does not set parity.** A
  games job at its cap, its one task in flight, is served and stands still;
  each claim that passes it over lifts it level with the job claimed
  (`scheduler::lift_passed_over`), so the newcomer, joining at the lowest
  served ratio and settling against the lowest of each worker's other
  candidates, splits the next 900 claims 2:1 with the veteran, the veteran's
  first claim within twelve (without the lift, it fails). *(Covered:
  `admin_api::a_newcomer_is_not_put_level_with_a_job_that_has_run_out_of_work`.)*
  (Thirty-second audit, passes 19 and 20.)
- `I-SCHED-3d` After a quiet spell (no job has claimed within the heartbeat
  timeout of now), "served" is measured from the latest claim, so a newcomer
  still joins level with the veteran, not a job nobody can run: 6/6, where it
  took 12 of 12. *(Covered:
  `admin_api::after_a_quiet_spell_a_newcomer_still_joins_level_with_the_jobs_served`.)*
- `I-SCHED-3e` **A newcomer is not starved behind a lagging job.** In a split
  fleet — 30% of claims from workers that can run only an old-floor job at
  10%, which then leads — a newcomer at 40% beside the majority's lagging job
  gets its share of the next 1,000 claims (200 to 350; joined at the leader it
  got none). *(Covered:
  `admin_api::in_a_split_fleet_a_newcomer_is_not_starved_behind_a_lagging_job`.)*
- `I-SCHED-3f` A job only a minority can run lags while served; a newcomer the
  same minority can run gets its share (50 to 120 of that minority's 200
  claims; it got none). *(Covered:
  `admin_api::a_minority_newcomer_is_not_starved_behind_a_lagging_minority_job`.)*
- `I-SCHED-3g` **A job nobody could run does not bank what it missed.** Unserved
  for a heartbeat timeout (a MAGPIE floor nobody met for an hour), it rejoins
  at parity on its first claim back (`scheduler::issue_claim`): the veteran
  gets 19 to 21 of the next 40 claims, where it got none. *(Covered:
  `admin_api::a_job_nobody_could_run_rejoins_at_parity_when_the_fleet_can`.)*
- `I-SCHED-3h` **Jobs that lag together keep their shares.** Two jobs at 45%
  that half the fleet runs, beside a 10% job the other half can only run,
  split their half evenly (490 to 510 of 1,000 each), where bounding each lag
  against the job just claimed gave 978 : 22. *(Covered:
  `admin_api::jobs_lagging_together_keep_their_shares`.)*
- `I-SCHED-3i` **A concurrent burst is paid back.** Thirty-two workers claiming
  at once over a 1% and a 99% job: the 1% job gets at most 45 of about 3,000
  claims (fair is 30, and 30 in practice), where forgiving the lag made it
  64 to 87. *(Covered:
  `admin_api::a_concurrent_burst_to_a_small_job_is_paid_back`.)*
- `I-SCHED-3j` **A newcomer is settled level with each class that runs it.** In
  a fleet where a job at 40% only the 20% of claims from MAGPIE 2 can run lags
  the job at 50% everyone runs, a newcomer everyone can run, joining at the
  lowest served ratio — the minority job's — is lifted level with the
  majority's job by the majority's first claim of it: the majority job gets
  its first claim within twelve and 640 to 690 of the majority's 800, where
  the newcomer took claims until the 331st. *(Covered:
  `admin_api::a_newcomer_everyone_can_run_is_not_put_level_with_a_minority_job`.)*
- `I-SCHED-3k` An allocation changed in the same split (20% to 19%) is a join
  and settles the same way: 515 to 570 of 800 to the job at 40%, where its
  first claim came 476th. *(Covered:
  `admin_api::an_allocation_changed_in_a_split_fleet_takes_nothing_over`.)*
- `I-SCHED-3l` A job nobody could run for an hour, returning in the same
  split, rejoins and settles: 640 to 690 of 800 to the majority's job, where
  its first came 331st. *(Covered:
  `admin_api::a_returning_job_in_a_split_fleet_takes_nothing_over`.)*
- `I-SCHED-3m` A job passed over for a moment is lifted to where the job
  claimed stood before its claim: a 50% job passed over while a 1% job was
  claimed takes its next turn at once and 140 to 160 of the next 300, where
  it waited 49 claims. *(Covered:
  `admin_api::a_job_passed_over_for_a_moment_waits_for_nothing`.)*
- `I-SCHED-3n` **A job with nothing to hand out banks no debt.** Held for 200
  claims beside a job at the same share, it does not take the claims after
  the hold in a row: the other job's first comes within three. *(Covered:
  `admin_api::a_job_with_nothing_to_hand_out_banks_no_debt`.)*
- `I-SCHED-3o` **A burst is paid back in a job's first hour too.** The jobs of
  3i activated through the endpoint, so both are settling: the 1% job gets at
  most 40 (30 in practice), where settling forgave the payback of a burst —
  307 to 324 — until each claim was checked for its turn under the job's
  dispatch lock. *(Covered:
  `admin_api::a_burst_in_the_first_hour_is_paid_back`.)*
- `I-SCHED-3p` **A decline undoes the settling.** A newcomer only the minority
  has the data for, declined `missing_data` through the endpoint by each
  majority worker it is issued to, gets 60 to 100 of the minority's 400
  claims (fair 80), where the majority's claims settled it past the
  minority's own job and it got none. *(Covered:
  `admin_api::a_newcomer_the_majority_declines_is_not_settled_at_its_pace`.)*
- `I-SCHED-3q` A newcomer settles against the lowest of the worker's other
  candidates, those just passed over included: with the minority's own job
  paused for one claim it still gets 80 to 120 of the minority's 200, where
  it was settled past the paused job and got none. *(Covered:
  `admin_api::a_newcomer_is_not_settled_past_a_job_paused_for_a_moment`.)*
  (Thirty-second audit, pass 20: the lag window removed, joining settled,
  claims checked for their turn. Each rule switched off fails its own tests:
  the settling 3j, 3k and 3l; the rejoin 3g and 3l; the pass-over lift 3c and
  3n; the turn check 3o (and 3i, in about one run in four); the undoing on a
  decline 3p; the lowest-other
  pace 3q. On pass 20's first design, 3j to 3m fail; on its second, 3o to 3q.)
- `I-SCHED-3r` **No claim is told there is nothing while work exists.** Three
  jobs at equal shares, 32 workers claiming together: every one of 1,920
  claims gets a task, 600 to 680 each, where a claim that found each job a
  claim past another for eight rounds answered `204` (67 to 166 of 1,920 across runs). *(Covered:
  `admin_api::equal_jobs_claimed_together_leave_no_worker_idle`.)*
- `I-SCHED-3s` A job whose dispatch lock another holder keeps costs a claim
  one wait: three claims each get the other job within 5 s, where the third
  waited 16 s and got nothing. *(Covered:
  `admin_api::a_busy_job_costs_a_claim_one_wait`.)*
- `I-SCHED-3t` A busy job stays a rival, with a ratio unit of slack: the 1%
  job beside a busy 99% one (both settling) takes at most two of five claims
  while the lock is held and 33 of about 3,000 in all, where it took every
  claim of the spell and the settling forgave them (49 where 30 is fair).
  *(Covered: `admin_api::a_busy_large_job_does_not_hand_a_small_one_its_claims`.)*
  (Thirty-second audit, pass 21.)
- `I-SCHED-3u` Repeated busy spells on a settling job are paid back: a job
  found busy is settled a ratio unit short for ten minutes, so a 10% job
  beside a busy 90% one gets its tenth over three spells (within 5), where
  each spell's lead was forgiven. *(Covered:
  `admin_api::repeated_busy_spells_on_a_settling_job_are_paid_back`.)*
  (Thirty-second audit, pass 22.)
- `I-SCHED-3v` Short, not unsettled: a newcomer found busy once in the split of
  3j is still settled, the majority job's first claim within sixteen, where it
  came 331st. *(Covered:
  `admin_api::a_newcomer_busy_once_is_still_settled`.)*
  (Thirty-second audit, pass 22.)
- `I-SCHED-4` Abandoned claims count toward a job's share. Abandon many claims
  on one job and confirm its share does **not** grow — excluding them would let
  a job with flaky workers accumulate more than its share. The counter is
  `jobs.claims_issued`, bumped by every claim issued; there is no
  `tasks_dispatched`, as this entry first called it. *(Covered:
  `scheduler::abandoned_claims_still_count_against_a_jobs_share`.)*
- `I-SCHED-5` Ties break on `created_at ASC`. *(Covered:
  `scheduler::equal_deficits_go_to_the_older_job`.)*
- `I-SCHED-6` **Both capability filters are part of candidate selection.** A
  worker whose `unsupported_jobs` covers the job furthest behind its share — or
  that is too old for it — is offered the next one in deficit order, not shut
  down. *(Covered:
  `scheduler::a_worker_that_cannot_run_the_job_furthest_behind_gets_the_next_one`.)*
- `I-SCHED-7` Version filtering: a worker on `1.9.0` is offered a job requiring
  `1.9.0` and not one requiring `1.10.0`. Include the `1.9.0` vs `1.10.0` pair
  specifically — lexical comparison passes every other case. *(Covered:
  `scheduler::a_1_9_worker_gets_a_1_9_job_and_never_a_1_10_one`.)*
- `I-SCHED-8` `ClaimOutcome::Idle` when work exists but none is available, vs
  `NoWorkExists` when no job is active. These mean opposite things to a client.
  *(Covered:
  `scheduler::idle_means_work_exists_and_no_work_exists_means_none_is_offered`.)*
- `I-SCHED-9` `Shutdown` with reason `magpie_too_old`, `data_out_of_date`, and
  `both` — and `both` leads on the version, because updating MAGPIE is the
  action that also fixes the data. *(Covered:
  `scheduler::each_shutdown_reason_names_what_the_worker_must_change`,
  `worker_api::a_stale_unsupported_entry_does_not_change_the_shutdown_reason`.)*
- `I-SCHED-10` A shutdown directive names the required tarball dates and
  version actually derived from the active jobs, not a hardcoded string.
  *(Covered: `scheduler::a_shutdown_names_what_the_active_jobs_actually_require`.)*
- `I-SCHED-11` **Reclamation**: a claim past the heartbeat timeout flips to
  `abandoned`, decrements `active_claim_count`, and returns its task to
  `available`. *(Covered:
  `scheduler::a_lapsed_claim_is_reclaimed_by_the_next_claim_for_its_job_only`.)*
- `I-SCHED-12` Reclamation is lazy — it happens on the next claim for that job,
  and a claim one second *inside* the timeout is not reclaimed. *(Covered:
  `scheduler::a_lapsed_claim_is_reclaimed_by_the_next_claim_for_its_job_only`.)*
- `I-SCHED-13` **Concurrent claimers**: N simultaneous claims against one job
  produce no duplicate seeds and no lost counter updates. Claims for one job
  serialize on its row before reading the seed cursor, so the `(job_id, seed)`
  unique index is a backstop rather than the mechanism; resolving the race by
  retrying the loser answered `204` past three-way contention. *(Covered:
  `scheduler::concurrent_claimers_neither_collide_nor_lose_a_count`,
  `worker_api::concurrent_claims_tile_the_seed_space_instead_of_colliding`.)*
- `I-SCHED-14` A task has one slot: a claimed or completed task is not handed
  to another worker, who gets a fresh task instead. *(Covered:
  `scheduler::a_taken_task_is_not_handed_to_another_worker`,
  `scheduler::a_claim_writes_its_row_and_counts_itself_on_the_task`.)*
- `I-SCHED-15` **The `declined` partial-index trap.** `task_claims_one_slot_idx`
  is partial on `WHERE state IN ('claimed','completed')`. Put `'declined'` in it
  and a declined claim holds the task's one slot for good: nobody can claim it
  again, the worker that declined it included, after fixing its data. Nothing
  else catches this. (Within an hour of the decline it is offered other work
  instead, `A-WORKER-19`; after it, the same task.) *(Covered:
  `scheduler::a_worker_that_declined_a_task_can_claim_the_same_task_again`.)*
  (It was two per-identity unique indexes, partial on
  `state NOT IN ('abandoned','declined')`, until the one-slot index replaced
  them in the thirty-third audit, pass 1.)
- `I-SCHED-16` One worker cannot hold two simultaneous claims on the same task,
  by either identity type. *(Covered:
  `scheduler::one_worker_never_holds_two_live_claims_on_one_task`.)*
- `I-SCHED-22` **One slot, in the schema.** A task never has two claims
  holding or having completed its slot, by any two identities: a second
  `claimed` or `completed` claim is refused by `task_claims_one_slot_idx`
  (lapsed and declined claims are outside it), and `tasks.active_claim_count`
  and `accepted_count` are held to 0 or 1 by a CHECK, so a drift fails the
  statement that drifted it. *(Covered:
  `scheduler::a_task_never_has_two_claims_on_its_one_slot`.)* (Thirty-third
  audit, pass 1.)
- `I-SCHED-17` A banned worker's claim is refused, by user id and by anon UUID.
  *(Covered: `worker_api::a_banned_identity_is_refused_however_it_authenticates`,
  `admin_routes::a_ban_by_either_identity_refuses_the_next_claim_and_unban_restores_it`.)*
- `I-SCHED-18` `release_claim` returns the task to `available` and decrements
  the counter. *(Covered through its callers:
  `worker_routes::a_claim_states_its_digests_and_a_missing_data_decline_releases_it_at_once`
  asserts the claim state, `active_claim_count` and the task state after a
  decline; `worker_api::a_failed_task_is_handed_straight_back` that the next
  worker gets the same task at once.)*
- `I-SCHED-19` An inactive or completed job is never selected. *(Covered:
  `scheduler::an_inactive_job_is_never_selected`,
  `worker_api::a_claim_racing_a_jobs_completion_hands_nothing_out`.)*
- `I-SCHED-20` **A restarted server does not judge a fleet it has not heard
  from.** A claim an hour past its heartbeat is *not* reclaimed by a process
  younger than the heartbeat timeout — the workers were heartbeating to a server
  that was not there — and the worker's heartbeat and result are then accepted.
  Once the grace has passed, a claim that stayed silent is reclaimed as ever.
  And the state `main` actually builds grants the grace (`A-BOUND-10`) — the
  first two tests set it by hand. *(Covered:
  `worker_api::a_restarted_server_does_not_abandon_claims_it_could_not_have_heard_from`,
  `worker_api::a_claim_still_silent_after_the_grace_is_reclaimed`,
  `boundaries::a_freshly_built_production_state_grants_the_restart_grace`.)*
- `I-SCHED-21` **An opening-rack consensus job reissues its unsettled racks.**
  It covers its space in ranges, then hands out unsettled racks as lists,
  from seeds past the end of the space: fewest analyses first, none another
  reissue holds, and none the claiming worker analysed while others are left
  -- but those too rather than nothing, so a rack wanting more analyses than
  the fleet has workers still settles. A rack settles once enough analyses
  agree, or at its most without a consensus; the running totals (`racks_analyzed`,
  `racks_settled`, `racks_without_consensus`) follow, and the job completes once
  every rack is settled. The rack lookup numbers each analysis, and holds
  their lists together to what one analysis can record: three analyses of
  12,000 moves list their best 10,922 each (32,767 / 3), from the top. Public,
  it returned a consensus job's hundred analyses of 32,767 moves whole.
  *(Covered:
  `worker_api::a_consensus_job_reissues_its_unsettled_racks_until_each_settles`;
  the standing itself, `U-RACK-11`.)*
- `I-OR-REISSUE-1` **A reissue's cost does not grow with the racks its
  identity analysed.** One identity that analysed a job's whole first pass is
  handed a full batch all the same (fewest analyses first, then by rack, none
  in flight), and another identity the next racks; with the first pass split
  between two identities, each is handed the other's racks first within the
  window of four batches it looks at, and nothing past it: with the other's
  racks pushed beyond the window, it is handed racks it has seen, where a walk
  would have found the other's (the test fails with the window widened to
  every rack; the second pass added this, since the rest passed against the
  walk too). The preference used to walk every unsettled rack until it found
  a batch the identity had not analysed, under the dispatch lock: 5.7 s a
  claim at 1,000,000 racks for an identity that had seen them all (PLAN.md,
  "What these reads cost"). *(Covered:
  `worker_api::a_reissue_looks_at_a_window_of_racks_not_every_one`; the
  timing, and that the preference is planned as a probe per rack whatever the
  statistics, measured by hand.)* (Thirty-third audit, pass 1.)
- `I-OR-REISSUE-2` **A reissue's in-flight read does not grow with the job's
  reissue history.** With two reissues completed, one claimed and one declined
  back to `available`, the next reissue leaves out exactly the open two's
  racks and is handed the completed ones'. The in-flight racks were read by
  visiting every reissue the job had ever made, under the dispatch lock:
  27–44 ms at 200,000 completed reissues, against 2–5 ms now (PLAN.md, "What
  these reads cost"). *(Covered:
  `worker_api::a_reissue_leaves_out_the_open_reissues_and_only_those`; the
  timing, measured by hand.)* (Thirty-third audit, pass 2.)

### `I-EXPECT-*` — capability negotiation (`jobs/mod.rs::expected_data`)

- `I-EXPECT-1` Two players on different lexicons yield two `kwg` and two `klv`
  entries. *(Covered:
  `scheduler::players_on_different_lexicons_each_contribute_a_kwg_and_a_klv`.)*
- `I-EXPECT-2` The same config on both sides yields one of each, not two
  identical rows. *(Covered:
  `worker_api::an_assignment_names_every_file_the_task_loads_and_no_others` —
  two players sharing a lexicon get one `kwg` entry, the deduplication the
  same-config case rests on.)*
- `I-EXPECT-3` A static player contributes no `winpct` entry. *(Covered: the
  same test.)*
- `I-EXPECT-4` A `leave_generation` job yields exactly its player's `kwg`,
  `letterdist` and `layout`, and never a `klv`, not even its player's own —
  its leaves come from the server-built artifact.
  *(Covered: `scheduler::a_leave_job_needs_its_lexicon_bag_and_board_and_never_leaves`.)*
- `I-EXPECT-5` Every entry carries the SHA-256 from `input_data`, not a name.
  *(Covered: `scheduler::every_expected_file_carries_the_pinned_rows_digest`.)*
- `I-EXPECT-6` An `opening_rack` job yields its single player's files plus the
  job's distribution and layout. *(Covered:
  `scheduler::an_opening_rack_job_needs_its_players_files_and_the_jobs_bag_and_board`.)*

### `I-JOB-*` — job creation and lifecycle (`routes/admin.rs` SQL, `jobs/registry.rs`)

These are the functions the `win_pct_model` bug lived in. Every SQL string that
job creation touches needs one caller here.

- `I-JOB-1` Creating each of the four job types inserts its config row with
  every column populated, and reads back identical; a games job created with
  only its target runs no match test, and stores the test's default
  confidence (95%) and a floor of 0, and the default threading, `igp`.
  *(Covered:
  `jobs::each_job_type_stores_every_setting_it_was_created_with`,
  `jobs::a_leave_generation_job_stores_every_setting_it_was_created_with`.)*
- `I-JOB-1b` A `games` job's batch is even (default 2): MAGPIE gives player 1
  the first move in each task's first game, so at a batch of 1 player 1 moved
  first in every game and the SPRT the match test replaced passed two
  identical players (thirty-second audit, pass 18). A games or game-pairs
  job's confidence is strictly between 50% and 100%: at 100% the interval
  never closes, and at half or less it is no test. *(Covered:
  `routes::admin::tests::a_games_batch_must_be_even`,
  `routes::admin::tests::a_confidence_outside_half_to_all_is_refused`,
  `routes::admin::tests::every_problem_is_reported`.)*
- `I-JOB-1c` **The match test is off unless asked for.** A games or pairs body
  without `test_enabled` runs no test and needs no `min_*`; with the flag off,
  `min_*` and `confidence_pct` are each refused by name rather than dropped
  (the job would otherwise play to its cap with no test, and nothing would
  say so); with it on, `min_*` is required, at least 1 and at most the cap.
  *(Covered:
  `routes::admin::tests::a_job_runs_no_test_unless_it_asks_for_one`,
  `routes::admin::tests::test_settings_without_the_test_are_refused`,
  `routes::admin::tests::the_test_needs_its_floor`.)*
- `I-JOB-1d` A leave job lists one occurrence target per generation
  (`target_rack_counts`, MAGPIE's `leavegen 100,200,500,…`): between 1 and 100
  of them, each between 1 and 1,000,000; an empty list, a zero, a negative, one
  past either bound is refused by name. The schema's CHECK refuses the same
  shapes (and a NULL element) below the API. *(Covered:
  `routes::admin::tests::a_leave_job_lists_between_one_and_the_most_generations_each_with_a_sane_target`;
  the form's parse, `F-FMT-15`.)*
- `I-JOB-1e` **An opening-rack job's consensus** -- the share of a rack's
  analyses that must agree on its best move, after at least and at most so
  many -- is stored as stated for a simming player (one analysis per rack, 100%,
  when the body says nothing), and refused by name where it cannot work: a
  share at or below 50% or above 100%, no analyses, a most below the fewest or
  above 100, and any consensus for a static player, whose analyses always
  agree. *(Covered:
  `jobs::an_opening_rack_jobs_consensus_is_stored_and_refused_where_it_cannot_work`.)*
- `I-JOB-1f` **The schema holds a games or pairs job's counts** to what
  `validate_job_body` requires, for a row a script or fixture writes: a batch
  and a cap of at least 1, a floor of at least 0, and with the test on a floor
  from 1 to the cap (`job_game_config_counts`, `job_game_pair_config_counts`).
  The stopping rule reads them unsigned, so a negative cap was one no job
  reached. A games row that states no batch gets 2, the least even one
  (KL-87). *(Covered:
  `jobs::a_games_or_pairs_jobs_counts_are_held_by_the_schema`.)* (Thirty-third
  audit, pass 1.)
- `I-JOB-1g` **A job's bingo bonus is 0 to 500**, refused by name at creation
  (-1, 501, 5000) and by the `jobs` CHECK below the API; both ends are kept as
  stated. The plausibility rules' score bounds are absolute, set for an
  ordinary bonus, so a bonus in the thousands made every honest batch
  implausible and wedged the job. *(Covered:
  `jobs::a_jobs_bingo_bonus_is_bounded`.)* (Thirty-third audit, pass 2.)
- `I-OR-EDIT-1` **An opening-rack job's consensus changes after creation, and
  the job follows** (`PATCH /api/admin/jobs/:id/consensus`). A job wanting one
  analysis per rack still keeps a progress row per rack, so raising its
  minimum and maximum after it completed restates every rack unsettled,
  reopens it inactive at 0% (`job.consensus_changed`, "min 1 -> 2, max 1 -> 3;
  4 racks unsettled", then `job.deactivated` from completed), demotes its final
  export to a snapshot, and, once given an allocation, reissues the racks from
  those rows; lowering them
  again while a reissue is in flight settles every rack, and that reissue's
  submission completes the job without counting its racks a second time, and
  every line of its corpus then carries its rack's standing, each rack having
  two analyses, though the maximum is one again (it was gated on the maximum
  alone, and hid the disagreements `racks_without_consensus` counts).
  Asking for what every rack already has leaves a completed job completed,
  and still demotes its final export and fails one building from before the
  edit ("the job's consensus settings changed while this export was
  building"): the standings each line carries were restated (thirty-third
  audit, pass 3; only a reopening demoted them, and the old standings went on
  being served as the completed job's corpus). *(Covered:
  `worker_api::an_opening_rack_jobs_consensus_can_change_and_the_job_follows`.)*
- `I-OR-EDIT-2` The edit refuses by name what creation refuses (a share at
  or below 50%, no analyses, a most below the fewest or above 100, any
  consensus for a static player), and refuses any job but an opening-rack
  one; a change that changes nothing writes no audit row; and a completed job
  it reopens comes back inactive at 0%, the 0% every completed job holds.
  *(Covered: `worker_api::a_consensus_edit_is_checked_and_reopens_inactive`.)*
- `I-OR-EDIT-3` **A finish check overtaken by a consensus edit does not
  complete the job.** A check that read every rack settled, completing after
  an edit unsettled them, leaves the job active, and its racks are reissued:
  `complete_unless_purged` re-checks `racks_settled` in its own update, which
  Postgres re-evaluates on the row the edit committed. Neither purge witness
  sees an edit. *(Covered:
  `worker_api::a_finish_check_overtaken_by_a_consensus_edit_does_not_complete_the_job`,
  driven as `I-STATS-9d` drives the purge.)* (Thirty-third audit, pass 1.)
- `I-OR-EDIT-4` **A consensus edit in progress costs a claim or a submission
  no lock wait.** While an edit of an active job holds its locks, a claim
  skips the job (a `204` in well under the dispatch lock's two seconds), a
  submission for one of its claims is a `503` at once (not after its claim's
  five), and a second edit is a `409`; once the edit is done, the claim's
  result is accepted. *(Covered:
  `worker_api::a_consensus_edit_in_progress_costs_claims_and_submissions_no_wait`.)*
  (Thirty-third audit, pass 1.)
- `I-OR-EDIT-5` **An edit that is refused, or changes nothing, holds
  nothing.** It is answered before the hold or a lock is taken, so a lapsed
  claim of the job is reclaimed at once after it: a games job's `400`, an
  opening-rack job's share out of range, and its settings sent back unchanged.
  Checked only under the locks, each held the job's claims off while it took
  them and then left the reclaim grace, a heartbeat timeout in which the job's
  lapsed claims were not reclaimed. *(Covered:
  `worker_api::a_refused_or_unchanged_consensus_edit_holds_nothing`.)*
  (Thirty-third audit, pass 2.)
- `I-OR-EDIT-6` **An action that waited on a job's row is told it was purged
  only when it was.** A force-complete or allocation change that took the row before a consensus edit queued on it is refused as
  running ("a purge, delete or consensus change of this job is running"): the
  edit's hold is not counted, and it was answered "the job was purged while
  this waited". A purge started meanwhile is still "purged", by count, after
  its hold is gone. *(Covered:
  `routes::admin::tests::an_action_that_waited_on_an_edit_is_not_told_the_job_was_purged`.)*
  (Thirty-third audit, pass 3.)
- `I-JOB-2` **`validate_shared_player_options` runs against real rows.** Two
  configs with different `winpct_id` are rejected; two with the same are
  accepted; two with different `movegen_margin` are accepted, since autoplay
  generates with a margin of 0 whatever a player states (thirty-third audit,
  pass 2: they were refused). The regression guard for the renamed column. *(Covered:
  `jobs::two_players_must_share_the_run_wide_settings_magpie_cannot_vary`. A
  static player has no model, so a static player against a simmer is accepted:
  `admin_api::a_games_job_may_pit_a_static_player_against_a_simmer`.)*
- `I-JOB-3` `validate_player_compatibility` rejects an incompatible
  lexicon/distribution pair and accepts a compatible one, using real
  `input_data` rows. *(Covered:
  `jobs::a_job_whose_lexicons_do_not_fit_its_distribution_is_refused`.)*
- `I-JOB-4` A job referencing a nonexistent `input_data` id fails cleanly with a
  400-shaped error, not a foreign-key 500. *(Covered:
  `jobs::a_job_naming_something_that_does_not_exist_is_a_clean_400`.)*
- `I-JOB-5` Creating a job writes no rows up front; a leave-generation job's
  first claim starts the seeding of its generation-1 universe. *(Covered:
  `admin_routes::creating_each_job_type_answers_it_inactive_and_unallocated`
  (no task for any type), `leave_gen::generation_ones_universe_is_seeded_by_the_first_claim_too`.)*
- `I-JOB-6` A job is created inactive at 0%; an allocation above 0% activates
  it and sets `activated_at`; 0% deactivates it, at 0%, without destroying
  tasks; completion is terminal and holds the job at 0%, and no allocation,
  not even 0%, moves a completed job. *(Covered:
  `jobs::a_job_moves_through_its_lifecycle_and_completion_is_final`,
  `admin_api::a_completed_job_cannot_be_deactivated`.)*
- `I-JOB-7` An allocation outside 0–100 is rejected, and the schema holds a job
  active exactly when above 0% (`jobs_allocation_is_status`): neither an active
  job at 0% nor an inactive or completed one above it can be written.
  *(Covered: `jobs::an_allocation_outside_0_to_100_is_refused`; the sum across
  jobs is `A-BOUND-7`.)*
- `I-JOB-8` `purge_job` deletes tasks and claims, leaves the job row, and lets
  task generation resume cleanly from the right seed -- the start of its
  space, which is what PLAN.md specifies. *(Covered:
  `jobs::a_purged_job_hands_out_its_seed_space_again_from_the_start` — seeds
  1, 3, 5 before and again after —
  `admin_api::a_job_can_be_purged_and_its_dispatch_counter_resets` — no task,
  no claim, zeroed counters, the job row kept —
  `admin_api::purging_a_job_removes_its_captured_positions_through_the_record`,
  and `admin_api::a_purged_job_rejoins_at_parity`.)*
- `I-JOB-9` `delete_job` cascades to tasks, claims, results and configs, and
  leaves no orphans in any table. *(Covered:
  `jobs::deleting_a_job_leaves_nothing_anywhere_that_points_at_it`,
  `admin_api::a_job_with_history_can_be_deleted_and_its_census_survives`.)*
- `I-JOB-10` Both write their census to `audit_log` **before** destroying, so
  the record survives the thing it describes. *(Covered:
  `jobs::a_purge_writes_its_census_before_it_destroys_anything`,
  `audit::every_destructive_admin_action_writes_exactly_its_record`,
  `admin_api::a_job_with_history_can_be_deleted_and_its_census_survives`.)*
  An opening-rack job's census counts its `opening_rack_progress` rows as
  `rack_standings`, which it left out until the October 2026 audit. *(Covered:
  `worker_api::an_opening_rack_jobs_consensus_can_change_and_the_job_follows`.)*
- `I-JOB-11` A player config referenced by any job -- a leave job's player
  included -- cannot be deleted. *(Covered:
  `jobs::a_player_config_in_use_by_any_job_cannot_be_deleted`.)*
- `I-JOB-12` `delete_user` leaves their contributions attributed but anonymised,
  per the design, and does not cascade away results. *(Covered:
  `admin_api::a_user_with_history_can_be_deleted`.)*
- `I-JOB-13` A job's `min_magpie_version` that is not `major.minor[.patch]` is
  `400` (it was read as 0.0.0, the most permissive floor). *(Covered:
  `jobs::a_malformed_magpie_floor_is_refused`, `version::tests::a_strict_parse_takes_only_a_whole_version`.)*
  (Fifteenth audit.)
- `I-JOB-14` A player config with more than 25 plies (MAGPIE's `MAX_PLIES`) or
  more than 10 recorded plies (what a captured position keeps) is `400`.
  *(Covered: `jobs::a_player_config_past_magpies_limits_is_refused`.)*
  (Fifteenth audit.)
- `I-JOB-14b` A simming player config is bounded by its iteration budget,
  never by a time limit: one that states no `time_limit_secs`, or one above 0,
  is `400` on that field (a limit makes how far a simulation gets depend on the
  contributor's hardware, and a null one is MAGPIE's 60-second default), and
  one with no `max_iterations` is `400` on that; `time_limit_secs: 0` with an
  iteration budget is created. And one with `sort_strategy = 'score'` is `400`:
  a simmer's candidates are the top plays by equity in a games job whatever
  the row says, so `score` would give one config two meanings. A static one
  may sort by score (twenty-ninth audit). *(Covered:
  `admin_api::a_simming_player_config_is_bounded_by_iterations_not_time`.)*
- `I-JOB-14c` An opening-rack job refuses a static player with a `best`
  recorder and more than one recorded play, and any player whose `num_plays` is
  below its `num_plays_recorded`: each would store fewer moves per rack than it
  asks for. A `best` simmer is accepted (twenty-ninth audit). *(Covered:
  `admin_api::an_opening_rack_job_cannot_rank_moves_with_a_best_recorder`.)*
- `I-JOB-14e` A leave job refuses a player that solves endgames (and so,
  perhaps, pre-endgames), naming the player field: its games end before the bag
  is small enough for either solver. The same player is accepted by a games
  job. *(Covered: `jobs::a_leave_job_refuses_a_player_that_solves_the_endgame`.)*
- `I-JOB-14f` A leave job states no sim cutoff, and one sent is refused on the
  field; a bingo bonus it states is kept. *(Covered:
  `jobs::a_leave_job_takes_a_bingo_bonus_and_no_sim_cutoff`.)*
- `I-JOB-14d` A leave job refuses a player that simulates, sorts on anything
  but equity, or asks for a rack info table, naming the player field, and
  leaves no job behind; a static equity player is accepted. A leave body is
  read as a leave config, though it names a `player_config_id` as an
  opening-rack body does. *(Covered:
  `jobs::a_leave_job_refuses_a_player_it_cannot_generate_leaves_with`,
  `routes::admin::tests::a_leave_body_is_read_as_a_leave_config_and_an_opening_rack_body_is_not`.)*
- `I-JOB-15` **A games or pairs request naming n ≥ 2 configs is a round
  robin**: C(n, 2) jobs (3 → 3, 4 → 6), every pairing once, seated in the
  order listed and named "{name}: A vs B" ("A vs B" unnamed), all inactive at
  0%, each audited `job.created`, and answered together as `{jobs}`; one
  config is a self-play job under the name as given. *(Covered:
  `jobs::a_round_robin_creates_a_job_for_every_pairing`,
  `routes::admin::tests::pairings_are_every_pair_once_in_the_order_given`; the
  form's preview of the same rules, `roundRobin.test.ts` (F-RR-1, F-RR-2).)*
- `I-JOB-16` **A round robin is created whole or not at all.** Every pairing
  is checked before anything is inserted, and one clash (two simmers on
  different win% models) refuses the request, naming that pairing, with no job
  written; a config named twice, more than twelve, none, or a name its
  pairing would push past 100 characters is refused on its field. *(Covered:
  `jobs::a_round_robin_with_one_bad_pairing_creates_nothing`,
  `routes::admin::tests::a_round_robin_names_one_to_twelve_configs_once_each`.)*

### `I-SUBMIT-*` — result submission (`jobs/mod.rs`, `jobs/*.rs`)

- `I-SUBMIT-1` A `games` result inserts one `game_results` row with NULL
  pentanomial and NULL divergent columns. *(Covered:
  `submissions::a_games_result_stores_one_row_with_no_pair_columns`.)*
- `I-SUBMIT-2` A `game_pairs` result inserts the pentanomial, and the database
  CHECK rejects a row whose buckets disagree with the counts: on the pair count,
  separately on player 1's half-points with the pair count right, and on the
  draws with both right (two win-and-draw pairs beside no ties); the route
  refuses each with its own message. The rules are clauses of one named
  constraint,
  `game_results_pentanomial_all_or_nothing`, not two constraints, so each is
  shown to fire on its own. *(Covered:
  `submissions::a_pairs_result_stores_its_pentanomial_and_the_schema_refuses_a_contradiction`.)*
- `I-SUBMIT-3` Accepting a result increments `accepted_count`, decrements
  `active_claim_count`, and completes the task, which has one slot, counting
  it in `tasks_completed` once. *(Covered:
  `submissions::an_accepted_result_completes_its_task`.)*
- `I-SUBMIT-4` A submission against a stale claim token is ignored, not
  accepted, and does not move any counter. *(Covered:
  `worker_api::submissions_for_reclaimed_or_already_accepted_claims_change_nothing`,
  `worker_api::a_claim_token_works_only_for_the_identity_it_was_issued_to`.)*
- `I-SUBMIT-5` *(Retired with redundancy: a task has one slot, so no claim
  replays another's captured positions.)*
- `I-SUBMIT-6` Captured positions are truncated to the player config's
  `num_plays_recorded`, and `num_moves` records the pre-truncation count.
  *(Covered: `submissions::captured_positions_keep_the_configured_moves_and_the_full_count`.)*
- `I-SUBMIT-7` `position_analysis_moves` rank 1 is the best move, and a rank-1
  lookup through its record is served by the `(record_id, rank)` index. This
  entry first asked for a partial index on rank 1 used by a dashboard
  aggregate; the schema removed both on purpose (see the migration's comment on
  `position_analysis_moves_record_idx`) — the aggregate is gone and every rank-1
  read goes through its record — so the test pins the index that exists and
  that the planner uses it. *(Covered:
  `submissions::the_best_move_is_rank_one_and_is_read_through_the_record_index`.)*
- `I-SUBMIT-9` A captured position a solver decided keeps its analysis
  (`peg`, `endgame`) and each move's projected spread and ranking depth -- a
  pre-endgame move with its win percentage -- and a position whose moves do
  not match its analysis (an endgame position with two moves, a pre-endgame
  move without a spread, a static one with a spread, an analysis MAGPIE does
  not write) is refused. *(Covered:
  `submissions::solved_positions_keep_their_analysis_spread_and_depth`.)*
- `I-SUBMIT-8` An opening-rack result stores one record per requested rack, and
  a result naming a rack the task did not dispatch is rejected — as is one that
  leaves a requested rack out. *(Covered:
  `worker_api::an_opening_rack_result_must_answer_the_racks_it_was_given`,
  `worker_api::a_batched_opening_rack_submission_keeps_each_racks_own_moves`.)*

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
- `I-LEAVE-2b` **A purge waits for a running merge rather than deadlocking with
  it.** A merge takes the staged rows and then the per-rack rows; a purge that
  deleted them the other way round stopped on a rack the merge had updated
  while holding racks the merge had yet to reach, and Postgres failed one of
  the two. Purge and delete take the job's merge lock before anything else.
  *(Covered:
  `leave_gen::a_purge_waits_for_a_running_merge_instead_of_deadlocking_with_it`,
  which fails against a purge without the lock: the purge is the deadlock's
  victim and deletes nothing.)*
- `I-LEAVE-2c` **A completed leave job's corpus includes what was still
  staged.** Force-completed mid-generation, a job holds accepted results no
  merge has folded in; its stream and its export settle it first. *(Covered:
  `leave_gen::a_completed_leave_jobs_corpus_includes_what_was_still_staged`.)*
- `I-LEAVE-3` Rack selection picks the racks furthest below target, skips racks
  an open claim is already forcing, and returns nothing once all are at target
  with no claim in flight. *(Covered:
  `leave_generation::selection_hands_out_the_racks_furthest_below_target_first`,
  `leave_generation::nothing_is_handed_out_once_every_rack_is_at_target`,
  `leave_gen::racks_out_with_an_open_claim_are_not_handed_out_again`.)*
- `I-LEAVE-3a` **Selection by sweep.** While more than
  `SWEEP_WHILE_TASKS_REMAIN` tasks' worth of racks are below target, racks are
  handed out in primary-key order from `leave_selection_cursors`, with no list
  of what is out: twelve tasks take the first twelve racks, none twice, with
  all twelve results still staged, and a merge does not move the cursor. A lap
  that runs off the end of the universe deletes its cursor with its last task,
  hands out nothing while a claim of the lap is still open, asks for a merge
  once only staged results remain, and then selects on exact counts -- a rack
  its result left short is forced again. A pass from the top that finds nothing
  below target, with nothing in flight or staged, starts the transition.
  *(Covered: `leave_gen::a_sweep_hands_out_the_racks_in_order_whatever_is_staged`,
  `leave_gen::a_lap_ends_with_its_results_in_and_merged_before_the_next_begins`,
  `leave_gen::a_sweep_that_finds_nothing_below_target_closes_the_generation`.
  The other leave tests run two racks a task, which keeps the 149-rack test
  universe under the threshold and on lowest-count-first selection.)*
- `I-LEAVE-4` Generation transition folds progress into a KLV, uploads it,
  records the digest, and marks the generation complete. *(Covered, tier 6
  opt-in: `magpie_leave::a_transition_folds_the_generation_into_the_klv_it_uploads_and_closes_it`.)*
- `I-LEAVE-5` `ON CONFLICT DO NOTHING` on the artifact row keeps the **first**
  digest, so a racing transition cannot rewrite history. *(Covered:
  `leave_generation::a_generations_first_digest_is_the_one_kept`.)*
- `I-LEAVE-6` Generation 0's zeroed KLV exists at job creation and sums to
  exactly zero. *(Covered, tier 6 opt-in:
  `magpie_leave::generation_zeros_klv_exists_at_creation_and_is_worth_exactly_nothing`,
  `magpie_routes::a_leave_job_is_created_inactive_with_its_generation_zero_leaves_stored`.)*
- `I-LEAVE-7` Every dispatched generation carries a non-null
  `previous_artifact_key`, including generation 1. *(Covered:
  `leave_generation::every_dispatched_generation_carries_its_predecessors_klv`.)*
- `I-LEAVE-8` **`rebuild_artifacts` reproduces bytes.** `run_transition` and
  `rebuild_artifacts` share `generation_klv` precisely so a rebuild cannot
  drift; fold, rebuild, compare digests. And a forced rebuild that writes
  different bytes records them as `served_sha256` — what workers are told to
  check the object against — while `sha256` keeps the first hash; and a check
  that rewrites nothing still sets it from the object the key holds, so an
  older object version copied back is served under its own hash again
  (fourteenth audit). *(Covered,
  tier 6 opt-in: `magpie_leave::a_rebuild_reproduces_every_generations_bytes`.)*
- `I-LEAVE-9` A job listing more than one generation's target advances to the
  next generation and finishes after the last. *(Covered:
  `leave_generation::a_two_generation_job_advances_and_finishes_after_its_last`;
  on real KLVs, `magpie_leave::a_two_generation_job_runs_to_completion_on_real_klvs`.)*
- `I-LEAVE-10` The task's `num_games` is the only termination condition — the
  rack target is not sent to the worker. *(Covered:
  `leave_generation::the_rack_target_is_not_sent_to_the_worker`.)*
- `I-LEAVE-11` **One transition per generation.** With every rack at target and
  no claim in flight, the first claim decision starts the transition and every
  later one is told there is no work yet, rather than starting a second fold of
  millions of rows. *(Covered:
  `leave_gen::only_one_claim_starts_a_generations_transition`.)*
- `I-LEAVE-12` **A transition that never finished is taken over**, once past the
  takeover timeout, and the takeover is recorded in `attempts`; a *completed*
  transition is never restarted however old it is. *(Covered:
  `leave_gen::a_transition_that_never_finished_is_taken_over`.)*
- `I-LEAVE-12a` **A restart hands an open transition to the next claim.** A
  transition runs on a spawned task, so one left open at startup belongs to a
  process that is gone; it is released there and then, not after the takeover
  timeout, and a completed one is left alone. *(Covered:
  `leave_gen::a_restart_hands_an_open_transition_to_the_next_claim`.)*
- `I-LEAVE-13` **The transition owner's row is committed** before the transition
  runs -- the claim transaction that decides a generation is complete commits
  rather than rolls back, or the row that stops a second transition would be
  discarded -- and a transition that *fails* hands ownership back immediately
  instead of waiting out the takeover timeout -- including one that fails
  reading the job's config or distribution before it starts, which kept
  ownership for the half hour until the thirty-first audit. *(Covered:
  `leave_gen::the_transition_owner_is_committed_before_the_transition_runs`,
  `leave_gen::a_transition_that_fails_before_it_starts_hands_ownership_back`.)*
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

- `I-LEAVE-16` **The public results feed of a leave job tiles it.** Read a
  generation at a time through the primary key rather than as one sort of
  every progress row, the pages return every row once, newest generation first
  and in the database's `rack` order within one, across the boundary between
  two generations. *(Covered:
  `leave_gen::the_leave_results_feed_pages_through_every_generation_in_order`.)*
- `I-LEAVE-16b` A made-up cursor on the leave results feed costs a page, not a
  walk: the feed stepped down from the cursor's generation one empty read at a
  time, so a cursor naming generation 2,147,483,647 held a display-pool
  connection for as long as the client waited. It now goes to the next
  generation with rows in one probe. *(Covered:
  `leave_gen::a_made_up_generation_in_a_cursor_costs_one_page`.)* (Twenty-fifth
  audit.)
- `I-LEAVE-17` **A count no game could produce is refused, because staged it
  wedges the generation.** A result reporting `i64::MAX` occurrences is a `400`
  and stages nothing, and an honest result for the same claim is then accepted
  and merges; the same number staged by hand makes `merge_staged` fail, twice
  running. *(Covered:
  `leave_gen::a_count_no_game_could_produce_is_refused_before_it_can_wedge_the_merge`.)*
- `I-LEAVE-18` **A missing generation-0 KLV heals.** A leave job whose zeroed
  KLV was never written (its build failed after creation or a purge committed)
  builds it on the next claim, one build at a time, and then dispatches.
  *(Covered, tier 6 opt-in:
  `magpie_leave::a_missing_generation_zero_klv_is_built_by_the_next_claim`.)*
  (Fifteenth audit.)
- `I-LEAVE-19` **A generation turns to its tail only where a lap would start**
  -- nothing in flight, nothing staged -- and stays there: a summary under the
  threshold mid-lap leaves the sweep running, a lap's end with claims out waits
  for them, and the tail, once begun (a cursor row with no rack), carries on
  with its own claims out. *(Covered:
  `leave_gen::a_generation_turns_to_its_tail_only_where_a_lap_would_start`.)*
  (Thirty-first audit: it turned mid-lap, and each claim then hashed every sweep
  claim's racks inside the dispatch lock, 0.45 s a claim at a thousand workers.)
- `I-LEAVE-20` **A rack counts however the worker spells it.** MAGPIE writes a
  rack's letters in its machine-letter order with blanks last (`AEINST?` for
  the forced `?AEINST`); matched exactly, every blank rack went uncounted and an
  English generation never closed. The whole universe forced in one task and
  reported reversed is counted rack for rack; one rack under two spellings in
  one result is refused as a duplicate. *(Covered:
  `leave_gen::a_rack_counts_however_the_worker_spells_it`,
  `jobs::leave_gen::tests::a_reported_rack_is_spelled_as_the_universe_spells_it`
  (German's `Ä` too), and tier 6's `M-4` against a real MAGPIE.)* (Thirty-second
  audit.)
- `I-LEAVE-21` **A merge sums what is staged in passes, exactly.** A pass per
  400,000 racks of the generation, each over a slice of the racks by hash, so
  a pass's hash table fits in memory however much is staged; here slices of
  about fifty over 149 racks, 110 results, three passes, every rack's total
  exact. *(Covered:
  `leave_gen::a_merge_of_a_backlog_sums_it_in_passes_exactly`.)* (Thirty-second
  audit.)
- `I-LEAVE-22` A merge takes one of the process's two merge turns only once it
  holds its job's lock, so merges waiting on one job leave every other job
  free to merge. *(Covered:
  `leave_gen::merges_waiting_on_one_job_leave_other_jobs_free_to_merge`.)*
  (Thirty-second audit.)
- `I-LEAVE-23` A declined leave task is reissued as it stands, the decliner
  included, and no rack is forced by two open claims: skipped for its
  decliner, the claim went to rack selection, which forced the same racks
  again on a second claim. *(Covered:
  `leave_gen::a_declined_leave_task_is_reissued_as_it_stands`.)*
  (Thirty-second audit.)
- `I-LEAVE-24` **Each generation closes at its own target.** Targets of 10 then
  30: every rack at 10 closes generation 1 and not generation 2, whose racks
  are still handed out and whose summary counts a rack at target only at 30;
  every rack at 30 closes generation 2 and completes the job. The job page's
  stats report the current generation's target and the whole list. *(Covered:
  `leave_generation::each_generation_closes_at_its_own_target`;
  `stats::leave_stats_report_the_current_generations_racks_against_its_universe`,
  `public_api::job_detail_carries_the_stats_block_of_its_type`.)*
- `I-LEAVE-25` **The universe probe reads an index whatever the generation's
  history.** Every leave claim asks whether its generation's universe exists,
  inside the dispatch lock; as an `EXISTS` it was a sequential scan through
  every older generation's rows once the generation was in the statistics, and
  so was the tail's "any rack below target". Two analysed generations of 30,000
  racks (where the `EXISTS` form plans a `Seq Scan`): both probes for the later
  one are index scans of the primary key and the pick index, in a custom plan
  and a generic one, and answer correctly. *(Covered:
  `leave_gen::the_universe_probe_reads_an_index_whatever_the_generations_history`.)*
  (Thirty-third audit, pass 3.)

### `I-RATE-*` — rating pools (`ratings.rs`)

`build_matrix` was checked by hand against a live database; these make it
permanent.

- `I-RATE-1` A matching `game_pairs` job's results enter the matrix. *(Covered:
  the control job every `I-RATE-2` test fits against
  (`ratings.rs`'s `assert_only_the_control_counted`), and
  `ratings::a_head_to_head_counts_pairs_and_scores_half_points_over_four`.)*
- `I-RATE-2` Excluded: wrong `variant`, wrong `letterdist_id`, wrong `layout_id`,
  a job whose player is not a pool member, a plain `games` job, and a self-play
  job (both seats one config), which is also left out of the run's
  `pairs_used` and `jobs_used`. One test per exclusion, because each is a
  separate clause. *(Covered:
  `ratings::a_pairs_job_in_another_variant_is_not_evidence`,
  `ratings::a_pairs_job_on_another_letter_distribution_is_not_evidence`,
  `ratings::a_pairs_job_on_another_board_layout_is_not_evidence`,
  `ratings::a_pairs_job_against_a_non_member_is_not_evidence`,
  `ratings::a_plain_games_job_is_not_evidence`,
  `ratings::a_self_play_job_is_not_counted_as_evidence` (thirty-second audit).)*
- `I-RATE-3` The pair is the unit: a head-to-head's `games` equals pairs, not
  games, and its score is the half-point total over four. *(Covered:
  `ratings::a_head_to_head_counts_pairs_and_scores_half_points_over_four`.)*
- `I-RATE-4` `recompute` writes one `rating_runs` row and one
  `player_config_ratings` row per member, with `is_anchor` set on exactly one.
  *(Covered: `ratings::a_fit_stores_one_run_and_one_rating_per_member`.)*
- `I-RATE-5` The anchor's stored rating equals `anchor_rating` exactly.
  *(Covered: `ratings::the_anchor_is_stored_at_exactly_its_anchor_rating`; it
  was stored as 1837.2499999….)*
- `I-RATE-6` A member with no path to the anchor is stored with
  `connected_to_anchor = false`. *(Covered:
  `ratings::a_member_with_no_path_to_the_anchor_is_stored_unconnected`.)*
- `I-RATE-7` Adding a member changes other members' ratings, and removing one
  changes them back — the property that makes a refit necessary. *(Covered:
  `ratings::membership_changes_move_everyone_and_removal_moves_them_back`.)*
- `I-RATE-8` Removing the anchor is refused. *(Covered: the route by
  `A-RATE-5`; the fit itself refuses a pool whose anchor has gone, by
  `ratings::a_pool_whose_anchor_is_not_a_member_is_refused_a_fit`.)*
- `I-RATE-9` `recompute_stale` refits a pool whose evidence grew and skips one
  whose `pairs_used` is unchanged. *(Covered:
  `ratings::the_sweep_refits_only_the_pools_whose_evidence_grew`.)*
- `I-RATE-9b` It also refits a pool whose membership changed without a refit —
  a config added or removed with no pairs in the pool leaves `pairs_used` where
  it was — and then leaves it alone. *(Covered:
  `ratings::the_sweep_refits_a_pool_whose_membership_changed_without_a_refit`.)*
  (Eleventh audit.)
- `I-RATE-10` Two pools with different scopes over the same jobs produce
  different, internally consistent fits. *(Covered:
  `ratings::pools_of_different_scopes_fit_different_consistent_ratings`.)*
- `I-RATE-11` A pool with one member (the anchor) and no games produces a run
  rather than an error. *(Covered:
  `ratings::a_pool_of_only_its_anchor_fits_to_an_empty_run`.)*
- `I-RATE-13` A fit stores each head-to-head's cross-table cell once, from
  the side of the config whose name sorts first, summing every job between
  the two whichever seats them: the score, the error from the pairs' score
  variance in the summed pentanomial (√(491/128000) for [1, 3, 8, 4, 4]) and
  the games-weighted spread. *(Covered:
  `ratings::a_head_to_head_is_stored_once_with_both_seatings_summed`.)*
- `U-RATE-1` to `U-RATE-3` The aggregation, without a database: two jobs
  seating the configs either way sum to the same pentanomial from one side,
  with the hand-computed mean 47/80, error √(491/128000) and spread 6.8, and
  from the other side the mirror image; twenty split pairs have an error of
  0, where counting games would give ≈7.9 points of win %; and a stored cell
  is served from both sides, the second its mirror (1 − the score and the
  prediction, the same error, −the spread). *(Covered: `ratings::tests`.)*
- `I-RATE-12` A pool deleted after the sweep listed the pools is skipped
  without an error logged, and the sweep goes on to the next. *(Covered:
  `ratings::the_sweep_skips_a_pool_deleted_mid_sweep_quietly`, which deletes
  the pool while the sweep waits on its fit lock and counts ERROR events.)*

### `I-STATS-*` — dashboard aggregates (`jobstats.rs`)

- `I-STATS-1` A `games` job's stats sum every result and run the match test
  over games. *(Covered:
  `stats::a_games_jobs_stats_sum_every_result_and_test_the_games`, with the
  interval checked against `U-STATS-1`'s independently computed value.)*
- `I-STATS-2` A `game_pairs` job's stats sum the pentanomial, and
  `units_completed` equals the pair count — **not** the divergent count.
  *(Covered: `stats::a_pairs_jobs_stats_sum_the_pentanomial_and_count_every_pair`.)*
- `I-STATS-3` `divergent_pairs` is reported and is not what the match test
  consumed.
  The divergent games also get a match score of their own -- their wins,
  losses and draws, and each player's mean score weighted by the divergent
  games, from the means MAGPIE reports for the subset (`game_results.
  divergent_p1_score_mean` / `_p2_`) -- which the job page shows beside the
  full one. *(Covered: `stats::divergent_pairs_are_reported_but_not_tested`.)*
- `I-STATS-1b` Each player's average score and the spread are the batches'
  means weighted by their games, for games and pairs jobs alike (10 games at
  400-380 and 30 at 440-450 are 430-432.5, spread -2.5; unweighted, 420-415).
  *(Covered: `stats::average_scores_weight_each_batch_by_its_games`; how the
  match score table prints them, `matchScore.test.ts`, `F-MATCH-1`.)*
- `I-STATS-4` A job with no results reports zeros and an even score whose
  interval is every score (0 to 1), not an error or a NaN, and no average
  score (null, not 0). *(Covered:
  `stats::a_job_with_no_results_reports_zeros_not_nan`.)*
- `I-STATS-5` Opening-rack stats count analysed racks against `total_racks`, from
  the running `jobs.racks_analyzed` total. *(Covered:
  `worker_api::analysed_racks_are_counted_as_they_arrive`.)*
- `I-STATS-5b`, `I-STATS-5d` *(Retired with redundancy: a task has one slot,
  so it has one result and nothing to count twice.)*
- `I-STATS-5c` A purge zeroes the running totals, the rack consensus ones and
  `opening_rack_progress` included. *(Covered:
  `admin_api::a_job_can_be_purged_and_its_dispatch_counter_resets`.)*
- `I-STATS-6` Leave-generation stats report racks at target against the
  universe, and the current generation. *(Covered:
  `stats::leave_stats_report_the_current_generations_racks_against_its_universe`.)*
- `I-STATS-7` `worker_contributions` attributes tasks to the right identity and
  totals correctly across both identity types. *(Covered:
  `stats::contributions_are_attributed_to_each_identity_across_both_kinds`,
  `worker_api::contributions_are_counted_as_they_arrive`.)*
- `I-STATS-7b` **A contributor is credited with each claim it completed**: the
  time the claim was held, claim to submission, in whole milliseconds, and the
  movegens its submission reported. Two workers, a task each, are each credited
  their own movegens (and the job both tasks' games); a claim held 90 s is
  credited 90 s; each of two opening-rack claims, and a leave task, is credited
  what it reported; each claim keeps its own movegens. *(Covered:
  `worker_api::each_accepted_claim_credits_its_time_and_movegens_to_its_contributor`,
  `worker_api::analysed_racks_are_counted_as_they_arrive`,
  `leave_generation::nothing_is_handed_out_once_every_rack_is_at_target`.)*
- `I-STATS-7d` **A submission must report plausible movegens**: left out,
  negative, fractional, past `i64` or more than a million per millisecond the
  claim was held, it is a `400` naming `movegens`, and stores nothing -- the
  claim stays open, the task unfinished, the contributor uncredited -- and a
  corrected submission is then accepted. *(Covered:
  `worker_api::a_result_without_plausible_movegens_is_refused_and_stores_nothing`,
  `routes::worker::movegens_tests::movegens_is_a_whole_number_in_i64_and_every_refusal_names_it`;
  every contract fixture's result reports a positive count, checked in
  `routes::worker::contract_fixtures`.)*
- `I-STATS-7c` **A purge or a delete gives back every contributor counter** the
  job's claims added -- tasks, compute time and movegens -- to the
  millisecond, summed from the claims it is about to delete with the
  submission's own expression (`CLAIM_COMPUTE_MS`), including a submission that
  landed mid-purge. *(Covered:
  `admin_api::purging_and_deleting_a_job_give_back_what_it_earned`,
  `admin_api::a_purge_waits_for_a_submission_in_flight_before_counting_contributions`.)*
- `I-STATS-8` ETA is `None` without recent throughput rather than infinity.
  *(Covered: `stats::the_eta_is_none_without_recent_throughput`.)*
- `I-STATS-8b` A games job's ETA is the units left at claims an hour × batch.
  *(Covered: `stats::the_games_eta_is_claims_times_the_batch`.)* (Sixteenth
  audit.)
- `I-STATS-8c` A job activated less than an hour ago is measured since its
  activation (at least a minute), not over the hour: ten minutes in, the hour's
  average read six times the real time left. *(Covered:
  `stats::a_new_jobs_eta_is_measured_since_it_was_activated`.)* (Twenty-second
  audit.)
- `I-STATS-8d` **A job's throughput** is the rate its ETA extrapolates, an
  hour, in its own unit: claims in the window × the batch, as games (10 a
  batch, two claims: 20 games/hour), pairs, rack analyses, or a leave job's
  games (`num_iterations` a task; it has a pace and no ETA). Nothing finished
  in the last hour, or a job that is not active, has none. *(Covered:
  `stats::throughput_is_recent_claims_times_the_batch_in_the_jobs_unit`.)*
- `I-STATS-9` **The finish check** completes a job once its match test
  decides and at the hard cap (inconclusive), and does **not** complete below
  `min_units` even with an interval clear of an even score. There is no
  `finish_if_done`, as this entry first named it: the check is
  `after_submission` in `routes/worker.rs`, which a submission runs on every
  `TEST_CHECK_EVERY`th (8) result for a job, or whenever it leaves the job
  nothing in flight — so it is tested through the worker API. *(Covered:
  `stats::a_job_completes_when_its_test_decides_at_min_games`,
  `stats::a_job_completes_at_its_hard_cap_inconclusive`,
  `stats::a_decided_interval_below_min_games_does_not_complete_the_job`; the
  interval itself by `U-STATS-1`, `-3`, and its error rate under repeated
  checks by `U-STATS-3b`.)*
- `I-STATS-9e` The verdict a job completed on is stored with the completion —
  status, player 1's interval (lower and upper) and units — and reported
  beside the live figures; a result in flight at completion lands and moves
  the live interval, and the stored verdict stays. *(Covered:
  `stats::a_job_completes_on_the_batch_that_decides_and_not_before`,
  `stats::a_job_whose_player_2_is_better_completes_saying_so`,
  `finish::under_steady_load_the_finish_check_runs_on_every_nth_submission`.)*
  (Eleventh audit; the in-flight half, twelfth.)
- `I-STATS-9f` A completed job's stats say how it was completed (`completion`:
  when, whether an admin forced it, and the server's reason — the match
  test's verdict (`player1_better`, `player2_better`, `inconclusive`),
  `reached_target` for a games or pairs job without a test, `last generation
  built`, or none when an opening-rack job's racks ran out),
  from its `job.completed` audit row; a job not completed has none. The job
  pages turn it into "Finished …: …" under the title. *(Covered:
  `finish::under_steady_load_the_finish_check_runs_on_every_nth_submission`,
  `finish::an_opening_rack_job_completes_once_its_racks_are_handed_out_and_all_accepted`,
  `finish::a_forced_completion_is_reported_as_forced`; the sentences,
  `format.test.ts` `F-FMT-12`.)*
- `I-STATS-9a` **Debounced under load.** With a claim held open the whole time,
  so the job is never idle, the check runs on the `TEST_CHECK_EVERY`th
  submission and not before, and the open claim's result is still accepted
  after completion. *(Covered:
  `finish::under_steady_load_the_finish_check_runs_on_every_nth_submission`.)*
- `I-STATS-9b` **When it decides, not before, and for player 2.** With a
  floor of 100 and a cap of 10,000, 68-32 over 100 games (lower bound 0.4937)
  leaves the job active; another 56-44 (124-76 over 200, 0.5035 to 0.7365)
  completes it; 28-72 (upper bound 0.4631) completes it with player 2 found
  better. *(Covered:
  `stats::a_job_completes_on_the_batch_that_decides_and_not_before`,
  `stats::a_job_whose_player_2_is_better_completes_saying_so`.)*
- `I-STATS-9c` **An opening-rack job completes** once its rack space is handed
  out and every task accepted, and a declined task keeps it active until that
  task is done. *(Covered:
  `finish::an_opening_rack_job_completes_once_its_racks_are_handed_out_and_all_accepted`,
  `finish::a_declined_opening_rack_task_keeps_its_job_active_until_it_is_done`.)*
- `I-STATS-9d` A finish check overtaken by a purge does not complete the job
  -- by either witness: the claim counter below what was observed, or a purge
  counted meanwhile however many claims followed it (nineteenth audit) -- and
  a games or game-pairs job hands out nothing past its cap. *(Covered:
  `admin_api::a_finish_check_overtaken_by_a_purge_does_not_complete_the_job`,
  `worker_api::games_jobs_hand_out_nothing_past_their_cap`.)*
- `I-STATS-9i` **A job with nothing left to hand out and nothing in flight
  completes on the next claim.** An opening-rack job, and a games job at its
  cap, deactivated while their last tasks were out: the results land with no
  finish check, and after reactivation the first claim that finds the job empty
  completes it (off the request, at most every ten seconds per job, at once
  after an activation). *(Covered:
  `finish::a_job_whose_last_results_landed_while_inactive_completes_once_reactivated`,
  `finish::a_games_job_at_its_cap_whose_results_landed_while_inactive_completes`.)*
  (Thirty-second audit.)
- `I-STATS-9j` **The opening-rack finish check reads indexes, not every
  job's tasks.** It runs on the submission that settles the job's last rack,
  before the worker is answered; as `EXISTS` a task and `NOT EXISTS` one not
  completed it was two sequential scans of `tasks`, whose rows every job keeps.
  With 30,000 of an older job's tasks laid down first, a completed claim each,
  and analysed (where the old form plans a `Seq Scan on tasks`), its plan is
  index scans of
  `tasks_seed_unique_idx`, `tasks_queue_idx` and `task_claims_open_idx`, custom
  and generic; it answers done with every task completed, and not done with a
  task available, a task claimed, or no task at all. *(Covered:
  `finish::the_opening_rack_finish_check_reads_indexes_not_every_jobs_tasks`.)*
  (Thirty-third audit, pass 3.)
- `I-STATS-9k` **The "anything still in flight" probe reads the open claims,
  not the job's tasks.** The finish checks, an export of a completed job, the
  job list's `stalled` and leave generation's in-flight reads ask it of
  `task_claims.job_id`; joined to `tasks` for the job, it probed `tasks` once
  per open claim in the fleet, and without statistics scanned every task the
  job ever had. With 30,000 of a job's tasks and another job's 200 open
  claims, and the two tables' statistics removed (where the old form reads
  `tasks`), the probe's plan is `task_claims_open_idx` with no `tasks`, custom
  and generic, with statistics and without, and it answers for both jobs.
  *(Covered:
  `finish::the_in_flight_probe_reads_the_open_claims_not_the_jobs_tasks`.)*
  (Thirty-third audit, pass 4.)
- `I-STATS-9g` **A job without a match test plays to its target.** A 90-10
  batch past its floor, which completes a job with a test, leaves it active
  and reports no test (`games.test` null); the batch that reaches `max_games` completes it with
  no verdict stored and `reached_target` as the reason, on a submission and on
  the idle path alike. A config row that says nothing of the test runs none.
  *(Covered: `stats::a_job_without_a_test_completes_at_its_target_and_not_before`,
  `finish::a_games_job_without_a_test_completes_at_its_target_once_reactivated`,
  `public_api::job_detail_carries_the_stats_block_of_its_type`.)*
- `I-STATS-10` A stats build takes one connection from its pool, not one per
  statement (eight): on a saturated display pool a build waited out the
  acquire timeout once per statement and answered in tens of seconds, where
  one wait (two, with the job page's own read of the job) makes it a quick
  `503`. *(Covered:
  `stats::a_stats_build_takes_one_connection`.)* (Thirty-second audit.)
- `I-STATS-10b` Viewers waiting on one job's build that fails are all told
  busy when it does: each retried it in turn, and on a saturated pool the k-th
  waited k acquire timeouts (six took six seconds at a one-second timeout; now
  about one). *(Covered:
  `stats::viewers_waiting_on_a_failed_build_are_answered_together`.)*
  (Thirty-second audit.)
- `I-STATS-11` The stats payload cache serves a payload until it expires or is
  forgotten (every admin action), and a build reads the job's row itself, so a
  copy read before an admin action is not cached as newer than it. *(Covered:
  `stats::the_stats_cache_follows_admin_changes`.)* (Seventeenth audit.)

### `I-INPUT-*` — input data import (`inputdata.rs`)

The archive walk is unit-tested (`U-ARCHIVE-*`); these are the database and
object-store half, against a GitHub played by a small HTTP server in the test
(the config's `github_api_url`/`github_raw_url` point at it; it serves a
tarball whole or in `split` chunks, and can hold a chunk back so a running
import can be watched) and a per-test MinIO bucket.

- `I-INPUT-1` A staged import inserts `input_data_import_rows` and no
  `input_data` rows until confirmed. *(Covered:
  `input_data::a_staged_import_writes_its_diff_and_no_input_data`.)*
- `I-INPUT-2` Confirming inserts `input_data` rows, dedupes by `(path, sha256)`,
  and reports what was new versus already present. *(Covered:
  `input_data::confirming_inserts_what_is_new_and_reports_what_was_already_there`.)*
- `I-INPUT-3` Only server-read roles (`letterdist`, `layout`) keep their bytes
  in the row; `kwg`/`klv`/`winpct` store a digest and NULL content. Enforced by
  the CHECK — assert the CHECK fires, not just that the code does it. *(Covered:
  `input_data::only_server_read_roles_keep_their_bytes_and_the_schema_insists`,
  `inputdata::tests::only_server_read_roles_keep_their_bytes`.)*
- `I-INPUT-8` `kwg` and `klv` rows carry an `object_key` and their bytes are in
  the object store, because the server builds wordmaps, rack info tables and
  word info tables from them (a word info table needs only the `kwg`); `winpct` rows carry neither, because nothing server-side builds
  anything from a win% model. The key is the digest, so re-importing a tarball
  whose lexica have not changed uploads nothing. *(Covered:
  `input_data::lexica_and_leaves_are_stored_once_by_digest`,
  `inputdata::tests::the_roles_a_derived_build_needs_go_to_the_object_store`.)*
- `I-INPUT-8b` A lexicon object whose bytes are not those imported is deleted
  by the build that finds it, and the next import uploads it again whole. An
  import skips an object that exists, so while the damaged one stayed, the
  build's remedy (import again) changed nothing. *(Covered:
  `input_data::a_damaged_lexicon_object_is_replaced_by_the_next_import`.)*
  (Thirty-second audit.)
- `I-INPUT-4` A second import of the same tarball is a no-op: every file is
  `known`, so the import goes straight to `nothing_new` — not `staged`, where it
  offered an Insert of 0 rows and waited a day to be expired — audited as
  `input_data.import_nothing_new` under the admin who started it, refused a
  confirmation, and never swept as unconfirmed. *(Covered:
  `input_data::a_second_import_of_the_same_tarball_is_a_no_op`.)*
- `I-INPUT-5` `fail_orphaned_imports` fails a row left `running` by a restart
  and leaves `staged`, `nothing_new` and `confirmed` rows alone. *(Covered:
  `input_data::a_restart_fails_running_imports_and_leaves_the_rest`.)*
- `I-INPUT-6` Deleting an `input_data` row referenced by a player config or job
  is refused. One that only `derived_data` refers to — a wordmap built from it
  — is deleted, and takes that derived data with it; it used to be refused
  with a bare foreign-key 409 while the page reported nothing using it.
  *(Covered: `input_data::an_input_file_in_use_cannot_be_deleted`,
  `input_data::a_file_only_derived_data_refers_to_can_be_deleted_and_takes_that_data_with_it`.)*
- `I-INPUT-7` A failed import records its error and stages nothing. *(Covered:
  `input_data::a_failed_import_records_its_error_and_stages_nothing`.)*
- `I-INPUT-9` An import staged and never confirmed expires after a day and says
  so. *(Covered: `admin_api::an_unconfirmed_import_expires_after_a_day_and_says_so`.)*
- `I-INPUT-10` An import has a time limit, download to staged (PLAN.md's "Whole
  task"), and one that outlasts it fails and says so. *(Covered:
  `input_data::an_import_that_outlasts_its_time_limit_fails`.)* (Thirty-first
  audit: the client's timeouts are per read, and a trickling download kept its
  import `running` as long as it lasted.)

### `I-AUDIT-*` — audit log (`audit.rs`)

- `I-AUDIT-1` Each helper writes the actor, target type, target id and job id it
  was given. *(Covered:
  `audit::each_audit_helper_records_who_did_what_to_which_target`.)*
- `I-AUDIT-2` A log written inside a rolled-back transaction does not persist —
  the audit trail cannot claim something that did not happen. *(Covered:
  `audit::an_audit_row_in_a_rolled_back_transaction_does_not_persist`.)*
- `I-AUDIT-3` Every destructive admin action writes exactly its record: one
  row naming what it did, who did it and to what — except the four that
  destroy recorded work (purge, job delete, user delete, rating-pool delete),
  which write that row *and* their census (`I-JOB-10`), exactly that pair, and
  an anchor move that brings a new member in, which writes that addition too.
  The entry first said "exactly one row"; the census is the second on purpose.
  *(Covered: `audit::every_destructive_admin_action_writes_exactly_its_record`,
  across a deactivation (an allocation change to 0%, its row carrying "50% ->
  0%"), complete, a consensus change, purge and delete of a job,
  user delete, ban,
  unban, input-file delete, player-config delete, pool-member removal, an
  anchor move and a pool delete. Deleting an input file or a player config
  wrote nothing.)*

### `I-ART-*` — object store (`artifacts.rs`)

Against a real MinIO (`TEST_S3_ENDPOINT`), one bucket per test.

- `I-ART-1` Put then get returns identical bytes, against MinIO. *(Covered:
  `artifacts::put_then_get_returns_identical_bytes`.)*
- `I-ART-2` Getting an absent key is a clean error, not a panic. *(Covered:
  `artifacts::getting_an_absent_key_is_a_clean_not_found`.)*
- `I-ART-3` A key is namespaced per job and generation, so two jobs cannot
  collide. *(Covered: `artifacts::two_jobs_and_two_generations_never_share_a_key`.)*
- `I-ART-4` With `S3_PUBLIC_ENDPOINT` set, a presigned download link is signed
  for that host and downloads there. *(Covered:
  `artifacts::a_download_link_is_signed_for_the_public_endpoint`.)*

### `I-EXPORT-*` — exports (`exports.rs`)

A job's corpus, written once to the object store: a completed job's final
corpus, or a snapshot of one still running (PLAN.md, "Exports"). The refusals were tested before; the success path — the objects,
their digests, the redirect, the purge — ran nowhere until `exports.rs`, which
runs against a real MinIO.

- `I-EXPORT-1` The export is the admin stream's corpus, as gzipped NDJSON
  behind a presigned URL with `row_count` and a digest recorded; a games job
  that captured positions gets a second object with its own count, size and
  URL, both under `exports/`. *(Covered:
  `exports::a_completed_jobs_export_is_its_stream_and_its_positions_behind_presigned_urls`.)*
- `I-EXPORT-2` A completed job's stream answers `303` to its newest ready
  final export, and with `?positions=true` to the positions object — until the
  export is older than the bucket keeps it (`EXPORT_LIFETIME_DAYS`), when the
  stream goes back to the database and the admin detail says `expired`.
  *(Covered:
  `exports::a_completed_jobs_stream_redirects_to_its_ready_export`.)*
- `I-EXPORT-3` A job with nothing captured exports one object, and an empty
  corpus is still a valid export — one gzip stream of nothing, `row_count` 0.
  *(Covered: `exports::an_export_of_a_job_with_nothing_captured_is_one_object`.)*
- `I-EXPORT-4` A purge deletes both objects and the row. *(Covered:
  `exports::a_purge_deletes_both_export_objects_and_the_row`.)*
- `I-EXPORT-5` At startup an export still `running` is failed with a reason;
  ready and failed ones are left alone; and an export reaped while its task
  still runs is not brought back `ready` when the task finishes, and removes
  the objects it uploaded (fifteenth audit: they were left for the lifecycle
  rule). *(Covered:
  `exports::startup_fails_exports_left_running_and_leaves_the_rest_alone`,
  `exports::an_export_reaped_while_it_ran_is_not_brought_back_ready`.)*
- `I-EXPORT-6` Any job can be exported, an active one with claims in flight
  included (it was "completed only"); a completed job not until its last
  claims have landed, and a claim whose worker vanished does not block it for
  ever. *(Covered: `admin_api::a_running_job_exports_a_snapshot`,
  `admin_api::a_completed_job_is_not_exported_until_its_claims_have_landed`,
  `admin_api::an_export_is_not_blocked_by_a_claim_whose_worker_vanished`.)*
- `I-EXPORT-7` The uploaded parts are one gzip stream of exactly the lines
  pushed, and the recorded digest and size describe those bytes. *(Covered:
  `exports::tests::the_parts_are_one_gzip_stream_of_what_was_pushed`.)*
- `I-EXPORT-8` One export of a job runs at a time, a running job's snapshot
  as much as a final export: a second request while one is `running` is a
  409. *(Covered:
  `exports::a_job_has_one_export_running_at_a_time`.)* (Thirteenth audit.)
- `I-EXPORT-9` A reader that stops early ends its corpus query: the connection
  is closed, not drained back into the pool. *(Covered for the results stream:
  `exports::a_reader_that_hangs_up_ends_its_corpus_query`. An export whose
  upload fails, and KLV generation, take the same `close_on_drop` and are not
  tested separately.)* (Thirty-first audit:
  each hang-up left Postgres building the whole corpus on a connection no cap
  counted; ten held a ten-connection pool, and every claim timed out.)
- `I-EXPORT-10` A results stream is complete exactly when it ends cleanly: a
  corpus query the database ends part-way (`pg_terminate_backend` after the
  first frame) ends the body in an error, where it ended as a finished
  download after 869 of 150,000 records. *(Covered:
  `exports::a_stream_the_database_cuts_off_ends_in_an_error`. The connection
  and a completed leave job's settle are taken before the response head, so
  their failures are a status; not tested separately.)* (Thirty-second audit,
  pass 20.)
- `I-EXPORT-11` An export that fails after writing its objects removes them:
  with marking the row ready made to fail (a trigger), the row says `failed`
  and the store holds nothing, where both objects were left for the
  thirty-day rule. *(Covered:
  `exports::an_export_that_fails_after_uploading_removes_its_objects`. A
  multipart upload that fails to complete is aborted, and a row that says
  `ready` after all keeps its objects; not tested separately.)* (Thirty-second audit, pass 20.)
- `I-EXPORT-12` A running job's export is a snapshot: ready and downloadable,
  `is_final` false with its `snapshot_at`, and never what the results stream
  serves — not while the job runs, and not once it has completed, when the
  stream reads the database until a final export is built and then redirects
  to that. The newest ready export was served whatever it was built from, so
  one taken mid-run became the completed job's corpus. *(Covered:
  `exports::a_running_jobs_export_is_a_snapshot_the_stream_never_serves`.)*
- `I-EXPORT-13` Whether an export is final is the job's state when its
  snapshot was taken: a job active then and completed while the export reads
  gives a snapshot, never redirected to. *(Covered:
  `exports::the_final_marker_is_the_jobs_state_when_its_snapshot_was_taken`.)*
- `I-EXPORT-14` The results and the captured positions are read in one
  snapshot: a result and its position committed between the two scans are in
  neither file. *(Covered:
  `exports::results_and_positions_are_read_in_one_snapshot`.)*
- `I-EXPORT-15` An export finishing while its completed job is being reopened
  (a consensus edit past its `unfinalize`, not yet committed) waits for the
  reopening and fails with it, removing its objects (a snapshot, as first
  fixed, until the second pass made `unfinalize` fail running exports:
  `I-EXPORT-16`). It read the job's committed `completed` and came back final
  for a job active again. *(Covered:
  `exports::an_export_finishing_while_its_job_reopens_fails`.)*
- `I-EXPORT-16` An export whose snapshot was read while its job was completed
  fails when a consensus edit reopens the job, even when the job has completed
  again before the export finishes; nothing redirects to it, and the next
  export is the completed job's final corpus. It was stored final — the corpus
  from before the edit's analyses, served as the completed job's until someone
  exported again — since `mark_ready` saw only the job's status at the end.
  *(Covered:
  `exports::an_export_spanning_a_reopening_and_a_second_completion_fails`.)*
  (Thirty-third audit, pass 2.)
- `I-EXPORT-17` **Whether a job captured positions is one index probe.** As
  `EXISTS` over the job's records it was a sequential scan of every older
  job's records first, for a job that did capture. With 30,000 of an older
  job's records and 4,000 of this one's, analysed (where the old form plans a
  `Seq Scan on position_analysis_records`), the probe is an index scan of
  `position_analysis_records_feed_idx`, custom and generic, and answers for a
  job with records and one without. *(Covered:
  `exports::whether_a_job_captured_positions_is_one_index_probe`.)*
  (Thirty-third audit, pass 4.)

### `I-DATA-*` — the pinned-row invariant

- `I-DATA-1` **Server-side reads use the pinned row.** Two `letterdist` rows
  with the same name and different bytes produce different `total_racks` for
  otherwise identical jobs. This is the one test that would catch the server and
  the worker disagreeing about the alphabet. *(Covered:
  `derived::two_distributions_with_one_name_size_two_different_rack_spaces`.)*
- `I-DATA-2` `seed_generation` and every MAGPIE conversion read the job's pinned
  distribution, not a filesystem path or a default. For the conversions this is
  structural — each runs in a throwaway directory written from
  `input_data.content` and the object store, and the distribution is stated on
  the command line rather than inferred from the lexicon's name — but a test
  that two distributions with one name produce two different derived hashes is
  what proves it. *(Covered: `leave_generation::seeding_reads_the_jobs_pinned_distribution_not_its_name`;
  and, tier 6 opt-in, `magpie_leave::two_distributions_with_one_name_build_two_different_klvs`
  — two KLVs rather than two wordmap hashes, the conversion the server runs
  for every generation.)*

### `I-DERIVED-*` — wordmaps, rack info tables and word info tables (`derived.rs`)

- `I-DERIVED-1` A job whose players ask for a wordmap queues exactly one
  `derived_data` row per (lexicon, distribution), however many players share
  the lexicon; one asking for a rack info table queues a `rit` row **and** the
  `wmp` row it is built from. *(Covered:
  `derived::a_shared_lexicon_queues_one_wordmap_and_a_table_queues_its_wordmap_too`.)*
- `I-DERIVED-2` A leave-generation job queues a wordmap and never a table.
  *(Covered: `derived::a_leave_job_queues_its_wordmap_and_never_a_table`,
  including when a player on the same lexicon elsewhere asks for a table; and
  as a precondition by every leave-job fixture in `leave_gen.rs` and
  `leave_generation.rs`, which asserts `derived_ready(job) == 1`.)*
- `I-DERIVED-3` A job with any unbuilt derived file is not dispatched, and the
  same job dispatches once the row says `built`. The single most important test
  here: without it, dispatching early carries no hash for a file its player
  asks for, which MAGPIE refuses (`derived_mismatch`), setting the job aside
  for the run on every worker that claims it. *(Covered:
  `worker_api::a_job_is_not_dispatched_until_its_derived_files_are_built`,
  `derived::tests::a_job_waits_for_anything_not_built`,
  `worker_api::a_dispatchable_jobs_hashes_are_remembered_for_the_process`; on a
  real MAGPIE, `M-10`.)*
- `I-DERIVED-4` A `failed` row blocks dispatch exactly as a `pending` one does.
  "Give up and send it anyway" is the wrong recovery and must be impossible to
  reach by accident. *(Covered:
  `worker_api::a_failed_derived_build_keeps_a_job_undispatched`.)*
- `I-DERIVED-5` Two builders cannot take the same row: the lease and
  `SKIP LOCKED` together. *(Covered:
  `derived::a_leased_row_is_left_to_its_builder_until_the_lease_lapses`,
  `derived::a_row_another_builder_is_taking_is_skipped_not_shared`.)*
- `I-DERIVED-6` A row queued under a builder version this binary does not have
  is left alone, not built — and blocks nothing behind it. Recording a hash
  against a builder that did not produce it is the failure this whole design
  exists to prevent. *(Covered:
  `derived::a_row_for_a_builder_this_binary_lacks_is_left_alone_and_blocks_nothing`;
  such a row at the head of the queue stopped every build behind it.)*
- `I-DERIVED-7` A build whose `kwg` row has neither bytes nor an object key fails
  with a message naming the remedy (delete the row and what pins it, and import
  again: re-importing alone leaves a known row as it is, which the message
  wrongly promised until the thirty-first audit), and is retried the bounded
  `MAX_ATTEMPTS` (3) times and then left `failed` — not retried forever, and
  not failed on the first attempt as this entry implied. Between attempts it
  waits, 5 and then 15 minutes: taken again at once it was still the oldest
  row, so one builder run spent all three in seconds and a passing S3 outage
  failed a build for good (thirty-second audit). *(Covered:
  `derived::a_build_from_a_lexicon_stored_before_object_keys_fails_naming_the_remedy`.)*
  A stalled input fetch ends after five minutes and is recorded as a failure
  (checked by hand against a listener that never answers: 300 s, the row back
  to `pending`; it used to hold the builder task for good).
- `I-DERIVED-8` A claim's `derived` entries name the table by
  `<lexicon>.<leaves>`, and two jobs on one lexicon with different leaves get
  two different names and two different hashes. *(Covered:
  `derived::two_jobs_on_one_lexicon_with_different_leaves_get_their_own_table`,
  `worker_api::a_rack_info_table_is_pinned_under_its_pairs_name`.)*
- `I-DERIVED-9` Each player's derived files come from that player's own rows:
  two players on one lexicon share a wordmap, two on different lexicons get one
  each. Nothing is shared between players, so this ought to fall out of the
  query — but comparing bots on two lexicons is a supported configuration that
  a wordmap keyed on the wrong player would silently break. *(Covered:
  `worker_api::players_on_different_lexicons_need_a_wordmap_each`; the shared
  case by `I-DERIVED-1`.)*
- `I-DERIVED-10` A job whose files were built under another builder -- after a
  deployment whose MAGPIE bumped a builder version -- has them queued under
  this binary's builder by the next claim that considers it. *(Covered:
  `worker_api::a_file_built_under_another_builder_is_queued_under_this_one_by_a_claim`.)*
  (Thirty-first audit: only creating or activating a job queued anything, so
  every such job answered `204` for good.)
- `I-DERIVED-11` A player asking for a word info table queues one `wit` row,
  named for its lexicon, with no leaves and no wordmap behind it; its job is
  not dispatched until it is built; then a claim pins its hash under `wit-1`
  and sets `use_wit` for that player only. A leave job's player may ask for
  one -- the table is the lexicon's, which no generation changes. *(Covered:
  `derived::a_word_info_table_is_the_lexicons_and_waits_like_the_others`;
  the build at tier 6, `magpie_smoke::a_word_info_table_is_built_from_the_lexicon_alone`
  and `M-13`.)*

---

## 3. API

The real Axum router in-process via `tower::ServiceExt::oneshot`, against a real
Postgres. No browser, no network, no server process. `tower` is already a direct
dependency, so this tier costs no new ones.

Shares tier 2's database harness; `birdtest::app(db.state().await)` is the
router, and `admin_headers(cfg, user)` signs a session cookie and CSRF pair for
a user without a login round trip. Tests that are *about* login go through
`/api/auth` like a browser, carrying cookies between requests.

**Every route in the table below needs at least an authorization test and a
happy path.** Where a route has interesting failure modes they are enumerated.

### `A-AUTHZ-*` — authorization, applied to every route

Write these as one table-driven test each rather than 50 separate functions.

The enumeration is a route table in `authz.rs`, and
`authz::the_route_table_is_every_route_the_router_serves` fails when the
router serves a route the table lacks, so a new route cannot escape the checks
below.

- `A-AUTHZ-1` Every `/api/admin/*` route returns 403 for an authenticated
  non-admin. Enumerate the routes from the router so a new one cannot be
  forgotten. *(Covered: `authz::every_admin_route_refuses_a_signed_in_non_admin`.)*
- `A-AUTHZ-2` Every `/api/admin/*` and `/api/me/*` route returns 401 for an
  anonymous caller. *(Covered: `authz::every_session_route_refuses_an_anonymous_caller`.)*
- `A-AUTHZ-3` Every mutating cookie-backed route rejects a request with no CSRF
  header, a mismatched one, and a missing cookie. *(Covered:
  `authz::every_cookie_backed_write_requires_the_csrf_pair`.)*
- `A-AUTHZ-4` `/api/worker/*` is CSRF-exempt by design — a worker sends a bearer
  token or `X-Worker-UUID`, neither of which a browser attaches automatically.
  *(Covered: `authz::worker_writes_need_no_csrf_token_even_alongside_session_cookies`.)*
- `A-AUTHZ-5` A session for a deleted user is rejected; a session for a demoted
  admin loses admin access without needing to expire. *(Covered:
  `authz::a_demoted_or_deleted_account_loses_access_without_its_session_expiring`.)*
- `A-AUTHZ-6` A revoked or deactivated API key is rejected. *(Covered:
  `authz::a_deactivated_or_revoked_api_key_is_refused_by_every_worker_endpoint`.)*

### `A-AUTH-*` — `routes/auth.rs`

- `A-AUTH-1` Register → confirm → login succeeds and sets a session cookie.
  The confirmation mail does not carry the username, the registrant's own
  text, to the address they typed (thirty-second audit).
  *(Covered: `auth_routes::register_confirm_and_login_gives_a_working_session`.)*
- `A-AUTH-2` An unconfirmed login is 403 with a message naming the fix.
  *(Covered: `auth_routes::an_unconfirmed_login_is_refused_with_the_fix_named`.)*
- `A-AUTH-3` **A wrong password and an unknown username return an identical
  401** — body and status both, so the endpoint cannot enumerate accounts.
  *(Covered: `auth_routes::a_wrong_password_and_an_unknown_username_answer_identically`.)*
- `A-AUTH-3b` Scoring a password does not hold the executor: a hundred
  characters of zxcvbn's substitution letters take it close to a second, and
  on the executor one address's registrations or resets stalled every request
  (`/health` 8.5 s). Scored off the executor, a single-threaded test
  runtime ticks throughout (a 1.08 s gap on the executor). A reset with a
  wrong link is refused before any scoring. *(Covered:
  `auth_routes::scoring_a_crafted_password_does_not_stall_the_server`.)*
  (Thirty-second audit.)
- `A-AUTH-3c` Scoring has turns of its own: sixteen crafted passwords being
  scored do not hold up a sign-in (3.9 s behind them on sign-in's Argon2
  turns), and each reset link buys five scorings an hour from any number of
  addresses, then `429`. *(Covered:
  `auth_routes::scoring_never_holds_up_a_sign_in_and_a_link_buys_few`.)*
  (Thirty-second audit.)
- `A-AUTH-4` **Registering a taken address returns the same body as a fresh
  registration.** Assert the bodies are byte-identical. *(Covered:
  `auth_routes::registering_a_taken_address_answers_exactly_like_a_new_registration`.)*
- `A-AUTH-4b` An account that never confirmed holds its address and username
  only while its confirmation link works: registering over it before then is
  answered like any taken address, with a notice saying an account is waiting;
  after, the next registration takes both. *(Covered:
  `auth_routes::an_expired_unconfirmed_account_gives_up_its_address_and_username`.)*
  (Eleventh audit.)
- `A-AUTH-4c` A username is taken whatever its case, and signs in whatever its
  case. *(Covered:
  `auth_routes::a_username_is_taken_and_signs_in_whatever_its_case`.)*
  (Thirteenth audit; the sign-in half, fourteenth.)
- `A-AUTH-4d` The auth and account routes refuse bodies over 16 KiB (`413`),
  and a rate-limit key longer than 128 bytes is kept as its digest. *(Covered:
  `auth_routes::an_oversized_auth_body_is_refused`,
  `ratelimit::key_tests::a_long_key_is_one_bucket_kept_small`.)* (Nineteenth
  audit.)
- `A-AUTH-4e` The notice to a confirmed address's owner names their account,
  and so does the reset mail: sign-in asks for a username, which someone who
  has forgotten they signed up, or their password, may have forgotten too.
  *(Covered: `auth_routes::the_owner_of_a_taken_address_is_told_their_username`,
  and the reset mail's in
  `auth_routes::a_reset_request_answers_the_same_for_known_and_unknown_addresses`.)*
  (Thirty-second audit.)
- `A-AUTH-4f` A username holds no line break, control or invisible (format)
  character: it is written into mail to an address's owner, and a stranger
  can register someone's address (KL-34). An account named so before the rule
  is mailed with those characters as `?`. Joiners stand only between letters
  of scripts written with them (Persian, Devanagari, Sinhala) or inside emoji
  sequences (`🏳️‍🌈`), and variation selectors only after a pictograph, an
  ideograph or on a keycap — never beside a Latin letter, accented or not, or
  a Cyrillic one (pass 7's first version let `zoë` plus a selector through). *(Covered:
  `auth_routes::a_username_cannot_carry_a_message_into_mail`,
  `routes::auth::tests::a_username_holds_no_line_break_or_hidden_character`.)*
  (Thirty-second audit.)
- `A-AUTH-4g` A name that differs from a taken one only in joiners or variation
  selectors is taken, since where a script allows them they may change nothing
  a reader sees (`ب‍ببب` beside `بببب`, `❤️❤❤` beside `❤❤❤`); and an
  unconfirmed twin whose link has expired gives the name up, as an exact one
  does. *(Covered:
  `auth_routes::a_name_differing_only_in_joiners_is_taken`.)* (Thirty-second
  audit.)
- `A-AUTH-4h` The notice to a taken address's owner is limited to five an hour
  per address, whatever client address asks, and one past it is skipped: the
  sixth registration is answered with the same bytes and sends nothing.
  *(Covered: `auth_routes::a_taken_address_is_sent_five_notices_an_hour_at_most`;
  with the limit removed, a sixth notice lands.)* (Thirty-second audit, pass 24.)
- `A-AUTH-4i` An address registers only bare (`x <victim@…>` and lists are
  refused) and only as SES would parse it: a dot-atom local part and host-name
  labels, so `a..b@x.com`, `.a@x.com` and `a@exa_mple.com` are refused rather
  than failing to send — which any visitor could have used to raise the
  mail-failed alarm. *(Covered:
  `routes::auth::tests::only_a_bare_address_is_an_email`.)* (Thirty-second
  audit; the SES syntax, pass 24.)
- `A-AUTH-5` Registration validates password strength, and rejects a password
  containing the username or email — and so does a password reset, which
  scored the new password without the account's context. *(Covered:
  `auth_routes::registration_refuses_weak_passwords_and_ones_built_from_the_username_or_email`,
  `auth_routes::a_reset_refuses_a_password_built_from_the_account_and_keeps_the_link`;
  the email's local part alone was accepted.)*
- `A-AUTH-6` A confirmation code is single-use: replaying it changes nothing.
  Opened again for an account it confirmed (a double click, a mail scanner that
  followed it first), it is answered "email already confirmed" rather than
  "invalid" with an offer to register again (twenty-second audit); for any
  other account it fails. *(Covered:
  `auth_routes::a_confirmation_code_works_once`.)*
- `A-AUTH-7` An expired confirmation code fails. *(Covered:
  `auth_routes::an_expired_confirmation_code_is_refused`.)*
- `A-AUTH-8` **A password-reset request returns the same body for a known and an
  unknown address**, and mail is sent off the request path so timing does not
  disclose either. *(Covered:
  `auth_routes::a_reset_request_answers_the_same_for_known_and_unknown_addresses`,
  `auth_routes::a_reset_request_does_not_wait_on_the_mail_it_sends`.)*
- `A-AUTH-8b` Every link in mail — a confirmation, a notice's sign-in and reset
  pages, a reset link — is on `PUBLIC_URL`, whatever `Host` or
  `X-Forwarded-Host` the request names: a link built from either would mail a
  reset token to a host the requester chose. *(Covered:
  `auth_routes::mailed_links_are_on_the_public_url_whatever_the_request_names`.)*
  (Thirty-second audit, pass 24.)
- `A-AUTH-9` A reset token is single-use, expires, and is invalidated by a
  successful reset. *(Covered:
  `auth_routes::a_reset_token_is_single_use_spent_by_any_reset_and_expires`.)*
- `A-AUTH-9b` A password reset and an email confirmation lock the account
  before its links, the order an admin's delete takes (`A-ADMIN-22`): with
  the account held, each waits holding none of its links. Links first, each
  deadlocked with a delete (a reset only when two links were out). *(Covered:
  `auth_routes::a_reset_and_a_confirmation_lock_the_account_before_its_links`.)*
  (Thirty-second audit.)
- `A-AUTH-10` Logout clears the cookie, so the browser is signed out. It does
  not revoke the token: a session is a signed token, not a stored row, so there
  is nothing per-session to delete, and a copy of the cookie taken before
  logout authenticates until its TTL. Revocation is per account:
  sign-out-everywhere (and a password reset) bumps the account's
  `session_generation`, which every token carries. *(Covered:
  `auth_routes::logout_clears_the_session_cookie_and_the_browser_is_signed_out`;
  revocation by `auth_api::bumping_the_session_generation_revokes_earlier_sessions`,
  `auth_api::signing_out_everywhere_revokes_the_callers_own_session_too`. The
  cookie removals lacked `Path=/`, so logout never signed a browser out.)*
- `A-AUTH-11` Rate limits: 11 registrations from one IP hits 429 with
  `Retry-After`; 6 reset requests for one **address** hits 429 even from
  different IPs — the half that matters, since IPs are cheap. Logins are
  limited both ways too (`A-BOUND-1`, `-2`). *(Covered:
  `auth_routes::the_eleventh_registration_from_one_address_is_rate_limited`,
  `auth_routes::the_sixth_reset_for_one_address_is_rate_limited_from_any_ip`.)*
- `A-AUTH-11b` An account's login bucket is the account's however its name is
  spelled: `TİM` signs in to `tim` (Postgres lowers `İ` to `i`) and shares its
  bucket. *(Covered:
  `boundaries::an_accounts_login_bucket_does_not_depend_on_how_its_name_is_spelled`;
  keyed on Rust's lowering, each spelling had a bucket of its own. Twentieth
  audit.)*
- `A-AUTH-11c` Redeeming confirmation and reset links is limited per address:
  the twenty-first in a minute is a 429. Both are unauthenticated writes on the
  main pool, and a reset scores a password first. *(Covered:
  `boundaries::redeeming_links_is_limited_per_address`.)* (Twenty-first audit.)
- `A-AUTH-12` A confirmed address and a password reset are on record
  (`user.email_confirmed`, `user.password_reset`); sign-in attempts are not,
  by design (PLAN, "Audit actions"). *(Covered:
  `auth_routes::a_confirmation_and_a_reset_are_on_record`.)* (Thirty-second
  audit, pass 22.)
- `A-AUTH-13` With `DEV_LOGIN` — the local stack's, and refused beside
  `SECURE_COOKIES=true` (`U-CFG-4`) — `GET /api/dev/login` signs a browser in
  as an account by name, and sends it to a path on this site (anything else
  goes to `/`); an unknown name is a `404`; without it the route does not
  exist. A `next` with any byte that is not visible ASCII goes to `/` too: a
  browser drops a tab from a `Location`, so `/<tab>/host` was `//host`, and a
  line break panicked the handler (thirty-third audit, pass 4). *(Covered:
  `auth_routes::the_dev_login_signs_a_browser_in_only_where_it_is_enabled`.)*

### `A-WORKER-*` — `routes/worker.rs`

- `A-WORKER-1` A claim with no body is rejected with a message naming the fix,
  not a bare 422. *(Covered:
  `worker_api::a_claim_without_a_usable_body_is_told_what_to_send` -- no body,
  `{}`, a body without `magpie_version` and a body that is not JSON are each a
  `400` in the API's error shape whose message names `magpie_version` and says
  to update MAGPIE. Until the ninth audit this was axum's plain-text `422`.)*
- `A-WORKER-2` A claim with a malformed `magpie_version` is rejected rather than
  assumed. Unparseable text reads as `0.0.0`, below the floor, so the worker is
  handed nothing and told to update (a floor of `0.0.0` would admit it; the
  shipped floor is `0.1.1`); a version that is not a string is a `400` naming
  the field. *(Covered:
  `worker_routes::a_malformed_magpie_version_is_refused_rather_than_assumed`.)*
- `A-WORKER-3` An `unsupported_jobs` list over 200 is **truncated, not
  rejected** — a truncated list costs at most a wasted claim — and the newest
  200 are kept: MAGPIE appends and never prunes, so keeping the first dropped
  the live entries (thirty-second audit, pass 19). *(Covered:
  `worker_routes::an_oversized_unsupported_list_is_truncated_not_rejected`.)*
- `A-WORKER-4` A first claim with no identity mints an anon UUID and returns it
  in the body; the client reusing it is recognised. An idle poll mints nothing,
  and only a claim can mint. *(Covered:
  `worker_api::a_worker_with_no_identity_is_persisted_only_when_given_a_task`,
  `worker_api::requests_other_than_a_claim_require_an_identity`; the body's
  shape by `C-9`.)*
- `A-WORKER-5` A client-invented UUID that is not in `anonymous_workers` is
  rejected with 401 and a message naming the fix. *(Covered:
  `worker_routes::an_invented_worker_uuid_is_refused_with_the_fix`.)*
- `A-WORKER-6` 204 for idle, and a `shutdown` body for each reason. These are
  different answers to different questions. *(Covered:
  `worker_routes::idle_and_each_shutdown_reason_are_distinct_answers`.)*
- `A-WORKER-7` A claim returns `expected_data` with digests, and the client
  declining with `missing_data` records `worker_data_gaps` and releases the
  claim immediately. *(Covered:
  `worker_routes::a_claim_states_its_digests_and_a_missing_data_decline_releases_it_at_once`;
  a decline's size by `A-BOUND-6`.)* A leave task carries its job's player,
  whole, the same on a reissue, with the top-level lexicon the player's and no
  `use_wordmap` of its own, and pins the player's lexicon and wordmap but not
  its leaves. *(Covered:
  `worker_routes::a_leave_claim_carries_its_player_and_pins_only_its_lexicon`.)*
- `A-WORKER-8` A decline with an unknown reason is rejected, the refusal
  listing them; the six known reasons (`missing_data`, `magpie_version`,
  `unknown_job_type`, `derived_mismatch`, `task_failed`, `time_limit`) are
  accepted, release the claim and are recorded on its audit row. *(Covered:
  `worker_routes::only_the_six_known_decline_reasons_are_accepted`.)*
- `A-WORKER-9` Heartbeat extends the claim, and has no effect for a stale
  token — reclaimed, declined, another identity's, or never issued. It is
  still answered `204`, by design rather than as a gap: the heartbeat has one
  answer in PLAN.md's worker contract, the client sends each once and ignores
  failures, and it finds out when it submits. "Rejected", as this entry first
  said, means "revives nothing". *(Covered:
  `worker_routes::a_heartbeat_extends_only_a_live_claim_of_the_caller`,
  `worker_api::a_claim_token_works_only_for_the_identity_it_was_issued_to`.)*
- `A-WORKER-10` Result submission with a valid token is accepted and publishes
  an SSE event. *(Covered:
  `worker_routes::an_accepted_result_is_published_to_the_jobs_live_stream`; a
  valid result of several megabytes by `A-BOUND-4`.)*
- `A-WORKER-11` Result submission with a stale token is silently ignored with a
  success status: `200 {"accepted": false}`, the same answer for a reclaimed
  claim, an already-accepted one, and another identity's token, and nothing
  moves. The client learns its result did not count, but not why. *(Covered:
  `worker_api::submissions_for_reclaimed_or_already_accepted_claims_change_nothing`,
  `worker_api::a_claim_token_works_only_for_the_identity_it_was_issued_to`,
  `fake_worker::a_stale_mode_submission_is_not_accepted_and_changes_nothing`.)*
- `A-WORKER-12` Each validation failure from tier 1 surfaces as a 400 with a
  message, not a 500. *(Covered:
  `worker_routes::every_implausible_games_result_is_a_400_that_says_why` --
  among them a captured position stating no move played from it, an empty
  one, or one scoring what no play can -- and the same for `game_pairs`,
  `opening_rack` (a static player's rack reported as a simulation, or with a
  simulation's iterations, `U-PLAUS-6`/`-7`) and leave results; an oversized
  body by `A-BOUND-5`.)*
- `A-WORKER-13` **Artifact fetch resolves only keys the server minted**; an
  arbitrary key is 404. Otherwise this is a read primitive for the whole bucket.
  *(Covered: `worker_routes::an_artifact_is_served_only_under_a_key_the_server_minted`.)*
- `A-WORKER-14` Worker endpoints are rate limited per identity: the 6th request
  in a second is 429, and a different identity is unaffected. *(Covered:
  `worker_routes::worker_requests_are_limited_per_identity`.)*
- `A-WORKER-14b` An account's worker is limited per API key: a key's sixth
  request in a burst is 429 and the account's other key is not. *(Covered:
  `worker_routes::an_accounts_workers_are_limited_per_key`.)* (Thirteenth
  audit.)
- `A-WORKER-16` Worker credentials are charged before their lookup (a
  main-pool query): each its own bucket, and one that has not resolved lately
  its address's too (burst 100). Made-up keys get 401s, then 429s with
  `Retry-After`, while a real worker at the same address -- whose credential
  resolved -- goes on being served, and another address is unaffected.
  *(Covered:
  `boundaries::made_up_worker_credentials_are_limited_per_address_and_real_ones_are_not`,
  `ratelimit::key_tests::unknown_credentials_pay_their_address_and_known_ones_do_not`.)*
  (Twenty-first audit; the twenty-second's redesign: the first gate refused
  every worker request from the address, real ones included. The
  twenty-third's order: an unknown credential pays its address first, so a
  refused flood leaves no per-credential bucket behind -- asserted on the
  limiter's size.)
- `A-WORKER-14c` A credential's claims and its work in hand (heartbeats,
  declines, results, artifacts) are limited separately, so idle machines
  sharing a key cannot spend a busy one's heartbeats. *(Covered:
  `worker_routes::idle_claims_cannot_starve_a_busy_machines_heartbeats`,
  and `worker_requests_are_limited_per_identity` for each bucket's own burst.)*
  (Thirty-first audit: on one bucket, eight machines on one identity had eleven
  of eleven heartbeats refused in five minutes.)
- `A-WORKER-17` A worker route refuses a caller it would refuse anyway -- no
  identity, or an unknown one -- before reading the body, and the routes other
  than the result accept at most 1 MiB. *(Covered:
  `worker_routes::a_caller_the_route_refuses_is_answered_before_its_body`,
  `worker_routes::a_claim_body_is_small`.)* (Thirty-first audit; see `U-ERR-6`.)
- `A-WORKER-18` **Nothing a caller does with bodies makes a heartbeat wait**:
  with the large-result budget spent by three stalled 60 MiB uploads, and two
  hundred claims stalled a byte short of a megabyte each, a heartbeat is
  answered at once; a fourth large result is refused at once with
  `Retry-After`; a first claim declaring more than 16 KiB is refused before its
  body; and one worker may have 64 MiB of large results in flight -- two 8 MiB
  ones at once, but not a 60 MiB one beside them -- its reservations given back
  as they end. *(Covered:
  `worker_routes::a_heartbeat_never_waits_behind_other_bodies`,
  `worker_routes::a_worker_may_send_a_share_of_large_results_at_once`,
  `worker_routes::a_chunked_first_claim_is_held_to_its_bound` — a first claim
  sent without a length is held to 16 KiB too, which the header check alone did
  not do, in the audit's second pass.)*
  (Thirty-first audit; one at a time, as first written, serialized a fleet on
  one key.)
- `A-WORKER-19` A task a worker declined is not offered to that worker again
  for an hour (outside leave generation, `I-LEAVE-23`): declined, it was the oldest available task and went straight
  back to whoever claimed next — the worker that had just failed it included —
  so one task that fails everywhere stopped every contributor claiming from
  its job (MAGPIE stops after five failures in a row). Another worker is still
  offered it. *(Covered:
  `worker_api::a_declined_task_is_not_handed_back_to_the_worker_that_declined_it`.)*
  (Thirty-second audit.)
- `A-WORKER-20` A result carrying captured positions for a job that does not
  capture them is refused, and nothing is stored; a capturing job's result
  with two positions for one turn of one game is refused too. *(Covered:
  `worker_api::positions_from_a_job_that_does_not_capture_them_are_refused`,
  `jobs::game::tests::one_captured_position_a_turn`.)* (Thirty-second audit.)
- `A-WORKER-21` A capturing job's result must carry positions from every game
  of its batch, and no result may hold a NUL in a string, a play over 256
  characters, a previous play scoring outside 0 to 100,000 or a bracketed tile
  over 8 characters, nor a position over 4,096, nor a decline a NUL in a
  missing file: each is a `400`,
  where a result with no positions was accepted and completed its task, a NUL
  was a `500` that left the claim open, and the rest were stored. *(Covered:
  `worker_api::a_capturing_jobs_result_is_complete_and_no_result_holds_what_cannot_be_stored`,
  `worker_api::a_decline_holding_a_nul_is_refused_and_the_claim_stays_declinable`,
  `plausibility::tests::a_nul_is_found_only_where_json_escapes_one`,
  `plausibility::tests::a_bracketed_tile_is_a_few_letters`; job creation's
  bound on a games batch, `routes::admin::tests::a_games_batch_is_bounded`.)*
  (Thirty-second audit, pass 21.)
- `A-WORKER-22` A claim states its build's `board_dim` and `rack_size`; a build
  other than 15 and 7 is answered an `unsupported_build` shutdown naming both,
  whatever jobs are on offer, and is handed no task and minted no identity; a
  claim that leaves either out is a `400` naming it. A `RACK_SIZE=8` build
  passed every other check while scoring every 7-tile bingo without its bonus.
  *(Covered: `worker_routes::a_build_for_another_board_or_rack_is_sent_away`;
  the fixture by `contract_fixtures::every_shutdown_reason_matches_what_the_server_sends`;
  MAGPIE's claim body by `test_the_claim_body_matches_the_claim_fixture`.)*
  (Thirty-third audit, pass 1.)
- `A-WORKER-23` Every assignment names its job (`job_name`: its name, or its
  type and the start of its id, never empty) and states the time limit its
  claim was given (`max_task_seconds`): its job's at the claim, which the
  claim's `deadline_at` is its claim time plus. A change to the job's limit
  moves later claims' limits and not an earlier one's, and two jobs with
  different limits, claimed from in one run, give each claim its own job's. *(Covered:
  `worker_routes::an_assignment_names_its_job_and_states_the_limit_it_was_claimed_under`;
  the field names by `C-11`.)*
- `A-WORKER-24` A claim past its deadline and the minute's grace lapses at the
  next reclamation **even while its worker heartbeats**, and its task goes
  back out, the claim marked an overrun (`A-WORKER-27`); inside the grace it
  stands, and a process in its startup grace lapses nothing on a deadline
  either. *(Covered:
  `worker_routes::a_claim_past_its_deadline_lapses_even_while_heartbeating`.)*
- `A-WORKER-25` A result for a claim past its deadline and the grace is
  answered `accepted: false` whether or not a reclamation got there first,
  and the claim is released then, its task available and nothing stored (and
  counted against the job as an overrun, `A-WORKER-29`); inside the grace, or
  while the process is in its startup grace, the result is accepted. *(Covered:
  `worker_routes::a_result_past_its_claims_deadline_is_refused_and_frees_the_task`.)*
- `A-WORKER-26` A `time_limit` decline is counted against its job, which the
  job's page shows (`job.time_limit_declines`); three in a row with no task of
  the job completed between set the job aside -- inactive at 0%, the reason
  on the job and its page, a `job.set_aside` audit row from its share -- and
  an accepted result between them starts the run again. An allocation puts it
  back, its reason cleared and its run started afresh. *(Covered:
  `worker_routes::three_time_limit_declines_in_a_row_set_the_job_aside`.)*
- `A-WORKER-27` A claim lapsed at its deadline while its worker still
  heartbeat is an **overrun**: reclamation marks it (`task_claims.overrun`,
  `pending`) and leaves the job's row alone, and the job's next claim counts
  it (`counted`, the job's `time_limit_declines` and `time_limit_streak`).
  Three in a row set the job aside as three `time_limit` declines do, with
  the same reason and `job.set_aside` row, and the claim that counted the
  third is handed another job's task. *(Covered:
  `worker_routes::three_overruns_with_the_worker_alive_set_the_job_aside_at_the_next_claim`.)*
- `A-WORKER-28` A claim lapsed at its deadline whose worker had gone silent
  -- never heartbeat, or stopped a heartbeat timeout before the claim lapsed
  -- counts toward nothing; what decides is whether the worker was alive when
  the claim lapsed (deadline and grace), not when reclamation came round, so
  one that heartbeat to the end and went quiet after is an overrun.
  *(Covered:
  `worker_routes::a_silent_workers_lapse_at_the_deadline_counts_toward_nothing`.)*
- `A-WORKER-29` `time_limit` declines, overruns and results refused past
  their deadline make one run, in the order they happened: an accepted result
  ends it, and an overrun whose deadline came before that result, counted
  after it, counts toward the total and not the run
  (`jobs.time_limit_streak_since`). A refused late result is counted at once;
  three in a row set the job aside, and nothing more is handed out. *(Covered:
  `worker_routes::declines_overruns_and_late_results_make_one_run_that_a_completion_ends`.)*
- `A-WORKER-30` A purge ends the run, deleting the claims an uncounted overrun
  is marked on; an allocation that puts a job back starts a run that an
  overrun from before it, counted after, is not part of. *(Covered:
  `worker_routes::a_purge_or_an_allocation_starts_the_run_afresh`.)*
- `A-WORKER-15` `client-version` reports the configured floor and a download
  URL. *(Covered:
  `worker_routes::client_version_reports_the_configured_floor_and_download_url`.)*

### `A-ADMIN-*` — `routes/admin.rs`

- `A-ADMIN-1` Player config create/get/list/delete round-trips, and a config in
  use cannot be deleted. *(Covered:
  `admin_routes::a_player_config_round_trips_and_one_in_use_cannot_be_deleted`.)*
- `A-ADMIN-2b` A job keeps the name it was created with, trimmed, and the jobs
  list and the job's page return it; one created without a name has an empty
  one (titled by its type, `F-FMT-13`); a name past 100 characters or of more
  than one line is refused on the field. *(Covered:
  `admin_routes::a_job_keeps_the_name_it_was_created_with`; E-4 creates one
  through the form and finds it as the title.)*
- `A-ADMIN-2c` A player config's and a rating pool's names take a job name's
  rule: past 100 characters, of more than one line, or with a control
  character is refused on the field; a pool's name is stored trimmed, so "X "
  is the pool "X" (a conflict). Both took any text, and a pool's name as typed
  (thirty-third audit, pass 1). *(Covered:
  `admin_routes::a_player_config_and_a_pool_take_a_job_names_rule`.)*
- `A-ADMIN-2` Creating a job of each type returns `{jobs: [job]}` and the
  job is inactive at 0%. *(Covered:
  `admin_routes::creating_each_job_type_answers_it_inactive_and_unallocated`;
  a leave job, which runs MAGPIE at creation, by the opt-in
  `magpie_routes::a_leave_job_is_created_inactive_with_its_generation_zero_leaves_stored`.)*
- `A-ADMIN-3` Job creation rejects: mismatched win% models, incompatible
  lexicon/distribution, an unknown `input_data` id. The win%-model rules on a
  single player — a simming player with no model, a static player with one —
  are enforced where the player is made, at player-config creation, not at job
  creation as this entry first listed them, so no job can name such a config;
  so is a simming player's candidate count, at least 2 (with one, MAGPIE plays
  it without simulating, every turn; thirty-third audit, pass 3).
  A config's `kwg_id`, `klv_id` and `winpct_id` must each name a file of that
  role (`A-BOUND-3`). A letter distribution MAGPIE cannot hold (`U-RACK-9`) is
  refused at creation, not by every claim as a 500. *(Covered:
  `admin_routes::job_creation_refuses_each_impossible_combination_and_says_which`,
  which asserts the player-config half too.)*
- `A-ADMIN-4` An allocation change (activating and deactivating) / complete /
  purge / delete each return the documented shape and are reflected in a
  subsequent read. *(Covered:
  `admin_routes::each_lifecycle_action_answers_its_shape_and_a_read_agrees`; the
  allocation cap by `A-BOUND-7`.)*
- `A-ADMIN-5` Import start → poll → confirm over HTTP, including that polling
  reports progress while running. The ref is resolved only among the
  repository's own branches and tags: a sha (whole or five characters), a
  pull request's ref or a `git describe` name — which `/commits/{ref}`
  resolved from any fork, served under the upstream's name — is not found,
  and `/commits/` is never asked (thirty-second audit). *(Covered:
  `input_data::an_import_is_started_polled_while_running_and_confirmed_over_http`.)*
- `A-ADMIN-5b` A branch named like a sha (`20260101`, a real one), a
  lightweight tag, an annotated tag (peeled to its commit) and a tag of a tag
  all resolve; a `heads/`, `tags/` or `refs/tags/` prefix picks a branch or a
  tag sharing one name; a tag naming a tree is refused. *(Covered:
  `input_data::a_ref_resolves_among_the_repositorys_own_branches_and_tags`.)*
  (Thirty-second audit.)
- `A-ADMIN-6` Confirming an import that is not `staged` is rejected; one with
  nothing new says so. *(Covered:
  `input_data::only_a_staged_import_can_be_confirmed`.)*
- `A-ADMIN-7` `input-data` list and delete, including the in-use refusal.
  *(Covered: `input_data::an_input_file_in_use_cannot_be_deleted`,
  `input_data::a_file_only_derived_data_refers_to_can_be_deleted_and_takes_that_data_with_it`.)*
- `A-ADMIN-8` `job/:id/data-gaps` reports what workers declined for. *(Covered:
  `admin_routes::data_gaps_report_what_workers_declined_for`.)*
- `A-ADMIN-9` `fleet` reports connected workers and their versions: a claim
  counts while open (claimed in the week) or once completed in the week,
  whenever it was claimed; one completed before the week or lapsed does not
  (thirty-third audit, pass 1: it read every claim ever made). *(Covered:
  `admin_routes::the_fleet_view_counts_workers_by_the_version_they_run`.)*
- `A-ADMIN-10` `backups` reports staleness from the `backups` table. *(Covered:
  `admin_routes::the_backups_view_reports_staleness_from_the_backups_table`.)*
- `A-ADMIN-11` `rebuild-artifacts` returns what it rebuilt and is idempotent.
  *(Covered: the refusal for a non-leave job by
  `admin_routes::only_a_leave_jobs_artifacts_can_be_rebuilt`; the rebuild,
  which runs MAGPIE, by the opt-in
  `magpie_routes::rebuilding_artifacts_restores_what_is_missing_and_is_idempotent`.)*
- `A-ADMIN-11b` A forced artifact rebuild is in the audit log before it
  rewrites anything: one that stopped part-way had replaced objects and
  written no row, the only one coming at the end. *(Covered, tier 6 opt-in:
  `magpie_routes::a_forced_rebuild_is_logged_before_it_rewrites_anything`.)*
  (Thirty-second audit.)
- `A-ADMIN-12` Ban and unban by user id and by anon UUID; a banned worker's next
  claim is refused; unban restores it. *(Covered:
  `admin_routes::a_ban_by_either_identity_refuses_the_next_claim_and_unban_restores_it`,
  `admin_api::an_identity_can_be_banned_once_and_unbanning_lifts_it`.)*
- `A-ADMIN-13` `audit-log` paginates and filters by job. *(Covered:
  `admin_routes::the_audit_log_pages_and_filters_by_job`.)*
- `A-ADMIN-14` `delete_user` over HTTP. *(Covered:
  `admin_api::a_user_with_history_can_be_deleted`; an admin cannot delete
  themself, `A-BOUND-8`.)*
- `A-ADMIN-15` Purging a completed job returns it to inactive at 0% (as a purge
  does every job), clears its stored verdict, and it can be activated again. *(Covered:
  `admin_api::purging_a_completed_job_returns_it_to_inactive`.)* (Eleventh
  audit's fix, twelfth audit's test.)
- `A-ADMIN-15b` An export's `job.export_started` row is written in the
  transaction that records the export: a refused one (a completed job's claims
  still in flight, one already running) writes none, and a begun one exactly
  one. *(Covered: `admin_api::a_running_job_exports_a_snapshot`.)*
  (Thirty-second audit.)
- `A-ADMIN-16` While a purge or delete holds a job's claims, a submission for
  one is answered `503` at once; and a hold that ends without committing spares
  the job's claims from reclamation, so the submission lands afterwards.
  *(Covered:
  `admin_api::a_purge_in_progress_neither_parks_submissions_nor_costs_its_claims`.)*
  (Twelfth audit.)
- `A-ADMIN-17` A worker request's `last_seen_at` touch does not wait on its
  identity's locked row (a purge giving back contributions, a submission
  committing). *(Covered:
  `admin_api::a_locked_identity_row_does_not_hold_up_its_requests`.)* (Twelfth
  audit.)
- `A-ADMIN-18` A deleted account's tombstone address cannot be squatted (a
  registered `<id>@deleted.invalid` no longer blocks the delete), and the bans
  in force are listed with the id lifting one takes. *(Covered:
  `admin_api::a_deleted_accounts_tombstone_cannot_be_squatted_and_bans_are_listed`.)*
  (Thirteenth audit.) Banning an identity that does not exist is `404`
  (`admin_api::an_identity_can_be_banned_once_and_unbanning_lifts_it`;
  fourteenth audit).
- `A-ADMIN-19` A purge or delete of a job whose purge or delete is already
  running is `409` (checked and held in one step: a double click got two), as
  are an allocation change (activating or deactivating it) and completing it;
  all are allowed once it has finished. *(Covered:
  `admin_api::a_second_purge_or_delete_is_refused_while_one_runs`.)* (Fourteenth
  audit.)
- `A-ADMIN-20` A job cannot pin two different files under one role and name
  (MAGPIE finds a file by name, so every worker would decline every task); the
  same file on both sides, and two differently named files, are still a job.
  *(Covered: `admin_api::a_job_cannot_pin_two_files_under_one_name`.)*
  (Thirty-first audit.)
- `A-ADMIN-21` A config or job no worker can run is refused at creation, each
  at its boundary: a margin past MAGPIE's largest equity (2,147,483.645 taken,
  .646 refused, either margin), `num_plays` past 200,000, `num_plays_recorded`
  past 32,767, and a board layout MAGPIE's loader would refuse — every worker
  failed every task of such a job, and `magpie contribute` stops after five
  failures in a row; and a match-test confidence of 100% (or just past it),
  whose logarithm of zero leaves the interval never closing, with 99.999%
  accepted. The layout check never accepts
  what MAGPIE's loader refuses, and agrees with it on CRLF files, blank lines,
  trailing whitespace, row widths, unknown squares and start squares (it is
  stricter only on parser quirks). *(Covered:
  `admin_api::a_config_or_job_no_worker_can_run_is_refused`,
  `routes::admin::tests::a_layout_magpie_would_refuse_is_refused`; the same
  parser reads each square and the start square for the saved-positions
  board, `board::tests::a_layout_is_read_square_by_square`.)*
  (Thirty-second audit.)
- `A-ADMIN-22` Deleting an account locks the account before its own rows:
  with the account held, the delete waits holding none of its reset links.
  *(Covered: `admin_api::deleting_an_account_locks_it_before_its_rows`.)*
  (Thirty-second audit.)
- `A-ADMIN-23` A job that needs a wordmap or a rack info table, on a
  distribution with more than two blanks (`english_super`), is refused at
  creation: MAGPIE builds neither for more than two and aborts, so the job
  never dispatched. The same job without them is created. *(Covered:
  `admin_api::a_wordmap_on_a_distribution_with_more_than_two_blanks_is_refused`.)*
  (Thirty-second audit.)
- `A-ADMIN-24` Retry resets the one failed build it names, by builder and by
  the files it is built from; it reset every failed row of that role and name,
  other builders' included. A word info table's row is listed buildable and
  retried under its own builder; both read every role but a wordmap as a rack
  info table's, so a failed WIT build could not be retried (thirty-third
  audit, pass 1). *(Covered:
  `admin_api::a_retry_resets_only_the_build_it_names`.)* (Thirty-second audit.)
- `A-ADMIN-25` A player config solves the endgame and the pre-endgame exactly
  as it states: off by default; the endgame alone states no pre-endgame
  setting; a pre-endgame with only its switch takes MAGPIE's schedule, written
  into the row; without nested lookahead the nested settings are null. Refused,
  each on its field: the pre-endgame without an endgame depth, an endgame past
  25 plies or a bag past 4, a pre-endgame setting without the pre-endgame, a
  nested setting without nested lookahead, a stage keeping one play or more
  than the stage before, a zero stride, an unknown opponent model, and nested
  strides that are not one per bag size. *(Covered:
  `admin_api::a_player_config_solves_the_end_of_the_game_only_as_it_states`.)*
- `A-ADMIN-26` A games or pairs job that captures positions refuses players
  that disagree on `num_plays_recorded` or `num_plies_recorded`, naming
  `capture_positions`: MAGPIE keeps one of each for the whole run, player 1's.
  Without capture they may differ. Keeping only first divergences
  (`capture_first_divergence`) is refused on a games job and on a pairs job
  that captures nothing, each naming that field, and is stored when a pairs
  job captures. *(Covered:
  `admin_api::a_capture_job_refuses_players_that_record_differently`,
  `jobs::each_job_type_stores_every_setting_it_was_created_with`.)*
- `A-ADMIN-27` A job's bingo bonus and sim cutoff may be stated and are kept
  as stated; left out, they are MAGPIE's 50 and 0.005; a negative bonus or a
  cutoff outside 0–100 is refused on its field. *(Covered:
  `admin_api::a_player_config_and_a_job_state_every_setting_a_task_needs`.)*
- `A-ADMIN-28` **Several jobs' allocations in one request**
  (`PUT /api/admin/jobs/allocations`), checked as a whole: 50/50 becomes 60/40,
  which one job at a time refuses (50 + 60 is over 100). A job named at 0% is
  deactivated, at 0%, and an inactive one named above 0% activated; each change
  is audited once, from what to what (`job.activated` / `job.deactivated` for a
  job switched on or off, `job.allocation_changed` for an active job's new
  share). A request totalling over 100%, naming
  a completed job, a job twice, an allocation outside 0–100, or nothing,
  changes nothing. *(Covered:
  `admin_api::allocations_are_set_together_and_checked_as_a_whole`; the route's
  authorization and CSRF pair, `authz`.)*
- `A-ADMIN-29` A purge waiting on a running rating fit holds up no submission:
  it marks every pool for a refit, under their fit locks, before it gives its
  contributors' counters back, so a contributor's submission for another job
  goes through while the purge waits. (A delete keeps the same order.)
  *(Covered: `admin_api::a_purge_waiting_on_a_rating_fit_holds_up_no_submissions`.)*
  (The test predates the entry, and cited `A-ADMIN-20`; thirty-third audit.)
- `A-ADMIN-30` The task time limit is each job's: 3600 unless its creation
  names another (shown in its settings, `/api/jobs/:id/config`), changed by
  `PATCH /api/admin/jobs/:id/time-limit`, refused on its field below 600 or
  above 86,400 at creation and after (and by the column's CHECK, so a test
  moves a claim's deadline rather than the limit; 600 itself accepted), an
  unknown job a `404`, and each change audited once (`job.time_limit_changed`,
  "max_task_seconds 3600 -> 1800", the admin as actor, the job as target) --
  a change to what it already is writes nothing; another job's limit is its
  own; the claims made after a change are given it. *(Covered:
  `admin_routes::the_task_time_limit_is_each_jobs_and_each_change_is_audited`,
  `audit::every_destructive_admin_action_writes_exactly_its_record`; that only
  an admin may, by `A-AUTHZ`.)*
- `A-ADMIN-31` A games or pairs job's `threading_mode` is `igp` unless the
  request says `pgp`, anything else a `400` on the field; its settings show
  it, and its tasks' requests carry it, beside the job's name. *(Covered:
  `admin_routes::a_games_jobs_threading_mode_is_on_its_settings_and_every_task`.)*

### `A-RATE-*` — `routes/ratings.rs`

- `A-RATE-1` Pool list and detail render the latest run, with its cross table
  from both sides: the score, its error (√11/16 for [2, 0, 1, 0, 1]) and the
  spread mirrored, beside the predicted score.
  *(Covered: `ratings::the_pool_pages_show_the_latest_run_with_its_residuals`,
  `worker_api::a_pools_residuals_are_the_ones_its_latest_fit_stored`.)*
- `A-RATE-2` A pool with no run renders empty rather than erroring. *(Covered:
  `ratings::a_pool_with_no_run_renders_empty`.)*
- `A-RATE-3` Creating a pool adds the anchor as a member automatically.
  *(Covered: `ratings::creating_a_pool_makes_its_anchor_a_member`.)*
- `A-RATE-3b` A pool is validated like a job: a variant no job has, a
  distribution id that is a layout row, an anchor rating whose scale would
  overflow, and a board or a letter distribution no job can be created on
  (thirty-second audit) are each a 400, and nothing is created. *(Covered:
  `ratings::a_pool_that_could_rate_no_one_is_refused`.)* (Eleventh audit.)
- `A-RATE-4` Adding and removing a member each trigger a refit and return a new
  `run_id`. *(Covered: `ratings::adding_and_removing_a_member_each_refit_the_pool`.)*
- `A-RATE-4c` Adding a config that is already a member (a second click, or
  the anchor) is answered `run_id: null`, with no `rating_pool.member_added`
  row and no new run; it was logged and refitted as an addition. *(Covered:
  `ratings::adding_a_config_that_is_already_a_member_changes_nothing`.)*
  (Thirty-third audit, pass 3.)
- `A-RATE-4b` Adding a config that does not exist is a `400` on
  `player_config_id`, and adding to a pool that does not exist a `404`; an
  anchor that does not exist is a `400` too. None was the `409` "still
  referenced" a bare foreign-key failure maps to. *(Covered:
  `ratings::adding_an_unknown_member_or_to_an_unknown_pool_says_which`,
  `ratings::a_pool_that_could_rate_no_one_is_refused`.)*
- `A-RATE-5` Removing the anchor is refused with a message naming what to do —
  move the anchor first (`A-RATE-8`). It once named an operation that did not
  exist until the eleventh audit, and then a new pool, until anchors could move.
  *(Covered: `ratings::removing_the_anchor_is_refused_with_the_fix_named`.)*
- `A-RATE-6` History (`GET /api/rating-pools/:id/history`, API-only since the
  ratings page dropped its history chart) returns points in time order and
  excludes unrated configs.
  *(Covered: `ratings::history_is_in_time_order_and_leaves_out_unrated_configs`,
  `admin_api::a_long_rating_history_is_thinned_but_keeps_its_ends`.)*
- `A-RATE-6b` History carries only six configs: the current members rated
  highest in the newest run. Every member's every point went out
  on each view of the public ratings page, which then drew a chart of it — 9.5 MB at 100 members, and forty views at
  once answered `503` to other readers — and a removed config could take one
  of the six. *(Covered:
  `ratings::history_carries_the_six_highest_current_members`.)* (Thirty-second
  audit, pass 25.)
- `A-RATE-7` Recompute is admin-only and returns a new run. *(Covered:
  `ratings::only_an_admin_can_recompute_and_it_stores_a_new_run`.)*
- `A-RATE-8` Moving the anchor to a config that is not a member adds it, pins
  it at the new rating (`is_anchor` on it alone) and refits in the request as
  an `anchor` run, logged old → new; moving only the rating shifts every rating
  by the same amount; sending what is already there changes nothing; and the
  old anchor can then be removed. *(Covered:
  `ratings::an_anchor_change_refits_sets_the_anchor_and_adds_it_as_a_member`.)*
- `A-RATE-8b` An anchor that does not exist is a `400` on its field, a rating
  out of range or an empty body a `400`, an unknown pool a `404`, and none
  changes the pool, its members or its runs. *(Covered:
  `ratings::a_bad_anchor_change_is_refused_and_changes_nothing`.)*
- `A-RATE-9` Deleting a pool cascades its members, runs, ratings and residuals,
  logs its census, keeps the games, frees the configs it pinned for their own
  delete, and is a `404` afterwards to read, delete or add to. *(Covered:
  `ratings::deleting_a_pool_cascades_frees_its_configs_and_is_a_404_after`.)*
- `A-RATE-10` The pool detail lists the pool's members now, by name, apart
  from the latest fit's ratings: a never-fitted pool lists its anchor, a member
  added since the fit (or whose refit failed) is listed with no rating, and one
  removed since is rated but not listed. The page built its membership from the
  ratings (`F-RATE-1`). *(Covered:
  `ratings::the_detail_lists_members_the_latest_fit_has_not_rated`.)*
  (Thirty-third audit, pass 4.)

### `A-PUBLIC-*` — `routes/public.rs`

- `A-PUBLIC-1` Job list paginates, and `per_page` is clamped to the maximum;
  jobs that tie on the sort key are each listed exactly once. *(Covered:
  `public_api::the_job_list_paginates_and_clamps_its_page_size`,
  `public_api::tied_jobs_and_users_are_each_listed_exactly_once`; ties had no
  `id` tie-break.)*
- `A-PUBLIC-1c` The job list's `stalled` flag is set for an active job with a
  decline and no accepted result in a day, and cleared by a result.
  *(Covered: `public_api::the_job_list_flags_a_stalled_job`.)* (Nineteenth
  audit: nothing tested it.)
- `A-PUBLIC-1e` A job's full configuration is public (`GET /api/jobs/:id/config`):
  the job's settings with files by name, its type's (a games job's batch, cap
  and test, or its target and `test_enabled` false without one), and every
  setting of each player config, in role order (a leave job's one player
  too, which carries its lexicon and wordmap), with the
  config's id to link to; it names no creator and carries no user id; an
  unknown job is a `404`. *(Covered:
  `public_api::a_jobs_full_configuration_is_public`; the page's key and full
  tables, grouped and side by side, by `jobSettings.test.ts`, `F-SET-1`.)*
- `A-PUBLIC-1d` Player configs are public (`GET /api/player-configs`, newest
  first, and `/:id`): every setting with files by name, and the config it was
  cloned from, but not who made it; an unknown config is a `404`. *(Covered:
  `public_api::player_configs_are_public`; the page's key and full table by
  `jobSettings.test.ts`, `F-SET-1`.)*
- `A-PUBLIC-1f` An opening-rack job's progress in the job list is racks
  settled of its rack space, as its own page counts them: a reopened job with
  every rack analysed and none settled lists 0 of 4, then 2 of 4. It listed
  racks analysed, so a job seeking a consensus read finished while it
  re-analysed. *(Covered:
  `worker_api::an_opening_rack_jobs_consensus_can_change_and_the_job_follows`.)*
- `A-PUBLIC-1b` `?status=` filters the job list and its total. *(Covered:
  `public_api::the_job_list_filters_by_status`.)* (Eighteenth audit.) Deleting
  a job ends its open streams (`sse::tests::closing_a_job_ends_its_streams`).
- `A-PUBLIC-2` Job detail returns the right stats block for each job type, and
  404s for an unknown id. *(Covered:
  `public_api::job_detail_carries_the_stats_block_of_its_type`.)*
- `A-PUBLIC-3` `job_results` paginates and filters, and returns `total = -1`
  where an exact count is deliberately not computed. A `?worker=` with no
  claims in this job is an empty page decided from their claims here, not by
  walking the job (twenty-sixth audit). *(Covered:
  `public_api::the_results_feed_paginates_and_filters_without_counting`,
  `worker_api::the_results_feed_walks_every_row_exactly_once`,
  `worker_api::the_results_feed_filters_by_who_a_name_is`.)*
- `A-PUBLIC-3b` A `?worker=` page is read through the contributor's claims in
  the job, newest completion first, and pages exactly in the unfiltered feed's
  order, including inside one opening-rack batch. A name that is both an
  account and a pseudonym is both contributors' work, merged (twenty-seventh
  audit). *(Covered:
  `public_api::the_filtered_feed_pages_through_a_contributors_claims`.)*
- `A-PUBLIC-3c` A contributor's claim range names no `state`, so the planner
  cannot choose the fleet-wide completed-claims index for it, and a name that
  is two identities is two pages merged, not one sort over both identities'
  claims (twenty-eighth audit). Each page breaks the tie at the cursor's time
  with a negation, not a row comparison the planner estimates from the time
  column a second time (twenty-ninth audit). *(Covered:
  `routes::public::tests::a_contributors_page_is_read_through_their_own_index`.)*
- `A-PUBLIC-3d` A results cursor holding a time before 4713 BC, where
  Postgres's `timestamptz` starts, reads as no cursor -- the first page --
  like any cursor this server did not produce, on an opening-rack job and a
  games job, with and without `?worker=`. chrono holds such a time, and
  Postgres refused it at the bind (22008): a `500` on a public route. A cursor
  time before 1970 is dropped (thirty-third audit, pass 4). *(Covered:
  `public_api::a_cursor_older_than_postgres_can_hold_reads_as_the_first_page`.)*
- `A-PUBLIC-4` `rack_lookup` finds an analysed rack, however it is typed, with
  its whole ranked list (each analysis's, cut short only when many analyses
  would together exceed what one can record: `I-SCHED-21`). A rack with no analysis yet is a `200` with an empty
  list, not a 404 as this entry first said — the rack is a valid question
  about a job that exists, and "nothing yet" is its answer; only an unknown job
  is a 404. Each ranked move carries its win percentage and its first two
  plies' statistics (`null` and `[]` for a static analysis). *(Covered:
  `public_api::rack_lookup_finds_an_analysed_rack`.)*
- `A-PUBLIC-4b` A games job's captured positions (`/api/jobs/:id/positions`)
  with one rack page newest first with their ranked moves, the rack however
  it is typed -- blank first, lower case -- spelt as MAGPIE spells one; a rack
  the distribution cannot spell is an empty page; signed out reads the same
  as signed in; no rack is a `400`, an opening-rack job a `400`, an unknown
  job a `404`. *(Covered: `public_api::captured_positions_are_searchable_by_anyone`.)*
- `A-PUBLIC-4c` A random position (`/positions/random`) is drawn from the
  tasks that have one, past tasks still being played -- the first captured of
  a task comes back, which the newest-position fallback never returns -- and
  is `null` for a job with no task, or none returned; signed out draws the
  same; an opening-rack job a `400`, an unknown job a `404`. *(Covered:
  `public_api::a_random_position_is_drawn_from_the_tasks_that_have_one`.)*
- `A-PUBLIC-4h` `/rack-samples` draws distinct racks an opening-rack job has
  analysed: none before it has any; from a small job, ten by default, every
  one when more are asked for, at least one when none are; from a job past a
  thousand analyses, by probes, ten distinct racks all of the job's. Another
  job type is a `400`, an unknown job a `404`. *(Covered:
  `public_api::rack_samples_are_distinct_analysed_racks`.)*
- `A-PUBLIC-4g` A simulated position's moves carry their win percentage and their
  first two plies' statistics however many more the job recorded (four
  stored, two returned), and the position what its player inferred of the
  opponent's leave (`inference`: leaves found, draws, mean equity, the ten
  most drawn), `null` where it has none. A position that
  claims an inference it cannot have -- on a static analysis, on a game's
  first turn, with eleven leaves, or leaves out of order -- is refused at
  submission (`plausibility::check_inference`). *(Covered:
  `public_api::a_simulated_position_shows_its_plies_and_its_inference`.)*
- `A-PUBLIC-4f` A game-pairs job keeping first divergences takes from each
  diverging pair both games' positions at one turn and nothing else (a lone
  position, two at different turns, or fewer pairs than `divergent_games`
  says are each a `400`); every saved position of a pairs job comes with its
  `partner`, the same turn of the pair's other game -- at random and by rack,
  where a rack both games hold finds the pair once, led by its first game --
  and `null` where the other game has no such turn; a games job's carry no
  `partner`. *(Covered:
  `public_api::a_pairs_job_keeps_first_divergences_and_shows_each_with_its_partner`,
  `public_api::a_pairs_position_without_a_partner_turn_says_so`, and
  `jobs::game_pair::tests::first_divergences_are_both_games_at_one_turn_of_one_position`
  for each way a pair can be malformed.)*
- `A-PUBLIC-4d` A job's board (`/api/jobs/:id/board`) is public: its layout
  square by square with the start square, and every letter of its
  distribution with its blank's spelling and score. *(Covered:
  `public_api::a_jobs_board_is_its_layout_and_letter_scores`.)*
- `A-PUBLIC-5` The SSE stream emits an event after a result is accepted, and the
  event body is byte-identical to what a page reload would fetch. *(Covered:
  `public_api::the_stream_sends_what_a_reload_would_fetch_after_each_result`,
  `sse::tests::pushes_coalesce_into_one_in_flight_and_one_pending`.)*
- `A-PUBLIC-6` The SSE stream ends cleanly when the client disconnects.
  *(Covered: `public_api::the_stream_unsubscribes_when_the_client_disconnects`,
  `sse::tests::subscribers_are_counted_and_forgotten_when_they_leave`.)*
- `A-PUBLIC-6b` Live streams are capped (2,000 across every job): past the cap
  a stream is a `503` with `Retry-After`, and an ended stream gives its place
  back. *(Covered: `routes::public::tests::a_stream_past_the_cap_is_told_to_come_back`.)*
  (Twentieth audit: a public route with no bound on open connections.)
- `A-PUBLIC-6c` One address holds at most 32 live streams, each for as long as
  its response lives; a refusal holds nothing, and another address is
  unaffected. *(Covered: `routes::public::tests::one_address_cannot_hold_every_stream`,
  and on the router `boundaries::one_address_holds_at_most_its_share_of_live_streams`,
  which fails if the handler stops holding its place for the stream's life.)*
  (Twenty-first audit: the global cap alone let one host hold every place.)
- `A-PUBLIC-6d` A job's live pushes are at most one per `JOB_STATS_CACHE_SECONDS`
  however its submissions arrive — six submissions over two and a half seconds
  under a ten-second interval push at most once — while an admin's change is
  pushed at once, mid-interval. *(Covered:
  `public_api::live_pushes_are_spaced_by_the_stats_interval_but_admin_changes_are_not`,
  `sse::tests::an_urgent_push_cuts_the_cool_down_short`,
  `public_api::submissions_during_a_cool_down_are_pushed_when_it_ends`, which
  checks that what arrived during a cool-down is pushed when it ends.)* (Thirty-second audit:
  a job whose submissions came slower than one build was rebuilt and pushed for
  each.)
- `A-PUBLIC-6a` The SSE stream ends when the process is told to stop: it has no
  end of its own, and graceful shutdown waits for every open response, so an
  open dashboard used to hold every deployment until the runtime's `SIGKILL`.
  *(Covered: `worker_api::a_live_stats_stream_ends_when_the_server_is_told_to_stop`.)*
- `A-PUBLIC-7` User and worker lists paginate and do not leak email addresses or
  key hashes — nor an anonymous worker's UUID, its only credential, which
  public endpoints replace with a derived pseudonym; contributors that tie are
  each listed exactly once, in every order. The worker list carries compute
  seconds, movegens and tasks, is ranked by movegens by default and by
  `?sort=compute|tasks` on request (each order its own, the same contributors
  in each), and refuses any other `sort` -- the retired `games` and `racks`
  among them. *(Covered:
  `public_api::contributor_lists_paginate_and_leak_no_credentials`,
  `public_api::tied_contributors_are_each_listed_exactly_once`,
  `public_api::tied_jobs_and_users_are_each_listed_exactly_once`,
  `worker_api::public_endpoints_name_anonymous_workers_by_pseudonym_only`; tied
  contributors were duplicated and skipped across pages.)*
- `A-PUBLIC-7a` Contributions by job type -- movegens, compute time and
  tasks: each accepted claim's movegens and compute time are credited to its
  job (`jobs.movegens`, the job page's `movegens`, and `jobs.compute_ms`)
  beside its contributor; `GET /api/workers/movegens` is the jobs' totals by
  type, every type present at 0 when nothing ran for it; a contributor's
  breakdown (`/api/workers/user/:id/movegens`,
  `/api/workers/anon/:anon_id/movegens`) is their claims' by type, its compute
  time and tasks counted from completed claims only; the jobs' compute time
  by type is the contributors' together; and all of them add up to the
  contributor list's columns, before and after a purge or a delete takes a
  job's out of all three. Only a contributor the list shows is answered -- an
  anonymous one by pseudonym, never by its UUID -- and anyone else is a `404`.
  The breakdown is an index-only walk of the contributor's own index, which
  carries each claim's movegens and claim time. *(Covered:
  `public_api::contributions_are_broken_down_by_job_type`,
  `public_api::a_contributors_breakdown_reads_only_their_index`,
  `public::tests::contributions_are_summed_into_their_own_type`,
  `admin_api::purging_and_deleting_a_job_give_back_what_it_earned`.)*
- `A-PUBLIC-8` **A NUL in what a caller sends is a `400`.** Postgres stores no
  NUL in text and refuses the statement (SQLSTATE `22021`, or `22P05` from
  JSON), which was a `500` and an error line: `?worker=%00` on a job's public
  results feed, a login name, a password-reset address. *(Covered:
  `public_api::a_nul_in_a_public_request_is_a_bad_request`; the mapping,
  `U-ERR-8`.)* (Thirty-third audit, pass 3.)
- `A-PUBLIC-9` The API gzips its JSON for a client that sends
  `Accept-Encoding: gzip` -- deployed, the ALB sends `/api/*` straight to the
  backend, past Nginx -- and not for one that does not; never its event
  stream (`text/event-stream`, whose events gzip would hold back), and never
  an answer with no body (an idle claim's `204`, a redirect). *(Covered:
  `public_api::the_api_compresses_its_json_and_never_its_event_stream`.)*

### `A-ACCOUNT-*` — `routes/account.rs`

- `A-ACCOUNT-1` `me` returns the current user and never a password hash.
  *(Covered: `account::me_returns_the_caller_and_never_a_password_hash`,
  `auth_api::the_account_page_reads_the_contribution_counter`.)*
- `A-ACCOUNT-2` A created API key is returned in full **exactly once** and never
  again by the list endpoint. *(Covered:
  `account::a_new_api_key_is_shown_once_and_never_listed`.)*
- `A-ACCOUNT-3` The 100-key cap is enforced, including against requests
  arriving together: it was a `COUNT` then an `INSERT`. *(Covered:
  `auth_api::concurrent_key_requests_cannot_exceed_the_key_limit`.)*
- `A-ACCOUNT-4` Deactivating a key stops it authenticating; reactivating
  restores it; revoking is permanent. *(Covered:
  `account::deactivation_suspends_a_key_reactivation_restores_it_and_revocation_is_final`.)*
- `A-ACCOUNT-4b` The `Bearer` scheme is read in any case (RFC 7235): `bearer
  <key>` was no credential, so a deactivated key's claim minted an anonymous
  identity. *(Covered: `account::the_bearer_scheme_is_read_in_any_case`.)*
  (Thirty-second audit.)
- `A-ACCOUNT-5` One user cannot see or modify another's keys. *(Covered:
  `account::one_user_cannot_see_or_change_anothers_keys`.)*
- `A-ACCOUNT-6` An account's key creation is a burst of a hundred (the cap), then
  ten an hour: a hundred keys at once are allowed, a key made after revoking one
  is `429`, and another account is unaffected. *(Covered:
  `account::key_churn_is_rate_limited_per_account`.)* (Fourteenth audit; the
  burst, fifteenth.)
- `A-ACCOUNT-7` A key's label is at most 100 characters. *(Covered:
  `account::a_key_label_is_bounded`.)* (Seventeenth audit.)
- `A-ACCOUNT-8` A key's whole life is on record: issued, suspended, resumed
  and revoked, each writes an audit row by the key's id (not its label, the
  owner's free text), where none did and a revoked key left no trace at all;
  a suspend or resume that changes nothing writes none. *(Covered:
  `account::a_keys_life_is_on_record`.)* (Thirty-second audit, pass 22.)
- `A-ACCOUNT-9` Resuming an account's keys is rate limited (`key_changes`):
  toggling a key 300 times, the last 50 resumes are `429`s with no row, and
  the owner can still suspend and revoke; unlimited, one account wrote 4,000
  audit rows in ten seconds. *(Covered: `account::key_changes_are_rate_limited_per_account`.)*
  (Thirty-second audit, pass 23.)

### `A-BOUND-*` — boundaries (`backend/tests/boundaries.rs`)

Limits and edges a coverage review found unasserted, each through the real
router. None found a bug; they pin a limit where moving it would otherwise be
silent.

- `A-BOUND-1` The eleventh login from one address is 429, even with the right
  password. *(Covered:
  `boundaries::the_eleventh_login_from_one_address_is_rate_limited_even_with_the_right_password`.)*
- `A-BOUND-2` One address guessing an account's password cannot lock its owner
  out: its eleventh attempt is 429, and the right password from another address
  signs in. And the hundred-and-first attempt on one username in a minute is
  429 from any address — the half that stops a distributed guess. (Until the
  eleventh audit the username limit was 10 a minute from everywhere, which let
  one address hold any account, an admin's included, out indefinitely.)
  *(Covered: `boundaries::a_stranger_cannot_lock_an_account_out_of_signing_in`,
  `boundaries::a_username_tried_from_everywhere_is_rate_limited`.)*
- `A-BOUND-3` A player config's `kwg_id`, `klv_id` and `winpct_id` must each
  name an `input_data` row of that role; the foreign keys only say the row
  exists. Each swap is a 400 naming both roles, and creates nothing. A
  `cloned_from_id` that names no config is a `400` on that field, not the
  generic foreign-key `409` (twenty-first audit). *(Covered:
  `boundaries::a_player_config_refuses_input_data_of_the_wrong_role`.)*
- `A-MIGRATE-1` A rolled-back image starts against a schema a newer one
  migrated (a migration the database has and the binary does not), and an
  applied migration whose file changed is still refused (twenty-ninth audit).
  *(Covered: `boundaries::a_rolled_back_image_starts_on_a_newer_schema`.)*
- `A-BOUND-4` A valid result bigger than axum's 2 MB default — a capture batch
  — is accepted, which only the route's own `MAX_RESULT_BYTES` limit allows.
  *(Covered: `boundaries::a_capture_result_of_several_megabytes_is_accepted`.)*
- `A-BOUND-5` A result over that limit is a 413 in the API's error shape and
  changes nothing. *(Covered:
  `boundaries::a_result_over_the_limit_is_a_413_in_the_api_shape_and_changes_nothing`.)*
- `A-BOUND-6` A decline naming a hundred files records the first 32
  (`MAX_MISSING_FILES`), each field cut to 128 characters — characters, not
  bytes — and still releases the claim. *(Covered:
  `boundaries::a_decline_list_is_cut_to_its_cap_and_each_field_to_its_bound`,
  `contract_fixtures::decline_gap_fields_are_bounded`.)*
- `A-BOUND-7` An activation that would take active allocations past 100% is
  refused, naming the total and what the other jobs hold, and two concurrent activations cannot together
  exceed it. *(Covered:
  `boundaries::an_activation_past_100_percent_is_refused_naming_the_headroom`,
  `boundaries::two_concurrent_activations_cannot_exceed_100_percent`.)*
- `A-BOUND-8` An admin cannot delete their own account: a 400 saying so, the
  account and its session untouched. *(Covered:
  `boundaries::an_admin_cannot_delete_their_own_account`.)*
- `A-BOUND-9` *(Retired with redundancy: a task has one slot.)*
- `A-BOUND-10` `AppState::new`, what `main` builds, starts the restart grace at
  start plus the heartbeat timeout (`I-SCHED-20`), so the grace cannot be lost
  in the wiring while every test that sets it by hand still passes. *(Covered:
  `boundaries::a_freshly_built_production_state_grants_the_restart_grace`.)*
- `A-BOUND-11` Every API answer, errors included, carries
  `X-Content-Type-Options: nosniff`. *(Covered:
  `boundaries::api_responses_are_not_sniffed`.)* (Sixteenth audit.)
- `A-BOUND-12` A malformed id in a path is a JSON `404` and a malformed query a
  JSON `400`, like every other failure -- as are an unknown endpoint (`404`) and
  a method an endpoint does not take (`405`, nineteenth audit). *(Covered:
  `boundaries::malformed_paths_and_queries_answer_json`.)* (Seventeenth audit.)

---

## 4. Contract

[`contract-fixtures/`](contract-fixtures/) holds one committed example of every
message crossing the birdtest↔MAGPIE boundary. `routes::worker::contract_fixtures`
parses each against the real wire types.

Client→server fixtures are deserialized into the types that actually handle the
request. Server→client fixtures are compared by **field structure, not bytes**,
so fields stay free to move before the first release while a renamed or dropped
field still fails.

Nineteen fixtures exist, and the set is complete: every message has one. The
first nine — an assignment of each original request shape (games, opening
racks, leave generation), a claim, a decline, and each of the four shutdown
reasons — were written by hand, and so were `result-games-inference.json`
(`C-3b`), in MAGPIE's key layout, MAGPIE's own test checking its output
carries every key, and `decline-time-limit.json` (`C-12`); `C-2`..`C-9` were
**captured from a real exchange** (the two captured assignments were given
`job_name`, `max_task_seconds` and `threading_mode` by hand when the server
began sending them, until the next recapture). MAGPIE's
`test/birdtest_contract/` carries a copy, brought up to date with each MAGPIE
pin, and CI's `magpie-contract` job runs MAGPIE's tests against this branch's
copy.

- `C-1` `assignment-opening-rack.json` — exists, and MAGPIE's
  `test/birdtest_contract/` carries a byte-identical copy. *(Covered:
  `contract_fixtures::assignments_carry_a_task_request_this_build_understands`,
  `contract_fixtures::the_assignment_envelope_matches_what_the_server_sends`;
  MAGPIE's `test_contract_fixtures_carry_every_key_contribute_reads`.)*
- `C-2` `assignment-game-pairs.json`, a pairs request told apart from games by
  its `job_type` alone: no `game_pairs` flag repeats the tag. *(Covered:
  `contract_fixtures::the_game_pairs_assignment_is_a_pairs_request`, and
  `U-WIRE-2` reads it.)*
- `C-3` `result-games.json`, with `capture_positions` on, its static,
  endgame and pre-endgame positions ones the assignment's two solving players
  could have run (`U-PLAUS-6`, against real MAGPIE output). *(Covered:
  `contract_fixtures::the_games_result_is_accepted_as_a_submission`.)*
- `C-3b` `result-games-inference.json`, from simming players that infer: a
  first-turn position with no inference, and a later one whose `inference`
  (leaves found, draws, mean equity, the most drawn leaves) is accepted and
  kept with the position. *(Covered:
  `contract_fixtures::the_inferring_games_result_is_accepted_and_keeps_its_inference`;
  MAGPIE's `test_inferring_players_report_their_inference`.)*
- `C-4` `result-game-pairs.json`, carrying the pentanomial — the most important
  one: it is the newest message and the one MAGPIE and birdtest most recently
  disagreed about. Its divergent subset passes `U-PLAUS-8`'s rules, and its
  positions `U-PLAUS-6`'s against its static players. *(Covered:
  `contract_fixtures::the_game_pairs_result_is_accepted_as_a_submission`.)*
- `C-5` `result-opening-rack.json`, a simulating player's analyses, which
  `U-PLAUS-6` accepts from the assignment's simming player. *(Covered:
  `contract_fixtures::the_opening_rack_result_is_accepted_as_a_submission`.)*
- `C-6` `result-leave-generation.json`, on MAGPIE's two-letter test
  distribution. *(Covered:
  `contract_fixtures::the_leave_generation_result_is_accepted_as_a_submission`.)*
- `C-7` `heartbeat.json`. *(Covered:
  `contract_fixtures::heartbeat_parses_as_a_heartbeat_body`.)*
- `C-8` `expected-data.json`, the digest list on an assignment, with a derived
  wordmap. *(Covered:
  `contract_fixtures::the_expected_data_block_matches_what_the_server_sends`.)*
- `C-9` `anon-uuid-assignment.json`, a first claim that mints a UUID.
  *(Covered: `contract_fixtures::a_first_claim_is_assigned_a_worker_uuid`.)*
- `C-11` Every assignment fixture names its job (`job_name`, never empty)
  and states its time limit (`max_task_seconds`, 600 to 86,400), and a games
  or pairs request its `threading_mode`, `igp` or `pgp` -- no other request
  type one. *(Covered:
  `contract_fixtures::every_assignment_names_its_job_its_time_limit_and_a_games_threading_mode`,
  and each envelope's shape, the opening-rack and leave ones included, by
  `the_assignment_envelope_matches_what_the_server_sends`.)*
- `C-12` `decline-time-limit.json`, a task stopped at its time limit: the
  token and `"reason": "time_limit"`, nothing missing. *(Covered:
  `contract_fixtures::a_time_limit_decline_parses_as_a_decline_body`.)*
- `C-10` `decline-missing-data.json`, a decline naming a file not found and one
  found with other bytes, parses as the body the server takes, with `actual`
  left out reading as its `null`; MAGPIE builds the same body key for key.
  *(Covered: `contract_fixtures::decline_parses_as_a_decline_body`; MAGPIE's
  `test_the_decline_body_matches_the_decline_fixture`.)* (Thirty-third audit,
  pass 3: until then nothing on MAGPIE's side read its copy.)

Client→server results are not only parsed: each runs through its job type's
validation, as a submission would, so a captured result the server would
refuse fails here.
MAGPIE's half, in `test/contribute_test.c`, is five tests:
`test_contract_fixtures_carry_every_key_contribute_reads` (every key
`contribute` reads off a server message is in its fixture, and the assigned
`worker_uuid` is in the form the client takes — the server's
`a_first_claim_is_assigned_a_worker_uuid` checks it sends that exact value),
`test_results_carry_every_key_the_server_reads` (the serializers a task's
result is built with still produce every key the result fixtures carry, so a
key renamed in MAGPIE fails there rather than every submission),
`test_only_a_data_shutdown_is_waited_out_for_a_set_aside_job` (the wait-or-exit
decision on each of the four shutdown fixtures),
`test_the_claim_body_matches_the_claim_fixture` (the claim body, built from the
fixture's version and ids, has its keys and values, and states this build's
`BOARD_DIM` and `RACK_SIZE`) and
`test_the_decline_body_matches_the_decline_fixture` (the decline body, built
for the files the fixture names, has its keys and values). MAGPIE's sanitizer CI
shard runs them (`contribute` in its `rest` shard).

Beyond the wire, `contribute_test.c` holds MAGPIE's regression tests for what
the contribute path plays and how it talks. Two came from the eleventh audit:
`test_a_rewritten_klv_is_read_again` (a KLV rewritten on disk under a name
already loaded is read again, and an unchanged one is not — from the second
leave task in a process the previous generation's KLV was being played, which
tier 6's `M-4`, one generation long, could not see) and the `Retry-After`
clamp in `test_http_retries_outlast_a_server_deployment` (a `429` was waited
out for the connect time in microseconds, read as seconds); and
`test_a_request_must_state_its_distribution_and_layout` now also refuses a
path-escaping name in any player object; and `test_a_player_must_state_every_setting`
refuses a player without its leaves (twelfth audit), which a load would
otherwise take from whatever was loaded last. `test_an_abandoned_temporary_is_removed`
(fourteenth audit): a write's `<name>.<pid>.tmp` sibling untouched for an hour
— a killed writer's, up to 1.9 GB for a rack info table — is removed by the
next write of the name, and a fresh one or another name's is not.
`test_inferring_players_report_their_inference`: a simming player that infers
reports, on each captured position past the first turn whose previous move was
not a pass, how many leaves its inference found, how many it drew, their mean
equity and up to ten of the most drawn, most drawn first — unless its turn had
one legal play, which is not simulated and is recorded `static` with that one
play and no inference; a first-turn position reports none, and a player that
does not infer reports none anywhere.
It also checks `result-games-inference.json` holds no key that output lacks.
From the thirty-third audit: `test_an_unverifiable_assignment_is_refused`
(an assignment with no `expected_data`, or one naming an algorithm other than
sha256, or listing a file it cannot check — no `files` array, an entry missing
its role, name or digest, or an unknown role — fails the claim, where it used
to run unverified; and each of the games, game-pairs, opening-rack and leave
assignment fixtures passes the check); `test_a_forced_turn_is_not_simulated`
(a simming player's turn with one legal play, a forced pass, runs no
simulation and says so, leaving the previous simulation's results in place,
which autoplay used to capture as that turn's analysis); and the two
settings-leak tests now also hold a contributor's sim margin forecast
(`-sm1`/`-sm2`, `-smargin`) to off, since no request states it. The claim's
libcurl check (`chttp_is_available`) has no unit test: a test build always has
libcurl.

**Capture** is `scripts/capture_contract.py`, a recording proxy that sits
between `magpie contribute` and the backend, forwards everything unchanged, and
writes the first body of each message type; nothing is normalised. Recapture
with a real MAGPIE:

```
MAGPIE_ROOT=../MAGPIE scripts/e2e_magpie_native.sh --cases capture --capture-out contract-fixtures
```

The `capture` case creates one job of each type, runs a contributor through the
proxy, and fails unless every fixture was captured. Copy the directory into
MAGPIE's `test/birdtest_contract/` in the same change;
[`contract-fixtures/README.md`](contract-fixtures/README.md) has the rest. A
hand-written fixture tests only that the author and the parser agree.

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

```
MAGPIE_ROOT=../MAGPIE e2e/run.sh [--no-build] [--keep] [-- <playwright args>]
```

`e2e/run.sh` builds the three `birdtest-e2e-*` images from the checkout (the
backend, the frontend, the fake worker), brings the stack up as its own compose
project on its own ports (`docker-compose.yml` plus `docker-compose.e2e.yml`:
project `birdtest-e2e`, web on 5280, backend on 8280), seeds a confirmed admin
through `scripts/seed.py`, starts the fake workers, runs Playwright, and tears
down containers, volumes and the mail outbox however the run ends. It needs a
MAGPIE build in `MAGPIE_ROOT`, because the backend refuses to start without one,
but no MAGPIE data. One Playwright worker and no retries: the journeys share one
stack and one allocation budget, and a journey that passes only on its second
try is a failure this tier exists to report. `admin.setup.ts` signs the seeded
admin in once and the admin journeys reuse its storage state.

- `E-1` An anonymous visitor browses the landing page, job list, a job detail
  page — its status on a row of its own, with no description beside an
  active status and "Paused …" or "Finished …" beside any other, then its
  four headline cards (Allocation, Tasks completed, Throughput, Estimated
  time left) — and the Contributions page: the site's totals
  (movegens, compute time, tasks) and the same by job type, then the
  contributor leaderboard, ranked by movegens and re-ranked by tasks at a
  click, with a contributor's own by job type under their row when their name
  is clicked.
  *(Covered:
  `e1-anonymous-browsing.spec.ts`.)*
- `E-2` Register → confirm the email → log in → generate an API key → see it
  exactly once → deactivate it; first, a bad username, `a@b` and no password
  are refused as red text under each field, before any request. *(Covered:
  `e2-register-and-api-key.spec.ts`, reading its code from the outbox.)*
- `E-3` An admin imports input data, reviews the staged diff, and confirms it.
  *(Covered: `e3-input-data-import.spec.ts`, against the fixture tarballs.)*
- `E-4` An admin creates two player configs and a game-pairs job between them
  with **Significance Test** ticked -- its players a checklist whose preview
  says "3 configs → 3 jobs" with a third ticked and "2 configs → 1 job", the
  job named for its pairing -- lands on `/admin/allocation` with the new job
  marked and first, activates it there by giving it an allocation (and
  deactivates it there at the end), and watches the dashboard
  update live over SSE as fake workers contribute; its admin page has the
  match score and Significance Test cards, the latter's sentence giving player 1's
  score per game. **The journey that justifies the tier**: the only place
  SSE, the built Svelte app, the scheduler and a worker are exercised together.
  *(Covered: `e4-live-dashboard.spec.ts`.)*
- `E-5` An admin bans a worker and that worker can no longer claim. *(Covered:
  `e5-ban-worker.spec.ts`.)*
- `E-6` A non-admin is redirected away from `/admin`, and an anonymous visitor
  from `/account`. *(Covered: `e6-redirects.spec.ts`.)*
- `E-7` The ratings page: an admin creates a pool, adds a config, sees the fit
  appear, removes it, and sees the ratings change. Covers the one flow where a
  write is expected to move numbers elsewhere on the page. The pool is created
  through its form, from the ratings list, with only its anchor; membership,
  the fit and the moved ratings go through the pool's page, and with three
  configs the cross table holds every head-to-head from both sides, each
  cell's hover what the ratings predict, each cell tinted by its record as it
  shows it (`data-record`), its last column the table's rating. Signed out,
  the cross table is the first card after the plot and there is no All
  configs card. *(Covered: `e7-ratings.spec.ts`.)*
- `E-8` A job detail page renders the pentanomial table — three rows (Won both,
  Won one, drew one, Even) with a column per player, each read from that
  player's buckets, the higher of a row marked and the even row never — the significance test's verdict in words ("Completed: decided: player 1
  is better, player 1 at … to … after … pairs", or inconclusive at the cap),
  its sentence (player 1's score per game and its 95% interval) and its
  folded explanation, and player 1's record in the match score card, not the
  Significance Test card. *(Covered: `e8-pentanomial.spec.ts`.)*
- `E-9` The password reset flow end to end, `a@b` refused under the field
  first. *(Covered:
  `e9-password-reset.spec.ts`, reading its link from the outbox.)*
- `E-10` A page renders correctly at phone width — one journey, not all of them.
  *(Covered: `e10-phone-width.spec.ts`: a Pixel 5 viewport, the job list, a
  job page (every job setting, which its card shows without a toggle, and
  the Player settings card opened to every setting, the players side by
  side) and both rankings, nothing wider than the screen and each ranking's
  own column inside its box — a pseudonym's sixteen characters pushed it out,
  thirty-second audit. The contributors' list is measured with a row in it;
  the job page again with a 32-character creator and contributor, and both
  lists with a 32-character username and a tombstone (and the contributors'
  list a pseudonym), which the seed does not register; the contributors'
  list shows only its ranked column beside the name — movegens, then compute
  time once ranked by it through its "Rank by" row, since the other
  columns' headers are not on a phone's screen. The site's movegens by job
  type and a contributor's breakdown, unfolded, at a BIGINT's widest each,
  stay inside the screen and the ranking's box.)* The screen is the device's width:
  compared with `innerWidth`, as it was, the check could not fail, because a
  phone's browser widens its layout viewport to fit what overflows — and the
  header's links ran to 533 pixels on a 393-pixel screen, "Sign in" and
  "Register" off it (thirty-first audit; the header now wraps). Checked against
  a build of the pages with the API mocked: 533 before, 393 after.
  `E-10b`: a rating pool's cross table, six long-named configs served from a
  route, scrolls sideways inside its card while the page does not, and its
  sticky first column keeps the names in view scrolled to the ratings at its
  far end.
- `E-11` An admin job page whose first read fails shows nothing the server
  did not say: the data gaps it read apart, the job's own allocation,
  read-only with a link to `/admin/allocation` and no Activate or Deactivate
  (the allocation is set only there), and the failed reads tried again and
  their error cleared; then the job's task time limit, 3600, changed to 1800
  in the Controls card and shown so in its Job settings ("30m (1,800
  seconds)"). And `E-11b`: a read
  started before a live payload does not land over it — a slow retry put back
  the status the stream had moved past, on a job that sends nothing more. And
  `E-11c`: a job deleted while its page is open (by another admin) is said to
  be gone and its actions disabled — only a failed read noticed before.
  *(Covered: `e11-admin-job-first-read.spec.ts`, three tests: the first
  `GET /api/jobs/:id` and the first two gap reads answered `503`; and the
  stream mocked to say `completed` while a retry's read is held, asserting
  the held read carried the old status; and a delete through the API.)* The reads ran one after another: the page said
  "No worker has declined this job", showed 100, and Activate sent it
  (thirty-second audit, pass 16).
- `E-12` A games job between two simmers made through the form with "Position Recorder" ticked
  (and no significance test, the form's default) shows a signed-out visitor
  one position at a time on its board: **Random position** until one follows
  a play, whose tiles are down with the play's outlined, both racks and
  scores with the player to move marked, and its ranked moves, each with its
  Win % and P1-S, P1-BP, P2-S and P2-BP (both players simulate; the fake
  workers shape each position by its mover's config, as MAGPIE does); a rack search
  for that rack typed backwards in lower case shows a position holding it
  (with **Next** and **Previous** when there are more); a rack nothing has is
  said to be. `E-12b`: at phone width the board fits the screen, nothing
  scrolls sideways, and the rack tiles stay square. *(Covered: `e12-saved-positions.spec.ts`, two tests; the
  fake workers play synthetic games whose positions are real boards.)*
- `E-13` An opening-rack job's page offers ten racks it has analysed under the
  search, and clicking one looks it up: a static player's, so its moves show
  score and equity and none of a simulation's columns (Win %, Iters, the
  per-ply ones; `E-12` shows a simmer's). *(Covered:
  `e13-opening-rack-samples.spec.ts`.)* (Thirty-third audit, pass 1: `E-12`
  and `E-13` ran static players and asserted simulation columns, which only
  the fake worker's simulating every position whatever its config let pass.)
- `E-14` A signed-out visitor reaches the player configs from the nav, opens
  one, reads how it searches beside its name and its key settings in a table
  (Lexicon first), grows the table to every setting with **All settings**,
  Lexicon still first, and shrinks it back. *(Covered:
  `e14-player-configs.spec.ts`.)*
- `E-15` The player-config form will not submit a stopping percentage of 0 or
  100, which the server refuses; 99.5 is accepted. *(Covered:
  `e15-player-config-form.spec.ts`.)*
- `E-16` The player-config form offers the pre-endgame only once the endgame
  is on, and turning the endgame off turns it off too; a schedule list that is
  not whole numbers is named and not sent; and a config made to solve both
  shows it on its page: "static, by equity · 6-ply endgame · PEG ≤2" beside
  its name, Uses Endgame "yes (6 plies)" and Uses Preendgame "yes (bag ≤ 2)",
  and under All settings the PEG Schedule MAGPIE defaults to. *(Covered: `e16-solving-player-config.spec.ts`.)*
- `E-17` A game-pairs job made through the form with "Position Recorder" and
  then "Only Where Each Pair First Diverges" ticked (offered only once saving
  is) says "yes (first divergences)" in its settings and shows a
  visitor a pair's two games at one turn, one player's move at a time
  on one board with a toggle between them: "Game 1 of the pair" and "Game 2
  of the pair", the same rack, each player to move in one, each with ranked
  moves; a search for that rack finds the pair once, both
  games of it. *(Covered: `e17-pair-divergences.spec.ts`, against the fake
  workers' synthetic first divergences.)*
- `E-18` An admin changes an inactive opening-rack job's consensus from its
  admin page's Consensus card: it starts at 1 and 1 with Consensus % and
  Save disabled, a maximum below the minimum is named under the inputs, and 2,
  3 and 80% save with "Saved: 0 racks unsettled."; the public page's settings
  then read Minimum Analyses Per Rack 2, Maximum Analyses Per Rack 3 and
  Consensus % 80%. *(Covered: `e18-opening-rack-consensus.spec.ts`; the
  reopening and completing it drives are tier 3's, `I-OR-EDIT-1`, `-2`.)*

### Reading confirmation codes

Two journeys need to read an emailed code, and they do it through
`MAIL_BACKEND=file`.

**First, shrink the problem.** Only `E-2` (register → confirm → log in) and
`E-9` (password reset) need a code at all. The other sixteen need none: those
that sign in use a *confirmed admin*, which `e2e/run.sh` seeds through
`scripts/seed.py` before Playwright starts — so those journeys start at login. That turns "how does the browser
read mail" into a question about two tests rather than eighteen.

**`MAIL_BACKEND=file`** writes one message per file into `MAIL_OUTBOX_DIR`,
named by timestamp and sanitised recipient
(`email.rs::outbox_file_name`), written under a temporary name and renamed so a
reader never sees half a message:

```
$MAIL_OUTBOX_DIR/20260910-191500-000123-e2e-<uuid>-at-example-invalid.txt
```

`docker-compose.e2e.yml` sets `MAIL_BACKEND=file` and `MAIL_OUTBOX_DIR=/outbox`,
bind-mounted from `E2E_OUTBOX_DIR` on the host, a temporary directory `run.sh`
makes and removes. A journey registers `e2e-<uuid>@example.invalid` and reads
the file matching its own address. **Naming by recipient is the point**, not a
convenience: it is what makes parallel journeys safe, and it is exactly what
the log-scraping approach cannot do, since the log is one stream with no key
tying a code to the registration that caused it. (`email::tests` pin the name
and that one readable message is written per send; `U-CFG-4` that the backend
refuses to start as `file` with no directory.)

`scripts/seed.py --mail-outbox DIR` (or `BIRDTEST_MAIL_OUTBOX`) reads the seeded
user's code the same way. Without it, seed.py still scrapes `docker compose
logs` for the console backend's output — kept as the fallback, not removed as
first planned, because the development stack and tier 6 run the console
backend, and there only one registration is in flight.

What was considered and rejected:

| Approach | Why not |
|---|---|
| **Mailpit / MailHog** container with an HTTP API | The one that looks best and is not. birdtest sends through the **SES SDK, not SMTP** ([email.rs](backend/src/email.rs)), so this needs an SMTP backend that production never executes — the E2E tier would be exercising a path that does not ship, which is backwards for the tier whose job is testing what does. |
| A **test-only endpoint** returning the latest code, env-gated | A permanent auth-bypass endpoint. One misconfiguration and anyone can confirm any account. |
| **Scraping `docker compose logs`** from Playwright | Zero code change, and what `seed.py` falls back to under the console backend. Cannot tell which code belongs to which registration when journeys run in parallel, and needs Docker daemon access from wherever Playwright runs. |
| **Reading the database** | Impossible, deliberately: `email_confirmations` stores only a hash, so a leaked dump cannot hand out working confirmation links. |
| A **fixed code** under a test flag | Weakens a real security property in a way that can leak into another environment. |

The cost accepted: a third mail backend is a third thing to keep working. It is
small, and it is written down in `docker-compose.e2e.yml` and `.env.example` so
it does not become folklore.

---

## 6. MAGPIE smoke

The only tier that runs a real MAGPIE. It catches what fixtures structurally
cannot: not whether the *shape* of a message is agreed, but whether MAGPIE's
actual behaviour matches the contract.

**Opt-in, then fail loudly.** Excluded from a default run. When you ask for it
and MAGPIE is missing, that is a hard error, not a skip — a green run must never
silently mean nothing was exercised.

It has two halves:

- **`scripts/e2e_magpie.py`** — the `M-*` cases, each a real `magpie contribute`
  against a real stack, selectable with `--cases`. Locally,
  `MAGPIE_ROOT=../MAGPIE scripts/e2e_magpie_native.sh [--cases M-5,M-11]` runs
  it without building a single image: stock Postgres and MinIO containers, the
  backend and derived-file builder straight from `cargo build`, and a `docker`
  shim first on `PATH` that answers the three compose calls the script makes;
  everything it creates is removed on exit. Nightly CI runs it against the
  compose stack. It needs a `portable_release` MAGPIE build (the backend
  image's instruction-set target, so a wordmap hashes the same on both sides)
  and a `download_data.sh` install.
- **The opt-in Rust tests** — `#[ignore]`d, run with
  `MAGPIE_BIN=../MAGPIE/bin/magpie cargo nextest run --run-ignored all`:
  `magpie_smoke.rs` (5; the server's own conversions in a scratch directory
  laid out by `magpie.rs`, and the builder versions read out of the binary; no
  database), `magpie_leave.rs` (7; leave transitions on real KLVs and a real
  object store, so also `TEST_S3_ENDPOINT`) and `magpie_routes.rs` (2; leave-job
  creation and `rebuild-artifacts` through the router). `MAGPIE_ROOT` (default
  `../MAGPIE`) is where `magpie_smoke.rs` finds MAGPIE's small test lexicon.
  The pull-request run skips these; the nightly runs them against the MAGPIE
  it builds, and they are part of a full local run. Run them against MAGPIE's
  sanitizer build (`make magpie`, the default `BUILD=dev`) as well as the
  release one when MAGPIE's side has changed: the twentieth audit's
  out-of-bounds write on a lower-case rack (`magpie_leave.rs`'s misnamed-rack
  case) passed on the release build and is a stack-buffer-overflow under
  AddressSanitizer.

The backend itself is not opt-in about MAGPIE: it runs a pinned MAGPIE for
every derived file and every leave-generation KLV, reads the builder versions
out of the binary at startup, and refuses to bind without one. `MAGPIE_BIN`
names it (`/usr/local/bin/magpie` in the image, a local checkout's `bin/magpie`
in development). So there is no second implementation to round-trip against;
what the Rust tests check is that the server's use of the one implementation
works — the directory it lays out, the names it invokes, the bytes it gets
back.

**Correctness is established by version and capability probe.**
`birdtest-contribute` reports `0.1.1`, the shipped `MIN_MAGPIE_VERSION` default; a checkout reporting anything lower is refused. The
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
  and is credited. *(Covered: `e2e_magpie.py` `case_games`.)*
- `M-2` One `game_pairs` task does the same, and **both pentanomial invariants
  hold on real games**: `sum(buckets) * 2 == games`, and
  `sum(i * bucket[i]) == 2 * wins + ties`. This is the check that proves
  MAGPIE's pentanomial and birdtest's validation agree. *(Covered:
  `case_pairs`. The invariants are enforced by the server's plausibility checks
  on every accepted pair result, so a clean run is the proof; it is no longer
  manual.)*
- `M-3` One `opening_rack` task lands, with one analysis per requested rack.
  *(Covered: `case_opening_racks`, for a static and a simming player, asserting
  the simulated statistics — win%, per-ply stats — are stored too. The static
  player asks for a wordmap, so the job also waits on the derived-file builder
  and MAGPIE finds its own copy agrees. A third job uses 3-tile racks: jobs
  choose a rack size from 1 to 7 and the request does not restate it, and a
  worker that required full racks refused every such task (MAGPIE `0a69b625`;
  reproduced, fixed in `34bf6f0c`; twenty-third audit).)*
- `M-4` One `leave_generation` task lands, is staged, moves the generation's
  live counters, and a merge folds it into `leave_rack_progress` -- every rack
  a real MAGPIE reported matching a rack of the universe, and a rack holding a
  blank counted (73 of 870 matched nothing before the thirty-second audit).
  *(Covered:
  `case_leave`, which also checks that nothing is written into MAGPIE's data
  directory.)*
- `M-5` A worker whose data digests do not match declines with `missing_data`
  rather than contributing unverified results. *(Covered: `case_missing_data`
  — the gap names the file with both digests, and nothing is stored.)*
- `M-6` A worker below the job's version floor never runs the job. The real
  server filters below-floor jobs out before dispatch, so a real worker is told
  `magpie_too_old` and claims nothing; MAGPIE's own `magpie_version` decline
  is reachable only from a server that hands the job out anyway, which the
  case builds from the capture proxy raising the floor on the way back. *(Covered:
  `case_version_floor`, both halves.)*
- `M-7` `capture_positions` on a real game produces positions whose CGP parses
  back through MAGPIE. *(Covered: `case_positions`. Found that with capture on,
  a static player played its *worst* move — a min-heap read at `[0]` — fixed
  in MAGPIE with regression tests in `gameplay_test.c` and `contribute_test.c`.)*
- `M-8` The KLV a generation transition builds loads in a real MAGPIE.
  *(Now structural: the transition **is** a real MAGPIE writing it. What is
  worth a case instead is that the CSV the server streams is one MAGPIE
  accepts — a rack it names differently, or a generation missing a rack, is
  refused rather than silently valued at zero. Covered, opt-in:
  `magpie_leave::a_rack_named_differently_is_refused_rather_than_valued_at_zero`,
  `magpie_leave::a_transition_folds_the_generation_into_the_klv_it_uploads_and_closes_it`.)*
- `M-9` Two contributors run concurrently without duplicate seeds — the
  concurrency check from `I-SCHED-13`, against the real client. *(Covered:
  `case_concurrent` — every task its own seed, the seeds tiling the space with
  no gap, each worker's claims its own.)*
- `M-10` A job with `use_rit` dispatches only once its table is built, and a
  real `magpie contribute` builds a table whose hash matches the server's and
  plays with it. *(Covered: `case_rack_info_table`, on MAGPIE's two-letter test
  lexicon and distribution (`CSW21_ab`, `english_ab`), so the table is small
  and quick rather than 1.9 GB.)*
- `M-11` A worker whose derived file does not match declines with
  `derived_mismatch` and both digests reach `worker_data_gaps`. *(Covered:
  `case_derived_mismatch`: the server's recorded wordmap hash is made one this
  build does not produce.)*
- `M-12` A player that solves plays its pre-endgame and endgame turns with
  MAGPIE's solvers: a capturing games job with one such player completes, and
  its captured positions include `endgame` and `peg` analyses whose every move
  has a spread and a depth, beside the other player's `static` ones, which
  have none. *(Covered: `case_solvers`, with a small schedule so a game takes
  seconds.)*
- `M-14` A game-pairs job keeping first divergences, on a real MAGPIE, stores
  from each pair that diverged both games' positions at one turn, on one board
  with one rack, each with that player's own different best move, and as many
  pairs as `game_results` counts divergent. *(Covered:
  `case_first_divergences`, equity against score, four tasks of five pairs.)*
  MAGPIE's own side, including that a pair played identically keeps nothing,
  is `magpie_test contribute`'s
  `test_a_pairs_first_divergence_is_both_games_at_one_turn`.
- `M-15` An opening-rack job seeking a consensus (at least 2, at most 3
  analyses a rack, 100%), with a real 1-ply simmer on the two-letter data,
  covers its eight racks, reissues them as lists from seeds past the end of
  the space, and completes with every rack settled after at least two
  analyses. *(Covered: `case_consensus`, one task a run until the job
  completes.)*
- `M-16` A games job between two real 2-ply simmers that infer, capturing
  positions: every position past a game's first turn keeps its inference
  (none on a first turn), with at most ten leaves listed, no more than were
  found, most drawn first, and draws exactly when leaves were found -- an
  inference can find no leave at all, at the default margin of 0, and is kept
  saying so. *(Covered: `case_inference`, one task.)*
- `M-17` A worker at 24 threads finishes a games job whose players solve their
  endgames and pre-endgames, 24 games at once: each game's solves get one
  thread. When every solve took all 24, the games needed 24 + 24 x 24 move
  generators of MAGPIE's 512 and magpie exited. *(Covered:
  `case_solvers_many_threads`, one task.)*
- `M-13` A job with `use_wit` dispatches only once its word info table is
  built; a worker whose table does not match the recorded hash declines with
  `derived_mismatch` and both digests reach `worker_data_gaps`; with the right
  hash a real `magpie contribute` builds the table from the lexicon alone (no
  wordmap) with the server's bytes and plays with it. *(Covered:
  `case_word_info_table`, on the two-letter data like `M-10`.)*

`M-10`, `M-13` and the `capture` case run on MAGPIE's small data, which the script
serves as a tarball from a GitHub stand-in it runs itself
(`--github-fixture-port`; the backend's `GITHUB_API_URL` and `GITHUB_RAW_URL`
point at it -- set through compose's `BIRDTEST_GITHUB_API_URL` and
`BIRDTEST_GITHUB_RAW_URL`, since GitHub Actions will not let a workflow override
a `GITHUB_*` variable -- and every other request is forwarded to GitHub). Every case has a
ten-minute ceiling (`CASE_TIMEOUT`), and each deletes the jobs and worker
directories it made.

That leaves `M-4` the expensive case: real English means the
3,199,724-rack universe above, seeded by the first claim. Of the two ways out
this section once weighed — a single slow generation on real English, or a
small bag both sides load — both are now in use: `M-4` keeps real English and
one generation, accepting a slow nightly case, and the leave-generation
`capture` runs on MAGPIE's own two-letter test distribution, which a real
MAGPIE is known to play with (MAGPIE's own tests use it), rather than on the
fixture's stub `NWL23.kwg`.

---

## Backups and restores

The backup scripts run in production as Fargate tasks and nowhere else, so
until they had a nightly job nothing ran them but production — and both had
broken without anyone noticing. `backups.rs`'s staleness rules are tier 1 and
`/admin/backups` is `A-ADMIN-10`; these are the scripts. Neither contacts AWS:
S3 is the stack's MinIO, and `backup.sh` and `restore-drill.sh` skip their
CloudWatch metrics when `AWS_S3_ENDPOINT` points at a stand-in object store
(`BACKUP_METRICS=false` skips them anywhere, `=true` sends them anywhere).

- `S-BACKUP-1` A backup taken while another session keeps committing rows
  succeeds and records an `ok = true` `backups` row whose digest and row counts
  are the manifest's. *(Covered: `scripts/backup-drill-check.sh`, step 1, which
  runs `backup.sh` exactly as its task does — the official `postgres:16` image,
  the script as `bash -c`.)*
- `S-BACKUP-2` **The restore drill of a backup taken under writes passes.** The
  manifest's row counts were read after `pg_dump` finished, outside its
  snapshot, so they included every row committed during the dump and the drill
  of any backup taken while the service was in use failed. `backup.sh` now
  holds one exported repeatable-read snapshot for the dump and every fact about
  it. The drill restores into a Postgres it starts inside its own container,
  as the production task does, and never into the stack's database (the check
  asserts it left nothing there). *(Covered: `backup-drill-check.sh`, step 2.)*
- `S-BACKUP-2b` The drill of an empty bucket (a new stack before its first
  backup) passes and says there is nothing to drill; the drill of a prefix with
  no backups in a bucket that has some fails, with a message. The empty case
  was meant to pass since the twenty-third audit but never could: `aws s3 ls`
  exits 1 on an empty listing, and `set -e` ended the drill, silently, before
  the check. *(Covered: `backup-drill-check.sh`, step 2b.)* (Twenty-fourth
  audit.)
- `S-BACKUP-3` A backup that cannot upload exits non-zero and leaves an
  `ok = false` row. *(Covered: `backup-drill-check.sh`, step 3.)*
- `S-BACKUP-4` A dump of the current schema restores into an empty database and
  comes back identical: row counts, referential integrity, the denormalized
  task counters, and `BYTEA` and `DOUBLE PRECISION` columns intact, over a seed
  of a row in each of `users`, `input_data`, `jobs`, `tasks`, `task_claims`,
  `game_results` and `backups` (it said "every table a result touches", which
  it never was: no leave or position-analysis row goes through it). *(Covered:
  `scripts/restore-roundtrip.sh`, which had fallen three `NOT NULL` columns
  behind the schema and could not run.)*
- `S-BACKUP-5` **A purged job's rows are copied back as RUNBOOK §2.2 says**
  (`scripts/restore-job.sh`): not before the scratch restore has finished, not
  from production as its scratch copy, not into a job production has active,
  not into a job production has completed since the purge, not beside an export
  of the job made since, not for a job the
  scratch copy holds nothing of; nothing with
  `COPYBACK_DUMP_ONLY`; not from a copy taken after the purge — one already
  holding the purge's audit row, whether of a games job that went on running or
  of a leave job with only its generation-0 artifact written back; a deleted job
  restored whole, its `jobs` row inactive, with its config, player config and
  input data, and resumed after a run stopped once the `jobs` row was in;
  batches smaller than a line;
  the job's merge lock held while it loads; every rating pool left to refit
  on the restored rows;
  position analyses (their moves, plies and inference) back through their
  parents' ids; a row production holds under a restored
  row's key with other contents stops the run with that batch not loaded and
  another job untouched; the same run, after §2.0, finishes what the stopped one
  began; a job loaded in several batches comes back row for row, and again
  changes nothing; the sequences end past the restored ids. *(Covered:
  `scripts/restore-job-check.sh`.)* (Thirty-first audit: §2.2 was a pasted loop
  that loaded each table in one statement, whose foreign-key queue -- 12 bytes
  a row, measured -- would exhaust a `db.t4g.micro` on a large job, and whose
  `ON CONFLICT DO NOTHING` dropped a conflicting row silently. Measured on 1.5
  million progress rows: the queue peaked at 18.9 MB in one statement and at
  3.2 MB, whatever the job's size, in batches.)

All three run nightly (`restore-roundtrip`, which runs `S-BACKUP-5` and `-7` too, and
`backup-drill` in `.github/workflows/nightly.yml`), each against the schema applied to an empty
database — which is also the migration replay the nightly list asks for. The
monthly production drill (`restore-drill.sh` on the newest real dump) remains
the real check of the backups themselves.

- `S-BACKUP-6` `dev-restore.sh` scrubs a restore unless `SCRUB=0`, and refuses
  any other value (`yes`, `true`, `2`, padded spaces) before a single compose
  call — `SCRUB=yes` used to skip the scrub, restoring a production dump's real
  addresses and password hashes. The scrub is of the copy the run created,
  not the stack's database. *(Covered: `scripts/dev-restore-check.sh`, a stub
  compose, in CI's `scripts` job; it fails with the scrub aimed at
  `birdtest`.)* (Thirty-second audit; the copy, pass 24.)
- `S-BACKUP-7` RUNBOOK §1's step that re-applies what a restore undid for
  security, its two blocks read from RUNBOOK.md as written, against a restored
  and a damaged database. It refuses URLs that reach the wrong instance either
  way; an apply before a whole export, without a reviewed `/tmp/after-exclude`,
  against another server than the export's, or with an exclusion that matches
  nothing (rolled back) or is not an id. It applies the audited actions the
  restored instance lacks (revocations, suspensions, resets, confirmations,
  bans and lifts, deletions — each target's last, an admin deleted, one whose
  transaction began before the restore point), and not a bad migration's
  unaudited changes nor an action from before the point; it leaves out an
  actor — its key revocations, suspensions and confirmations too — or one
  action by id, keeps the restored password when the last reset is left out
  or the account was deleted there by a left-out action (and lists the
  passwords it copies), demotes only from the reviewed list and keeps an edit
  to it (showing the proposal beside it); an action an hour after the
  restored instance's newest row and hours before the damaged one's is
  applied; it refuses ids the two logs use for different rows (the restored
  instance writing after the repoint, a renumbered damaged log); pasted
  again, it changes nothing; and it asks only for actions the backend writes.
  Of 26 faults put into the step one at a time (the pass 25 reviewer's 22 and
  four more), each fails it but one: the completion mark set before the
  demotion file is written, which would take a failure between the two lines
  to show.
  *(Covered: `scripts/reapply-check.sh`, nightly in `restore-roundtrip`.)*
  (Thirty-second audit, pass 23; the fourth form, pass 24; the ids and the
  cases above, pass 25.)
- `S-RUNBOOK-1` Every bash block in RUNBOOK.md parses: an operator pastes them
  during an incident, and one that does not leaves a continuation prompt that
  swallows what is pasted next — an apostrophe in a `${STAMP:?…}` message did
  that to the restore's backup-settings command. README.md's blocks too. Every
  fence is parsed: blocks indented in a list are read too (the first cut read
  16 of 25), a fence labelled anything but exactly `bash`, `sql` or `text`
  (`Bash`, `sh`, `bash title=…`, unlabelled), a fence run after any other text
  on its line (a quote mark, a list marker, a nested list), a fence never
  closed, a line less indented than its block's fence, or a closer indented
  past it is refused; code must be printable ASCII (a no-break space after a
  backslash ended the command); a block
  must parse with nothing said (`bash -n` only warns of a heredoc never
  terminated) and must not end in a backslash; and a block that mentions
  `aws` must turn the pager off in its first line of code (the pager
  swallowed a pasted `wait`; an export after the first call was too late).
  *(Covered: `scripts/runbook-check.sh RUNBOOK.md README.md`, `bash -n` per
  block, in CI's `scripts` job.)*
  (Thirty-second audit.)
- `S-SCRUB-1` A scrubbed dump holds no credential: every anonymous worker's
  UUID is replaced, with its claims, ban and audit rows following it and what
  it did kept, and an open claim's token is replaced; twice over, as the script
  allows. *(Covered: `account::a_scrubbed_dump_keeps_no_worker_credential`,
  which also pins `anonymous_workers`' columns to the ones the script copies.)*
  (Thirty-first audit: the UUIDs, each a whole credential, survived scrubbing.)
- `S-SCRUB-2` A scrubbed dump holds no ban reason (an admin's free text about a
  person), in the ban or its audit row; and the script refuses to run unless
  asked for by name (`-v dev_copy=1`), since its old usage line pointed it at
  production; `dev_copy` must be true, not merely set. *(Covered: the reasons
  in `account::a_scrubbed_dump_keeps_no_worker_credential`; the refusal —
  run from a file, and pasted into an interactive psql, where it aborts the
  transaction so nothing after it runs, whatever `ON_ERROR_ROLLBACK` a
  psqlrc sets — and
  `dev-restore.sh`'s restoring into a copy swapped in, in one transaction,
  only when whole and scrubbed — a dump cut in its data, one missing a file,
  SIGTERM and SIGHUP part-way, a copy that cannot be renamed, SIGINT during a
  held swap (which it waits out and reports), two restores at once, a failed
  artifact mirror (exit 1), TERM, HUP and INT at half-second steps through a
  whole run (36 runs), and a Ctrl-C to the whole process group during the
  mirror (pass 25: the compose CLI caught it, and the container went on
  mirroring with `--remove` after the script exited), each leave a `birdtest`,
  a message that says which, the backend running and no mirror container — and `dev-dump.sh`'s writing
  aside, leaving nothing when it fails or is stopped and the old snapshot
  when stopped replacing it, and its artifact mirror, by hand against the real
  mc image: the audit's passes 23 and 24 replayed each against a throwaway
  compose stack.)*

## Terraform

`infra/tests/variables.tftest.hcl`, run by `terraform test` in `infra/` (CI's
`terraform` job). Its runs plan against mock AWS providers (`mock_provider`,
Terraform 1.7 and later): no credentials, no account, nothing created. The
mocks give the data sources what the plan needs (two zones, an account id, a
policy document that is JSON) and nothing else. A refusal run names the
variables it expects refused, and fails if the plan succeeds or fails anywhere
else. Terraform skips a validation that reads a variable already refused, so a
run that breaks one variable expects only that one.

- `S-TF-1` Every variable validation refuses a wrong value and lets the right
  one through: the eight variables with no default as prod.tfvars sets them,
  every default, the first apply's `desired_count = 0` with the schedules off,
  RUNBOOK §5's copy (`-dr`, another region and its zones), and every range at
  its edge plan; a value past each edge — the name suffix, regions, zones,
  images, storage and retention bounds, Fargate's CPU and memory pairs for the
  web, backup and builder tasks, disk sizes, dump jobs, the alert address, the
  SES domain and sender, the public URL, the mail rate and the MAGPIE floor
  (`v0.2.0` and `0.2.0-rc1` refused, `0.2` planned: what the backend's
  `Version::parse_strict` reads at startup) — is refused by the variable that
  guards it. `desired_count` above one (or below zero) is refused:
  the single-instance rule (KL-82). Broken on purpose, each side shows: with
  the count's bound at two, `two_tasks_are_refused` fails; with a certificate
  regex that no real ARN matches, the good plan fails. *(Covered:
  `infra/tests/variables.tftest.hcl`, 58 runs with `S-TF-2`'s, `S-TF-3`'s and `S-TF-4`'s.)*
  (Thirty-third audit, pass 1: a validation was first evaluated by a real plan,
  since `terraform validate` evaluates none. Pass 2 added the MAGPIE floor, the
  one value the backend refuses at startup that the plan let through.)
- `S-TF-2` The cross-variable rules KL-62 left open: `derived_builder_image`
  at another tag than `backend_image`, or tagged beside an untagged one, is
  refused, and the same tag, no tag on either (`latest`), or images named by
  digest plan; an ACM ARN from another region, or another kind of ARN, is
  refused; a sender outside `ses_domain`, or at a domain whose name merely ends
  in it, is refused, and one at a subdomain, in any case, plans;
  `github_token_parameter_arn` as a parameter's name, or as the ARN of a
  parameter in another region, is refused, and an SSM parameter ARN in the
  stack's region plans. *(Covered: `infra/tests/variables.tftest.hcl`.)*
  (Thirty-third audit, pass 1; pass 4 added the GitHub token parameter: a name
  was refused by IAM half-way through the apply, another region's ARN by every
  task at start.)
- `S-TF-3` A deployment the circuit breaker rolls back mails the alerts topic:
  the `-deploy-failed` rule matches `aws.ecs`'s "ECS Deployment State Change"
  with `eventName` `SERVICE_DEPLOYMENT_FAILED` only, on two resources (the
  backend's service and the frontend's), and has a target. That the target is
  the alerts topic (a mock's ARN is unknown at plan on CI's Terraform 1.9.8),
  that the resources are the two services' ARNs, and that the mail arrives,
  are known only at apply (below; README's "Check that the alarms reach you"
  checks the pattern against each live service).
  *(Covered:
  `infra/tests/variables.tftest.hcl`, `a_failed_deploy_is_alerted`.)*
  (Thirty-third audit, pass 2: a rollback was silent, and the next apply
  redeployed the release it abandoned.)
- `S-TF-4` The split services: the backend's stops its one task before
  starting the next (minimum 0%, maximum 100%: the single-instance rule) and
  the frontend's rolls (100%, 200%), each serves only its own container's
  target group, and each task definition holds its own container alone, the
  frontend's running `frontend_image`. *(Covered:
  `infra/tests/variables.tftest.hcl`,
  `the_backend_stops_first_and_the_frontend_rolls`.)* (October 2026: a page
  change stopped the backend, because the two shared one task with minimum
  0%.)

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

**Terraform, beyond its variables.** `infra/` is checked by `terraform
validate`, by the variable validations' plans ([Terraform](#terraform)), and by
applying it. Asserting what the plan builds tests the plan, not the
deployment, and the failure mode that matters — an apply that breaks
production — is not reachable from a test suite. `S-TF-3` reads the one
alert rule whose pattern the plan knows, and `S-TF-4` the two services'
deployment settings, which decide whether a deploy has a gap; that EventBridge delivers ECS's
deployment event to the topic, and the service ARN in its pattern, need a real
account (a mock apply fails on the random ARNs the mocks make).
Backup *restores* are covered by the monthly drill, which is the real check.

**The SES mail backend, against SES.** Every tier runs `console` or `file`;
what they share — composing the message, sending off the request path
(`A-AUTH-8`) — runs under both. The real SDK is run against a local endpoint
(`email::tests::a_refused_send_keeps_what_ses_said`: a refusal is logged
as SES's `code: message`, where it read `service error`, and a send with no
answer as its causes; `a_send_nobody_answers_times_out`: a send an endpoint
takes and never answers fails after its time, where it hung silently;
`a_burst_of_sends_is_paced_in_order`: eight sends at four a second reach SES
spread over two seconds and in the order they were queued, where a burst past
the account's rate was refused, and then the newest waiter won each turn;
`a_full_mail_queue_refuses_rather_than_growing`;
`a_mail_that_waited_past_its_link_is_not_sent` — pass 25), and
`email::tests::every_failed_send_is_logged_as_the_alarm_expects` ties each
failure line's `alarm = "mail_failed"` field to the alarm's JSON filter in
`infra/ses.tf`. What SES itself accepts,
delivers and counts as a bounce is exercised by production, and by the first
confirmation email after a deploy.

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
changes (a version exactly at the floor, an allocation of 0, an empty top
tier) rather than sampling the interior.

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
`dev.py`, because a real worker cannot be fed stubs. That chain is six steps,
which is why it belongs in code rather than in prose.

### The fixture tarball

`fixtures/data-<date>.tgz`, built by `fixtures/build.sh` exactly the way
MAGPIE-DATA builds the real ones (`cp -RL`, `tar -czf`, `split`), so import's
chunk-walking and extraction are exercised for real rather than bypassed. The
build pins mtimes, owners and order, so re-running it does not dirty git. Each
version is a directory under `fixtures/versions/` whose files are mostly
symlinks into the repository, which `cp -RL` dereferences:

- `20260101` — the version tier 5 seeds from, a single file
  (`data-20260101.tgz`, 653 bytes).
- `20260201` — one file re-cut under new bytes and one new one, so importing it
  after `20260101` stages a diff with every disposition in it (`E-3`). Split
  into chunks (`data-20260201.tgz.aa`, `.ab`) the way MAGPIE-DATA ships a large
  version.

Small, because of a useful asymmetry: the server only ever *parses*
`letterdist` and `layout` bytes — that is what `input_data.content` is for. It
does now hand `kwg` and `klv` bytes to MAGPIE, but only for a job whose players
ask for a wordmap, a rack info table or a word info table, which in tier 5 none
do. The `winpct`
rows stay digest-only. So the fixture carries:

| Path | Contents |
|---|---|
| `letterdistributions/english_fixture.csv` | A **deliberately tiny bag**, not real English: the 13-tile, six-letter `testdist.csv` the leave tests use. The server parses this. |
| `layouts/standard15.txt` | The real 244-byte file. |
| `lexica/NWL23.kwg` | A stub. Never read below tier 6. |
| `lexica/NWL23.klv2` | A stub. |
| `strategy/winpct.csv` | A stub (`20260201` adds `winpct_fixture.csv`). |

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
| The 13-tile fixture bag | **431** | 149 |

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

Serving it offline takes two settings and a static tree.
[`inputdata.rs`](backend/src/inputdata.rs) resolves a MAGPIE-DATA ref through
`GITHUB_API_URL` and fetches tarballs from `GITHUB_RAW_URL`, both defaulting to
the real hosts (`MAGPIE_DATA_REPO` substitutes only the `owner/repo` segment).
`fixtures/build.sh` also writes `fixtures/github/`, the two answers an import
asks GitHub for — `api/repos/birdtest/fixtures/git/ref/heads/main`, the branch
as JSON naming a fixed sha, and `raw/birdtest/fixtures/<sha>/versioned-tarballs/` holding the
tarballs — which the e2e stack's `fixtures` service (Nginx,
`fixtures/nginx.conf`) serves with the backend's two URLs pointed at it. Tier 2
points the same two settings at an in-process stand-in that serves a tarball
the test builds (`I-INPUT-*`), and tier 6 runs its own for MAGPIE's small data
(`e2e_magpie.py --github-fixture-port`).

### `scripts/seed.py`

Empty database → work flowing. Drives the **real HTTP API** rather than writing
SQL, so seeding is itself a smoke test of registration, confirmation, validation
and job creation. Two things have no endpoint and are done directly: promoting a
user to admin, and reading the confirmation code.

```
scripts/seed.py [--api URL] [--job-type TYPE] [--tarball-date YYYYMMDD]
                [--magpie-root PATH] [--min-magpie-version V]
                [--mail-outbox DIR] ...
```

1. Register a user, read the confirmation code, confirm it. The code comes
   from the mail, not the database: `email_confirmations` stores only a hash,
   which is the point — a leaked dump must not hand out working confirmation
   links. With `--mail-outbox DIR` (tier 5) it reads the outbox file for its own
   address; without, it scrapes the backend's log, which is what the console
   backend of `dev.py` and tier 6 leaves it — see [Reading confirmation
   codes](#reading-confirmation-codes).
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
   created earlier keep declining an unreleased local build for ever. Or,
   with `--dev-job` (repeatable), dev.py's named jobs instead -- the two on
   the two-letter test data import it first (`--small-tarball-date`,
   `--small-git-ref`) -- or, with `--no-job`, none.
6. Give it an allocation (`PUT /api/admin/jobs/allocations`), which activates
   it; dev.py's new jobs share what the active ones leave free.

Re-running is safe: an unconfirmed account is confirmed, an imported tarball is
skipped, and an active job of the same type -- and name, for dev.py's -- is
reused rather than duplicated.

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
scripts/dev.py [-w N] [--threads N] [--leavegen-job] [--opening-rack-job]
               [--games-job] [--pairs-job] [--no-browser] ...
```

Brings up the stack, waits for health, seeds it -- the admin, the input data,
and the jobs its job flags ask for, which stack, and none without one -- starts
`N` workers, and opens a browser. It is tier 6's setup with the assertions and the teardown removed, and
it calls the same `seed.py`. `--help` lists the rest; README has the table.

**Both scripts exist and are exercised.** `dev.py` has been run end to end
against real MAGPIE contributors: results land, the pentanomial's two
invariants hold on real games, and the test (then the SPRT, now the match
test) reads all pairs rather than a filtered subset.

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
the leave-generation cost noted above -- except that a job flag with `_ab`
appended (`--leavegen-job_ab`) runs on MAGPIE's two-letter test data
(`english_ab`, eight racks), which dev.py serves through tier 6's GitHub
stand-in, so its leave-generation and opening-rack jobs finish in minutes.
Without `_ab`, every job runs on `--lexicon` and the english distribution.
Each job flag becomes `seed.py --dev-job` (`leave_generation_ab`, say); with
none, `--no-job`.

`--no-browser` for SSH sessions and CI.

---

## CI

GitHub Actions.

**Per pull request** (`.github/workflows/ci.yml`), in parallel:

1. **backend** — three kinds of job. **backend-lint**: `cargo clippy --locked
   --all-targets -- -D warnings` and `cargo test --locked --doc`.
   **backend-build**: the test binaries, built once and archived (`cargo
   nextest archive`). **backend-test**, four jobs once the archive is built,
   each with a Postgres 16 service and a MinIO container
   (`TEST_DATABASE_URL`, `TEST_S3_ENDPOINT`): a quarter of the tests each,
   dealt out in turn (`--partition count:N/4`), tiers 1, 2, 3 and 4
   (`backend/.config/nextest.toml` kills a test at ten minutes). One job
   running them all took some seven minutes; a quarter takes about the time
   of its longest test. The `#[ignore]`d tier-6 tests are not run here; the
   nightly runs them.
2. **frontend** — `npm ci`, `npm run check`, `npm test` (tier 1F), `npm run
   build`.
3. **images** — the backend image (which builds the pinned MAGPIE too), a probe
   that the image's MAGPIE runs and reports its builders, the derived-file
   builder image, and the frontend image.
4. **e2e** — tier 5: MAGPIE at the commit `docker/Dockerfile` pins, built `portable_release`
   inside `debian:bookworm-slim` (the backend image's glibc), Playwright's
   Chromium, the backend and fake-worker images from `docker/Dockerfile` and
   the frontend's from `frontend/`, with buildx, then
   `e2e/run.sh --no-build`; the Playwright report and traces are uploaded on
   failure. The image builds are cached between runs: buildx's layer cache,
   and the Dockerfile's cargo cache mounts carried in the Actions cache keyed
   on `Cargo.lock`, so a run whose lockfile is unchanged compiles only the
   backend crate. Built from scratch, the backend's release build was nearly
   six of the job's nine minutes.
5. **terraform** — `terraform fmt -check -recursive`, `terraform validate`
   and `terraform test` (`S-TF-*`, against mock providers; no AWS
   credentials).
6. **scripts** — `scripts/dev-restore-check.sh`: `dev-restore.sh`'s `SCRUB`
   rule against a stub `COMPOSE`, with no Docker; `scripts/runbook-check.sh`:
   every bash block in RUNBOOK.md and README.md parses, fences labelled and
   placed so none goes unread, and each `aws` block turns the pager off first;
   and `scripts/fake-worker-fixtures.sh --check`: the fake worker's captured
   submissions in `backend/src/jobs/testdata/` are what it emits now
   (`U-FAKE-6`) (five minutes at most).
7. **magpie-contract** — MAGPIE's half of the contract: check out MAGPIE at the
   commit `docker/Dockerfile` pins, copy this branch's `contract-fixtures/` over its
   `test/birdtest_contract/`, and run `magpie_test contribute`; then
   `magpie_test builderhash` (the wordmap, rack info table, word info table
   and both KLV builders -- `createdata klv` and `rackequity2klv` -- against
   their pinned hashes), and then every command the server invokes (`convert
   dawg2wordmap`, `convert klvwmp2rit`, `convert kwg2wit`, `createdata klv`,
   `convert rackequity2klv`), run as
   the server runs it on MAGPIE's two-letter test data, each required to exit 0
   with no error and to write its file. A fixture
   changed here and not in MAGPIE fails here; a MAGPIE-side change is caught
   by the nightly run.

**Nightly** (`.github/workflows/nightly.yml`):

- **e2e** — tier 6: MAGPIE built `portable_release` with a real
  `download_data.sh` install, the compose stack (`postgres`, `minio`,
  `backend`) with its GitHub URLs on the script's stand-in, and
  `scripts/e2e_magpie.py` running every `M-*` case, then the opt-in Rust
  tests (`cargo nextest run --run-ignored ignored-only`) against the same
  MAGPIE. Run twice, as a matrix: against the commit `docker/Dockerfile` pins
  (what production runs -- a pin that refused every short opening rack passed
  every other check, twenty-third audit) and against the branch head.
  Dispatchable by hand against another MAGPIE ref (the head leg).
- **restore-roundtrip** — the schema applied to an empty database (the
  migration replay), then `scripts/restore-roundtrip.sh` (`S-BACKUP-4`),
  `scripts/restore-job-check.sh` (`S-BACKUP-5`) and
  `scripts/reapply-check.sh` (`S-BACKUP-7`).
- **backup-drill** — Postgres and MinIO up, the schema applied, then
  `scripts/backup-drill-check.sh` (`S-BACKUP-1`..`3`, and `S-BACKUP-2b`).

Nightly failures are an alert rather than a blocked merge, because they are
slower and more environment-sensitive than a pull request should wait on.

**While the pin is unpushed.** GitHub serves a commit only once it is on a
branch it can reach, so until `docker/Dockerfile`'s `MAGPIE_COMMIT` is pushed to
`birdtest-contribute`:
- `images`, `e2e` and `magpie-contract` fail: the image fetches the commit, and
  the other two check it out.
- The nightly's `e2e` fails on both legs: the pin leg at its MAGPIE checkout,
  and the head leg at "Start the stack", whose backend image (compose builds it
  from `docker/Dockerfile`) fetches the pinned commit too. `restore-roundtrip`
  and `backup-drill` build no image and are unaffected.
- `backend`, `frontend`, `terraform` and `scripts` are unaffected.

Push the pin before relying on CI. Once pushed, it must stay reachable: a
rebase or force-push of the branch past it breaks rebuilding that release,
and so rolling back to it.

---

## Conventions

| Tier | Lives in | Run with |
|---|---|---|
| 1 | `#[cfg(test)] mod tests`, in-file | `cargo nextest run --lib` |
| 1F | `*.test.ts` beside the source in `frontend/src/lib/` | `npm test` |
| 2, 3 | `backend/tests/*.rs`, harness in `backend/tests/common/` | `cargo nextest run` |
| 4 | `routes::worker::contract_fixtures`, over `contract-fixtures/` | `cargo nextest run --lib` |
| 5 | `e2e/tests/*.spec.ts` | `e2e/run.sh` |
| 6 | `scripts/e2e_magpie.py`; `backend/tests/magpie_*.rs`, `#[ignore]` | `scripts/e2e_magpie_native.sh`; `cargo nextest run --run-ignored all` |

**The runner is `cargo nextest run`** (run from `backend/`). It runs each test in
its own process, and `backend/.config/nextest.toml` reports a test as slow after
a minute and kills it after ten, so a hang fails the run instead of stalling it.
`cargo test` runs the same tests without that ceiling; CI uses nextest, and
`cargo test --doc` for the doctests nextest does not run.
`--run-ignored all` adds the tier-6 Rust tests to everything else; a full local
run is `cargo nextest run --run-ignored all` with every variable below set.

Environment variables:

| Variable | Needed by | |
|---|---|---|
| `TEST_DATABASE_URL` | tiers 2–3 | A database on a server where the role may create databases; tests clone a template there and never write to it. The compose Postgres will do. |
| `TEST_S3_ENDPOINT` | the object-store tests (`artifacts.rs`, `exports.rs`, `input_data.rs`, the leave-job tests, `magpie_leave.rs`) | A MinIO; the credentials default to the compose MinIO's (`birdtest`/`birdtestbirdtest`). |
| `MAGPIE_BIN` | the tier-6 Rust tests; the backend itself; `scripts/dev.py` | A MAGPIE binary, default `../MAGPIE/bin/magpie`. |
| `MAGPIE_ROOT` | `e2e/run.sh`, `scripts/e2e_magpie_native.sh`, `magpie_smoke.rs` | A MAGPIE checkout, default `../MAGPIE`, with `bin/magpie` built `portable_release` (and, for tier 6, a `download_data.sh` install in `data/`). |
| `MAGPIE_DATA_PATH` | `scripts/dev.py` | A real MAGPIE-DATA install. |

A test that needs one fails, rather than skips, when what it names is missing.

**Name a test after the claim it proves, not the function it calls.**
`a_negative_standard_deviation_is_rejected`, not `test_check_game_aggregate`. A
failing name should tell you what broke without opening the file.

**Assert the reason, not just the status.** A test that accepts any 400 passes
when the handler rejects the request for the wrong reason. Match on the error
code or a distinctive part of the message.

**A test that needs a service it cannot find fails; it does not skip.** The one
exception is tier 6, which is excluded from the default run by `#[ignore]` — but
once selected, it fails loudly like everything else.

**A test written for an entry starts its doc comment with the id** (`/// I-SCHED-6:
…`, `describe('F-FMT-1 …')`, `test('E-4: …')`), and an entry lists its tests.
Where the test proves something other than the entry's first wording, the doc
comment says so and the entry is corrected here, with the reason.

**Where a bug has been found by hand, the fix lands with the test that would
have caught it**, and the test names the bug. The three named when this list
was written all have theirs: `I-JOB-2` (the renamed `winpct_id` column),
`U-FAKE-3` (the fake worker's opening-rack shape), and `M-2` (the pentanomial
invariants, no longer a manual run).
