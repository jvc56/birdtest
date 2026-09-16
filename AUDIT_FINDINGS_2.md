# birdtest audit — findings, version 2

Branch: `audit/birdtest-2026-09-15`, off `main` at `6a72333`.
MAGPIE changes: `birdtest-contribute` only, commit `65a246d5` on top of
`dc007fc5`, plus an uncommitted fixture bump in the checkout (section 10,
U1). **Neither is pushed** (see U1).
Date: 2026-09-15 / 2026-09-16.

**This is the sixth audit.** It builds on [AUDIT_FINDINGS_1.md](AUDIT_FINDINGS_1.md)
(the fifth audit, branch `audit/birdtest-2026-09-14-pass2`, merged as PR #6)
and, through it, on the four unnumbered records before it (branches
`audit/birdtest-2026-09-11`, `-09-13`, `-09-13-pass2` and `-09-14`, whose
shared `AUDIT_FINDINGS.md` on `main` is the fourth audit's record). The
highest numbered file anywhere in the history was `AUDIT_FINDINGS_1.md`, so
this is `AUDIT_FINDINGS_2.md`; both earlier files are untouched. Where an
entry revisits a prior item it names it ("prior U2", "prior M2").

Two things happened on `main` after the fifth audit and before this one, and
they are where most of this audit's findings come from: `dbd285d` implemented
[MAGPIE_DEPENDENCY.md](MAGPIE_DEPENDENCY.md) (a pinned MAGPIE in the backend,
server-built wordmap and rack-info-table hashes, the `derived_data` queue and
builder task, MAGPIE 0.5.0 and a floor of 0.5.0 in the code), and `0436ddb`
folded that schema into `0001_initial.sql`. Neither had been audited.

This file is the authoritative record of every code-versus-`PLAN.md` decision
made in this audit, and of the bugs, MAGPIE argument gaps, critical-path
analysis, performance and storage findings behind them.

**Counts: 9 code-wins (PLAN.md updated to match the code), 3 plan-wins (code
changed), and 4 items left for human input, since decided and implemented
(section 10).**

The default bias is that the code wins and `PLAN.md` is brought level with it.
The code was changed only where it was wrong, or where the plan described the
behaviour the rest of the system needs.

---

## 0. What the prior audit left open

The prior record's section 9a lists six decisions (U1–U6), all marked
implemented. Each was checked against the code rather than taken on trust:

| Prior item | Still true? | How it was checked |
|---|---|---|
| U1 residuals stored with each fit | Yes | `ratings::fit_and_store` writes `rating_run_residuals`; `routes::ratings::pool_detail` reads them and builds no matrix |
| U2 push `birdtest-contribute`, floor `0.4.0` | Pushed; the floor has since moved to `0.5.0` **in the code only** | `origin/birdtest-contribute` was `dc007fc5` when this audit started, equal to the local branch. `dbd285d` raised `config.rs`, compose, README, TESTING.md and the fixtures to `0.5.0` but left `infra/variables.tf`, `backend/.env.example` and the `jobs` column default at `0.4.0` — see B2 |
| U3 defaults materialized | Yes | `magpie_defaults.rs`; `create_player_config` and `create_job` write them; MAGPIE `contribute_require_keys` refuses a request without them |
| U4 leave throughput: tune `num_iterations` | Nothing to build | — |
| U5 `rebuild-artifacts` inline | Nothing to build | — |
| U6 a poison task blocks an SPRT job at its cap | Nothing to build | — |
| Prior M2, reversed 2026-09-15 | The reversal holds | `validate_player_config_body` accepts `use_rit`; the table is named `<lexicon>.<leaves>`; a job waits for its `derived_data` rows; MAGPIE loads a table only against a pinned hash (`config_contribute_ensure_rack_info_table`) |

Nothing was left pending. The prior record's section 10 (the Python worker is
never described as a production client) was re-checked and still holds
(section 9 below).

---

## 1. How this audit was run

- Read `PLAN.md` in full; diffed its schema block mechanically against
  `backend/migrations/0001_initial.sql` (**they differed** — K1); read the prior
  record in full; then every backend module — scheduler, worker routes,
  registry, all four job types, `derived.rs`, `magpie.rs`, `inputdata.rs`,
  plausibility, jobstats, SSE, admin, public and rating routes, ratings,
  exports, auth, config, `main.rs`, `build-derived`, the test harness — plus CI,
  Docker, compose, the Terraform, the scripts, the frontend's API types and
  pages that touch the new features, and the four operational documents.
- On MAGPIE's `birdtest-contribute` at `dc007fc5`: `src/impl/contribute.c` in
  full; in `src/impl/config.c` every contribute executor, both resets,
  `config_contribute_load_lexicon_and_variant`, the wordmap and rack-info-table
  provisioning, `config_load_lexicon_dependent_data`'s flag handling; in
  `autoplay.c` how a game's seed is chosen under threading; the contract
  fixtures against birdtest's; `players_data.c` for the flag defaults;
  `word_info_table.h` for what a WIT does.
- Backend: `cargo clippy --locked --all-targets -- -D warnings` and
  `cargo test --locked` against Postgres 16 (the compose `postgres` service).
  **154 tests before, 158 after** (89 unit and contract, 69 integration), all
  passing, clippy clean. `svelte-check`: 0 errors, 0 warnings.
- A synthetic-history `EXPLAIN` (200,000 completed claims on one job) for the
  index added in P2, with and without the index.
- MAGPIE: `make magpie_test` (`-Werror`, address/undefined/leak sanitizers),
  then `./bin/magpie_test contribute` and `./bin/magpie_test config`, both
  passing; `git clang-format --diff` reports nothing on the changed lines.
- End to end: `scripts/e2e_magpie.py` against an isolated compose project
  (`birdtest-e2e`, its own volumes and ports, backend image built from this
  branch) with a `portable_release` MAGPIE built from `65a246d5` reporting
  `0.5.1`, and a MAGPIE-DATA `data-20251004` import through the real API.
  **This is the first end-to-end run to exercise the derived-file path** (B5):
  the static opening-rack player and the leave job now ask for a wordmap, the
  script runs the builder for them, and MAGPIE builds its own copy and compares
  it against the server's hash. See section 11 for the result.

---

## 2. MAGPIE arguments that change a task's outcome

The prior record's table (its section 2) was re-derived against `dc007fc5`
rather than reused, by tracing every field the three arg builders read to
where the contribute path sets, resets or verifies it. Everything in it still
holds. What is new since it was written:

| Setting | Changes results? | How it is set now | Change |
|---|---|---|---|
| Wordmap bytes (`.wmp`) | Yes, if built from a different `.kwg` or by a different builder | Server builds a reference copy, pins its SHA-256 on the claim (`expected_data.derived`); the worker builds and compares, declines `derived_mismatch` | Checked (`dbd285d`), no change |
| Rack info table bytes (`.rit`) | Yes: precomputed leave values | Same, named `<lexicon>.<leaves>`; never loaded without a pin | Checked, no change |
| **Word info table use (`-wit`, `-wit1`, `-wit2`)** | **Yes, if the table is stale**: a `.wit` is a per-substring letter mask move generation prunes with — built from the lexicon on disk it prunes nothing legal, built from an older one it prunes plays that exist | **Was: whatever the contributor's `settings.txt` or an earlier command left.** Now: off for both players before every load | **M1** |
| Per-game seeds under threading | No: `autoplay_get_next_iter_output` draws every game's seed from one PRNG under a mutex in iteration order, so game *i* gets the same seed on 1 thread or 16 | — | Checked, no change |
| `-threads` for a simulation | Yes, and inherently: a sampled simulation is thread-count dependent by construction | Excluded by design (prior audits; PLAN.md, simming jobs are excluded from equality cross-checks) | No change |

### M1 — a contributor's word info table applied to every task

*Plan wins on intent: **code changed** (MAGPIE).*

- **What a WIT is.** `src/ent/word_info_table.h`: one trie per word length
  whose terminal nodes hold, per position, the union of letters that can sit
  outside an occurrence of that substring in any longer word. Move generation
  uses it to skip anchors whose forbidden set is nonzero. A phony or a
  non-word path permits all letters, so a correct table prunes nothing legal;
  a table built from an older lexicon prunes plays the current lexicon has.
- **How MAGPIE decides.** `config_load_lexicon_dependent_data` reads the two
  players' `use_when_available` flags for `PLAYERS_DATA_TYPE_WIT` and, when no
  `-wit*` argument is being applied (contribute passes `use_wit_has_value =
  false` for all three), keeps whatever they were and loads `<lexicon>.wit`
  if the flag is on and the file exists. The flag defaults to off
  (`players_data_create`), and is switched on by `-wit`, `-wit1` or `-wit2`
  in `settings.txt` or by any earlier command in the same process.
- **What the code did.** `config_contribute_load_lexicon_and_variant` set the
  wordmap and rack-info-table flags before the load (prior M1) and left the
  WIT flag alone. This checkout's own `data/lexica` holds a `CSW24.wit`, so
  the case is not hypothetical: a contributor who had run `-wit true` for
  their own analysis played every contributed task through that table, and
  nothing in a task — no `expected_data` entry, no pinned hash — checked which
  lexicon it was built from.
- **What PLAN.md said.** For wordmaps: "a wordmap already sitting in `./data`
  from an earlier job is not switched on by its mere presence". The same
  principle, unstated for the third file.
- **Fix.** The flag is set to `false` for both players in the same loop that
  sets the other two, before the load. birdtest offers no WIT setting and
  pins no hash, so there is no "on" case.
- **Version.** `MAGPIE_VERSION` is `0.5.1`, so a build with this can be told
  from one without. **birdtest's floor stays `0.5.0`**: the backend refuses to
  start with a pinned MAGPIE below its own floor, and the image's pin
  (`docker/Dockerfile`, `MAGPIE_COMMIT`) is `dc007fc5`, which reports `0.5.0`.
  Moving the pin needs the commit on GitHub. See U1.
- **Test.** `test_lexical_flags_are_set_before_the_load` now forces the WIT
  flag on (the way a settings file leaves it; `-wit true` itself refuses to
  run without a file) and asserts the load clears it.
- **PLAN.md** gains a paragraph under "What the worker does".

---

## 3. Bugs and footguns

### B1 — purging or deleting an opening-rack job scanned the moves table once per task

*Code changed; PLAN.md schema block updated (K1).*

- **What the code did.** `position_analysis_moves` carried its own
  `task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE`, kept — the
  migration comment said — "for the cascade rather than for reads", after the
  job-wide aggregates that once read it were removed. There was no index on
  it. `purge_job` and `delete_job` both end in `DELETE FROM tasks WHERE
  job_id = $1`, and Postgres runs the cascade per deleted parent row: for each
  task, `DELETE FROM position_analysis_moves WHERE task_id = $1` — a
  sequential scan of the largest table in the schema.
- **Why it matters.** A full English opening-rack job is ~6,400 tasks at 500
  racks each over ~32 million move rows. Its purge was ~6,400 sequential
  scans of that table, inside one transaction holding the job's dispatch lock
  and every open claim's row: hours, during which the job's claims time out,
  the admin's request outlives the ALB's 300-second idle timeout, the handler
  future is dropped and the transaction rolls back — so the purge never
  completes at all. The record's cascade (`position_analysis_moves_record_idx`
  on `(record_id, rank)`) already reaches every move.
