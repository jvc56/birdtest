# birdtest — Testing

[PLAN.md](PLAN.md) says what birdtest is and why it is built that way.
[README.md](README.md) says how to run it. This document says **what we
guarantee and how we check it**: six tiers, what each one is allowed to touch,
and the shared machinery underneath them.

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
| 2 | **Integration** | Real Postgres. No HTTP. | — | Every run |
| 3 | **API** | Real router in-process + Postgres. No browser. | — | Every run |
| 4 | **Contract** | Committed fixtures. Nothing live. | — | Every run |
| 5 | **End-to-end** | Full stack in Docker + a real browser. | `fake_worker.py` | Every run |
| 6 | **MAGPIE smoke** | Full stack + a real MAGPIE build. | `magpie contribute` | Nightly, opt-in |

Tiers 1–4 need nothing but a Postgres. Tier 5 needs Docker and Playwright. Tier
6 additionally needs a MAGPIE checkout with real MAGPIE-DATA, and is the only
tier that is not run by default.

### Status today

| Tier | Tests | Where |
|---|---|---|
| 1 Unit | 46 | `#[cfg(test)]` in `inputdata`, `backups`, `jobs::klv`, `jobs::racks`, `version`, `compat`, `stats::sprt`, `stats::bradley_terry` |
| 2 Integration | **0** | — |
| 3 API | **0** | — |
| 4 Contract | 5 | `routes::worker::contract_fixtures` |
| 5 End-to-end | **0** | — |
| 6 MAGPIE smoke | 2 (`#[ignore]`) | `jobs::klv` round-trips |

Tier 2 is the largest gap and the highest value. Every SQL string in the
codebase is currently unverified: `sqlx::query` is checked at runtime, so the
compiler sees opaque text. A three-column `INSERT` into a table with a fourth
`NOT NULL` column passed `cargo check` and all 33 tests, and would have failed
on every leave-generation job in production. One tier-2 test that creates a
leave-generation job catches that class outright.

---

## 1. Unit

Pure logic, no I/O. Rust idiom: `#[cfg(test)] mod tests` in the same file as the
code.

**May not**: open a socket, connect to a database, or read a file. Fixture bytes
come from `include_bytes!`; anything that needs a path uses a `tempdir`.

What belongs here:

- SPRT log-likelihood ratio and boundary computation, against hand-computed
  values rather than "it runs". (`testdist.csv` in
  `backend/src/jobs/testdata/` is the existing precedent for compiled-in
  fixture bytes.) Including the property that makes the pentanomial the right
  sample: the pair view and the per-game view must agree on the mean, and
  filtering to divergent pairs must be shown to break that.
- The Bradley-Terry rating fit: that the anchor holds, that the scale matches
  the Elo formula, that the result is order-independent, that an undefeated or
  unplayed config stays finite and is flagged rather than guessed, and that a
  non-transitive cycle flattens the ratings while showing up in the residuals.
- `version::Version` ordering — specifically that `1.9.0 < 1.10.0`, the trap the
  three integer columns exist to avoid.
- Rack unranking and the `total_racks` dynamic-programming count, including that
  unranking order is stable (results are recorded against an index, so a change
  silently re-points existing rows).
- `klv::build` structure: node layout, the word-index algorithm, a zeroed KLV
  summing to exactly zero.
- `compat.rs`'s ported lexicon/leaves/distribution rules against its
  known-good/known-bad table. **An uncovered combination must be rejected, not
  guessed** — the table is the artifact worth maintaining.
- Tarball entry validation: `..` segments, symlinks, non-regular entries, the
  20× compression-ratio cap checked continuously rather than at the end, and
  each size and count cap aborting the whole import rather than skipping an
  entry.
- Task state derivation from `(accepted_count, active_claim_count, redundancy)`.

## 2. Integration

Real Postgres, migrations applied, **no HTTP layer**. This is where birdtest's
genuinely hard logic lives, because most of it is SQL and concurrency rather
than Rust.

