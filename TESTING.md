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
| 1 Unit | 167 | `#[cfg(test)]` in `jobs::plausibility` (23), `inputdata` (17), `jobs::racks` (15), `stats::bradley_terry` (12), `stats::sprt` (12), `error` (8), `config` (7), `routes::admin` (7), `jobs::handler` (6), `backups` (5), `auth::api_key` (4), `auth::session` (4), `clientip` (4), `version` (4), `auth::csrf` (3), `compat` (3), `derived` (3), `extract` (3), `jobs::opening_rack` (3), `magpie` (3), `models::job` (3), `routes::public` (3), `sse` (3), `email` (2), `jobs::dispatch` (2), `ratelimit` (2), `exports`, `jobs`, `jobs::game`, `jobs::game_pair`, `routes`, `routes::auth` (1 each) |
| 1F Frontend unit | 112 | Vitest, `frontend/src/lib/`: `format.test.ts` (19), `api.test.ts` (16), `auth.test.ts` (9), `sse.test.ts` (11), `importWatch.test.ts` (9), and `charts/`: `ratingDotPlot.test.ts` (17), `ratingHistory.test.ts` (14), `residuals.test.ts` (11), `pentanomial.test.ts` (6) |
| 2 Integration | 145 | `backend/tests/`: `leave_gen.rs` (28), `ratings.rs` (25), `scheduler.rs` (18), `jobs.rs` (14), `stats.rs` (15), `input_data.rs` (11), `derived.rs` (8), `leave_generation.rs` (7), `exports.rs` (7), `submissions.rs` (5), `artifacts.rs` (4), `audit.rs` (3) |
| 3 API | 169 | `backend/tests/`: `worker_api.rs` (43), `admin_api.rs` (31), `auth_routes.rs` (17), `worker_routes.rs` (16), `boundaries.rs` (18), `public_api.rs` (12), `admin_routes.rs` (10), `authz.rs` (7), `account.rs` (6), `auth_api.rs` (5), `finish.rs` (3), `fake_worker.rs` (1) |
| 4 Contract | 14 | `routes::worker::contract_fixtures`, over 16 fixtures; MAGPIE checks its half in `test/contribute_test.c` |
| 5 End-to-end | 10 | Playwright journeys `E-1`..`E-10` in `e2e/tests/*.spec.ts`, plus the `admin.setup.ts` sign-in they share; run by `e2e/run.sh` |
| 6 MAGPIE smoke | 10 cases + 14 | `scripts/e2e_magpie.py`'s cases `M-1`..`M-7`, `M-9`..`M-11` against a real `magpie contribute` (natively via `scripts/e2e_magpie_native.sh`, or the nightly compose job); and 14 opt-in `#[ignore]` Rust tests that run the server's own MAGPIE (`MAGPIE_BIN`): `magpie_smoke.rs` (5), `magpie_leave.rs` (7), `magpie_routes.rs` (2) |

The tier-2/3 split is by the ids a file proves; many tier-2 files also drive
the router to reach a state, and several tier-3 files read the database
directly to assert one. With the tier-6 tests selected, `cargo nextest run
--run-ignored all` runs 509 backend tests (the per-tier counts above are
from `cargo nextest list --run-ignored all` and `vitest`, twentieth audit;
they had drifted by up to 17).

Tier 2 was the largest gap and the highest value, and is now the largest tier.
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
| `stats/sprt.rs` | 1 | Covered — values pinned to independently computed numbers (`U-STATS-*`) |
| `stats/bradley_terry.rs` | 1 | Covered |
| `version.rs` | 1 | Covered |
| `compat.rs` | 1 | Covered |
| `jobs/racks.rs` | 1 | Covered (`U-RACK-*`) |
| `derived.rs` | 1 + 2 (+ 6) | Covered — naming and the gate at tier 1, the queue at tier 2 (`I-DERIVED-*`); the build itself at tier 6 (`magpie_smoke.rs`, `M-10`) |
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
| `artifacts.rs` | 2 | Covered (`I-ART-*`, needs `TEST_S3_ENDPOINT`) |
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
| `lib/api.ts` (error mapping, CSRF header) | 1F | Covered (`F-API-*`) |
| `lib/sse.ts` | 1F | Covered (`F-SSE-*`) |
| `lib/auth.ts` | 1F | Covered (`F-AUTH-*`) |
| Chart maths | 1F | Covered (`F-CHART-*`). The arithmetic moved out of the components into `lib/charts/*.ts` so it could be tested; the `.svelte` files that draw it are exercised only by tier 5 |
| Every page under `routes/` | 5 | Partial — the ten journeys. `/users`, `/admin/backups`, `/admin/derived-data`, `/admin/fleet` and `/admin/users` are in none of them; their endpoints are tier 3 |

### Scripts and cross-repo