- **Fix.** The column and its foreign key are gone; the insert no longer
  binds it; `scripts/e2e_magpie.py`'s one query through it now goes through
  the record. Two redundant indexes were dropped with it (S1).
- **Test.** `admin_api::purging_a_job_removes_its_captured_positions_through_the_record`:
  a capture job's record, move and ply rows all go with the purge.

### B2 — the MAGPIE floor disagreed with itself, and the deployment carried the low one

*Code changed; PLAN.md updated (K2, K3). A deployment blocker.*

- **What the code did.** `dbd285d` raised the floor to `0.5.0` — the first
  MAGPIE that checks a wordmap or a rack info table against the pinned hash,
  and the first that loads a table at all — in `config.rs`, compose, README,
  TESTING.md, the test harness and the contract fixtures. It left
  `infra/variables.tf` (`default = "0.4.0"`, which is what the ECS task
  definition injects), `backend/.env.example` (`0.4.0`) and the `jobs` column
  default (`min_magpie_minor DEFAULT 4`, with a comment describing `0.4.0` as
  the first version whose results depend on nothing but the task) alone.
- **Why it matters.** A Terraform deployment would have admitted `0.4.0`
  workers, which verify no derived hash and play with whatever wordmap sits
  on their disk — on jobs whose whole point since `dbd285d` is that every
  worker's derived files agree with the server's. `create_job` writes the
  server's floor explicitly, so the column default is unreachable through the
  API, but a restore or a hand insert would take it.