**Harness**: each test creates a uniquely named throwaway database on the server
in `TEST_DATABASE_URL` (defaulting to the compose Postgres), runs
`sqlx::migrate!`, and drops it at the end. Per-test databases rather than a
shared one because `cargo test` runs in parallel threads and these tests are
about contention — a shared database would make them interfere in ways that look
like the bugs they exist to find. Not `testcontainers` initially: the compose
Postgres is already running for development, and a container per test costs
seconds where a `CREATE DATABASE` costs milliseconds. Revisit if isolation
starts to bite.

What belongs here:

- **The claim lifecycle**: claim → submit → counters land → task completes at
  `accepted_count = redundancy`.
- **Reclamation**: a claim past the heartbeat timeout flips to `abandoned`,
  decrements `active_claim_count`, and returns an at-capacity task to
  `available`.
- **The `declined` partial-index trap.** `task_claims_user_unique_idx` and
  `task_claims_anon_unique_idx` are partial on
  `WHERE state NOT IN ('abandoned','declined')`. Drop `'declined'` from either
  and a worker that declines a task is permanently barred from claiming it again
  after fixing its data. Nothing else catches this.
- **Concurrent claimers**: N tasks claimed simultaneously against one job
  produce no duplicate seeds (the `(job_id, seed)` unique index resolves the
  race, and the loser retries) and no lost counter updates.
- **Deficit job selection**, including that both capability filters run *before*
  `MIN(priority)`. A worker whose `unsupported_jobs` covers the entire top tier
  must be offered work from the next tier, not shut down. Filtering after the
  priority computation produces exactly that bug and looks correct in isolation.
- **Version filtering** against a real set of jobs: a worker on `1.9.0` is
  offered a job requiring `1.9.0` and not one requiring `1.10.0`.
- **`expected_data` composition**: two players on different lexicons yield two
  `kwg` and two `klv` entries; the same config on both sides yields one of each;
  a static player contributes no `winpct`; a `leave_generation` job yields
  exactly `kwg`, `letterdist`, `layout` and never a `klv`.
- **Leave generation**: the bulk `leave_rack_progress` upsert under concurrent
  submissions; generation transition folding progress into a KLV, writing the
  artifact, recording its digest; `ON CONFLICT DO NOTHING` keeping the *first*
  digest; generation 0's zeroed KLV existing and every generation carrying a
  non-null `previous_artifact_key`.
- **`rebuild_artifacts` reproduces bytes.** `run_transition` and
  `rebuild_artifacts` share `generation_means` precisely so a rebuild cannot
  drift; a test that folds, rebuilds, and compares digests is what keeps that
  true.
- **Position capture deduplication**: under `redundancy = 2`, identical replayed
  games produce one set of `position_analysis_records` rows, not two.
- **Destructive endpoints**: `purge_job` / `delete_job` / `delete_user` leave
  counters consistent and write their census to `audit_log` *before* destroying.
- **Server-side reads use the pinned row.** Two `letterdist` rows with the same
  name and different bytes must produce different `total_racks` for otherwise
  identical jobs. This is the one test that would catch the server and the
  worker disagreeing about the alphabet.

## 3. API

The real Axum router in-process via `tower::ServiceExt::oneshot`, against a real
Postgres. No browser, no network, no server process. `tower` is already a direct
dependency, so this tier costs no new ones.

Shares tier 2's database harness; the only addition is building an `AppState`
pointed at the test database.

What belongs here:

- Registration → confirmation → login, including that an unconfirmed login is
  `403` and a wrong password and an unknown username return the same `401`.
- CSRF enforced on cookie-backed endpoints, and deliberately exempt on
  `/api/worker/*`.
- Rate limiting: `429` with `Retry-After`, per worker identity.
- Admin authorization: `403` for authenticated non-admins and for anonymous
  workers, on every `/api/admin/*` route.
- The worker protocol end to end at the HTTP level: a bodyless claim rejected
  with an error naming the fix rather than a bare `422`; an `unsupported_jobs`
  list over 200 truncated rather than rejected; `204` versus a `shutdown`
  directive and the difference between `Idle` and `NoWorkExists`; each shutdown
  reason, with `both` leading on the MAGPIE version; decline releasing the claim
  immediately.
- Artifact fetch: only keys the server minted resolve; anything else is `404`.
- Pagination and filtering on the public API.

## 4. Contract

[`contract-fixtures/`](contract-fixtures/) holds one committed example of every
message crossing the birdtest↔MAGPIE boundary. `routes::worker::contract_fixtures`
parses each against the real wire types.