| Area | Owning tier | Status |
|---|---|---|
| `worker/fake_worker.py` output shapes | 1 | Covered — a captured submission for every job type and every `--mode` (`U-FAKE-*`) |
| `scripts/seed.py` | 5, 6 (used by both) | Covered by use: `e2e/run.sh` and `e2e_magpie.py` both seed through it |
| `scripts/dev.py` | Manual | Deliberate — see [Not tested](#what-is-deliberately-not-tested) |
| `scripts/backup.sh`, `restore-drill.sh`, `restore-roundtrip.sh` | Nightly | Covered (`S-BACKUP-*`) |
| birdtest ↔ MAGPIE wire | 4 + 6 | Covered — `C-1`..`C-9` on both sides, and tier 6 |

---

## 1. Unit

Pure logic, no I/O. Rust idiom: `#[cfg(test)] mod tests` in the same file as the
code.

**May not**: open a socket, connect to a database, or read a file. Fixture bytes
come from `include_bytes!`; anything that needs a path uses a `tempdir`.

### Covered before the list

`stats::sprt`, `stats::bradley_terry`, `version`, `compat`, `backups`,
`derived`, `magpie`, `clientip`, `routes::admin`'s settings validation, and the
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
  minimal two-letter one, and reports the right tile counts and machine letters.
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
- `U-RACK-9` A distribution with more letters than MAGPIE's `MAX_ALPHABET_SIZE`
  (50) is refused, naming the file; one at the limit parses. MAGPIE loaded a
  longer one and wrote past every per-letter array. *(Covered:
  `racks::tests::a_distribution_past_magpies_alphabet_is_refused`; job creation
  refuses one in `A-ADMIN-3`.)* (Twenty-second audit.)

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
- `U-ERR-4` A `sqlx::Error` converted into `AppError` becomes a 500 whose public
  message does **not** contain the SQL string or the database URL. A leaked
  query in an error body is the failure this test exists for. *(Covered:
  `error::tests::a_database_error_does_not_leak_the_query_or_the_url`,
  `error::tests::a_driver_error_is_not_shown_to_the_client`.)*
- `U-ERR-6` A constraint violation is a 409 that does not name the constraint,
  and a pool timeout is a 503 with `Retry-After`. *(Covered:
  `error::tests::constraint_violations_are_conflicts_without_the_constraint_text`,
  `error::tests::a_pool_timeout_is_a_503_with_retry_after`.)*

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
  `MAIL_OUTBOX_DIR` — fails startup naming the setting rather than becoming its
  default. *(Covered:
  `config::tests::a_malformed_value_is_refused_rather_than_defaulted`.)*

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
- `U-WIRE-7` `SprtParams::from` reads the same alpha/beta/elo values out of a
  `GameConfig` and a `GamePairConfig`, so the two job types cannot diverge.
  *(Covered: `job::tests::sprt_params_read_the_same_settings_from_games_and_pairs`.)*

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
  `registry::store_result` runs for both job types; there is no
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
- `U-PLAUS-4` A negative `num_moves` is refused (cast to `usize` it was larger
  than any list), and per-ply statistics must be numbered from 0 in order, with
  a bingo percentage in [0, 100] and a finite, non-negative average score.
  *(Covered:
  `plausibility::tests::a_worker_cannot_report_more_moves_than_it_generated`,
  `plausibility::tests::per_ply_statistics_are_statistics`.)* (Eleventh audit.)

### `U-STATS-*` — pinned numbers (`stats/sprt.rs`, `stats/bradley_terry.rs`)

Every other statistics test checks a property (monotone, symmetric, order-free),
which a formula wrong by a constant factor passes. These compare against values
computed outside the code, in 40-digit decimal from PLAN.md's formulas.

- `U-STATS-1` The bounds at the default α = β = 0.05 are ln(0.05/0.95) and
  ln(0.95/0.05), ±2.944438979…. *(Covered:
  `sprt::tests::the_default_bounds_are_the_documented_logarithms`.)*
- `U-STATS-2` A games tally, W21 L7 D2 at ±10 Elo, scores LLR 1.125953543…; a
  pentanomial [1, 3, 7, 3, 2] scores 0.207499670…. *(Covered:
  `sprt::tests::a_games_tally_scores_the_documented_llr`,
  `sprt::tests::a_pentanomial_scores_the_documented_llr`.)*
- `U-STATS-3` A losing tally reaches H0: 28-72 over 100 games is −3.1400…,
  past the lower bound; 29-71 is −2.9347…, just inside and still running; the
  mirror images pass and run. *(Covered:
  `sprt::tests::a_losing_tally_reaches_h0_and_one_game_short_does_not`.)*
- `U-STATS-4` A Bradley-Terry standard error equals the analytic
  (400/ln 10)/√(n·p·(1−p)): 49.1348… Elo for an even 50 games, a tenth of that
  at 5,000, and 1.2687… for 75% over 100,000. *(Covered:
  `bradley_terry::tests::more_games_narrow_the_standard_error`.)*

### `U-FAKE-*` — fake worker shapes (`worker/fake_worker.py`)

The fake worker is the only client below tier 6, so shape drift is invisible
until a job of that type is run — which is how its opening-rack submission came
to answer with a field the server had never accepted.

- `U-FAKE-1` A captured `games` submission deserializes into
  `GameResultsResponse` and passes `process_response`. *(Covered:
  `plausibility::tests::the_fake_workers_games_submission_passes_validation`.)*
- `U-FAKE-2` A captured `game_pairs` submission does the same, **including** the
  pentanomial cross-checks. *(Covered:
  `plausibility::tests::the_fake_workers_game_pairs_submission_passes_the_pentanomial_cross_checks`.)*
- `U-FAKE-3` A captured `opening_rack` submission deserializes into
  `PositionAnalysisResponse`. *(Covered:
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

Fixtures live in `backend/src/jobs/testdata/fake_worker_*.json`, one per job
type and mode, and are regenerated by the commands in
`backend/src/jobs/testdata/README.md`, not hand-edited.

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
  another kind is refused. The current `data-20251004.tgz` carries such aliases,
  and refusing them refused the whole release. *(Covered:
  `inputdata::tests::a_symlink_alias_is_pinned_with_its_targets_bytes`,
  `inputdata::tests::a_symlink_that_is_not_an_alias_inside_the_archive_is_refused`.)*
- `U-ARCHIVE-6` The limits are PLAN.md's table; changing one is a design change.
  *(Covered: `inputdata::tests::the_walk_limits_are_the_ones_the_design_states`.)*
- `U-ARCHIVE-7` A file whose name MAGPIE would refuse as a path — a `.`, a
  space, an empty name — is not importable, so no job can be pinned to it and
  stop every worker it reaches. *(Covered:
  `inputdata::tests::ignores_what_birdtest_does_not_pin`.)* (Twelfth audit.)

---

## 1F. Frontend unit

Pure TypeScript, no browser, no network. Vitest, run by `npm test` (`vitest
run`) in `frontend/`, and by CI after `npm run check`. `fetch` and
`EventSource` are stubbed per test; nothing renders a component. Instead the
chart arithmetic was moved out of the `.svelte` files into plain modules under
`lib/charts/` (`ratingDotPlot.ts`, `ratingHistory.ts`, `residuals.ts`,
`pentanomial.ts`), which the components import and the tests call directly —
so `@testing-library/svelte`, once planned here, was never needed.

This tier exists because the charts contain real arithmetic — a scale, a bucket
index, a threshold — and getting those wrong produces a plausible-looking
picture rather than an error. Writing it found three: `duration` chose its unit
before rounding (59.6 s read "60s"), `api.ts` rejected a non-JSON body with a
`SyntaxError`, and `RatingHistoryChart` coloured by rank, so two lines swapped
colours whenever their ratings crossed.

### `F-FMT-*` — `lib/format.ts`

Each entry's tests are the `describe` block named for its id.

- `F-FMT-1` `workerLabel` renders a username when present, "Anonymous" plus a
  short UUID prefix when not, and never leaks a full UUID. *(Covered:
  `format.test.ts`.)*
- `F-FMT-2` `duration` renders seconds, minutes, hours and days at the right
  boundaries, and `null` as a dash rather than "null". *(Covered:
  `format.test.ts`; the unit is now chosen after rounding.)*
- `F-FMT-3` `datetime` renders `null` as a dash and a valid ISO string as a
  local time; an unparseable string does not throw. *(Covered:
  `format.test.ts`.)*
- `F-FMT-4` `jobTypeLabel` covers all four job types, and an unknown type falls
  back to the raw string rather than "undefined" — including a name like
  `toString` that an object literal answers. *(Covered: `format.test.ts`.)*
- `F-FMT-5` `sprtLabel` covers all four statuses. *(Covered:
  `format.test.ts`.)*
- `F-FMT-6` A blank optional number is `null`, never 0 (Svelte binds a cleared
  number box as `null`, and `Number(null)` is 0, which the player-config form
  wrote into configs that cannot be edited), and a request's blank required
  fields are named before it is sent (the job form). *(Covered:
  `format.test.ts`.)* (Twenty-second audit.)

### `F-API-*` — `lib/api.ts`

- `F-API-1` A non-GET request sends `x-csrf-token` read from the cookie; a GET
  does not. *(Covered: `api.test.ts`.)*
- `F-API-2` A 204 resolves to `undefined` rather than throwing on an empty body.
  *(Covered: `api.test.ts`.)*
- `F-API-3` A 4xx with a JSON error body rejects with an `ApiError` carrying
  `status`, `code`, `message` and `fields`. *(Covered: `api.test.ts`.)*
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
  import runs; it stops once staged (keeping the id) and forgets a failed one;
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

### `F-CHART-*` — chart maths

Test the pure functions; do not snapshot the SVG.

- `F-CHART-1` `RatingDotPlot` places the anchor's dot at the anchor rating, and
  a config one standard error away at the expected offset. *(Covered:
  `charts/ratingDotPlot.test.ts`.)*
- `F-CHART-2` It clamps a runaway error bar rather than letting one
  barely-measured config flatten the scale, and still reports the true number in
  the table. The anchor has no bar at all, whatever error the fit stored for it
  (with no games, `f64::MAX`, which was drawn at the cap). *(Covered:
  `charts/ratingDotPlot.test.ts`.)*
- `F-CHART-3` A config with `connected_to_anchor: false` is listed as unrated
  and **not** drawn at a position. *(Covered: `charts/ratingDotPlot.test.ts`.)*
- `F-CHART-4` `RatingHistoryChart` caps at six series, picks them by latest
  rating, and reports how many it omitted. *(Covered:
  `charts/ratingHistory.test.ts`.)*
- `F-CHART-5` It assigns colour by config identity: a config keeps its colour
  when ratings cross between fits, drawn colours are always distinct, and a
  config that holds its own palette slot (the one its id hashes to) keeps it
  under any filtering. That is narrower than "filtering never repaints a
  survivor": with six colours and ids that can hash to one slot, a config that
  lost a clash sits in a free slot, and a different set of survivors can free
  or take that slot. Every survivor keeping its colour under every filter would
  need a palette as large as the set of configs ever drawn. *(Covered:
  `charts/ratingHistory.test.ts`; it used to colour by rank.)*
- `F-CHART-6` `ResidualMatrix` sorts by absolute residual descending, and flags
  the non-transitive case only when at least three head-to-heads exceed the
  threshold on enough pairs to be at least three standard errors out — the
  same misses on ten pairs each do not raise it. *(Covered: `charts/residuals.test.ts`; the component now sorts
  itself instead of drawing in the order it is handed.)*
- `F-CHART-7` The job page maps pentanomial buckets to the right labels — index
  0 is "P1 lost both", index 4 "won both". An off-by-one here inverts the
  reading of every paired job. *(Covered: `charts/pentanomial.test.ts`.)*
- `F-CHART-8` Percentages are computed against pairs, not games, and a zero
  denominator renders 0.0% rather than `NaN`. *(Covered:
  `charts/pentanomial.test.ts`.)*

### `F-AUTH-*` — `lib/auth.ts`

- `F-AUTH-1` `refreshSession` sets the store to the user on 200 and to `null` on
  any failure — a 401, a 5xx, or no network — distinguishing "signed out" from
  "not yet known" (`undefined`). Not `null` only on 401, as this entry first
  said: the layout guards wait while the store is `undefined`, so leaving it
  there during an outage would leave every guarded page waiting forever; `null`
  lets them resolve. *(Covered: `auth.test.ts` — 200, 401, a 503 and a
  network failure, in flight, and a later 401 replacing a user.)*
- `F-AUTH-2` `signOut` clears the store even if the request fails, so the UI
  cannot be left showing a session that is gone. *(Covered: `auth.test.ts`.)*

---

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
let job    = db.games_job(/* redundancy */ 2, /* games per batch */ 2).await;
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
- `I-SCHED-3` A job at 0% is offered to nobody, exactly as an inactive one is:
  every claim goes to the other active job, and with every active job at 0%
  the answer is `204`, not a shutdown — including when a parked job is too new
  for the worker or in its unsupported set, which shuts nobody down until the
  job is raised above 0% (`a_parked_job_shuts_nobody_down`). There is no
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
  `abandoned`, decrements `active_claim_count`, and returns an at-capacity task
  to `available` — including a redundancy-2 task with one slot accepted and the
  other lapsed, which reopens rather than staying `claimed` or completing.
  *(Covered: `scheduler::a_lapsed_claim_is_reclaimed_by_the_next_claim_for_its_job_only`,
  `boundaries::a_task_with_one_accepted_and_one_lapsed_slot_reopens_for_someone_else`.)*
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
- `I-SCHED-14` A task at `redundancy` active claims is not handed to an
  additional worker, and the worker whose result filled a slot is never offered
  that task again. *(Covered:
  `scheduler::a_task_at_redundancy_is_not_handed_to_a_third_worker`,
  `boundaries::a_task_with_one_accepted_and_one_lapsed_slot_reopens_for_someone_else`,
  `worker_api::redundancy_does_not_starve_a_worker_holding_a_slot`.)*
- `I-SCHED-15` **The `declined` partial-index trap.** `task_claims_user_unique_idx`
  and `task_claims_anon_unique_idx` are partial on
  `WHERE state NOT IN ('abandoned','declined')`. Drop `'declined'` from either
  and a worker that declines a task is permanently barred from claiming it again
  after fixing its data. Nothing else catches this. *(Covered:
  `scheduler::a_worker_that_declined_a_task_can_claim_the_same_task_again`.)*
- `I-SCHED-16` One worker cannot hold two simultaneous claims on the same task,
  by either identity type. *(Covered:
  `scheduler::one_worker_never_holds_two_live_claims_on_one_task`.)*
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
  that was not there — and the worker's heartbeat and result are then accepted
  (`worker_api::a_restarted_server_does_not_abandon_claims_it_could_not_have_heard_from`).
  Once the grace has passed, a claim that stayed silent is reclaimed as ever
  (`worker_api::a_claim_still_silent_after_the_grace_is_reclaimed`). And the
  state `main` actually builds grants the grace
  (`boundaries::a_freshly_built_production_state_grants_the_restart_grace`,
  `A-BOUND-10`) — the tests above set it by hand.

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
- `I-EXPECT-4` A `leave_generation` job yields exactly `kwg`, `letterdist` and
  `layout`, and never a `klv` — its leaves come from the server-built artifact.
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
  every column populated, and reads back identical. *(Covered:
  `jobs::each_job_type_stores_every_setting_it_was_created_with`,
  `jobs::a_leave_generation_job_stores_every_setting_it_was_created_with`.)*
- `I-JOB-2` **`validate_shared_player_options` runs against real rows.** Two
  configs with different `winpct_id` are rejected; two with the same are
  accepted; two with different `movegen_margin` are rejected. The regression
  guard for the renamed column. *(Covered:
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
- `I-JOB-6` Activation sets `allocation` and `activated_at`; deactivation clears
  the schedule without destroying tasks; completion is terminal. *(Covered:
  `jobs::a_job_moves_through_its_lifecycle_and_completion_is_final`,
  `admin_api::a_completed_job_cannot_be_deactivated`.)*
- `I-JOB-7` An allocation outside 0–100 is rejected. *(Covered:
  `jobs::an_allocation_outside_0_to_100_is_refused`; the sum across jobs is
  `A-BOUND-7`.)*
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
- `I-JOB-11` A player config referenced by any job cannot be deleted. *(Covered:
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
  `active_claim_count`, and completes the task at `redundancy`. *(Covered:
  `submissions::accepted_results_move_the_task_counters_and_complete_it_at_redundancy`.)*
- `I-SUBMIT-4` A submission against a stale claim token is ignored, not
  accepted, and does not move any counter. *(Covered:
  `worker_api::submissions_for_reclaimed_or_already_accepted_claims_change_nothing`,
  `worker_api::a_claim_token_works_only_for_the_identity_it_was_issued_to`.)*
- `I-SUBMIT-5` **Position capture deduplication**: under `redundancy = 2`,
  identical replayed games produce one set of `position_analysis_records`, not
  two. *(Covered: `worker_api::redundant_captured_positions_are_recorded_once`.)*
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
  `rebuild_artifacts` share `generation_means` precisely so a rebuild cannot
  drift; fold, rebuild, compare digests. And a forced rebuild that writes
  different bytes records them as `served_sha256` — what workers are told to
  check the object against — while `sha256` keeps the first hash; and a check
  that rewrites nothing still sets it from the object the key holds, so an
  older object version copied back is served under its own hash again
  (fourteenth audit). *(Covered,
  tier 6 opt-in: `magpie_leave::a_rebuild_reproduces_every_generations_bytes`.)*
- `I-LEAVE-9` A job with `generation_count > 1` advances to the next generation
  and finishes after the last. *(Covered:
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

### `I-RATE-*` — rating pools (`ratings.rs`)

`build_matrix` was checked by hand against a live database; these make it
permanent.

- `I-RATE-1` A matching `game_pairs` job's results enter the matrix. *(Covered:
  the control job every `I-RATE-2` test fits against
  (`ratings.rs`'s `assert_only_the_control_counted`), and
  `ratings::a_head_to_head_counts_pairs_and_scores_half_points_over_four`.)*
- `I-RATE-2` Excluded: wrong `variant`, wrong `letterdist_id`, wrong `layout_id`,
  a job whose player is not a pool member, and a plain `games` job. One test per
  exclusion, because each is a separate clause. *(Covered:
  `ratings::a_pairs_job_in_another_variant_is_not_evidence`,
  `ratings::a_pairs_job_on_another_letter_distribution_is_not_evidence`,
  `ratings::a_pairs_job_on_another_board_layout_is_not_evidence`,
  `ratings::a_pairs_job_against_a_non_member_is_not_evidence`,
  `ratings::a_plain_games_job_is_not_evidence`.)*
- `I-RATE-3` The pair is the unit: a head-to-head's `games` equals pairs, not
  games, and its score is the half-point total over four. Under redundancy 2 a
  task is evidence once, through its first accepted copy. *(Covered:
  `ratings::a_head_to_head_counts_pairs_and_scores_half_points_over_four`,
  `ratings::a_redundant_task_is_evidence_once_through_its_first_accepted_copy`.)*
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

### `I-STATS-*` — dashboard aggregates (`jobstats.rs`)

- `I-STATS-1` A `games` job's stats sum every result and compute SPRT over
  games. *(Covered:
  `stats::a_games_jobs_stats_sum_every_result_and_test_the_games`, with the
  LLR checked against `U-STATS-2`'s independently computed value.)*
- `I-STATS-2` A `game_pairs` job's stats sum the pentanomial, and
  `units_completed` equals the pair count — **not** the divergent count.
  *(Covered: `stats::a_pairs_jobs_stats_sum_the_pentanomial_and_count_every_pair`.)*
- `I-STATS-3` `divergent_pairs` is reported and is not what SPRT consumed.
  *(Covered: `stats::divergent_pairs_are_reported_but_not_tested`.)*
- `I-STATS-4` A job with no results reports zeros and an LLR of 0, not an error
  or a NaN. *(Covered: `stats::a_job_with_no_results_reports_zeros_not_nan`.)*
- `I-STATS-5` Opening-rack stats count analysed racks against `total_racks`, from
  the running `jobs.racks_analyzed` total, and count a task's racks **once** even
  when two redundant claims of it are accepted. *(Covered:
  `worker_api::analysed_racks_are_counted_once_per_task_as_they_arrive`.)*
- `I-STATS-5b` The job list's `units_completed` reads the same kind of running
  total and agrees with `game_stats` on a redundancy-2 job, and every aggregate
  reads the same copy the total counted: the first *accepted*, in acceptance
  order under the task's row lock — not the earliest `submitted_at`, which was
  the submitting transaction's start and so could name the copy accepted
  second. *(Covered: `worker_api::redundant_results_for_one_task_count_once`,
  `worker_api::the_copy_every_aggregate_reads_is_the_first_accepted_not_the_first_begun`;
  `submitted_at` is now `clock_timestamp()`.)*
- `I-STATS-5d` **Concurrent submissions for one task count once.** Two
  redundant claims of a task submitting at the same moment (each blocked,
  before commit, on a lock the other holds) still add the task's games to the
  running total once. *(Covered:
  `worker_api::concurrent_redundant_results_count_once`.)*
- `I-STATS-5c` A purge zeroes both running totals. *(Covered:
  `admin_api::a_job_can_be_purged_and_its_dispatch_counter_resets`.)*
- `I-STATS-6` Leave-generation stats report racks at target against the
  universe, and the current generation. *(Covered:
  `stats::leave_stats_report_the_current_generations_racks_against_its_universe`.)*
- `I-STATS-7` `worker_contributions` attributes tasks to the right identity and
  totals correctly across both identity types. *(Covered:
  `stats::contributions_are_attributed_to_each_identity_across_both_kinds`,
  `worker_api::contributions_are_counted_as_they_arrive`.)*
- `I-STATS-8` ETA is `None` without recent throughput rather than infinity.
  *(Covered: `stats::the_eta_is_none_without_recent_throughput`.)*
- `I-STATS-8b` A games job's ETA divides by the redundancy: units left at claims
  an hour × batch ÷ redundancy. *(Covered:
  `stats::the_games_eta_divides_by_redundancy`.)* (Sixteenth audit.)
- `I-STATS-8c` A job activated less than an hour ago is measured since its
  activation (at least a minute), not over the hour: ten minutes in, the hour's
  average read six times the real time left. *(Covered:
  `stats::a_new_jobs_eta_is_measured_since_it_was_activated`.)* (Twenty-second
  audit.)
- `I-STATS-9` **The finish check** completes a job at SPRT significance and at
  the hard cap, and does **not** complete below `min_units` even with a crossed
  LLR. There is no `finish_if_done`, as this entry first named it: the check is
  `after_submission` in `routes/worker.rs`, which a submission runs on every
  `SPRT_CHECK_EVERY`th (8) result for a job, or whenever it leaves the job
  nothing in flight — so it is tested through the worker API. *(Covered:
  `stats::a_job_completes_when_its_llr_crosses_at_min_games`,
  `stats::a_job_completes_at_its_hard_cap_without_a_verdict`,
  `stats::a_crossed_llr_below_min_games_does_not_complete_the_job`; the
  bounds themselves by `U-STATS-1`, `-3`.)*
- `I-STATS-9e` The verdict a job completed on is stored with the completion —
  status, LLR and units — and reported beside the live figures; a result in
  flight at completion lands and moves the live LLR, and the stored verdict
  stays. *(Covered:
  `stats::a_job_completes_on_the_batch_that_crosses_the_bound_and_not_before`,
  `stats::a_job_driven_to_h0_completes_with_its_sprt_failed`,
  `finish::under_steady_load_the_finish_check_runs_on_every_nth_submission`.)*
  (Eleventh audit; the in-flight half, twelfth.)
- `I-STATS-9a` **Debounced under load.** With a claim held open the whole time,
  so the job is never idle, the check runs on the `SPRT_CHECK_EVERY`th
  submission and not before, and the open claim's result is still accepted
  after completion. *(Covered:
  `finish::under_steady_load_the_finish_check_runs_on_every_nth_submission`.)*
- `I-STATS-9b` **At the bound, not before, and at H0.** 71-29 over 100 games
  (LLR 2.9347 against 2.9444) leaves the job active; another 54-46 (3.0693)
  completes it; 28-72 completes it with its verdict failed. *(Covered:
  `stats::a_job_completes_on_the_batch_that_crosses_the_bound_and_not_before`,
  `stats::a_job_driven_to_h0_completes_with_its_sprt_failed`.)*
- `I-STATS-9c` **An opening-rack job completes** once its rack space is handed
  out and every task accepted, and a declined task keeps it active until that
  task is done. *(Covered:
  `finish::an_opening_rack_job_completes_once_its_racks_are_handed_out_and_all_accepted`,
  `finish::a_declined_opening_rack_task_keeps_its_job_active_until_it_is_done`.)*
- `I-STATS-9d` A finish check overtaken by a purge does not complete the job
  -- by either witness: the claim counter below what was observed, or a purge
  counted meanwhile however many claims followed it (nineteenth audit) -- and
  an SPRT job hands out nothing past its cap. *(Covered:
  `admin_api::a_finish_check_overtaken_by_a_purge_does_not_complete_the_job`,
  `worker_api::sprt_jobs_hand_out_nothing_past_their_cap`.)*
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
  the object store, because the server builds wordmaps and rack info tables
  from them; `winpct` rows carry neither, because nothing server-side builds
  anything from a win% model. The key is the digest, so re-importing a tarball
  whose lexica have not changed uploads nothing. *(Covered:
  `input_data::lexica_and_leaves_are_stored_once_by_digest`,
  `inputdata::tests::the_roles_a_derived_build_needs_go_to_the_object_store`.)*
- `I-INPUT-4` A second import of the same tarball is a no-op. *(Covered:
  `input_data::a_second_import_of_the_same_tarball_is_a_no_op`.)*
- `I-INPUT-5` `fail_orphaned_imports` fails a row left `running` by a restart
  and leaves `staged` and `confirmed` rows alone. *(Covered:
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

### `I-AUDIT-*` — audit log (`audit.rs`)

- `I-AUDIT-1` Each helper writes the actor, target type, target id and job id it
  was given. *(Covered:
  `audit::each_audit_helper_records_who_did_what_to_which_target`.)*
- `I-AUDIT-2` A log written inside a rolled-back transaction does not persist —
  the audit trail cannot claim something that did not happen. *(Covered:
  `audit::an_audit_row_in_a_rolled_back_transaction_does_not_persist`.)*
- `I-AUDIT-3` Every destructive admin action writes exactly its record: one
  row naming what it did, who did it and to what — except the three that
  destroy recorded work (purge, job delete, user delete), which write that row
  *and* their census (`I-JOB-10`), exactly that pair. The entry first said
  "exactly one row"; the census is the second on purpose. *(Covered:
  `audit::every_destructive_admin_action_writes_exactly_its_record`, across
  deactivate, complete, purge and delete of a job, user delete, ban, unban,
  input-file delete, player-config delete and pool-member removal. Deleting an
  input file or a player config wrote nothing.)*

### `I-ART-*` — object store (`artifacts.rs`)

Against a real MinIO (`TEST_S3_ENDPOINT`), one bucket per test.

- `I-ART-1` Put then get returns identical bytes, against MinIO. *(Covered:
  `artifacts::put_then_get_returns_identical_bytes`.)*
- `I-ART-2` Getting an absent key is a clean error, not a panic. *(Covered:
  `artifacts::getting_an_absent_key_is_a_clean_not_found`.)*
- `I-ART-3` A key is namespaced per job and generation, so two jobs cannot
  collide. *(Covered: `artifacts::two_jobs_and_two_generations_never_share_a_key`.)*

### `I-EXPORT-*` — exports (`exports.rs`)

A completed job's corpus, written once to the object store (PLAN.md,
"Exports"). The refusals were tested before; the success path — the objects,
their digests, the redirect, the purge — ran nowhere until `exports.rs`, which
runs against a real MinIO.

- `I-EXPORT-1` The export is the admin stream's corpus, as gzipped NDJSON
  behind a presigned URL with `row_count` and a digest recorded; a games job
  that captured positions gets a second object with its own count, size and
  URL, both under `exports/`. *(Covered:
  `exports::a_completed_jobs_export_is_its_stream_and_its_positions_behind_presigned_urls`.)*
- `I-EXPORT-2` A completed job's stream answers `303` to its newest ready
  export, and with `?positions=true` to the positions object — until the
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
- `I-EXPORT-6` Only a completed job can be exported, not until its last claims
  have landed, and a claim whose worker vanished does not block it for ever.
  *(Covered: `admin_api::only_a_completed_job_can_be_exported`,
  `admin_api::a_completed_job_is_not_exported_until_its_claims_have_landed`,
  `admin_api::an_export_is_not_blocked_by_a_claim_whose_worker_vanished`.)*
- `I-EXPORT-7` The uploaded parts are one gzip stream of exactly the lines
  pushed, and the recorded digest and size describe those bytes. *(Covered:
  `exports::tests::the_parts_are_one_gzip_stream_of_what_was_pushed`.)*
- `I-EXPORT-8` One export of a job runs at a time: a second request while one
  is `running` is a 409. *(Covered:
  `exports::a_job_has_one_export_running_at_a_time`.)* (Thirteenth audit.)

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

### `I-DERIVED-*` — wordmaps and rack info tables (`derived.rs`)

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
  here: without it, dispatching early sends a worker no `derived` entry, which
  it reads as a server that checks nothing. *(Covered:
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
- `I-DERIVED-7` A build whose `kwg` row predates `object_key` fails with a
  message naming the remedy (re-import the tarball), and is retried the bounded
  `MAX_ATTEMPTS` (3) times and then left `failed` — not retried forever, and
  not failed on the first attempt as this entry implied. *(Covered:
  `derived::a_build_from_a_lexicon_stored_before_object_keys_fails_naming_the_remedy`.)*
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
  *(Covered: `auth_routes::register_confirm_and_login_gives_a_working_session`.)*
- `A-AUTH-2` An unconfirmed login is 403 with a message naming the fix.
  *(Covered: `auth_routes::an_unconfirmed_login_is_refused_with_the_fix_named`.)*
- `A-AUTH-3` **A wrong password and an unknown username return an identical
  401** — body and status both, so the endpoint cannot enumerate accounts.
  *(Covered: `auth_routes::a_wrong_password_and_an_unknown_username_answer_identically`.)*
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
- `A-AUTH-9` A reset token is single-use, expires, and is invalidated by a
  successful reset. *(Covered:
  `auth_routes::a_reset_token_is_single_use_spent_by_any_reset_and_expires`.)*
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
  rejected** — a truncated list costs at most a wasted claim. *(Covered:
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
  a decline's size by `A-BOUND-6`.)*
- `A-WORKER-8` A decline with an unknown reason is rejected; the five known
  reasons (`missing_data`, `magpie_version`, `unknown_job_type`,
  `derived_mismatch`, `task_failed`) are accepted. *(Covered:
  `worker_routes::only_the_five_known_decline_reasons_are_accepted`.)*
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
  `worker_routes::every_implausible_games_result_is_a_400_that_says_why`, and
  the same for `game_pairs`, `opening_rack` and leave results; an oversized body
  by `A-BOUND-5`.)*
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
- `A-WORKER-15` `client-version` reports the configured floor and a download
  URL. *(Covered:
  `worker_routes::client_version_reports_the_configured_floor_and_download_url`.)*

### `A-ADMIN-*` — `routes/admin.rs`

- `A-ADMIN-1` Player config create/get/list/delete round-trips, and a config in
  use cannot be deleted. *(Covered:
  `admin_routes::a_player_config_round_trips_and_one_in_use_cannot_be_deleted`.)*
- `A-ADMIN-2` Creating a job of each type returns `{job}` and the
  job is inactive with no allocation. *(Covered:
  `admin_routes::creating_each_job_type_answers_it_inactive_and_unallocated`;
  a leave job, which runs MAGPIE at creation, by the opt-in
  `magpie_routes::a_leave_job_is_created_inactive_with_its_generation_zero_leaves_stored`.)*
- `A-ADMIN-3` Job creation rejects: mismatched win% models, incompatible
  lexicon/distribution, an unknown `input_data` id. The win%-model rules on a
  single player — a simming player with no model, a static player with one —
  are enforced where the player is made, at player-config creation, not at job
  creation as this entry first listed them, so no job can name such a config.
  A config's `kwg_id`, `klv_id` and `winpct_id` must each name a file of that
  role (`A-BOUND-3`). A letter distribution MAGPIE cannot hold (`U-RACK-9`) is
  refused at creation, not by every claim as a 500. *(Covered:
  `admin_routes::job_creation_refuses_each_impossible_combination_and_says_which`,
  which asserts the player-config half too.)*
- `A-ADMIN-4` Activate / deactivate / complete / purge / delete each return the
  documented shape and are reflected in a subsequent read. *(Covered:
  `admin_routes::each_lifecycle_action_answers_its_shape_and_a_read_agrees`; the
  allocation cap by `A-BOUND-7`.)*
- `A-ADMIN-5` Import start → poll → confirm over HTTP, including that polling
  reports progress while running. *(Covered:
  `input_data::an_import_is_started_polled_while_running_and_confirmed_over_http`.)*
- `A-ADMIN-6` Confirming an import that is not `staged` is rejected. *(Covered:
  `input_data::only_a_staged_import_can_be_confirmed`.)*
- `A-ADMIN-7` `input-data` list and delete, including the in-use refusal.
  *(Covered: `input_data::an_input_file_in_use_cannot_be_deleted`,
  `input_data::a_file_only_derived_data_refers_to_can_be_deleted_and_takes_that_data_with_it`.)*
- `A-ADMIN-8` `job/:id/data-gaps` reports what workers declined for. *(Covered:
  `admin_routes::data_gaps_report_what_workers_declined_for`.)*
- `A-ADMIN-9` `fleet` reports connected workers and their versions. *(Covered:
  `admin_routes::the_fleet_view_counts_workers_by_the_version_they_run`.)*
- `A-ADMIN-10` `backups` reports staleness from the `backups` table. *(Covered:
  `admin_routes::the_backups_view_reports_staleness_from_the_backups_table`.)*
- `A-ADMIN-11` `rebuild-artifacts` returns what it rebuilt and is idempotent.
  *(Covered: the refusal for a non-leave job by
  `admin_routes::only_a_leave_jobs_artifacts_can_be_rebuilt`; the rebuild,
  which runs MAGPIE, by the opt-in
  `magpie_routes::rebuilding_artifacts_restores_what_is_missing_and_is_idempotent`.)*
- `A-ADMIN-12` Ban and unban by user id and by anon UUID; a banned worker's next
  claim is refused; unban restores it. *(Covered:
  `admin_routes::a_ban_by_either_identity_refuses_the_next_claim_and_unban_restores_it`,
  `admin_api::an_identity_can_be_banned_once_and_unbanning_lifts_it`.)*
- `A-ADMIN-13` `audit-log` paginates and filters by job. *(Covered:
  `admin_routes::the_audit_log_pages_and_filters_by_job`.)*
- `A-ADMIN-14` `delete_user` over HTTP. *(Covered:
  `admin_api::a_user_with_history_can_be_deleted`; an admin cannot delete
  themself, `A-BOUND-8`.)*
- `A-ADMIN-15` Purging a completed job returns it to inactive, clears its
  stored verdict, and it can be activated again. *(Covered:
  `admin_api::purging_a_completed_job_returns_it_to_inactive`.)* (Eleventh
  audit's fix, twelfth audit's test.)
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
  are activating, deactivating and completing it; all are allowed once it has
  finished. *(Covered:
  `admin_api::a_second_purge_or_delete_is_refused_while_one_runs`.)* (Fourteenth
  audit.)

### `A-RATE-*` — `routes/ratings.rs`

- `A-RATE-1` Pool list and detail render the latest run, with residuals.
  *(Covered: `ratings::the_pool_pages_show_the_latest_run_with_its_residuals`,
  `worker_api::a_pools_residuals_are_the_ones_its_latest_fit_stored`.)*
- `A-RATE-2` A pool with no run renders empty rather than erroring. *(Covered:
  `ratings::a_pool_with_no_run_renders_empty`.)*
- `A-RATE-3` Creating a pool adds the anchor as a member automatically.
  *(Covered: `ratings::creating_a_pool_makes_its_anchor_a_member`.)*
- `A-RATE-3b` A pool is validated like a job: a variant no job has, a
  distribution id that is a layout row, and an anchor rating whose scale would
  overflow are each a 400, and nothing is created. *(Covered:
  `ratings::a_pool_that_could_rate_no_one_is_refused`.)* (Eleventh audit.)
- `A-RATE-4` Adding and removing a member each trigger a refit and return a new
  `run_id`. *(Covered: `ratings::adding_and_removing_a_member_each_refit_the_pool`.)*
- `A-RATE-4b` Adding a config that does not exist is a `400` on
  `player_config_id`, and adding to a pool that does not exist a `404`; an
  anchor that does not exist is a `400` too. None was the `409` "still
  referenced" a bare foreign-key failure maps to. *(Covered:
  `ratings::adding_an_unknown_member_or_to_an_unknown_pool_says_which`,
  `ratings::a_pool_that_could_rate_no_one_is_refused`.)*
- `A-RATE-5` Removing the anchor is refused with a message naming what to do —
  an anchor is fixed, so a pool anchored elsewhere (it named an operation that
  did not exist until the eleventh audit).
  *(Covered: `ratings::removing_the_anchor_is_refused_with_the_fix_named`.)*
- `A-RATE-6` History returns points in time order and excludes unrated configs.
  *(Covered: `ratings::history_is_in_time_order_and_leaves_out_unrated_configs`,
  `admin_api::a_long_rating_history_is_thinned_but_keeps_its_ends`.)*
- `A-RATE-7` Recompute is admin-only and returns a new run. *(Covered:
  `ratings::only_an_admin_can_recompute_and_it_stores_a_new_run`.)*

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
  claims (twenty-eighth audit). *(Covered:
  `routes::public::tests::a_contributors_page_is_read_through_their_own_index`.)*
- `A-PUBLIC-4` `rack_lookup` finds an analysed rack, however it is typed, with
  its whole ranked list. A rack with no analysis yet is a `200` with an empty
  list, not a 404 as this entry first said — the rack is a valid question
  about a job that exists, and "nothing yet" is its answer; only an unknown job
  is a 404. *(Covered: `public_api::rack_lookup_finds_an_analysed_rack`.)*
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
- `A-PUBLIC-6a` The SSE stream ends when the process is told to stop: it has no
  end of its own, and graceful shutdown waits for every open response, so an
  open dashboard used to hold every deployment until the runtime's `SIGKILL`
  (`worker_api::a_live_stats_stream_ends_when_the_server_is_told_to_stop`).
- `A-PUBLIC-7` User and worker lists paginate and do not leak email addresses or
  key hashes — nor an anonymous worker's UUID, its only credential, which
  public endpoints replace with a derived pseudonym; contributors that tie are
  each listed exactly once. *(Covered:
  `public_api::contributor_lists_paginate_and_leak_no_credentials`,
  `public_api::tied_contributors_are_each_listed_exactly_once`,
  `public_api::tied_jobs_and_users_are_each_listed_exactly_once`,
  `worker_api::public_endpoints_name_anonymous_workers_by_pseudonym_only`; tied
  contributors were duplicated and skipped across pages.)*

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
- `A-ACCOUNT-5` One user cannot see or modify another's keys. *(Covered:
  `account::one_user_cannot_see_or_change_anothers_keys`.)*
- `A-ACCOUNT-6` An account's key creation is a burst of a hundred (the cap), then
  ten an hour: a hundred keys at once are allowed, a key made after revoking one
  is `429`, and another account is unaffected. *(Covered:
  `account::key_churn_is_rate_limited_per_account`.)* (Fourteenth audit; the
  burst, fifteenth.)
- `A-ACCOUNT-7` A key's label is at most 100 characters. *(Covered:
  `account::a_key_label_is_bounded`.)* (Seventeenth audit.)

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
  refused, naming the headroom, and two concurrent activations cannot together
  exceed it. *(Covered:
  `boundaries::an_activation_past_100_percent_is_refused_naming_the_headroom`,
  `boundaries::two_concurrent_activations_cannot_exceed_100_percent`.)*
- `A-BOUND-8` An admin cannot delete their own account: a 400 saying so, the
  account and its session untouched. *(Covered:
  `boundaries::an_admin_cannot_delete_their_own_account`.)*
- `A-BOUND-9` A redundancy-2 task with one slot accepted and one lapsed reopens
  for a third worker with the same seed, and is never offered back to the
  worker whose result was accepted. *(Covered:
  `boundaries::a_task_with_one_accepted_and_one_lapsed_slot_reopens_for_someone_else`.)*
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

Sixteen fixtures exist, and the set is complete: every message has one. The
first eight — an assignment of each original request shape (games, opening
racks, leave generation), a claim, a decline, and each of the three shutdown
reasons — were written by hand; `C-2`..`C-9` were **captured from a real
exchange**. MAGPIE's `test/birdtest_contract/` carries a byte-identical copy of
all sixteen, and CI's `magpie-contract` job runs MAGPIE's tests against this
branch's copy.

- `C-1` `assignment-opening-rack.json` — exists, and MAGPIE's
  `test/birdtest_contract/` carries a byte-identical copy. *(Covered:
  `contract_fixtures::assignments_carry_a_task_request_this_build_understands`,
  `contract_fixtures::the_assignment_envelope_matches_what_the_server_sends`;
  MAGPIE's `test_contract_fixtures_carry_every_key_contribute_reads`.)*
- `C-2` `assignment-game-pairs.json`, carrying `game_pairs: true`. *(Covered:
  `contract_fixtures::the_game_pairs_assignment_is_a_pairs_request`, and
  `U-WIRE-2` reads it.)*
- `C-3` `result-games.json`, with `capture_positions` on. *(Covered:
  `contract_fixtures::the_games_result_is_accepted_as_a_submission`.)*
- `C-4` `result-game-pairs.json`, carrying the pentanomial — the most important
  one: it is the newest message and the one MAGPIE and birdtest most recently
  disagreed about. *(Covered:
  `contract_fixtures::the_game_pairs_result_is_accepted_as_a_submission`.)*
- `C-5` `result-opening-rack.json`, a simulating player's analyses. *(Covered:
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

Client→server results are not only parsed: each runs through its job type's
validation, as a submission would, so a captured result the server would
refuse fails here.
MAGPIE's half, in `test/contribute_test.c`, is four tests:
`test_contract_fixtures_carry_every_key_contribute_reads` (every key
`contribute` reads off a server message is in its fixture),
`test_results_carry_every_key_the_server_reads` (the serializers a task's
result is built with still produce every key the result fixtures carry, so a
key renamed in MAGPIE fails there rather than every submission),
`test_only_a_data_shutdown_is_waited_out_for_a_set_aside_job` (the wait-or-exit
decision on each of the three shutdown fixtures) and
`test_the_claim_body_matches_the_claim_fixture` (the claim body, built from the
fixture's version and ids, has its keys and values). MAGPIE's sanitizer CI
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
  page and the contributor leaderboard. *(Covered:
  `e1-anonymous-browsing.spec.ts`.)*
- `E-2` Register → confirm the email → log in → generate an API key → see it
  exactly once → deactivate it. *(Covered: `e2-register-and-api-key.spec.ts`,
  reading its code from the outbox.)*
- `E-3` An admin imports input data, reviews the staged diff, and confirms it.
  *(Covered: `e3-input-data-import.spec.ts`, against the fixture tarballs.)*
- `E-4` An admin creates two player configs and a game-pairs job, activates it
  with an allocation, and watches the dashboard update live over SSE as fake
  workers contribute. **The journey that justifies the tier**: the only place
  SSE, the built Svelte app, the scheduler and a worker are exercised together.
  *(Covered: `e4-live-dashboard.spec.ts`.)*
- `E-5` An admin bans a worker and that worker can no longer claim. *(Covered:
  `e5-ban-worker.spec.ts`.)*
- `E-6` A non-admin is redirected away from `/admin`, and an anonymous visitor
  from `/account`. *(Covered: `e6-redirects.spec.ts`.)*
- `E-7` The ratings page: an admin creates a pool, adds a config, sees the fit
  appear, removes it, and sees the ratings change. Covers the one flow where a
  write is expected to move numbers elsewhere on the page. There is no form for
  creating a pool, so the journey creates it over the API and starts at the
  ratings list; membership, the fit and the moved ratings go through the page.
  *(Covered: `e7-ratings.spec.ts`.)*
- `E-8` A job detail page renders the pentanomial table with the five buckets
  labelled, and the SPRT status text. *(Covered: `e8-pentanomial.spec.ts`.)*
- `E-9` The password reset flow end to end. *(Covered:
  `e9-password-reset.spec.ts`, reading its link from the outbox.)*
- `E-10` A page renders correctly at phone width — one journey, not all of them.
  *(Covered: `e10-phone-width.spec.ts`: a Pixel 5 viewport, the job list and a
  job page, nothing wider than the screen.)*

### Reading confirmation codes

Two journeys need to read an emailed code, and they do it through
`MAIL_BACKEND=file`.

**First, shrink the problem.** Only `E-2` (register → confirm → log in) and
`E-9` (password reset) need a code at all. The other eight need a *confirmed
admin*, which `e2e/run.sh` seeds through `scripts/seed.py` before Playwright
starts — so those journeys start at login. That turns "how does the browser
read mail" into a question about two tests rather than ten.

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
  live counters, and a merge folds it into `leave_rack_progress`. *(Covered:
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

`M-10` and the `capture` case run on MAGPIE's small data, which the script
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
  with a row in every table a result touches. *(Covered:
  `scripts/restore-roundtrip.sh`, which had fallen three `NOT NULL` columns
  behind the schema and could not run.)*

Both run nightly (`restore-roundtrip` and `backup-drill` in
`.github/workflows/nightly.yml`), each against the schema applied to an empty
database — which is also the migration replay the nightly list asks for. The
monthly production drill (`restore-drill.sh` on the newest real dump) remains
the real check of the backups themselves.

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

**The SES mail backend.** Every tier runs `console` or `file`; `ses` is
exercised by production and by the first confirmation email after a deploy.
Testing it would mean a mocked SDK, which tests the mock. What is shared —
composing the message, sending off the request path (`A-AUTH-8`) — runs under
the other two.

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
ask for a wordmap or a rack info table, which in tier 5 none do. The `winpct`
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
asks GitHub for — `api/repos/birdtest/fixtures/commits/main` resolving to a
fixed sha, and `raw/birdtest/fixtures/<sha>/versioned-tarballs/` holding the
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

**Per pull request** (`.github/workflows/ci.yml`), six jobs in parallel:

1. **backend** — a Postgres 16 service and a MinIO container, with
   `TEST_DATABASE_URL` and `TEST_S3_ENDPOINT` pointing at them; `cargo clippy
   --locked --all-targets -- -D warnings`, then `cargo nextest run --locked
   --no-fail-fast` (tiers 1, 2, 3 and 4; `backend/.config/nextest.toml` kills a
   test at ten minutes) and `cargo test --locked --doc`. The `#[ignore]`d
   tier-6 tests are not run here; the nightly runs them.
2. **frontend** — `npm ci`, `npm run check`, `npm test` (tier 1F), `npm run
   build`.
3. **images** — the backend image (which builds the pinned MAGPIE too), a probe
   that the image's MAGPIE runs and reports its builders, the derived-file
   builder image, and the frontend image.
4. **e2e** — tier 5: MAGPIE at the commit `docker/Dockerfile` pins, built `portable_release`
   inside `debian:bookworm-slim` (the backend image's glibc), Playwright's
   Chromium, then `e2e/run.sh`; the Playwright report and traces are uploaded
   on failure.
5. **terraform** — `terraform fmt -check -recursive` and `terraform validate`
   (no AWS credentials).
6. **magpie-contract** — MAGPIE's half of the contract: check out MAGPIE at the
   commit `docker/Dockerfile` pins, copy this branch's `contract-fixtures/` over its
   `test/birdtest_contract/`, and run `magpie_test contribute`; then
   `magpie_test builderhash` (the wordmap, rack info table and both KLV
   builders -- `createdata klv` and `rackequity2klv` -- against their pinned
   hashes), and then every command the server invokes (`convert dawg2wordmap`,
   `convert klvwmp2rit`, `createdata klv`, `convert rackequity2klv`), run as
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
  migration replay), then `scripts/restore-roundtrip.sh` (`S-BACKUP-4`).
- **backup-drill** — Postgres and MinIO up, the schema applied, then
  `scripts/backup-drill-check.sh` (`S-BACKUP-1`..`3`, and `S-BACKUP-2b`).

Nightly failures are an alert rather than a blocked merge, because they are
slower and more environment-sensitive than a pull request should wait on.

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
