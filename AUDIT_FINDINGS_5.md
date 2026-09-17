# birdtest audit — findings, version 5

Branch: `audit/birdtest-2026-09-17`, off `audit/birdtest-2026-09-16-pass2` at
`36d6499` (which is `main` at `6a72333` plus the sixth, seventh and eighth audits
and the commits that implemented their decisions, none of it merged — see "Where
this branch sits").
MAGPIE: `birdtest-contribute` at `d93dacaf`, **unchanged by this audit** and
level with `origin` (checked with `git ls-remote`). It was verified rather than
assumed — section 9 — and nothing was found missing.
Date: 2026-09-17.

**This is the ninth audit.** It builds on [AUDIT_FINDINGS_4.md](AUDIT_FINDINGS_4.md)
(the eighth, branch `audit/birdtest-2026-09-16-pass2`), [AUDIT_FINDINGS_3.md](AUDIT_FINDINGS_3.md)
(the seventh, `audit/birdtest-2026-09-16`), [AUDIT_FINDINGS_2.md](AUDIT_FINDINGS_2.md)
(the sixth, `audit/birdtest-2026-09-15`), [AUDIT_FINDINGS_1.md](AUDIT_FINDINGS_1.md)
(the fifth, PR #6) and, through them, the four unnumbered records before those
(`AUDIT_FINDINGS.md` on `main`). The highest numbered file anywhere in the
history was `AUDIT_FINDINGS_4.md`, so this is `AUDIT_FINDINGS_5.md`; the earlier
files are untouched. Where an entry revisits a prior item it names it ("prior
U1").

This file is the authoritative record of every code-versus-`PLAN.md` decision
made in this audit, and of the bugs, races, MAGPIE argument trace, critical-path
analysis, performance and storage findings behind them.

**Counts: 4 code-wins (PLAN.md or TESTING.md updated to match the code), 8
plan-wins (code changed), 3 items left for human input (U1–U3).**

The default bias is that the code wins and `PLAN.md` is brought level with it.
The code was changed only where it was wrong, or where the plan described the
behaviour the rest of the system needs.

---

## Where this branch sits

`audit/birdtest-2026-09-16-pass2` has not been merged to `main`, has no pull
request and was never pushed (`origin` has no such branch). Its last commit,
`36d6499`, is the human's implementation of the eighth audit's three decisions
(join at parity, stage leave results, export captured positions) — about 1,900
changed lines that **no audit had read**, and that the eighth audit's record says
was **never run against a real MAGPIE** ("Run it once before merging"). Branching
from `main` would have meant auditing code eleven commits of fixes behind, and
producing a branch that conflicts with all of them, so — as the seventh and
eighth audits did — this branch is cut from that head. Merging it merges
everything since `main`.

`36d6499` therefore got the closest reading, and five of this audit's findings
come from it (R1, P1, B1, U1, U2). The most severe one does not: P0 is older
than every audit, and was found only because this one ran a full-size export
against a live server and watched the server stop.

---

## 0. What the prior audits left open

| Prior item | Still true? | How it was checked |
|---|---|---|
| Eighth audit, 11a — "the real-MAGPIE end-to-end suite was not rerun after these three changes … run it once before merging" | **Done here, three times**, natively (section 13). Every job type passes against this branch with MAGPIE `d93dacaf` | `scripts/e2e_magpie.py`, unmodified |
| Eighth audit — "dispatch the nightly workflow once after merge" | **Still not possible.** The branches are unmerged, so the nightly still runs `main`'s workflow and still fails every night on `make magpie BUILD=release` (`gh run view 35210850997`: "release is a target; run 'make release'"), which the seventh audit fixed on these branches | `gh run list`, `gh run view --log-failed` |
| Eighth audit, U1 (join at parity) — decided (a), implemented | The implementation does what was decided. What was decided has a hole the decision did not have in front of it: **this audit's U1** | Section 11; demonstrated with a probe test |
| Eighth audit, U2 (stage, then merge) — decided (b), implemented | Sound as designed — every interleaving of stage, merge, claim decision and transition was re-derived (section 4). Two things the design needed and did not get: a lock order against purge (**R1**) and an index that serves selection (**P1**); one consequence it did not follow through (**B1**). What it recorded as undecided is now two questions (**U2**, **U3**) | Sections 3, 4, 6, 7 |
| Eighth audit, U3 (positions export) | Holds; the artifact path was exercised live (section 13) | — |
| Eighth audit, "smaller, noted" — identity before rate limit; `?rack=` canonicalisation; concurrent imports in memory | Unchanged. One addition to the second: this audit found the database's collation is `en_US.utf8`, under which `?` sorts *inside* the letters rather than before them. `?rack=` is an equality, so it is unaffected; the leave feed and selection order on `rack` within the database and are self-consistent | Section 3, B4 |
| Seventh audit, U1 — lapsed claims of jobs nobody claims from | Stands | Not re-litigated |
| Sixth audit — `tasks_claimed_idx` unused, left alone as tiny | Left alone again | — |
| Every K-item of the prior four records | Hold | Spot-checked while reading; the schema block in PLAN.md is byte-identical to the migration before and after this audit (`diff`) |

---

## 1. How this audit was run

- Read `PLAN.md` in full (6,761 lines), diffed its schema block mechanically
  against `backend/migrations/0001_initial.sql` (identical before and after), read
  the eighth audit's record in full, then the diff of `36d6499` line by line.
- Backend, read in full: `scheduler.rs`, `jobs/leave_gen.rs`, `jobs/registry.rs`,
  `jobs/mod.rs`, `jobs/dispatch.rs`, `jobs/handler.rs`, `jobs/game.rs`,
  `routes/worker.rs`, `exports.rs`, `sse.rs`, `main.rs`, `error.rs`, the
  migration; and the parts of `routes/admin.rs` (activate, deactivate, complete,
  purge, delete, census, export, merge-progress), `routes/public.rs` (job list,
  results feed, admin stream), `inputdata.rs`, `auth/api_key.rs`, `routes/auth.rs`
  and `jobstats.rs` that this audit's findings touch.
- MAGPIE `birdtest-contribute` at `d93dacaf`: the three contribute executors,
  both resets, `config_contribute_load_lexicon_and_variant`,
  `config_fill_game_args`, `config_fill_sim_args`, `config_fill_autoplay_args`,
  `impl_move_gen`, the whole `Config` struct field by field, and the claim,
  decline and shutdown code in `contribute.c` against the five non-assignment
  contract fixtures.
- **Measured, not estimated.** Query plans and timings on a throwaway Postgres 16
  capped at 2 CPUs and 1 GB (closer to a `db.t4g.micro` than the 12-core
  development machine): first on a 400,000-rack synthetic generation, then on a
  **real full-size generation** — the 3,199,724 racks the end-to-end run's leave
  job seeded — which is where the headline numbers come from. Server
  responsiveness was measured by polling `/health` every 200–250 ms while the
  operation under test ran.
- Backend: `cargo clippy --locked --all-targets -- -D warnings` clean; `cargo test
  --locked`: **178 tests before, 187 after**, all passing. Frontend: `npm run
  check`, 0 errors, 0 warnings (the frontend is unchanged).
- MAGPIE: `./bin/magpie_test contribute` passes; the three assignment fixtures
  are byte-identical in the two repositories; `magpie builders` reports
  `0.1.0`/`nehalem`; `help convert` lists the three conversions birdtest runs.
  Per a standing instruction for this machine, `cppcheck`/`clang-tidy` were not
  run, and every build was capped (`-j 2`, `nice`).
- End to end: `scripts/e2e_magpie.py`, unmodified, with a `portable_release`
  MAGPIE at `d93dacaf` against a **native** stack (section 13) — no Docker image
  was built.

---

## 2. MAGPIE arguments that change a task's outcome

The prior tables were not reused. The trace was re-run from both ends against
`d93dacaf`: every key the executors read, and every field of `struct Config`
asked whether autoplay, move generation or a simulation can read it on the
contribute path. Everything the prior records list still holds. Checked this
time and not named before:

| Setting | Changes results? | How it is covered | Change |
|---|---|---|---|
| `challenge_bonus` | No | Read only by game-history code (GCG import, challenge events); autoplay, `impl_move_gen` and `impl_sim` never see it | None |
| `overtime_penalty_points`, `overtime_period_ms` | Would, under a clock | `config_fill_autoplay_args` passes them, but autoplay applies them only for a player with `use_play_chooser`, which requires `pN_play_chooser_time_ms >= 0`; `config_contribute_reset_player_settings` sets both to `-1` before every task | None |
| `position_play_cap` (`max_num_display_plays`) raising a simmer's `num_plays` | Yes, with capture on | Set from the request's `num_plays_recorded` on every games task; autoplay raises `num_plays` only when `captures_positions`; job creation refuses a capture job whose simmers would be raised (prior fix) | None |
| `endgame_*`, `peg_*`, `tt_fraction_of_mem` | Would, under the PlayChooser or the endgame/PEG commands | Reached only through `use_play_chooser` (off, as above) or commands contribute never issues | None |
| `print_boards`, `game_string_options`, `show_bu`, `use_mmap_for_rit` | No — output and loading mode | — | None |
| `games_before_force_draw_start` (leave generation) | Yes | Not a `Config` field: the executor passes the literal `0` to `config_autoplay` | None |

**No gap found.** The server half was already complete (the contract test pins
both directions, and this audit's three end-to-end runs exercised every request
shape through `config_contribute_validate_common`), and no MAGPIE change was
needed.

---

## 3. Bugs and footguns

### B1 — a force-completed leave job's export missed everything still staged

*Plan wins: **code changed** (K5). From `36d6499`.*

- **What the code did.** A leave job's corpus is `leave_rack_progress`
  (`exports::export_query`), and since `36d6499` an accepted result reaches that
  table only at a merge. A job whose last generation closes is drained by its
  transition. A job an admin **force-completes** mid-generation is not: claims
  already out are still played and accepted, and their results sit in
  `leave_rack_staging` until the half-hourly sweep. An export is refused while
  claims are open — and allowed the moment they settle, up to thirty minutes
  before their results are merged.
- **Why it matters.** PLAN.md's rule for exports is "only once its results have
  settled", because "an export started in that window missed them, and every
  later download was redirected to it". The staged window is that bug again
  through a new door.
- **Fix.** `exports::settle` merges what a leave job has staged, waiting for a
  merge already running; the export's background task calls it before reading a
  row, and so does the admin stream for a completed job.
- **Tests.** `leave_gen::a_completed_leave_jobs_corpus_includes_what_was_still_staged`.
  Also verified live (section 13): a real MAGPIE result staged, the job
  force-completed, exported — the backend logged `merged staged leave results
  folds=1 racks=385` as the export began, the artifact held 3,199,724 lines of
  which 1,164 had occurrences, exactly the database's count, and nothing was
  left staged.

### B2 — a deployment inside a generation transition stalled the job for half an hour

*Code changed; PLAN.md updated (K6).*

- **What the code did.** A transition runs on a spawned task; its
  `leave_generation_transitions` row is the only evidence it exists. A
  transition that *fails* hands ownership back at once; a process that *dies* is
  covered by a 30-minute takeover timeout. But the ordinary way a process dies
  is a deployment, the service is a single instance whose old task stops before
  the new one starts, and startup already reaps `running` imports and exports on
  exactly that reasoning.
- **Fix.** `leave_gen::release_orphaned_transitions`, called at startup beside
  the other two reapers: it backdates every open transition, which is the same
  hand-back a failed transition performs, so the next claim takes over through
  the existing path and `attempts` records it.
- **Safety.** If two instances ever did overlap, a live transition could be
  taken over; `close_generation` is conditional on the row still being open, so
  the second close is refused and the object (same key, same inputs) is
  overwritten with the same bytes.
- **Test.** `leave_gen::a_restart_hands_an_open_transition_to_the_next_claim`.

### B3 — a request body that did not parse was answered by the framework, not the API

*Plan wins: **code changed** (K1, K2). Older than every audit.*

- **What the code did.** Every JSON route took its body through axum's `Json`
  extractor, whose rejection is axum's own: `text/plain`, with `400` (malformed),
  `415` (no content type) and `422` (wrong shape). Observed on the live server:
  `Failed to deserialize the JSON body into the target type: missing field
  'magpie_version' at line 1 column 2` → `422 text/plain`.
- **What PLAN.md says.** "Every failure is JSON with the same shape, whatever the
  status", with a closed list of codes; and, of one error in particular, "a
  bodyless claim is rejected with an error that names the fix rather than a bare
  `422`, because that error is what a stale MAGPIE build will show a contributor
  after launch." TESTING.md's `A-WORKER-1` specified the same test and had no
  test behind it.
- **Fix.** `extract::ApiJson`, used by all nineteen JSON routes: the same
  extractor with `AppError` as its rejection (`400 bad_request` naming what the
  parser found, `413 payload_too_large` past the route's limit). The claim
  handler takes `Result<ApiJson<ClaimBody>, AppError>` and rewrites the message
  to say what to send and that a MAGPIE sending neither field predates the
  protocol. Large bodies are parsed on the blocking pool (P0).
- **Wire compatibility.** MAGPIE treats every `4xx` other than `429` the same way
  (returns it to the caller, which prints the first 200 characters of the body),
  so `422` → `400` changes nothing it does; the second and third end-to-end runs
  are with this extractor on every worker request.
- **Tests.** `extract::tests::*` (three), `worker_api::a_claim_without_a_usable_body_is_told_what_to_send`.

### B4 — a test of mine that assumed byte order, and what it turned up

Not a product bug; recorded because the next person will meet it. The first
version of the leave-feed test sorted its expectation in Rust and failed: the
test database's collation is `en_US.utf8`, under which `?` is ignored at the
first level, so `?AAABBC` sorts between `AAABBCD`'s neighbours, not before them.
RDS's default is the same. Everything that orders on `rack` does so *inside*
Postgres (selection, the feed's seek, `generation_klv`'s stream), so it is
self-consistent, and the test now takes its expected order from the database.
The eighth audit's note about `?rack=` canonicalising by code point is
unaffected — that lookup is an equality — but anyone adding a comparison of a
database-ordered rack list against an application-ordered one should know.

### B5 — checked and found sound (no change)

- `join_at_parity` does what was decided: baseline arithmetic, the `0%` case,
  activation under the activation lock, purge after the counters are zeroed.
  (What was decided is U1.)
- `stage_fold`, `merge_staged`, the `NeedsMerge` decision and the transition's
  drain: section 4.
- `TailMerges` (in-memory, per job, forgotten on delete); the sweep's first tick
  at startup; one job's failed merge not stopping the others.
- The positions export and `?positions=true`.
- The SSE broadcaster: subscribe and the empty-channel cleanup are under one
  mutex; pushes coalesce as described.
- Python worker: section 8a.

---

## 4. Race conditions

### R1 — a purge or a delete deadlocked with a running merge

*A bug under objective 2: **fixed** (K3). From `36d6499`.*

- **The inversion.** `merge_staged` is one statement that takes the staged rows
  (`DELETE … RETURNING`) and then updates the per-rack rows, in whatever order
  its plan visits them. `purge_job` deleted `leave_rack_progress` **first** and
  `leave_rack_staging` **second**. Run together: the purge deletes racks in
  physical order until it reaches one the merge has already updated, and waits;
  the merge then reaches a rack the purge has already deleted, and waits.
  Postgres breaks the cycle after `deadlock_timeout` by failing whichever
  transaction checks first. `delete_job` has the same shape through its
  cascades. The comment in `purge_job` said "a merge running right now holds the
  rows it took; this waits for it" — that was the intent, and it is true only
  if the purge never holds a row the merge still wants.
- **Demonstrated**, not inferred. The test plays the merge by hand on its own
  connection (the merge lock, the staged rows, the generation's *last* rack),
  starts a purge through the API, then reaches for the *first* rack. Against the
  code as it was: the purge was the deadlock's victim — a `500`, and `(tasks,
  progress rows)` left at `(1, 149)`, nothing deleted. A full-size merge runs
  61–88 s (eighth audit's measurement), every half hour and once a minute near a
  generation's end, so the window is not small.
- **Fix.** `leave_gen::lock_merges` — the advisory lock `merge_staged` already
  takes, exposed — is taken by `purge_job` and `delete_job` **before anything
  else**, the dispatch lock included. A purge now waits for a running merge
  while holding nothing, and no merge starts until it commits (a merge that
  finds the lock held gives up; a transition's drain waits, then finds its rows
  gone and its ownership row deleted, and closes nothing). Nothing takes the
  merge lock while holding another lock, so the order is merge → dispatch →
  claim → task → job everywhere.
- **Test.** `leave_gen::a_purge_waits_for_a_running_merge_instead_of_deadlocking_with_it`;
  fails against the old purge as described.

### Examined and found sound

- **Stage vs merge.** A submission that commits while a merge runs is not in the
  merge statement's snapshot and stays staged; the `DELETE … RETURNING` feeding
  the `UPDATE` makes a staged result folded exactly once.
- **The closing decision.** Under the dispatch lock `next_step` reads, in this
  order, the selectable racks, the claims in flight, and whether anything is
  staged — each a fresh READ COMMITTED snapshot. A submission committing between
  the first and second reads shows as staged at the third; between the second
  and third, it showed as in flight at the second. There is no interleaving in
  which a generation closes with a result neither in flight nor staged nor
  merged. The order of those three reads is load-bearing, and the code has it
  right.
- **Transition drain.** `run_transition` waits for the merge lock, merges, then
  streams; nothing can be staged for the generation meanwhile (it closes only
  with no claim in flight, and nothing is dispatched for it while its
  transition row is live).
- **B2's startup release vs a live transition**: see B2, "Safety".
- **`settle` (B1) vs a purge**: `settle` takes only the merge lock, which the
  purge now takes first; whichever is second waits and then finds nothing to do.
- **`ApiJson` and `spawn_blocking`** introduce no shared state: each moves an
  owned value to the blocking pool and back. A dropped request abandons its
  blocking task's result, not a lock.
- Everything the prior four records examined — claim vs reclaim vs decline vs
  heartbeat vs submit, redundant submissions serializing on the task row,
  activation, the key limit, the fit lock — was re-read where this audit's
  changes came near it, and holds.

---

## 5. Critical path

Traced statement by statement again for a leave job, the path `36d6499` changed.
**Claim:** identity → `candidate_jobs` → reclaim → cache hits → `BEGIN`, lock
timeout, dispatch lock → `current_generation` → `transition_in_progress` →
`universe_exists` → `next_available` → **selection** → previous artifact key →
task, request and claim inserts → task update → guarded job update → `COMMIT`.
**Submit:** identity → claim `FOR UPDATE` → task `FOR UPDATE` → job → template
(cache) → decode and validate → `leave_records` → request row → closed-generation
check → staging insert → live counters → claim, task, job and identity counters →
`COMMIT` → spawned SSE push.

Every *statement* on both paths is one the decision needs, as the prior audits
found. What was wrong was **what ran on the threads those paths share**, and one
statement's plan.

### What moved off it

| Change | Path | Why it was safe |
|---|---|---|
| **C1 — export compression and hashing run on the blocking pool** (`exports::upload_rows`), and rows arrive as text Postgres already serialized rather than being parsed and re-serialized per row | Every request the server handles: an export's compression loop never yielded, and the worker thread doing it was the one the runtime's socket events were waiting on (P0) | The export is a background task whose only consumer polls a row. Same bytes out: the artifact's SHA-256 and length are computed on the same stream and were checked against a download (section 13); `exports::tests::the_parts_are_one_gzip_stream_of_what_was_pushed` |
| **C2 — the import's gunzip-untar-hash of a whole tarball** runs on the blocking pool | Same mechanism, seconds of computation per import | The archive goes in and comes back out; the progress counter it bumps is atomic |
| **C3 — every Argon2 hash and verify** (register, login, reset) runs on the blocking pool | Tens of milliseconds per call by design; the reset used to hash *inside* its transaction | The call sites await the same value |
| **C4 — a universe's COPY chunks are built on the blocking pool** (`seed_generation`) | `/health` reached **1.3 s** while a universe was seeded (debug build); **3.6 ms** after | The chunk is a pure function of `(index, start)`; the COPY stream is unchanged |
| **C5 — request bodies of 256 KiB and up are parsed on the blocking pool** (`extract::ApiJson`), and **a submission is decoded and validated there** (`registry::store_result`) | A result may be 64 MiB; decoding it and running the plausibility rules is tens to hundreds of milliseconds with no `await` | Validation is a pure function of the payload; nothing is locked across the hop that was not locked before |
| **C6 — leave selection walks an index** (P1) | 4.9 s → 0.2 ms inside the job's dispatch lock | Same rows, same order (section 6) |

### Examined and left on the path

| Kept | Why |
|---|---|
| The live counters bump in `stage_fold` (`leave_generation_progress`) — display-only, in the submit transaction | One single-row upsert, the same trade the prior audits accepted for `jobs.*` and the identity counters; moving it out would make the dashboard's "live" figures a second staging problem |
| `stage_fold`'s read of the request row and the closed-generation check | The first decides which generation the result belongs to; the second is a guard the prior audits kept deliberately |
| The previous-artifact-key read on a leave claim | One primary-key probe; caching it per generation would need invalidation on purge for a fraction of a millisecond |
| Everything the prior records list | Decided |

---

## 6. Performance — most severe first

1. **An export stalled the whole server — claims, heartbeats and submissions
   included — for as long as it ran.** *Fixed (C1; K7).* `upload_rows` read rows
   from a cursor and compressed and hashed them inline. The database delivers
   rows faster than they compress, so the loop's `await` never had to wait and
   the task never yielded; Tokio's worker that last polled the I/O driver is the
   one new socket events wait on, and that was the worker compressing.
   **Observed** with one full-size leave generation exporting: twelve runtime
   threads, eleven parked on a futex, one at 100%; `/health` unanswered past a
   10 s timeout and an admin poll past 30 s, *for minutes*. That was a debug
   build, where gzip is 20–50× slower than release; in a release build the same
   mechanism produces shorter stalls (each run of the loop between forced yields
   is tens of milliseconds, and the driver is only re-polled every 61 of them),
   repeated for the length of the export — and an opening-rack corpus takes
   minutes (eighth audit: 72 s per million records). Nothing in the backend used
   `spawn_blocking` at all. **After:** the same export completes in **34 s** with
   `/health` at **2.1 ms median, 7.8 ms worst**; SHA-256 and length of the
   download match the row. The same fix was applied to every other computation
   of that size (C2–C5).
2. **Every leave claim read and sorted the whole generation, inside the
   dispatch lock.** *Fixed (P1; K4).* Selection orders on `(occurrence_count,
   rack)`; the index was `(job_id, generation, occurrence_count)`. Counts tie in
   their millions — every rack starts at zero, and most of the 3.2 million full
   racks are rare enough to stay there until forced — so the index could not
   supply the order and the plan was a scan of the generation and a top-N sort.
   **Measured on a real full-size generation: 4.8–4.9 s per claim** (2 CPUs,
   1 GB; three runs, parallel and not). The dispatch lock's other claimants give
   up after 2 s, so a leave job with more than one worker answered most of them
   `204` while work existed. PLAN.md recorded 47 ms for this query; that was
   measured on counts that rarely tied. **After: 0.2 ms**, with
   `leave_rack_progress_pick_idx` carrying `rack`. The index alone was not
   enough, and briefly made it worse: given the order, the planner ran the
   `NOT EXISTS` exclusion as a nested loop over the held-out racks (it assumes
   ten elements per `unnest`) — **3.5 s at a tenth of full size**. The exclusion
   is now `NOT IN` over an uncorrelated subquery, which Postgres runs as a hashed
   subplan; it filters its own NULLs, which is what makes `NOT IN` safe. What
   remains grows with what is *staged*, not with the universe: U2.
3. **The public results feed of a leave job sorted every progress row of the job
   for each page.** *Fixed (K8).* `ORDER BY generation DESC, occurrence_count ASC,
   rack ASC` is an order no index runs in. **Measured over HTTP on one full-size
   generation: 4.0 s a page** (3,988 ms in SQL), public, unauthenticated and
   unmetered, eight at a time before the display pool's bound — and linear in
   generations. It is read a generation at a time now, newest first, each read a
   seek into the selection index (two statements, because a row comparison is an
   index condition only when it stands alone): **7 ms** over HTTP, 0.1 ms in
   SQL. `leave_gen::the_leave_results_feed_pages_through_every_generation_in_order`
   pins that the pages still tile the job across a generation boundary.
4. **Selection's cost now grows with what is staged.** *Flagged, U2.* About a
   microsecond per held-out rack to build the hash and skip the index entries:
   9 ms with 20 results staged, 160–290 ms with 400 (200,000 racks), measured at
   400,000 racks and independent of the universe.
5. **A merge rewrites most of a generation as non-HOT updates** (eighth audit),
   now over a larger relation: U3.
6. **`GET /api/admin/fleet`, the evidence sweep, `worker_contributions`** —
   unchanged, decided by prior audits, on the display pool or a background sweep.

Also measured and fine: a purge of a full-size leave job is **3.0 s** with
`/health` at 3.7 ms worst.

---

## 7. Storage

### Changed

`leave_rack_progress_pick_idx` gained `rack` (P1). **It is not free, and the
first version of this record's migration comment said otherwise until it was
measured.** On a full English generation the index is **180 MB where the one
without `rack` was 22 MB**: nearly all of the old index's keys were equal, so
Postgres deduplicated them, and unique keys cannot be. A generation is therefore
590 MB (258 heap, 152 primary key, 180 selection index) rather than 432, kept for
the life of the job. Against that: leave jobs could not dispatch at more than one
claim per five seconds without it. The alternative that keeps the small index is
U3.

The migration and PLAN.md's schema block were edited together and are
byte-identical.

### Flagged — U3, and unchanged from prior records

U3 below. Unchanged: `leave_rack_progress` kept for the life of the job; captured
CGPs as `TEXT`; `audit_log` and `worker_data_gaps` growing with declines;
`tasks_claimed_idx` unused; no index on `audit_log`'s filters or
`task_claims.claimed_at`. New tables from `36d6499` are bounded:
`leave_rack_staging` is emptied by every merge, `leave_generation_progress` is a
row per generation.

---

## 8. PLAN.md reconciliation

"Code wins" means PLAN.md (or TESTING.md) was updated to match the code. "Plan
wins" means the code was changed (and the document updated wherever its wording
also needed it).

| # | Subject | Code | PLAN.md said | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | The shape of an error for a body that does not parse | axum's plain text, with `400`/`415`/`422` | "Every failure is JSON with the same shape, whatever the status", and a closed list of codes | **Plan wins, code changed**; `payload_too_large` added to the list | B3 |
| K2 | A claim with no body | A bare `422` | "rejected with an error that names the fix rather than a bare `422`" | **Plan wins, code changed** | B3. TESTING.md's `A-WORKER-1` had specified it with no test behind it |
| K3 | Purge and delete vs a running merge | Opposite lock order; deadlock | The purge paragraph lists its locks and why each is taken in the order it is — and predates merges | **Code changed; PLAN.md's purge paragraph gained the merge lock** | R1 |
| K4 | Claim-time rack selection | A scan and sort of the generation (4.9 s) | "materializing it is what lets claim-time selection be a single indexed `ORDER BY occurrence_count` query"; 47 ms in the cost table | **Plan wins, code changed**; the table now carries both measurements and says why the first was wrong | P1 |
| K5 | When a leave job may be exported | As soon as no claim is open | "And only once its results have settled" | **Plan wins, code changed**; PLAN.md says that for a leave job settled includes merged | B1 |
| K6 | What startup reaps | Imports and exports | The same — and, of transitions, only the takeover timeout | **Code changed; PLAN.md updated** (transition paragraph, Health and startup) | B2. Not a contradiction; the plan's own reasoning for the other two applies |
| K7 | What an export costs the server | Compression inline on an async worker | "spawns a task … the bytes never pass through the backend and never touch the connection pool the cap exists to protect" — true of connections, silent about threads | **Code changed; PLAN.md's Exports section says where the compression runs and why** | P0 |
| K8 | The leave results feed | One statement sorting the job | "With the feed indexes and cursor pagination, a page costs a page" | **Plan wins, code changed**; Pagination describes the leave feed | Performance item 3 |
| K9 | Decline reasons | Five, `derived_mismatch` among them, and `missing` sent with two | Four; "`missing` is present only for the first" | **Code wins** | MAGPIE sends `derived_mismatch` with both digests; the handler has accepted it since the sixth audit |
| K10 | TESTING.md, contract fixtures | Eight exist, the opening-rack one among them | "Five fixtures exist … `assignment-opening-rack.json` — **missing**" | **Code wins** | Stale since the fifth audit added it |
| K11 | TESTING.md, `A-WORKER-8` | Five decline reasons | "the three known reasons" | **Code wins** | — |
| K12 | The size of a generation | 590 MB with the wider index | "258 MB of heap, 172 MB of indexes"; "a 430 MB relation" | **Code wins** (after P1) | Section 7 |

Counted: K9–K12 are code-wins (4). K1–K8 changed code (8). U1–U3 are
unresolved (3).

**Also updated in PLAN.md, not discrepancies:** the Worker Client paragraph about
`fake_worker.py` had lost a clause in an earlier edit and repeated its own file
name; it now says in one place that MAGPIE is the only production client and
what the script is for. The directory listing names `extract.rs`.

**Checked and found in agreement:** the schema block (byte-identical); the
scheduler section against `candidate_jobs`, `join_at_parity` and
`shutdown_or_idle`; the leave-generation claim steps and "What a merge costs"
against `next_step`, `stage_fold` and `merge_staged`; the Exports section against
`exports.rs` (apart from K5, K7); the Worker and Admin route tables against the
routers; the contract fixtures in both repositories (identical).

---

## 8a. The Python worker

Looked for, in every document, the compose file, the Dockerfile, the scripts, the
Terraform, CI, the frontend and the backend: any description or treatment of
`worker/fake_worker.py` as a production client. **None found; nothing to
correct** — the same result as the prior four records. The script's own
docstring opens "Test tooling only. The one production client is MAGPIE itself";
the compose service is behind a `fake-worker` profile marked end-to-end-suite
only; RUNBOOK says "Never use `worker/fake_worker.py` for this"; TESTING.md
confines it to tier 5. The one edit made is the PLAN.md paragraph noted above,
which was garbled rather than wrong.

---

## 9. MAGPIE `birdtest-contribute`

Checked directly rather than assumed:

- **It runs every job type against this branch.** Three end-to-end runs with a
  `portable_release` build of `d93dacaf` (section 13).
- **Its half of the contract.** `./bin/magpie_test contribute` passes. The three
  assignment fixtures are byte-identical in the two repositories. The other five
  fixtures — a claim, a decline, three shutdowns — are not pinned on MAGPIE's
  side, so they were traced by hand: `claim_task_over_http` writes exactly
  `magpie_version` and `unsupported_jobs`; `decline_over_http` writes
  `claim_token`, `reason`, `missing` with `role`/`name`/`expected`/`actual`, and
  the five reasons it sends are the five the server accepts; `print_shutdown`
  reads `message`, `required_magpie_version`, `download_url` and
  `required_tarball_dates`. All agree with the fixtures and with
  `routes/worker.rs`.
- **What the server runs.** `magpie builders` → `0.1.0`, `nehalem`, builders 1;
  `help convert` lists `dawg2wordmap`, `klvwmp2rit`, `rackequity2klv`;
  `createdata klv` built the generation-0 KLV in every run.
- **The argument trace**: section 2.
- **B3's status change** (`422` → `400` for a malformed claim) needs nothing from
  MAGPIE: it handles every non-`429` `4xx` identically.

**No change was made to `birdtest-contribute`**, and none on any other MAGPIE
branch. The checkout was on `birdtest-contribute` throughout, clean before and
after, at `d93dacaf` = `origin/birdtest-contribute`. `docker/Dockerfile`'s pin
(`0f6a4cb1`, the parent of `d93dacaf`) is reachable from the published branch and
builds the same converters.

One thing worth doing on the MAGPIE side that is not a gap: pinning the claim,
decline and shutdown shapes in `test/contribute_test.c` the way the assignments
are, so the hand trace above becomes a test. It needs the message builders in
`contribute.c` exposed for testing; nothing is wrong today, so it was left.

---

## 10. Deployment blockers

### Resolved in this audit

| # | Blocker | Resolution |
|---|---|---|
| D1 | **Starting an export takes the worker API down for its duration** | P0 / C1 |
| D2 | A leave-generation job cannot sustain more than one claim per ~5 s at full size; concurrent claimers are told `204` | P1 |
| D3 | A purge of a leave job fails with a `500` if a merge is running (and either may be the victim) | R1 |
| D4 | Every deploy that lands inside a generation transition idles that job for thirty minutes | B2 |
| D5 | A contributor on a pre-protocol MAGPIE is shown a framework error instead of the instruction PLAN.md promises | B3 |
| D6 | The staging design (`36d6499`) had never been run against a real MAGPIE | Section 13 |

### Not resolvable from here

- **Nothing since `main` is merged.** Three audits' fixes, the three decisions and
  this audit sit on unpushed branches; `main`'s nightly has failed every night
  this week on a bug fixed on them. Merging is the human's.
- After the merge, dispatch the nightly once (prior item).

### Checked, no change needed

Migrations run before bind; required secrets fail startup when absent; CSRF on
every cookie-backed mutation; the server refuses to start without a working
MAGPIE at or above its floor; the RDS parameter group's `wal_compression` and
`max_wal_size` are valid for Postgres 16; the MAGPIE pin is fetchable.

---

## 11. Left for human input

### U1 — a job that joins "at parity" inherits the ratio of whichever job is furthest behind, and a job can be far behind for reasons that are not debt

*Builds on the eighth audit's U1, decided (a) and implemented.*

`join_at_parity` sets a joining job's ratio to the **lowest** ratio among the
other jobs offering work. That is right when every job offering work is being
served. It is wrong when one of them is *not*: a job whose derived files are
still building (or failed), a job pinned to data the fleet does not have yet
(PLAN.md: "a job can now be created that nobody can run"), a job whose MAGPIE
floor is above most of the fleet, a leave job mid-transition. Such a job's ratio
stands still while the others' climb, the newcomer joins level with *it*, and
then — for every worker that cannot run the lagging job — the newcomer is first
in the candidate list until it has caught up with the jobs that were actually
running. That is the eighth audit's U1 ("the old job gets nothing for a month")
again, reachable whenever one lagging job is active.

**Demonstrated.** A veteran job (100,000 claims, 40%), a lagging job (20%, a
floor above the fleet's MAGPIE, no claims), and a newcomer activated at 40%: of
the next twelve claims the newcomer took **twelve** and the veteran none. (Run as
a throwaway test against this branch and removed; it is three `UPDATE`s on top of
`admin_api::a_newly_activated_job_joins_at_parity_instead_of_taking_everything`.)

Left alone because every repair is a choice about what "the fleet's current
position" means when the fleet is not one queue:

- **(a)** Parity with the jobs *being served*: add `jobs.last_claimed_at`
  (free to maintain — it rides the `UPDATE jobs` every claim already makes) and
  take the minimum over other offering jobs that issued a claim in the last N
  minutes, falling back to today's rule when there are none. Fixes the stalled
  and the unrunnable job. Does **not** fix a job that a *minority* of the fleet
  can run: it is served, recently, and still lags.
- **(b)** Parity with the job that issued the most recent claim (start-time fair
  queuing's literal rule: virtual time is the tag in service). Robust to a
  lagging job of either kind; noisy when two groups of workers serve disjoint
  jobs, where "most recent" is whichever group asked last.
- **(c)** Parity with the **maximum**. Right for every lagging job; wrong for the
  mirror case, a job racing *ahead* because it is the only one some workers can
  run — a newcomer level with it is starved until the rest catch up.
- **(d)** Leave it, and say so in PLAN.md next to the limitation it already
  states for the lagging job itself.

**Recommendation: (a) with N = the heartbeat timeout**, and the minority-fleet
case recorded as a known limit. It needs one column and the decision on N.

### U2 — claim-time selection now costs in proportion to what is staged

*Builds on the eighth audit's U2 as built, and on P1.*

Between merges, the racks of every staged result are held out of selection —
correctly — and they are exactly the racks at the head of the selection index.
So a claim builds a hash of them and skips that many index entries: about a
microsecond each. Measured: 9 ms with 20 results staged, 160–290 ms with 400
(200,000 racks). Staged results accumulate for up to thirty minutes, so that is
fleet size × task rate: twenty workers on two-minute tasks is ~300 results
(~150 ms per claim, inside the dispatch lock); a hundred workers is ~1,500
(~1 s), at which point the lock's two-second wait starts turning claims away.
Not reachable today; reachable by the fleet the project wants.

- **(a)** Merge by *volume* as well as by time: when a claim finds more than K
  results staged, ask for a merge (rate-limited like the tail merge). Bounds
  selection at K × `racks_per_task`. Costs WAL: a merge re-images most of the
  generation whatever it carries (eighth audit: ~1 GB each with the two
  settings), so a busy fleet merging every minute or two is tens of GB an hour —
  the cost staging was built to avoid.
- **(b)** A selection cursor: remember, per generation, the `(count, rack)` the
  last selection ended at, and start the next scan there; reset it at every
  merge and whenever a claim of the generation lapses. Selection becomes
  independent of what is staged. New state on the path that has needed the most
  race fixes; it has to be exact about lapsed claims, whose racks return to the
  head.
- **(c)** Leave it and watch: log selection time when it passes 100 ms.

**Recommendation: (c) now, (b) before a leave job is opened to a large fleet.**

### U3 — the selection index costs 160 MB a generation

Section 7. The wider index is what makes selection an index walk (P1) and the
feed a seek (performance item 3), and it is 180 MB a generation where its
predecessor was 22 MB.

- **(a)** Keep it (as this branch does). Deterministic dispatch order — ties go
  by rack — and a feed ordered furthest-from-target first.
- **(b)** Return to the narrow index: select `ORDER BY occurrence_count` alone
  and let ties fall in the order the index holds them; page the public feed by
  rack through the primary key. Saves ~160 MB a generation and a little WAL per
  merge. Gives up reproducible selection among tied racks (nothing depends on
  it; tests would need loosening) and changes what the public feed shows first.
- **(c)** Either, plus dropping closed generations' rows from the *selection*
  index with a partial index — not expressible today, since "current
  generation" is not a constant; it would need a maintained flag column, which
  is the same redesign PLAN.md already records as undecided under "What a merge
  costs".

**Recommendation: (a)** until storage is the constraint; (b) is a small change
if it becomes one.

### Smaller, noted rather than asked

- **A submission's validation now runs on the blocking pool while the claim and
  task rows are locked.** It ran inside the same locks before, on an async
  worker. Decoding *before* taking the locks would shorten them, but needs the
  job type, which is read under them; left.
- **`seed_generation`'s COPY is not aborted explicitly** if a chunk fails to
  build; it never was (`copy.send` could already fail). The transaction rolls
  back either way.
- The eighth audit's three small notes stand.

### To do after merge (not decisions)

1. Merge; then dispatch the nightly workflow once.
2. Consider pinning the claim, decline and shutdown shapes in MAGPIE's tests
   (section 9).

---

## 12. Verification

- Backend: `cargo clippy --locked --all-targets -- -D warnings` clean; `cargo test
  --locked` against Postgres 16: **187 tests** (96 unit and contract, 91
  integration), all passing (178 before this audit). The five `magpie_smoke`
  tests are `#[ignore]` by design.
- Frontend: `npm run check` — 0 errors, 0 warnings. Unchanged by this audit.
- MAGPIE at `d93dacaf`: `./bin/magpie_test contribute` passes; unchanged.
- R1's test fails against the old purge (the purge is the deadlock's victim,
  nothing deleted) and passes against the new. U1's probe was run and removed.
- P0, P1 and the feed were measured before and after on the same data; the
  "before" plans for P1 were taken inside a rolled-back transaction that put the
  old index back.
- The schema block in PLAN.md is byte-identical to the migration.

---

## 13. End-to-end runs

`scripts/e2e_magpie.py`, **unmodified**, against a native stack: a throwaway
`postgres:16` (2 CPUs, 1 GB) and MinIO (both stock images, nothing built), the
backend and `build-derived` run from this branch's debug build with
`MAGPIE_BIN` pointing at the checkout's `portable_release` MAGPIE, and a PATH
shim that turns the script's three `docker compose` calls (psql, the builder,
the backend's log) into their native equivalents. No Docker image was built —
the eighth audit skipped its rerun because those builds had exhausted this
machine's memory, and a standing instruction says to ask first.

**Every job type passed, three times** — after R1, B1, B2 and P1; again with
`ApiJson` on every worker request; and a third time on the build this record
describes, with submissions decoded on the blocking pool:

| Job | Result |
|---|---|
| `games` | 2 accepted claims |
| `game_pairs` | 2 accepted claims, pentanomial stored |
| `opening_rack`, static, with a wordmap | derived files built natively; the worker's own wordmap agreed; 2 accepted claims |
| `opening_rack`, simming | 2 accepted claims, simulated statistics stored |
| `leave_generation` | 2 accepted claims; results **staged**, live counters moved, `merge-progress` folded exactly what was staged, nothing left staged, nothing written into MAGPIE's data directory |

The backend log held **no** `ERROR` or `WARN` line in any of the three. This is the
first time the staging design has met a real worker.

Then, against the same live stack and its real 3,199,724-rack generation:

- **P1.** Selection `EXPLAIN ANALYZE`: 0.2 ms; with the old index and query put
  back inside a rolled-back transaction, 4,902 / 4,886 / 4,814 ms.
- **The feed.** First page over HTTP: 4.0 s on the old code, 7 ms on the new,
  pages disjoint.
- **B1 and P0 together.** One more real MAGPIE task (staged, not merged); the job
  force-completed; an export started. Old build: the server stopped answering
  (`/health` past 10 s, the export poll past 30 s, thread dump as in section 6).
  New build: `ready` in 34 s, 3,199,724 rows, 10.3 MB; `/health` polled 136 times
  during it, 2.1 ms median, 7.8 ms worst; the download's SHA-256 and length match
  the row; 1,164 lines with occurrences, the database's count exactly; nothing
  left staged.
- **C4.** `/health` while a universe was seeded: 1,283 ms worst before, 3.6 ms
  after (64 s, 317 polls).
- **Purge** of the full-size job: 3.0 s, `200`, `/health` 3.7 ms worst — R1's
  lock in the path.

Both containers and the scratch databases were removed afterwards.

---

## 14. Tests added

| Test | What it pins |
|---|---|
| `leave_gen::a_purge_waits_for_a_running_merge_instead_of_deadlocking_with_it` | R1; fails against the old code |
| `leave_gen::a_completed_leave_jobs_corpus_includes_what_was_still_staged` | B1 |
| `leave_gen::a_restart_hands_an_open_transition_to_the_next_claim` | B2 |
| `leave_gen::the_leave_results_feed_pages_through_every_generation_in_order` | Performance item 3: the per-generation read still tiles the job, in the database's order |
| `exports::tests::the_parts_are_one_gzip_stream_of_what_was_pushed` | C1: parts and tail are one gzip stream of what was pushed; digest and length describe it; no short part |
| `extract::tests::a_body_that_does_not_parse_is_an_api_error`, `…::a_body_over_the_routes_limit_is_a_413_in_the_same_shape`, `…::small_and_large_bodies_parse_alike` | B3, C5 |
| `worker_api::a_claim_without_a_usable_body_is_told_what_to_send` | B3 / TESTING.md `A-WORKER-1` |