Client→server fixtures are deserialized into the types that actually handle the
request. Server→client fixtures are compared by **field structure, not bytes**,
so fields stay free to move before the first release while a renamed or dropped
field still fails.

**The rule**: any change to a worker-API wire type updates the fixture in the
same commit. A fixture that no longer matches is not a stale file, it is a
statement that has become false.

MAGPIE's half is not done. Until the fixtures are copied into that repository
and read by its tests, this tier pins one side of a two-sided contract.

## 5. End-to-end

Playwright against the full stack in Docker, with `fake_worker.py` supplying
contributions — **the only tier that uses it**, and the reason it exists.
A browser journey needs contributions to arrive on cue and land at predictable
values; a real MAGPIE would supply neither, and would put a C build in the way
of a suite that runs on every pull request. Journeys, not assertions per field — anything that can be checked
at tier 3 belongs at tier 3, because a failure there names the cause and a
failure here names a symptom.

Runs against the **built** frontend served by Nginx, not the Vite dev server,
because the built artifact is what ships.

The journeys worth having:

1. An anonymous visitor browses the landing page, job list, a job detail page
   and the contributor leaderboard.
2. Register → confirm the email → log in → generate an API key → see it exactly
   once → deactivate it.
3. An admin imports input data, reviews the staged diff, and confirms it.
4. An admin creates two player configs and a game-pairs job, activates it with
   an allocation, and watches the dashboard update live over SSE as fake workers
   contribute.
5. An admin bans a worker and that worker can no longer claim.
6. A non-admin is redirected away from `/admin`, and an anonymous visitor from
   `/account`.

Journey 4 is the one that justifies the tier: it is the only place SSE, the
built Svelte app, the scheduler and a worker are all exercised together.

**Open question — reading confirmation codes.** `MAIL_BACKEND=console` writes
codes and reset links to the backend's stdout, which a browser test cannot read
without scraping container logs. A `MAIL_BACKEND=file` that writes one message
per file into a shared volume would make journey 2 deterministic and is probably
worth adding before this tier is written.

## 6. MAGPIE smoke

The only tier that runs a real MAGPIE. It catches what fixtures structurally
cannot: not whether the *shape* of a message is agreed, but whether MAGPIE's
actual behaviour matches the contract.

**Opt-in, then fail loudly.** Excluded from a default run. When you ask for it
and MAGPIE is missing, that is a hard error, not a skip — a green run must never
silently mean nothing was exercised. This follows the precedent already set by
`jobs::klv`'s round-trip tests, which are the first two members of this tier:
`MAGPIE_BIN` (default `../../MAGPIE/bin/magpie`) and `MAGPIE_DATA_PATH` (default
`../../MAGPIE/data`), `#[ignore]` by default, and an `assert!` naming the remedy
when the binary is absent.

**Correctness is established by capability probe, not by version.** `contribute`
lives on the unreleased `birdtest-contribute` branch, so there is no version
string that discriminates — `MIN_MAGPIE_VERSION` is `0.0.1`, a placeholder for
the release that implements the `expected_data` check. The probe asks the binary
what it can do: that `contribute` is a registered command, and that it accepts
the current required claim body. When production versions become real, this
becomes a version check and the probe retires.

**This tier cannot use the synthetic fixture lexica** — nor can the dev
environment, for the same reason. The fixture's `NWL23.kwg` is a stub; a real
MAGPIE would load it and fail, or worse, not fail.
Tier 6 seeds from a real MAGPIE-DATA install (`scripts/seed.py --real-data`,
pointing at `MAGPIE_DATA_PATH`), which is also what makes it a genuine check that
birdtest's pinned digests match what `download_data.sh` actually installs. If
they diverge the client declines every task and the tier fails — surfacing the
mismatch as a red build rather than as a dead job in production.

That makes a leave-generation smoke expensive here: real English means the
914,624-leave universe above at job creation. Two ways out, in preference order:
keep tier 6's leave-generation case to a single generation and accept a slow
nightly job, or place the tiny fixture distribution on MAGPIE's own `-path`
search list so both sides load the same small bag — which is exactly the trick
`jobs::klv`'s round-trip tests already use to guarantee birdtest and MAGPIE are
reading the same alphabet. The second is better if it works; **verify that a
real MAGPIE actually plays with a real `NWL23.kwg` against a five-letter bag
before relying on it**, because that combination has never been run.