- **Fix.** `0.5.0` everywhere, with each place's comment saying what `0.5.0`
  is the first to do and that the value must not exceed what the image's
  pinned MAGPIE reports (the backend refuses to start otherwise —
  `main.rs`). The Terraform description says so; the Dockerfile comment,
  which claimed CI's `magpie-contract` job pins the same commit (it tests the
  branch head, deliberately), is corrected.

### B3 — staged imports were never expired

*Plan wins: **code changed** (K4).*

- **What the code did.** `PLAN.md` (Importing a tarball): "Unconfirmed
  imports are garbage-collected after 24 hours." Nothing did it: the only
  reaper was `fail_orphaned_imports` at startup, for rows left `running`. A
  staged import keeps a row per file, with the bytes of every letter
  distribution and layout, and shows a diff against `input_data` as it was
  when the import ran.
- **Fix.** `inputdata::expire_unconfirmed_imports`, run hourly from `main.rs`:
  imports `staged` for over 24 hours are marked `cancelled` with a reason (the
  state already existed in the CHECK, unused) and their staged rows deleted.
  The objects an import uploaded stay — they are keyed by digest and are
  exactly what the next import would upload. `confirm_import` locks the row
  and requires `staged`, so an expiry racing a confirmation is sound in both
  orders (the loser sees the other's state).
- **Test.** `admin_api::an_unconfirmed_import_expires_after_a_day_and_says_so`.

### B4 — nothing local could build a derived file, and the end-to-end run never tried

*Plan wins on intent: **code changed** (K12). A testing and development gap.*

- **What the code did.** Since `dbd285d` a job whose players ask for a
  wordmap or a rack info table is not dispatched until a `derived_data` row is
  `built`, and only `build-derived` — run as a scheduled ECS task in
  production — writes one. `docker-compose.yml` had no such service,
  `scripts/dev.py` ran nothing of the kind, and `scripts/e2e_magpie.py` set
  `use_wordmap: False` on its leave job and on no player, so the nightly
  end-to-end run — the one test that runs a real MAGPIE against the real
  server — never exercised the server-built hash, the worker's build, or the
  comparison between them. Locally, a leave-generation job created from the
  form (which defaults `use_wordmap` on) sat undispatched with nothing to fix
  it short of `cargo run --bin build-derived` on the host.
- **What PLAN.md said.** "`docker compose up` is the whole stack", and the
  nightly run "runs one real task per job type".
- **Fix.** A `derived-builder` service in compose (the `derived-builder` image
  target, the same mounted MAGPIE the backend runs, under a profile so `up`
  does not start it; `docker compose run --rm derived-builder` drains the
  queue once). `e2e_magpie.py` gives the static opening-rack player and the
  leave job a wordmap and runs the builder for both, asserts the queue is
  empty afterwards, and now treats "declining this task" in MAGPIE's output
  as a failure (a `derived_mismatch` would otherwise have shown up only as a
  timeout). README and PLAN.md's Development section say how to run it.

### B5 — checked and found sound (no change)

- Every claim-path race the prior audits closed (prior R1–R4 and their
  predecessors) was re-read and still holds; section 4.
- `insert_game_results` read player 1's `num_plays_recorded` on every games
  submission, capture or not. Skipped when there are no positions (C1).
- `check_batch_against_task` looked the job up from the task it was handed,
  with the job already in the caller's hand (C1).

---

## 4. Race conditions

No new race was found. What was examined, including everything the two
unaudited commits added:

- **The derived gate vs. the builder.** `ready_for_job` (and before this
  audit `status_for_job`) runs before the dispatch lock and outside the claim
  transaction. A `derived_data` row can change between that read and the
  commit only toward `built` (the builder) or from `failed` to `pending`
  (an admin retry), and neither makes a hash a claim carries wrong: the
  hashes are read from `built` rows, and a `built` row is never rewritten
  (`take_next` selects `pending` or lapsed `building` rows only; `retry`
  touches `failed` only). The cache added in C2 rests on the same argument,
  stated in full on `DerivedCache`.
- **Two builder tasks on one row.** `take_next` locks with `FOR UPDATE SKIP
  LOCKED` and stamps a 45-minute lease; a builder that dies leaves a lease to
  lapse. Two builders finishing the same row (a lease lapsed mid-build and
  another took it) write the same hash — a build is a pure function of its
  inputs, which is the premise the whole design rests on.
- **Import expiry vs. confirmation** (B3): both update the row with a state
  predicate; whichever commits second sees the other's state and does
  nothing (expiry) or answers `409` (confirm).
- **Seeding a leave universe vs. a purge.** The seeding holds the dispatch
  lock for the whole `COPY`; a purge waits on it and then deletes what was
  written. A claim that finds the universe missing after that starts the
  seeding again.
- **A transition that failed vs. reissue.** `run_leave_generation_transition`
  backdates `started_at` on failure; `transition_in_progress` then reads
  false, so a lapsed task of the generation can be reissued — correct, since
  nothing was built.
- **Finish check vs. a claim being issued.** Unchanged from prior R2/R3: the
  claim's `UPDATE jobs … WHERE status = 'active'` and the finish check's
  `complete_unless_purged` serialize on the job row, and whichever commits
  second sees the other's write.
- **The completed-claims index** (P2) changes no locking; it is a read plan.

---

## 5. Critical path

The critical path is a worker getting its next task and getting its result
accepted. Traced statement by statement, as the prior record did, with the
two unaudited commits included.

**Claim** (`POST /api/worker/task`): identity lookup with throttled touch and
ban check (one statement); `candidate_jobs`; one reclaim statement for the
tier; per candidate: **the derived gate** (new in `dbd285d`), dispatch lock,
reissue or generate, `anonymous_workers` insert (new workers only), claim
insert, task counter update, guarded job update, `expected_data`, commit.

**Submit** (`POST /api/worker/result`): identity; claim `FOR UPDATE`; task
`FOR UPDATE`; job read; validation and plausibility; batch checks against the
request; record insert; claim, task and job counters; identity counter;
commit. Then inline: `load_job` and the debounced finish check. Then spawned:
the SSE payload.

### What moved off it, or shrank, in this audit

| Change | Path | Why it was safe |
|---|---|---|
| **C2** The derived gate is answered from memory once a job is dispatchable (`derived::DerivedCache`); a miss takes a pool connection, a hit does not | Claim, per candidate job | A dispatchable answer cannot change for the life of the process: needs are fixed at creation (configs and job configs are immutable), rows only move toward `built`, nothing deletes one (`input_data` refuses a delete while referenced), and the builder identity the query matches on is a constant of the binary. A waiting job is asked about every time, so a build is noticed the moment it lands. `delete_job` forgets the entry |
| **C1** The `num_plays_recorded` read in `insert_game_results` is skipped when a submission carries no positions, which is every games job without capture | Submit | The value only decides how many moves to keep, and there are none |
| **C1** `check_batch_against_task` takes the job id the caller already holds instead of looking it up through the task | Submit (opening racks) | Same value, one fewer round trip inside the task's row lock |

### Kept on the path, and why

| Kept | Why |
|---|---|
| The derived gate itself (a miss) | The hashes travel with the claim; a task issued before they exist would carry none, which a worker reads as a server that checks nothing |
| Finish check, debounced | Gates dispatch; decided by the prior audits |
| `users`/`anonymous_workers.tasks_completed` in the submit transaction | Decided (prior audits): a single-row update against counter accuracy |
| `expected_data` per claim | The worker needs it to verify data |
| Re-expanding an opening-rack range at submit | Needed for the exact-racks check |
| The SSE payload | Already spawned and coalesced; the two reads that grew with history are addressed in P2 rather than moved |

---

## 6. Performance — most severe first

1. **Purging or deleting an opening-rack job scanned the moves table per task.**
   *Fixed (B1).* Impact: on a full English opening-rack job, thousands of
   sequential scans over tens of millions of rows inside one transaction —
   hours, past the ALB's 300 s timeout, so the purge rolled back and never
   completed, while the job's dispatch lock and every open claim row were held.
2. **The ETA and the job list's `stalled` flag walked a job's whole claim
   history for a question about its last hour.** *Fixed (P2).*
   `estimate_eta` runs on every detail view and every live push;
   `list_jobs` asks the day-window question per active job with a recent
   decline. `task_claims` has no job column, so the plan went tasks-of-job →
   claims-of-task → filter, linear in the job's age. A partial index on
   `(completed_at DESC) WHERE state = 'completed'` bounds both by the fleet's
   recent completions. Measured on a synthetic job with 200,000 completed
   claims and 300 in the last hour: 9 index pages and 300 rows read through
   the new index, against a parallel sequential scan filtering 200,000 rows
   (3,082 pages) without it. Impact before: tens of milliseconds per push on a
   400,000-task job, growing with every task; on a two-year-old job, the
   dominant cost of every live update. `worker_data_gaps_job_idx` is now
   `(job_id, reported_at DESC)` for the same question's other half.
3. **The derived gate ran a six-table join for every candidate job on every
   claim.** *Fixed (C2).* One to two milliseconds per candidate, before the
   dispatch lock, on the hottest path in the system; with several jobs in a
   tier, several per claim. Now a hash-map lookup after the first dispatch.
4. **A games submission paid a read it never used.** *Fixed (C1).* One
   round trip per submission on every games and pairs job without capture.
5. **`worker_contributions` groups every completed claim of the job on every
   push.** *Unchanged; prior U6 decided to wait for the slow-stats log line.*
   136 ms at 44,000 claims (PLAN.md's table), linear from there. Still the
   largest read in the payload; the decision stands.
6. **Leave-generation throughput per job.** *Unchanged; prior U4.*
7. **`rebuild-artifacts` inline.** *Unchanged; prior U5.*

---

## 7. Storage

### Fixed

| # | What | Change |
|---|---|---|
| S1 | `position_analysis_moves.task_id` (16 bytes plus the FK per row, ~500 MB per full English job) | Dropped (B1) |
| S1 | `position_analysis_records_task_idx (task_id, rack)` duplicated `position_records_task_idx (task_id)` — two indexes maintained on every position insert, one never needed | Wider one dropped |
| S1 | `position_analysis_plies_move_idx (move_id)` duplicated the leading column of `UNIQUE (move_id, ply)` | Dropped |
| P2 | `task_claims_completed_idx` on `(completed_at DESC) WHERE state = 'completed'` | Added: one entry per completed claim, a claim is minutes of work, so the write cost is nothing measurable |
| P2 | `worker_data_gaps_job_idx` | `(job_id, role, name)` → `(job_id, reported_at DESC)`; the admin view groups a job's gaps either way |
| B3 | Staged import rows and their bytes for imports nobody confirmed | Expired after a day |

### Flagged, not changed

- **`leave_rack_progress` keeps every generation's rows for the life of the
  job** (~500 MB per English generation with its two indexes). Deliberate:
  they are what `rebuild-artifacts` derives a generation's KLV from, and the
  design treats the object store as derivable. A job of 10 generations is
  5 GB that is never read again once its artifacts are verified. A policy
  (drop or archive a closed generation's rows once its artifact has been
  verified, keeping the hash) would be a real trade-off against the rebuild
  guarantee — U2.
- **`rating_run_residuals`** (new since the prior audit, its U1) adds
  members² ÷ 2 rows per run to the members-per-run the prior record noted:
  a pool of 20 configs with an active job refits every two minutes, so
  ~720 × 210 ≈ 150,000 residual rows a day. Prior U7 decided no retention
  for rating runs; the number is recorded here so that decision is revisited
  with it — U2, since decided: runs older than a month are thinned to one a
  day (section 10).
- **`worker_data_gaps` and `audit_log` `task.declined` rows** grow with
  declines: up to 32 gap rows plus one audit row per decline. Bounded in
  practice by workers × jobs (a client remembers a job it declined) but reset
  every time a client restarts. Prior U7 — U2.
- **Captured positions store the CGP as `TEXT`** (~130–270 bytes each for
  English, below the TOAST compression threshold): 1.8 GB for the 9 million
  positions of a 400,000-game capture job. A packed machine-letter encoding
  would be several times smaller and is a schema and wire change — U3.
- **`tasks_claimed_idx ON tasks (state) WHERE state = 'claimed'`** is not used
  by any query this audit could find (reclamation reaches tasks through the
  open-claims index and the primary key; the stats counts use
  `tasks_job_idx`). Left alone: it is tiny (claimed tasks only) and dropping
  an index nothing measures is not worth a migration edit on its own.

---

## 8. PLAN.md reconciliation

"Code wins" means PLAN.md was updated to match the code. "Plan wins" means the
code was changed (and PLAN.md updated wherever its wording also needed it).

| # | Subject | Code | PLAN.md said | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | Schema block | `0436ddb` folded `derived_data` into the migration at a different position with different comments; `leave_generation_artifacts.builder`'s comment differed | The block still carried the pre-fold text | **Code wins** | Regenerated mechanically from the migration, after B1/P2's edits; identical by `diff` |
| K2 | `MIN_MAGPIE_VERSION` default (config table) | `0.5.0` | `0.4.0` | **Code wins** | `dbd285d` raised it; the table was not updated. Terraform, the env example and the column default were brought level with the code (B2) |
| K3 | Version negotiation: what the floor is and why | `0.5.0`; MAGPIE now reports `0.5.1` | "default to **`0.4.0`**: the first MAGPIE version whose results depend on nothing but the task" | **Code wins** | Rewritten: `0.5.0` is the first that checks derived files and loads a table; `0.4.0`'s gap named; why `0.5.1` is not yet the floor (U1) |
| K4 | Unconfirmed imports | Never expired | "garbage-collected after 24 hours" | **Plan wins, code changed** | B3. The plan's behaviour is cheap, bounded and what an admin expects |
| K5 | Admin API table | Has `GET /api/admin/workers`, `GET /api/admin/derived-data`, `POST /api/admin/derived-data/retry` | Listed none of the three | **Code wins** | Added with their semantics |
| K6 | Audit actions table | Writes `rating_pool.created`, `rating_pool.member_added`, `rating_pool.member_removed`, `derived_data.retried` | Listed none | **Code wins** | Added |
| K7 | Admin routes | `/admin/derived-data` exists | Not listed | **Code wins** | Added |
| K8 | Directory structure | `derived.rs`, `magpie.rs`, `magpie_defaults.rs`, `magpie_standard15.txt`, `bin/build-derived.rs`, `infra/derived.tf`, `frontend/…/admin/derived-data/` exist | Not in the tree | **Code wins** | Added |
| K9 | Contribution table and `?worker=` | Pseudonym (`anon_id`), never the UUID | "username or anonymous UUID", twice | **Code wins** | The prior audits made the UUID a credential; the two sentences predate that |
| K10 | Config table | `MAGPIE_BIN`, `MAGPIE_THREADS`, `MAGPIE_SCRATCH_DIR` are read | Not listed | **Code wins** | Added |
| K11 | Word info table | Left to the contributor's settings (MAGPIE) | "a wordmap already sitting in `./data` … is not switched on by its mere presence" — the principle, stated for wordmaps only | **Plan wins on intent, code changed (MAGPIE)** | M1; PLAN.md extended to the third file |
| K12 | Local development and the nightly run | No builder in compose or the scripts; e2e turns wordmaps off | "`docker compose up` is the whole stack"; the nightly run "runs one real task per job type" | **Plan wins on intent, code changed** | B4; PLAN.md's Development section documents the on-demand builder |

Counted: K1, K2, K3, K5–K10 are code-wins (9). K4, K11, K12 changed code (3).

**Also updated in PLAN.md, not discrepancies:** two bullets under "What these
reads cost" for P2 and B1; "Where the builds run" describes the cache (C2).

**Checked and found in agreement:** every prior K-item still holds; the
claim loop and submission descriptions; the leave-generation claim steps; the
provenance section against `config_contribute_ensure_wordmap` /
`_ensure_rack_info_table`; the wire contract against the fixtures and
MAGPIE's readers; the Python worker's status (section 9).

---

## 9. Python worker

Searched README, RUNBOOK, TESTING, PLAN, MAGPIE_DEPENDENCY, compose, the env
examples, the Dockerfile, the Terraform, CI, the scripts, the frontend and the
backend for any description of `worker/fake_worker.py` as a production client.
**None found; nothing to correct.** Its docstring, the compose profile comment
("end-to-end suite only"), README's table and its "Contributors are always real
MAGPIE" paragraph, `scripts/dev.py`'s refusal to run it, RUNBOOK's "Never use
`worker/fake_worker.py` for this", TESTING.md's tier table and PLAN.md's Worker
Client and Development sections all say MAGPIE is the only contributor. The
only backend reference is the fixture test pinning its opening-rack submission
shape. The landing page says "You need only MAGPIE — no Python, no Docker".

---

## 10. Left for human input — decided and implemented (2026-09-16)

Each item's recommendation was taken. The original options are kept below the
table for the record.

| # | Decision | What was done |
|---|---|---|
| U1 | **Publish `65a246d5`, move the pin, raise the floor to `0.5.1`** | `docker/Dockerfile`'s `MAGPIE_COMMIT` (both stages) is `65a246d5`, whose MAGPIE reports `0.5.1`. The floor is `0.5.1` in `config.rs` and its comment, the `jobs` column default (`min_magpie_patch DEFAULT 1`) and its comment in the migration and PLAN.md's schema block (still identical), `infra/variables.tf`, `docker-compose.yml` (both services), both env examples, the three contract fixtures in both repositories, `backend/tests/common/mod.rs`, the `magpie.rs` unit tests, `scripts/dev.py`, README, TESTING.md, MAGPIE_DEPENDENCY.md and PLAN.md, whose "floor stays `0.5.0` until the pin moves" paragraph now states the rule rather than the wait. MAGPIE's README `builders` example prints `0.5.1`. A build reporting `0.5.0` or lower now gets `magpie_too_old`. **The push is still to do**: this session's permission mode refused `git commit` and `git push` in the MAGPIE checkout, so `origin/birdtest-contribute` is still `dc007fc5`, `65a246d5` is local, and the fixture and README bump sits uncommitted in that checkout's working tree. Until `65a246d5` is on GitHub the backend image cannot build (the Dockerfile's shallow fetch of the pin fails) and `MIN_MAGPIE_VERSION=0.5.1` refuses to start against a `dc007fc5` binary. CI's `magpie-contract` job is unaffected meanwhile: MAGPIE's contribute test never reads a fixture's `min_magpie_version`. Existing databases must be reset (PLAN.md, "Resetting the database after a schema change"): the single migration was edited in place |
| U2 | **(b)**, as a thinning rather than a cut | `ratings::thin_old_runs`, hourly from `main.rs`: runs older than `RUN_FULL_RESOLUTION` (30 days) are thinned to the last run of each UTC day, the pool's first run kept whatever its day; ratings and residuals cascade; batches of 1,000 runs, no fit lock. The recommendation said "delete runs older than N days except the first". That would have started the history chart's past at the window's edge, while a day is the resolution the chart draws at anyway (500 points over the pool's life), so the thinning keeps everything the chart shows and bounds the tables all the same: an active twenty-member pool holds ~21,600 full runs (~4.5 million residual rows) plus 20 rating and ~210 residual rows a day beyond that, instead of ~150,000 new residual rows a day forever. The newest run is the last of its day and so always survives, however long the pool has been quiet. The "few hundred thousand runs" trigger was not built: a gate that waits for a table to grow before bounding it buys nothing but a large first delete. Option (c), `leave_rack_progress`, is untouched, as recommended. Schema comments (migration and PLAN.md's block), PLAN.md ("Ratings") and RUNBOOK.md describe it. Test: `admin_api::old_rating_runs_are_thinned_to_the_last_of_each_day` |
| U3 | **(a)** Leave the CGP as `TEXT` | Nothing to build |
| U4 | **(a)** Wait for the slow-stats log line | Nothing to build |

### The items as they were put

#### U1 — publish MAGPIE `65a246d5`, move the pin, raise the floor to `0.5.1`

The MAGPIE commit is local. The backend image builds `MAGPIE_COMMIT =
dc007fc5` from GitHub, so the pin cannot move to a commit that is not there,
and the floor cannot rise past what the pinned MAGPIE reports (the backend
refuses to start). Until it is pushed, CI's `magpie-contract` and nightly
jobs check out the branch head from GitHub and test `0.5.0` without M1;
nothing breaks, because the floor is `0.5.0`. **Recommendation: push, move
the Dockerfile pin, then raise `MIN_MAGPIE_VERSION` in `config.rs`, the
column default and its comment, `infra/variables.tf`, compose, both env
examples, the fixtures in both repositories, the test harness, README,
TESTING.md and PLAN.md to `0.5.1`**, as the prior audits did for `0.2.0` and
`0.4.0`. All outward-facing; not done here.

#### U2 — retention (re-raised with numbers)

Prior U7 decided no retention yet. Section 7 adds the figures for the two
tables that have grown a dimension since: `rating_run_residuals` (~150,000
rows a day for a 20-config pool with an active job) and `leave_rack_progress`
(~500 MB per English generation, kept for the rebuild guarantee). Options:
**(a)** keep waiting, as decided; **(b)** cap `rating_runs` per pool
(delete runs older than N days except the first, with residuals cascading),
which the history endpoint already thins to 500 points; **(c)** drop a closed
generation's progress rows once `rebuild-artifacts` has verified its object
against the recorded hash, keeping the hash. **Recommendation: (b) when a pool
first passes a few hundred thousand runs; (c) only if disk actually becomes
the constraint**, since it trades away the ability to re-derive a KLV.

#### U3 — a compact encoding for captured positions' CGPs

**(a)** Leave `TEXT`: readable, and the corpus has no consumer yet.
**(b)** Store a packed machine-letter board (one byte a square, 225 bytes,
plus racks and scores) and render the CGP on read: several times smaller for
the largest capture table, but a wire and schema change and a decoder on
every read path. **Recommendation: (a) until the first consumer exists**,
which will say what shape it wants.

#### U4 — `worker_contributions` per push (prior U6, restated)

Unchanged. Options as before: **(a)** wait for the slow-stats log line;
**(b)** a per-(job, identity) counter maintained in the submit transaction,
which is one more row lock per submission. **Recommendation: (a)**, still.

---

## 11. Verification

- Backend: `cargo clippy --locked --all-targets -- -D warnings` clean;
  `cargo test --locked` against Postgres 16: **158 tests** (89 unit and
  contract, 69 integration), all passing (154 before this audit). The five
  `magpie_smoke` tests are `#[ignore]` by design. `svelte-check`: 0 errors,
  0 warnings. After section 10's implementation: **159 tests** (89 unit and
  contract, 70 integration), all passing, clippy clean; the frontend is
  untouched. MAGPIE's `./bin/magpie_test contribute` passes at `65a246d5`
  with the fixtures at `0.5.1`. The backend image was not rebuilt and the end
  to end run not repeated: the pin now names a commit GitHub does not yet
  have (U1).