What it runs: one task of each job type through `magpie contribute` against a
seeded stack, asserting the results land and are credited.

---

## The shared substrate

Tiers 5 and 6 and the dev environment all need the same thing: an empty database
turned into a state where work can flow. They diverge only on which data seeds
it — the fixture tarball for tier 5, a real MAGPIE-DATA install for tier 6 and
`dev.py`, because a real worker cannot be fed stubs. That chain is six steps and three of
them did not exist a month ago, which is why it belongs in code rather than in
prose.

### The fixture tarball

`fixtures/data-<date>.tgz`, built by `fixtures/build.sh` exactly the way
MAGPIE-DATA builds the real ones (`cp -RL`, `tar -czf`, `split`), so import's
chunk-walking and extraction are exercised for real rather than bypassed.

Under 2 KB, because of a useful asymmetry: the server only ever *reads*
`letterdist` and `layout` bytes — that is what `input_data.content` is for. The
`kwg`, `klv` and `winpct` rows are digest-only; nothing server-side opens them.
So the fixture carries:

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

`seed_generation` inserts one `leave_rack_progress` row per leave and
`klv::build` constructs a trie over all of them, before the job is usable. On
real English that is nearly a million rows per leave-generation job created —
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
and job creation. The single exception is promoting a user to admin, which has
no endpoint by design and is a direct `UPDATE`.

```
scripts/seed.py [--api URL] [--real-data] [--job-type TYPE]
```

1. Register a user, read the confirmation code, confirm it.
2. Promote to admin (SQL — `is_admin` is settable through no endpoint).
3. Import input data and confirm the staged diff. The fixture tarball by
   default; a real MAGPIE-DATA install under `--real-data`.
4. Create player configs pinning those rows.
5. Create a job of the requested type.
6. Activate it with an allocation.

### What tiers 2 and 3 must *not* share

They do not call the seed. They construct exactly the state each test needs.

The moment integration tests depend on a realistic fixture, every test is
coupled to its contents and the cases that matter become unreachable: zero
active jobs, an empty top priority tier, a worker locked out of every job, a job
at capacity, a claim one second past its timeout. Those need precise state, not
plausible state. Tiers 2 and 3 share the migration and nothing above it.

This is the boundary that erodes first, because reusing the seed is always
easier in the moment.

---

## The development environment

```
scripts/dev.py [--workers N] [--no-browser]
```

Brings up the stack, waits for health, seeds it, starts `N` workers, and opens a
browser. It is tier 6's setup with the assertions and the teardown removed, and
it calls the same `seed.py --real-data`.

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

1. `cargo clippy --all-targets -D warnings`, `cargo test` (tiers 1 and 4), and
   `npm run check`. No services needed.
2. Tiers 2 and 3 against a Postgres service container.
3. Tier 5: compose up, seed, Playwright.

**Nightly**:

- Tier 6, with a built MAGPIE and a real `download_data.sh` install.
- A migration replay from an empty database.
- `scripts/restore-roundtrip.sh` — dump, drop, restore, verify.

Nightly failures are an alert rather than a blocked merge, because they are
slower and more environment-sensitive than a pull request should wait on.

---

## Conventions

| Tier | Lives in | Run with |
|---|---|---|
| 1 | `#[cfg(test)] mod tests`, in-file | `cargo test` |
| 2, 3 | `backend/tests/` | `cargo test --test '*'` |
| 4 | `routes::worker::contract_fixtures` | `cargo test` |
| 5 | `e2e/` | `npx playwright test` |
| 6 | `#[ignore]`, marked with a reason | `cargo test -- --ignored` |

Environment variables: `TEST_DATABASE_URL` (tiers 2–3), `MAGPIE_BIN` and
`MAGPIE_DATA_PATH` (tier 6 and `scripts/dev.py`, required by both).

**A test that needs a service it cannot find fails; it does not skip.** The one
exception is tier 6, which is excluded from the default run by `#[ignore]` — but
once selected, it fails loudly like everything else.