- MAGPIE: `make magpie_test` (sanitizers), `./bin/magpie_test contribute` and
  `./bin/magpie_test config` pass at `65a246d5`; `git clang-format --diff` is
  empty.
- End to end (section 1): run twice, the second time against a backend image
  rebuilt from the final commit (`30f4855`). **Every job type passed both
  times**: games and game pairs (2 accepted claims each), opening racks
  static and simming (2 each), and leave generation (98 s and 115 s, 2 each).
  The two wordmap jobs went through the derived path in full: the server
  queued and built `wmp NWL23` (builder `wmp-1`, target `nehalem`,
  128,361,169 bytes, hash `214a46d7…` — the value the contract fixture
  happens to carry), the claim stated it, and MAGPIE's own copy agreed, so
  both jobs were dispatched and completed with no `derived_mismatch` and
  no decline of any kind in its output. The second run found the wordmap
  already built and already matching (2 s builder runs). The backend log held
  no error or warning line in either run. The stack was torn down afterwards.

---

## 12. Tests added

| Test | What it pins |
|---|---|
| `admin_api::purging_a_job_removes_its_captured_positions_through_the_record` | B1: records, moves and plies cascade from the task through the record |
| `admin_api::an_unconfirmed_import_expires_after_a_day_and_says_so` | B3 |
| `worker_api::a_dispatchable_jobs_hashes_are_remembered_for_the_process` | C2: a waiting job is not remembered, a dispatchable one is, and forgetting re-consults the gate |
| `derived::tests::the_cache_remembers_only_what_it_is_told_and_forgets_on_request` | C2 |
| `worker_api::redundant_captured_positions_are_recorded_once` (updated) | Counts moves through the record, since the column is gone |
| MAGPIE `test_lexical_flags_are_set_before_the_load` (extended) | M1: the word info table flag is cleared before the load |
| `admin_api::old_rating_runs_are_thinned_to_the_last_of_each_day` | U2: the first run, each old day's last and every recent run survive; a deleted run's ratings and residuals go with it; a second pass deletes nothing |
| `scripts/e2e_magpie.py` (extended) | B4: two jobs go through the derived-file path end to end; a decline in MAGPIE's output fails the run |
