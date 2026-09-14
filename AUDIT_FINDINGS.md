# birdtest audit — findings

Branch: `audit/birdtest-2026-09-14`, off `main` at `449cafb`.
MAGPIE changes: `birdtest-contribute` only, commits `22c4c25f` and `cb390035`, pushed.
Date: 2026-09-14.

**This is the fourth audit.** It builds on three earlier ones, all merged into
`main`: `audit/birdtest-2026-09-11`, `audit/birdtest-2026-09-13`, and
`audit/birdtest-2026-09-13-pass2`. The last of those left the
`AUDIT_FINDINGS.md` this file replaces, with all six of its open decisions
implemented. Where an entry here revisits one of those findings, it says so and
names it (for example "prior R4"). Everything else is new.

This file is the authoritative record of every code-versus-`PLAN.md` decision
made in this audit, plus the bugs, races, MAGPIE argument gaps, critical-path
changes and performance findings behind them. Each entry says what the code
did, what the plan said, what was decided and why, so a reviewer can check the
decision without reading the diff.

**Counts: 11 code-wins (PLAN.md updated to match the code), 11 plan-wins (code
changed), and 9 items left for human input (section 9).**

The default bias is that the code wins and `PLAN.md` is brought level with it,
because the plan is a summary. The code was changed only where it was plainly
wrong, or where the plan described the behaviour the rest of the system needs.

---

## How this audit was run

- Read `PLAN.md` in full, the previous `AUDIT_FINDINGS.md` in full, and then the
  claim, submit, job-type, stats, SSE, admin, public, ratings, export and auth
  code, the migration, the test harness, CI, Docker, compose and Terraform.
- On MAGPIE's `birdtest-contribute`, read `src/impl/contribute.c` in full, and
  in `src/impl/config.c` the contribute executors, the per-player reset, the
  argument table, `config_create`'s defaults, and the arg builders the
  executors reach (`config_fill_game_args`, `config_fill_sim_args`,
  `config_fill_autoplay_args`, `impl_move_gen`, `impl_sim`,
  `config_load_lexicon_dependent_data`). Also read the parts of `autoplay.c`,
  `sim_results.c`, `autoplay_results.c` and `move_gen.c` needed to decide
  whether a setting changes results.
- Checked prior audit items for anything reopened since. All six section-8
  decisions are still in the code as described.
- Backend: `cargo clippy --locked --all-targets -D warnings` and `cargo test
  --locked` against a real Postgres 16, before and after every group of
  changes. **130 tests before, 136 after the audit's own fixes, 141 after the section 9a
  decisions**, all passing, clippy clean. `svelte-check` clean after the form changes.
- MAGPIE: `make magpie_test` (`-Werror`, address/undefined/leak sanitizers),
  then `./bin/magpie_test contribute` and `./bin/magpie_test config`, all
  passing. The release `magpie` binary builds and reports `0.2.0`.
- `PLAN.md`'s schema block was diffed mechanically against
  `backend/migrations/0001_initial.sql` after every schema change. They are
  identical.
- End to end: `scripts/e2e_magpie.py` against an isolated compose stack with the
  release MAGPIE build. The outcome is recorded in section 7.

---

## 1. MAGPIE arguments that change a task's outcome

The brief: every MAGPIE setting that can change a result must be set by the
dispatched task, and never left to whatever the contributor's MAGPIE already
has. **It was not.** Seven gaps were found and fixed. Two of them (M1, M2)
meant opening-rack simulations ignored the job's settings entirely.

### How MAGPIE decides a setting's value in `contribute`

MAGPIE's `Config` holds two kinds of setting.

- **Per-player** (`p1_*`, `p2_*`): plies, plays, iterations, stop condition,
  time limit, BAI threshold and sampling rule, inference and utility weights,
  recorder and sort. `config_contribute_reset_player_settings` resets these
  before each request is applied (a prior audit added that), so a null in a
  request meant MAGPIE's default.
- **Run-wide**: bingo bonus, movegen margin, simulation cutoff,
  multi-threading mode, small plays, seed, and a second copy of every simulation
  setting (`plies`, `num_plays`, `max_iterations`, …). **Nothing reset these.**
  They kept whatever the contributor's `settings.txt`, a command run before
  `contribute`, or the previous task had set.

Autoplay (games, pairs, leave generation) reads the per-player copies. The CLI
entry points the opening-rack executor uses, `impl_move_gen` and `impl_sim`,
read the **run-wide** copies. That split is the root of M1.

### Enumeration

Every MAGPIE setting in `config.c`'s argument table that can influence a result,
and how each job type sets it. **Before** is how it stood on `birdtest-contribute`
when this audit started; **after** is how it stands now.

| Setting (flag) | Changes results? | games / pairs | opening rack | leave gen | Before | After |
|---|---|---|---|---|---|---|
| Lexicon (`-l1/-l2`, `-lex`) | Yes | per player, from request | player, from request | request | Set | Set |
| Leaves (`-k1/-k2`, `-leaves`) | Yes | per player | p1 from request; **p2 left over** | fetched KLV, both | **Gap (M3)** | Set: p2 gets the player's leaves |
| Letter distribution (`-ld`) | Yes | request | request | request | Set | Set |
| Board layout (`-bdn`) | Yes | request | request | request | Set | Set |
| Variant (`-var`) | Yes | request | request | request | Set | Set |
| Bingo bonus (`-bb`) | Yes: part of every play's score | **never set** | **never set** | **never set** | **Gap (M4)** | Reset to MAGPIE's default |
| Challenge bonus (`-cb`) | No: game-history challenges only; autoplay has none | — | — | — | n/a | n/a |
| Recorder (`-r1/-r2`) | Yes | per player | p1; **p2 left over** | reset to `best` | **Gap (M3)** | Both seats from request |
| Sort (`-s1/-s2`) | Yes | per player | p1; **p2 left over** | reset | **Gap (M3)** | Both seats from request |
| Plies (`-pl1/-pl2`, `-plies`) | Yes | per player | **analysis read run-wide `-plies`** | reset (static) | **Gap (M1)** | Player's, copied to run-wide |
| Candidate plays (`-np1/-np2`, `-numplays`) | Yes: candidates simmed, and move-list capacity | per player | **run-wide** | reset | **Gap (M1)** | Player's |
| Max iterations (`-i1/-i2`, `-iterations`) | Yes | per player | **run-wide** | reset | **Gap (M1)** | Player's |
| Min play iterations (`-mi1/-mi2`) | Yes | per player | **run-wide** | reset | **Gap (M1)** | Player's |
| Stop condition (`-sc1/-sc2`, `-scondition`) | Yes | per player | **run-wide** | reset | **Gap (M1)** | Player's |
| Time limit (`-tl1/-tl2`, `-tlim`) | Yes, and hardware-dependent | per player | **run-wide** | reset | **Gap (M1)** | Player's. See U3 |
| BAI threshold, sampling rule (`-th*`, `-sa*`) | Yes | per player | **run-wide** | reset | **Gap (M1)** | Player's |
| Inference in sim (`-si1/-si2`, `-sinfer`) | Yes | per player | **run-wide, against the contributor's loaded game history** | reset | **Gap (M1)** | Off: an opening rack has no previous play to infer from |
| Inference margin (`-im*`) | Yes | per player | **run-wide** | reset | **Gap (M1)** | Player's |
| Utility weights (`-uwin*`, `-uspread*`, `-uspreadscale*`) | Yes | per player | **run-wide** | reset | **Gap (M1)** | Player's |
| Sim cutoff (`-cutoff`) | Yes: when two simmed plays count as equivalent | **never set** | **never set** | n/a | **Gap (M4)** | Reset |
| Movegen margin (`-mmargin`) | Yes: which plays an `equity` recorder keeps | **player 1's only** | **never applied, never reset** | **never reset** | **Gap (M4, M5)** | Reset, then applied from whichever player states it |
| Win% model (`-winpct`) | Yes, for simmers | **player 1's only** | player's | n/a | **Gap (M5)** | Whichever player states one |
| Multi-threading mode (`-mtmode`) | Yes, for simmers: threads per simulation | **never set** | n/a | n/a | **Gap (M4)** | Reset to per-game parallelism |
| Small plays (`-sp`, `-numsmallplays`) | Yes: turns the move list into the endgame's small-move list | n/a | **never set** | n/a | **Gap (M4)** | Reset off |
| Seed (`-seed`) | Yes | request | **process state** | **process state** | **Gap (M2, M6)** | Games: request. Racks: hash of the rack. Leave gen: request (new field) |
| Game pairs (`-gp`) | Yes | request | n/a | n/a | Set | Set |
| Play chooser (`-pc1/-pc2`) | Yes | reset off | reset off | reset off | Set | Set |
| Overtime (`-otpenalty`, `-otperiod`) | Only with the play chooser, which is off | — | — | — | n/a | n/a |
| Endgame, PEG (`-eplies`, `-etlim`, `-etopk`, `-ttfraction`, `-peg*`) | Not reached: autoplay uses them only through the play chooser (off), and movegen/sim never | — | — | — | n/a | n/a |
| Wordmap (`-w1/-w2`, `-wmp`) | No: exact accelerator, same moves | per player | player | request | Set explicitly | Set explicitly |
| Rack info table (`-rit*`, `-ritmmap`) | No: exact precomputed leave values (checked in `move_gen.c`'s fast path) | when stated | when stated | left | n/a | n/a |
| Threads (`threads` in `contribute.txt`) | For simmers only: sampling order | contributor's | contributor's | contributor's | n/a | Flagged, U3 |
| Heat map (`-useheatmap`) | No: bookkeeping | — | — | — | n/a | Reset anyway |
| `maxnumdplays`, `shplies` | Not for stored results: sim display sorting covers every play, and the writer caps explicitly. **But** with capture on, autoplay raises a simmer's `num_plays` to `maxnumdplays` | set from player 1 | writer cap from request | n/a | n/a | Flagged, U2 |
| Leave-gen run shape (`leavegen_max_games`, force-draw start, rack target, write files) | Yes | — | — | set per task | Set | Set |
| Output formatting (`-hr`, `-pfrequency`, board display) | No | — | — | — | n/a | `print_interval` reset: cosmetic, but it printed sim progress per rack |
| Data paths (`-path`) | Chooses files, but every file is digest-verified and the task is declined on mismatch | — | — | — | Covered | Covered |

### M1 — opening-rack analysis ran with the contributor's simulation settings, not the job's

*Plan wins: **code changed** (MAGPIE).*

- **What the code did.** `config_contribute_opening_rack` applied the job's
  player to player 1's per-player settings, like every executor does, and then
  analysed each rack through `impl_move_gen` and `impl_sim`. Those are the
  CLI's `generate` and `simulate` entry points, and `config_fill_sim_args` and
  `impl_move_gen_override_record_type` read **run-wide** `config->plies`,
  `num_plays`, `max_iterations`, `stop_cond_pct`, `time_limit_seconds`, and so
  on. Nothing in the executor wrote those. The job's `num_plies`, `num_plays`,
  `max_iterations`, `stopping_pct`, `time_limit_secs`, threshold, sampling rule
  and utility weights only decided whether the rack was simmed at all (the
  executor checked `p1_sim_plies > 0`). The simulation then ran on MAGPIE's
  compile-time run-wide defaults (plies 5, ~10¹² iterations, a 60-second time
  limit) or on whatever the contributor's `settings.txt` said.
- **Inference had the same flaw, with a second one on top.**
  `sim_with_inference` was run-wide and defaulted on. `impl_sim` then inferred
  from `config->game_history`, which the executor never resets. For a
  contributor who had loaded a GCG earlier in the same session, every rack's
  simulation inferred an opponent rack from an unrelated game.
- **Why it matters.** Two contributors analysing the same rack under the same
  job ran different simulations. And the job's own settings did nothing, so a
  job asking for 1,000 iterations ran until the 60-second time limit on every
  rack. The prior audit measured a simming opening-rack job at "~57 s per rack"
  and called it inherent (prior performance item 11). That figure is what M1
  predicts, and section 7 records the time after the fix.
- **What PLAN.md said.** "for each rack in the batch, load the CGP, apply the
  single player config, run move generation (and simulation when the player's
  `num_plies` is above 0)" — the player's settings, in other words.
- **Fix.** `config_contribute_use_player_settings_for_analysis` copies the
  player's simulation settings into the run-wide ones before the batch is
  analysed, and turns inference off, since an opening rack has no previous play.
  Exposed through `config.h` and unit-tested
  (`test_opening_rack_analysis_uses_the_players_settings`, which starts from a
  config with `-plies 5 -numplays 7 -iterations 99` and asserts the fixture
  player's 4 / 10 / 1000 win).

### M2 — an opening rack's simulation seed came from the process

*Neither side covered it: **code changed** (MAGPIE).*

- `config_fill_sim_args` seeds the simulation from `config->seed`, which no
  opening-rack request sets. It was the process start time, a `-seed` in
  `settings.txt`, or the seed of the last games task the worker ran.
- **Fix.** `contribute_rack_seed` gives each rack a 64-bit FNV-1a hash of the
  rack string. That value is the same on every machine, so a single-threaded
  analysis of a rack reproduces, and it adds no wire field.
- Simulation with more than one thread is still non-deterministic. That is
  inherent to parallel sampling and recorded as U3.

### M3 — in opening-rack analysis, player 2's leaves and settings were left over from an earlier task

*Plan wins: **code changed** (MAGPIE).*

- **What the code did.** The executor loaded the lexicon with a NULL player-2
  lexicon and NULL player-2 leaves, and applied the request only to player 1.
  `config_load_lexicon_dependent_data` treats a NULL name as "keep what is
  loaded". So player 2 kept whatever leaves it had: another job's, or the
  `<lexicon>_birdtest_previous` KLV an earlier leave-generation task had
  fetched. Neither is a file this task's `expected_data` verified. Player 2's
  recorder and sort were also left over. In a simulation, player 2 is the
  opponent whose replies are played out.
- **Fix.** Player 2 gets the player's leaves, and the request is applied to both
  seats.

### M4 — run-wide settings no request states were never reset

*Plan wins: **code changed** (MAGPIE).*

- **Settings affected.** Bingo bonus (part of every play's score), sim cutoff,
  movegen margin (reset only by the games executor), multi-threading mode (which
  decides whether a simmer's simulations get one thread or all of them), and
  small plays (which turn the move list into the endgame's small-move list).
- **What PLAN.md said.** "A setting a request leaves null takes MAGPIE's
  compile-time default — never the value an earlier task or the contributor's
  `settings.txt` left behind." That was true of per-player settings only.
- **Fix.** `config_contribute_reset_shared_settings`, called by all three
  executors, resets them to `config_create`'s values. The cutoff's literal
  became `CONFIG_DEFAULT_USER_CUTOFF` so the two cannot drift. Exposed and
  unit-tested (`test_shared_settings_do_not_leak_between_tasks`, starting from
  `-bb 35 -sp true`).

### M5 — the win% model and movegen margin were read from player 1 only

*Plan wins: **code changed** (MAGPIE and birdtest). Paired with B1.*

- MAGPIE read `win_pct_model` from player 1's object. A static player has none,
  so for a static player 1 against a simming player 2 no model was named.
  `config_load_win_pcts` then kept whatever model an earlier task had left
  loaded, or loaded the default name. A job like that could only reach a worker
  once B1 was fixed, because birdtest refused to create one.
- **Fix.** `contribute_stated_by_either` reads each shared option from whichever
  player states it, preferring player 1. birdtest refuses a job whose players
  both state one and disagree.

### M6 — a leave-generation task had no seed

*Neither side covered it: **code changed** (both repositories, wire contract).*

- **What the code did.** `LeaveRequest` carried no seed, and
  `config_contribute_leave_gen` never set `config->seed`. Every game of a leave
  task was seeded from process state. Consecutive leave tasks on one worker (with
  no games task in between) drew the same per-game seed sequence over different
  forced racks, and a reissued task did not replay what it was first given.
- **Fix, birdtest.** `leave_requests.seed BIGINT NOT NULL` (in the migration and
  PLAN.md's schema block). A seed is drawn with `rand::random()` when the task is
  created and stored, so a reissue replays it. It is sent as a decimal string, as
  games' seed is.
- **Fix, MAGPIE.** The executor requires `seed` (`json_get_uint64_string`) and
  sets `config->seed` from it. The contract fixture carries it, and MAGPIE's
  fixture key test requires it.
- **Test.** `leave_gen::a_leave_task_carries_its_seed_and_a_reissue_replays_it`.

### M7 — the version floor did not exclude builds with the gaps above

*Neither: **code changed** (both repositories).*

- An unfixed `birdtest-contribute` build reported `0.1.0`, which met the server
  floor. It would keep contributing results that depend on its own settings, and
  it ignores the new leave-generation seed field. The floor exists for exactly
  this case, and a previous audit set the precedent when it moved the floor from
  `0.0.0` to `0.1.0`.
- **Fix.** MAGPIE reports `0.2.0`. birdtest's `MIN_MAGPIE_VERSION` default, the
  `min_magpie_*` column defaults, the Terraform variable, compose, both env
  examples, README, TESTING.md, `scripts/dev.py`, the contract fixtures, the test
  harness and PLAN.md all move to `0.2.0`, with the reason stated where the old
  wording said "first version that implements the protocol correctly".
- **Consequence:** see U8. CI's MAGPIE jobs check out `birdtest-contribute` from
  GitHub, and the branch there does not have these commits yet.

---

## 2. Bugs

### B1 — a games job could not pit a static player against a simmer

*Plan wins: **code changed**.*

- **What the code did.** `validate_shared_player_options` required
  `p1.winpct_id == p2.winpct_id`. Player-config creation refuses a `winpct_id` on
  a static player and requires one on a simmer, so a static player's is always
  NULL and a simmer's never is. Every static-versus-simmer job was refused as
  "player configs disagree on the win% model".
- **What PLAN.md said.** "Run games — autoplay using any player configuration;
  supports pure static players (no simulation), simming players, or any mix."
- **Fix.** Refuse only when both players state a model and the models differ.
  The movegen margin is still compared strictly. The MAGPIE half is M5.
- **Test.** `admin_api::a_games_job_may_pit_a_static_player_against_a_simmer`:
  static against simmer in either seat is created, and two simmers on different
  models are still refused.

### B2 — redundant leave-generation results were each folded into the generation

*Plan wins: **code changed**.*

- **What the code did.** `LeaveGenHandler::insert_record` added every accepted
  submission's occurrences to `leave_rack_progress`. At `redundancy` N, a task's
  racks were counted N times, so a generation reached its occurrence target on
  1/N of the coverage it names, and closed early.
- **What PLAN.md said.** "Every aggregate that treats results as observations —
  SPRT, progress counts, the job list and rating evidence — reads **one result
  per task**, the first accepted." The list did not name leave generation, but
  the principle is stated without exceptions. With M6, redundant claims now
  replay the same seed, so the copies are the same games.
- **Fix.** `insert_record` is split. `credit_claim` writes `leave_records`;
  `fold_into_generation` updates progress, and only the first accepted result
  reaches it. PLAN.md names leave generation.
- **Test.** `leave_gen::only_the_first_result_for_a_leave_task_is_folded`.
- **Trade-off.** Recorded as U4: with more than one MAGPIE thread, leave
  generation is not deterministic, so a discarded copy is a different sample, not
  a duplicate.

### B3 — an export of a completed job could be short, and was then served forever

*Plan wins on intent: **code changed**.*

- **What the code did.** `exports::start` required `status = 'completed'` and
  nothing else. A job is marked completed the moment SPRT crosses (or an admin
  forces it). But the claims already out keep being played and accepted: the
  submit path checks the claim, not the job's status, and PLAN.md's SPRT section
  relies on exactly that. An export built in that window missed those results,
  and `job_results_stream` redirected every later download of the job to it.
- **What PLAN.md said.** "a completed job's results are immutable, so an export
  is built once and reused". That is true only once in-flight claims have
  settled.
- **Fix.** `start` refuses (`409`) while any claim of the job is open. No claim
  can be issued against a completed job, so once none is open the results really
  are fixed. PLAN.md's export section and admin table are updated.
- **Test.** `admin_api::a_completed_job_is_not_exported_until_its_claims_have_landed`.

### B4 — stale or wrong statements in PLAN.md

Covered in the reconciliation table (section 6, K11–K18). None of them is a
code bug.

---

## 3. Race conditions

### R1 — purge and delete against an in-flight submission: deadlock and counter drift

*Plan wins on intent: **code changed**. Revisits prior R2 and prior R4.*

- **What the code did.** A submission locks, in order: its claim (`FOR
  UPDATE`), its task, the job's row (`count_first_result`, `tasks_completed`),
  then its identity's row. `purge_job` took the dispatch lock, then **the job's
  row**, counted contributions (`release_contributions`), then deleted claims.
  `delete_job` took no lock at all before counting; its `DELETE FROM jobs`
  locked the job row and cascaded into tasks and claims.
- **Race 1 — deadlock.** A submission holds claim C and task T, and reaches
  `UPDATE jobs`. The purge holds the job row and reaches `DELETE task_claims`,
  which needs C. Each waits on the other. Postgres fails one after
  `deadlock_timeout`. If that is the submission, the worker's batch is lost to a
  500 and retried into `accepted: false`; if it is the purge, the admin gets a
  500.
- **Race 2 — permanent leaderboard drift.** A submission commits after the
  purge's `release_contributions` read (so its claim was not yet `completed` in
  that snapshot) but before the purge's delete. Its identity was credited, the
  claim was destroyed, and nothing ever handed the credit back.
- **Prior R4 said** "Lock ordering is consistent everywhere". It checked submit,
  reclaim, decline and claim, but not purge or delete.
- **Fix.** `lock_open_claims` locks the job's open claims (`FOR UPDATE OF c`,
  `state = 'claimed'`) before the job row, in both purge and delete. Delete also
  now takes the dispatch lock and the job row first. Any submission that got its
  claim lock first commits before the purge counts, and later ones wait and then
  find their claim gone.
- **Test.** `admin_api::a_purge_waits_for_a_submission_in_flight_before_counting_contributions`.
  A transaction plays a submission (claim locked, marked completed, identity
  credited, not committed) while a purge runs. The test commits it after 300 ms
  and asserts the credit was handed back. Against the old code the counter
  stays at 1.

### R2 — re-dispatch of an existing task ran outside the dispatch lock

*Plan wins: **code changed**. Revisits prior R2.*

- **What the code did.** `registry::acquire` called `next_available` (`FOR
  UPDATE SKIP LOCKED` on a task) before any lock, and only generation paths
  took `try_lock_job_dispatch`. A re-dispatch claim holding task T then reached
  `UPDATE jobs` while a purge held the job row and waited on T in `DELETE FROM
  tasks`: the same deadlock shape as R1.
- **What PLAN.md said.** "Acquiring a task takes the job's dispatch lock first."
- **Fix.** `acquire` takes the dispatch lock (bounded, 2 s) before anything,
  for every job type. The four per-type calls are gone, and leave generation's
  `lock_claim_decisions` is now only a test helper's name for the same lock.
- **Cost.** Re-dispatch claims now serialize per job for their whole
  transaction rather than only at `UPDATE jobs`. They already serialized there,
  and generation claims already held this lock. Recorded in section 4.

### R3 — seeding a generation's universe inside a claim request

*Plan wins on intent: **code changed**. Lifecycle, listed here because it is a
cancellation race.*

- **What the code did.** The first claim to find generation N ≥ 2 current
  called `ensure_universe` inline: a `COPY` of 3.2 M rows, measured at 56–66 s
  on a 12-core dev box, inside the claim transaction. If the client gives up,
  axum drops the handler future and the transaction rolls back (PLAN.md states
  this for the transition). MAGPIE's request timeout is 120 s. On a database
  slower than that at this write — `db.t4g.micro` is the default instance class —
  every claim started the seeding and every one was cancelled, and the job never
  left generation N.
- **What PLAN.md said.** "seeded when its generation *opens* instead — by the
  first claim that finds it current … and off the critical path."
- **Fix.** `generate_leave_gen` checks `leave_gen::universe_exists` (one index
  probe) and returns `Acquired::NeedsUniverse`. The scheduler rolls back and
  spawns `seed_leave_universe`, which takes the dispatch lock with
  `pg_try_advisory_xact_lock`, seeds and commits. Claims meanwhile wait at most
  their 2 s bound and move on. An interrupted seeding rolls back whole, and the
  next claim starts it again. The try-lock means a burst of claims cannot pile up
  seeders that each hold a pool connection while they wait.
- **Test.** `leave_gen::the_next_generations_universe_is_seeded_off_the_claim_path`
  (renamed and rewritten from the claim-seeds-it version): the claim answers
  `204` at once, the universe appears in full, and the next claim gets
  generation-2 work.

### R4 — completion versus export

This is B3. It is a race between a job's status flipping and its last results
landing.

### R5 — checked and found sound

- **Submit vs reclaim vs decline** — unchanged since prior R4, re-verified:
  each re-checks `state = 'claimed'` under the claim lock.
- **Reading `accepted_count` as "first result".** Every accepted result
  increments it in the transaction that stores the result, and that transaction
  holds the task row lock the reader takes. Two concurrent submissions for one
  task cannot both read zero. (This replaces a row count; see C2.)
- **The seeding task vs claims.** Claims can read `leave_rack_progress` only
  under the dispatch lock, and the seeder holds that lock until its `COPY`
  commits, so no claim sees a partial universe. A seeder that fails to take the
  lock exits, and a later claim retries.
- **Purge vs the seeding task.** Purge takes the dispatch lock blocking, so it
  waits for a seeding to commit. The seeding's `EXISTS` check runs under the
  lock, so it does not re-seed a purge-seeded generation 1.
- **Delete-user vs submissions.** Delete-user locks only the user row. A
  submission reaches the user row last, after claim, task and job, so there is
  no cycle.
- **SSE spacing.** The loop keeps its `pushes` entry through the pause, so
  submissions arriving meanwhile coalesce into the next round rather than
  starting a second loop.

---

## 4. Critical path — what moved, what stayed

The critical path is getting a worker its next task and getting its result
accepted.

| # | Change | Was | Now | Why safe |
|---|---|---|---|---|
| C1 | Leave-generation universe seeding | Inside the claim request and transaction | On its own task; the claim answers at once | Nothing in the request needs it, and claims cannot read the universe until the seeding commits (R3) |
| C2 | "First accepted result for this task" | `COUNT(DISTINCT task_claim_id)` over the task's stored records — up to 10,000 rows for an opening-rack batch, read inside the task and job locks | One column, `accepted_count`, read by the lock statement the submission already ran | Equivalent under the task lock (R5). Drift in `accepted_count` would already mis-dispatch the task, and RUNBOOK §2.3 repairs it |
| C3 | Live stats rebuilds | Back to back while any submission asked for another round | At least 1 s apart | Display only. Spaced so a watched job does not hold a pool connection continuously on the aggregates, a pool the claim and submit paths share |
| C4 | Finish-check in-flight query | Ran on 7 of every 8 leave-generation submissions, for a check that always returns false | Skipped for leave generation | Leave generation completes in its transition |
| C5 | Re-dispatch | Lock-free until `UPDATE jobs` | Under the dispatch lock from the start | **A cost paid for correctness** (R2). Claims per job already serialized on the job row, and generation claims already held this lock |

**Left on the critical path deliberately, flagged as U1:** the `audit_log`
insert on every claim and every submission, and the identity contribution
counters in the submit transaction. Both are observational, and both are inside
the transaction on purpose: the audit row is atomic with the action it records
(PLAN.md, "Audit actions"), and the counters were a decided design (prior U2).
Moving either off the path trades durability for latency and needs a decision.

**Also left, not worth moving:** `expected_data` (one query per claim), and
parsing the pinned letter distribution per claim. Both are cheap, and the claim
needs both.

---

## 5. Performance — most severe first

1. **A leave-generation job could stall permanently at generation 2.**
   *Fixed (R3, C1).* Seeding took 56–66 s on a 12-core dev box and ran inside a
   request that MAGPIE abandons at 120 s. On a slower database (the default
   `db.t4g.micro` is 2 vCPUs with burst credits) the seeding never completed,
   and every claim restarted it.
   *Impact avoided: a job stuck forever, plus a `COPY` of 3.2 M rows repeated
   every two minutes.*
2. **Simming opening-rack jobs ran each rack to MAGPIE's 60-second time limit
   regardless of the job's iteration budget.** *Fixed (M1).* The job's
   `max_iterations` never reached the simulation, and the run-wide default is
   ~10¹², so only the time limit stopped it. A 500-rack batch was about 8 hours
   of one worker's time, when a budgeted job could take minutes. The prior audit
   measured ~57 s per rack. Section 7 records the time after the fix.
3. **A pool's public rating history grew without bound.** *Fixed.* A pool with an
   active job is refit every two minutes, 720 runs a day, each with a row per
   member, and `/api/rating-pools/:id/history` returned all of them. After a
   month of one active job at ten members, that is over 200,000 points per page
   view on an unauthenticated route. It is now thinned to ≤ 500 runs spaced
   evenly over the pool's life, with the first and newest kept, using one window
   scan of the pool's runs (small rows, indexed by `(pool_id, computed_at)`).
   *Test:* `admin_api::a_long_rating_history_is_thinned_but_keeps_its_ends`.
4. **Purge and delete could deadlock against a submission.** *Fixed (R1).* Each
   occurrence costs a `deadlock_timeout` (1 s) plus a failed request on one side.
   On a busy job, a purge was likely to hit one.
5. **A watched busy job rebuilt its live stats back to back.** *Fixed (C3).*
   The payload is several aggregates over the job's history: `worker_contributions`
   136 ms at 44,000 claims, `game_pair_stats` 54 ms, `leave_gen_stats` 210 ms
   (PLAN.md's measured table), rebuilt continuously on one of twenty pool
   connections while submissions kept arriving. Now at most one per second.
6. **Opening-rack submissions counted up to 10,000 stored rows to learn one
   bit.** *Fixed (C2).* Roughly 10–50 ms per submission inside the task and job
   locks.
7. **Flagged, not fixed: display reads that grow with a job's history.**
   `jobstats::worker_contributions` and `estimate_eta` scan the job's claims on
   every detail view and every push. The job list's `stalled` flag runs two
   `NOT EXISTS` subqueries over the job's claims per active job with recent
   data gaps. Seconds, not minutes, at plausible volumes, and `compute`'s
   slow-query log line is the designed trigger for acting. See U6.
8. **Flagged, not fixed: minute-long rack-universe writes in admin requests.**
   Job creation and purge both seed generation 1 inline. Purge does so holding
   the job row and every open claim, which holds that job's submissions and
   pool connections for the duration. See U9.
9. **Flagged: unbounded append-only tables.** `audit_log` gains two rows per
   task, and `rating_runs` 720 per active pool per day. No retention exists. See
   U7.

---

## 6. PLAN.md reconciliation

"Code wins" means PLAN.md was updated to match the code. "Plan wins" means the
code was changed (PLAN.md was then updated wherever its wording also needed
it).

| # | Subject | Code | PLAN.md | Decision | Reasoning |
|---|---|---|---|---|---|
| K1 | Leave-gen universe seeding | Inline in the claim request | "by the first claim … off the critical path" | **Plan wins** | R3. The code was on the critical path and cancellable, and "off the critical path" is the stated intent. Claim step 2, the long-operations paragraph, the aggregation section, the claim loop and the dispatch-lock paragraph are updated |
| K2 | Static vs simmer games | Refused | "any mix" | **Plan wins** | B1. The refusal came from a comparison that could never succeed; nothing in the design wants it. Admin semantics updated |
| K3 | Redundant leave results | Folded per claim | "one result per task" for every aggregate | **Plan wins** | B2. The principle is stated without exceptions, and M6 makes redundant copies identical. Bullet now names leave generation and says "first" is `accepted_count` under the lock |
| K4 | Leave task seed | None on the wire | None in the contract | **Plan wins (code changed)** | M6. Neither side covered an outcome-affecting setting. Schema block, wire example, `seed` note, claim step 4, worker behaviour and client section updated |
| K5 | Exports of completed jobs | Built while results still arrive | "results are immutable" | **Plan wins** | B3. PLAN.md's promise is what makes an export worth reusing, so the code has to wait until it is true |
| K6 | Purge / delete locking | Job row before claims | Dispatch lock only | **Plan wins (code changed)** | R1. The plan described the dispatch lock; the lock order it relied on was never stated, and was wrong. Purge paragraph updated |
| K7 | Re-dispatch locking | No dispatch lock | "Acquiring a task takes the job's dispatch lock first" | **Plan wins** | R2. Request-handling step 4, the claim loop and the dispatch paragraph now say re-dispatch is included |
| K8 | Live stats pushes | Coalesced, back to back | "one at a time per job" | **Code wins, then improved** | C3. The coalescing matched; the spacing is new. Result-submission step 7, the live-updates section, the key notes and the public API table updated |
| K9 | Rating history | Every run | "Every stored run's ratings" | **Code wins, then improved** | Performance item 3. Ratings page section and public API table updated |
| K10 | "First accepted result" | Counted stored rows | Silent on mechanism | **Code wins, then improved** | C2. Stated in the redundancy bullet |
| K11 | Design decisions table | Debounced SPRT (prior U3) | "SPRT evaluated on every result submission" | **Code wins** | Stale since the last audit implemented the debounce |
| K12 | Scaling, "Debounced live stats" | Implemented | "Measured as unnecessary so far" | **Code wins** | Stale; rewritten to say what is done and what remains |
| K13 | Key notes, task queue | Per-job advisory lock plus `SKIP LOCKED` | "without lock contention" | **Code wins** | Claims against one job serialize by design (prior audit) |
| K14 | Key notes, live updates | Coalesced pushes | "after every accepted task result" | **Code wins** | Stale since prior C1 |
| K15 | Design table, pre-aggregation | Four job counters plus identity counters | "exactly two … running totals" | **Code wins** | Stale since prior U2 |
| K16 | Admin API table, delete user | Anonymizes, keeps claims | "Delete a user account and all their task claims and records" | **Code wins** | The table contradicted PLAN.md's own semantics section, which matches the code |
| K17 | Frontend routes, `/jobs` | No stream; loaded on visit | "Live-updated via SSE" | **Code wins** | There is only a per-job stream. Building a list stream is a feature, not a fix |
| K18 | Captured positions' play cap | Player 1's `num_plays_recorded` for both players; capture raises a simmer's `num_plays` | "the player config's `num_plays_recorded`" | **Code wins** | MAGPIE has one cap per run and the server truncates with the same number, so they agree. Stated, together with the capture side effect (U2) |
| K19 | Null request settings | Only per-player settings reset | "takes MAGPIE's compile-time default" | **Plan wins** | M4. The statement is the design, and run-wide settings broke it |
| K20 | Opening-rack executor | Analysis read run-wide settings; player 2 left over | "apply the single player config" | **Plan wins** | M1, M2, M3 |
| K21 | Shared options on the worker | Player 1's only | "validated … agree" | **Plan wins** | M5, the MAGPIE half of K2 |
| K22 | MAGPIE floor | `0.1.0` | `0.1.0`, "first version that implements the protocol correctly" | **Code changed, PLAN updated** | M7. The floor is what keeps builds with the gaps above out |

Counted: K8–K18 are code-wins (11). K1–K7 and K19–K22 are plan-wins, where the
code changed (11).

**Checked and found in agreement (no change):** the SPRT debounce and quiet-fleet
cover; the job-list and identity counters and their decrements; keyset pagination
and its three cursor shapes; ban uniqueness; the leave-transition ownership,
takeover and purge-refusal rules; version negotiation; decline bounds; capability
filtering before `MIN(priority)`; the 64 MiB body limit; graceful shutdown; the
single-instance deployment settings. The Python worker's status is covered in
section 10.

---

## 7. MAGPIE `birdtest-contribute`

**All changes are on `birdtest-contribute`, in commits `22c4c25f` (sections 1–8)
and `cb390035` (U5's `task_failed` decline, and fixtures for U3); none on `main`
or any other branch.** The branch was checked out at `eb603694`, the previous
audit's unpushed commit. All three are now pushed to `origin` (U8).

### What was missing, and was fixed

| # | What | Where |
|---|---|---|
| M1 | Opening-rack analysis uses the player's simulation settings; inference off | `config_contribute_use_player_settings_for_analysis`, called by the opening-rack executor |
| M2 | Per-rack simulation seed from the rack | `contribute_rack_seed`, in `config_contribute_analyze_rack` |
| M3 | Player 2 gets the player's leaves and settings in opening-rack analysis | `config_contribute_opening_rack` |
| M4 | Run-wide settings reset per task | `config_contribute_reset_shared_settings`, called by all three executors; `CONFIG_DEFAULT_USER_CUTOFF` |
| M5 | Win% model and movegen margin from whichever player states them; opening racks apply the margin | `contribute_stated_by_either`, `config_contribute_apply_movegen_margin` |
| M6 | Leave generation requires and uses the request's `seed` | `config_contribute_leave_gen` |
| M7 | `MAGPIE_VERSION` `0.2.0` | `config.c` |
| — | Contract fixtures copied from birdtest (`seed`, `0.2.0`); the leave key list requires `seed` | `test/birdtest_contract/`, `test/contribute_test.c` |
| — | Two unit tests | `test_shared_settings_do_not_leak_between_tasks`, `test_opening_rack_analysis_uses_the_players_settings` |

### Checked and still present

The six worker endpoints and their headers; the required claim body; numeric
version comparison; the three decline reasons; digest verification through
`data_filepaths` with a cache keyed on inode and ctime at nanosecond resolution;
the retry policy; `seed` parsed with `strtoull`; the pentanomial; the `game_index`
flattening; wordmap staleness sidecars; the settings snapshot and restore around
`contribute`; per-player setting reset; and the opening-rack play cap and
`num_moves` (prior M1).

### End-to-end run

`scripts/e2e_magpie.py`, run against an isolated compose stack (backend image
built from this branch) with the release `magpie` from `22c4c25f` reporting
`0.2.0`, which the new floor admits. The seeding went through the real API,
including a MAGPIE-DATA `data-20251004` import. **Every job type passed:**

| Job | MAGPIE run | Result |
|---|---|---|
| games | 2 s | ok, 2 accepted claims |
| game_pairs | 2 s | ok, 2 accepted claims |
| opening_rack, simming (`num_plies` 2, `num_plays` 5, `max_iterations` 60) | 2 s | ok, 2 accepted claims |
| opening_rack, static | 2 s | ok, 2 accepted claims |
| leave_generation | 67 s (job creation seeded generation 1 in 38.6 s) | ok, 2 accepted claims |

What the database held afterwards:

- **Leave-generation tasks carry distinct server-chosen seeds** in
  `leave_requests.seed` (M6), and MAGPIE ran them. The executor now refuses a
  request without one, so the tasks passing shows the field was read.
- **Simmed opening racks store win percentages, and `num_moves` is 5 per rack**:
  exactly the job's `num_plays`. The previous audit ran this same job and
  recorded "72–100 ranked candidates per rack", and **342 s** for the job. Both
  figures are MAGPIE's run-wide defaults, not the job: 100 candidate plays, and
  simulation until the time limit, because the job's 60-iteration budget never
  reached the simulation. Under the same job config it now takes **2 s** and
  ranks the 5 plays it asks for. This is M1 confirmed against a real MAGPIE, and
  it resolves the prior audit's performance item 11, which recorded the 342 s as
  inherent cost.

**A second run, after the section 9a decisions** (backend from `c495b10`, MAGPIE
from `cb390035`), also passed for every job type:

- **Leave-generation job creation took 0.8 s, against 38.6 s before** (U9).
  Creation wrote no rack universe. The first claim got `204` and started the
  seeding task, and MAGPIE's own retry picked up work once it committed. The
  leave task's MAGPIE run went from 67 s to 126 s because it now includes that
  seeding and MAGPIE's idle waits, not because a task got slower.
- **The audit log held no `task.claimed` or `result.submitted` rows** (U1),
  only admin and import events.
- **The simmer, now with `time_limit_secs` 0** (U3), was accepted at creation
  and ran in 2 s as before.
- **No task failed, so no `task_failed` decline was sent.** U5 is covered by
  `worker_api::a_failed_task_is_handed_straight_back` on the server side and by
  the MAGPIE build. It was not exercised end to end.

---

## 8. Deployment and implementation blockers

| Finding | Status |
|---|---|
| Leave-generation jobs could stall permanently past generation 1 on a slow database (R3) | **Fixed** |
| An export could be short and then served forever (B3) | **Fixed** |
| Purge and delete deadlocked against submissions (R1) | **Fixed** |
| MAGPIE results depended on each contributor's settings (section 1) | **Fixed**, and enforced by the `0.2.0` floor (M7) |
| **`birdtest-contribute` is not pushed.** CI's `magpie-contract` and the nightly end-to-end job check out the branch from GitHub, where it lacks this audit's MAGPIE commit and the previous audit's `eb603694`. With the floor at `0.2.0`, the nightly job's MAGPIE (reporting `0.1.0`) gets a `magpie_too_old` shutdown | **Resolved** (U8): pushed after the decision was taken |
| Existing development databases fail migration after this change, because `0001_initial.sql` was edited in place | Expected under the single-migration convention (PLAN.md, "Resetting the database") |
| Verified unchanged since the previous audit: migrations before bind, graceful shutdown, ALB `idle_timeout` 300 s, ECS stop-then-start, SSM secrets, config validation, Nginx body limit and SSE buffering, CI coverage, no MAGPIE in the backend image | Holds |

---

## 9. Left for human input

Each of these has reasonable arguments on more than one side, so the code and
PLAN.md were left as they are.

### U1 — observational writes inside the submit transaction

**What it is.** Every submission writes a `result.submitted` audit row and
increments its identity's `tasks_completed`, and every claim writes
`task.claimed`, all inside the transaction a worker waits on. None of these
decides anything.

- **(a) Leave as is.** Audit rows are atomic with the actions they describe;
  counters cannot drift. Costs two small writes per task on the hot path, and
  the identity row lock serializes one account's concurrent submissions for the
  transaction's final statements.
- **(b) Move the audit rows to a buffered async writer.** Faster, but a crash
  loses rows for actions that happened, which breaks PLAN.md's "an audit failure
  rolls back what it describes".
- **(c) Drop `task.claimed` and `result.submitted` entirely.** `task_claims`
  already records both events with timestamps and identities, so the audit rows
  duplicate them. This halves `audit_log` growth (U7) too.

**Recommendation: (c) for the two worker events, keeping audit rows for admin
actions.** They are the only audit actions with a table of their own that
already says the same thing.

### U2 — `capture_positions` changes what a simmer simulates

With capture on, MAGPIE's autoplay raises each simming player's `num_plays` to
`position_play_cap`, which is player 1's `num_plays_recorded`. So a job with
capture on can play different games from the same job with capture off. Player
2's simmer is also raised by player 1's number.

- **(a)** Validate at job creation that a capture job's simmers have `num_plays
  ≥` player 1's `num_plays_recorded`, so the raise is a no-op.
- **(b)** Change MAGPIE to cap the captured list at `num_plays` instead of
  raising it.
- **(c)** Document it and leave it.

**Recommendation: (a).** It keeps "capture only decides what is recorded" true
without touching MAGPIE's autoplay.

### U3 — simulations are machine-dependent by construction

A simmer's result depends on its thread count (`threads` in `contribute.txt`)
and, when `time_limit_secs` is null or binding, on hardware speed (MAGPIE's
default limit is 60 s). Every setting is now pinned, but two honest workers
still produce different simulated rankings. SPRT on simming games is a
statistical test and tolerates this. Cross-checking redundant claims for
equality (PLAN.md's proposed next integrity step) does not.

- **(a)** Require `time_limit_secs = 0` (no limit) on simming configs, so
  iteration budgets decide, and document thread-dependence.
- **(b)** Also pin threads per task from the server. Fair across workers, but it
  wastes contributors' cores.
- **(c)** Accept it, and exclude simming jobs from any future equality
  cross-check.

**Recommendation: (a) plus (c).**

### U4 — redundant leave-generation results when MAGPIE is multi-threaded

B2 folds one result per task, which is right when copies replay the same games.
With `threads > 1`, leavegen's forced draws depend on shared rack-list state
across threads, so copies differ, and the discarded copy is real coverage.

- **(a) Keep B2.** Consistent with every other aggregate, and never double-counts.
- **(b)** Fold all copies and treat them as independent samples. More data,
  but "one result per task" stops being a system-wide rule.
- **(c)** Refuse `redundancy > 1` for leave generation. It has no current use.

**Recommendation: (c).** There is no integrity use for redundant leave tasks
today, and refusing them removes the question.

### U5 — a task that fails locally holds its slot for the heartbeat timeout

When a MAGPIE executor errors, `contribute_submit_result` stops the heartbeat
and submits nothing. The claim stays `claimed` for up to 300 s before another
worker can have it.

- **(a)** Add a `task_failed` decline reason and send it.
- **(b)** Leave it. Failures are rare, and five minutes is bounded.

**Recommendation: (a)**, as an additive change at the next MAGPIE release.

### U6 — display reads that grow with a job's history

Performance item 7: `worker_contributions` and `estimate_eta` per detail view and
push, and the job list's `stalled` subqueries.

- **(a)** Wait for `SLOW_STATS_THRESHOLD` log lines.
- **(b)** Per-job, per-identity contribution counters, like the leaderboards'.
- **(c)** A background refresh of the payload.

**Recommendation: (a).** The log line was built for exactly this decision.

### U7 — `audit_log` and `rating_runs` have no retention

- **(a)** Partition `audit_log` by month and drop old partitions of worker
  events.
- **(b)** Thin `rating_runs` older than N days to one per day.
- **(c)** Leave it until storage says otherwise.

**Recommendation: (c) now, with U1 (c) removing most of `audit_log`'s growth.**

### U8 — push `birdtest-contribute`

This is an action, not a design question. Section 8 has the details.

### U9 — generation 1 is seeded inline at job creation and purge

Creation seeds generation 1 in the creating request (37–56 s measured). Purge
does too, holding the job row and every open claim for the duration.

- **(a)** Make generation 1 lazy too: create and purge write nothing, and the
  first claim starts the seeding task (R3's path). Fast admin actions, no lock
  held for a minute.
- **(b)** Leave it. These are admin actions behind a 300 s ALB timeout.

**Recommendation: (a).** It reuses R3's path. It changes `initialized` in the
create response, and a few tests.

---

## 9a. Decisions taken — implemented

All nine recommendations above were accepted, and all are carried out on this
branch. Where an item's recommendation was to change nothing, the decision is
recorded as taken.

| Item | Decision | What it took |
|---|---|---|
| U1 | **(c)** Drop `task.claimed` and `result.submitted` | Both writes removed, one each from `scheduler::issue_claim` and `submit_result`. A write gone from each of the claim and submit transactions. `audit_log` keeps admin, account and decline events. The census test now checks the `job.deleted` row, and that claims and submissions write none. PLAN.md's audit table, claim loop, submission step 5 and dashboard audit note updated |
| U2 | **(a)** Refuse a capture job whose simmers capture would raise | `validate_capture_play_cap`, run for `games` and `game_pairs` with `capture_positions` on: every simming player's `num_plays` (MAGPIE's 100 when null) must be at least player 1's `num_plays_recorded`. Error names the player and the remedy. Test: `a_capture_job_refuses_simmers_that_capture_would_change` |
| U3 | **(a) + (c)** Simmers set no time limit and an iteration budget; simming jobs excluded from equality cross-checks | Player-config creation refuses a simmer without `max_iterations`, or with `time_limit_secs` other than 0. MAGPIE applies a limit only above 0 (checked in `bai_result.c`), and a null means its 60 s default. The form sends 0 and says why; the contract fixtures' simmers, and the e2e simmer, state 0. PLAN.md's cross-checking paragraph now excludes simming jobs. Test: `a_simming_player_config_is_bounded_by_iterations_not_time` |
| U4 | **(c)** Leave generation at redundancy 1 | `validate_job_body` refuses `redundancy > 1` for leave generation, and the form disables the field. B2's fold-first rule stays as a guard for state creation no longer writes. Unit test: `leave_generation_runs_at_redundancy_one` |
| U5 | **(a)** A `task_failed` decline | Server accepts the reason. MAGPIE (`contribute.c`, `decline_failed_task`) sends it when an executor fails and when the server refuses a result, so the slot comes back at once instead of after the heartbeat timeout. The job is not marked unsupported, so a one-off failure does not lock the worker out; the consecutive-failure guard still ends a run that fails every time. A failed decline is dropped rather than ending the run. Additive on the wire. Test: `worker_api::a_failed_task_is_handed_straight_back` |
| U6 | **(a)** Wait for the slow-stats log line | Nothing to build |
| U7 | **(c)** No retention yet | Nothing to build; U1 removed most of `audit_log`'s growth |
| U8 | **Push `birdtest-contribute`** | Pushed: `origin` moved from `cac07a8a` to `cb390035` |
| U9 | **(a)** Generation 1 seeded lazily | `registry::initialize_job_state` deleted. Job creation and purge write no rack universe; the first claim finds it missing and starts R3's seeding task, as for every later generation. The create response is `{ job }` (`initialized` dropped from the backend and `api.ts`). Test: `generation_ones_universe_is_seeded_by_the_first_claim_too`. PLAN.md's purge paragraph, claim step 2, long-operations paragraph, creation response and restore section updated |

---

## 10. Python worker

Searched the whole tree (README, RUNBOOK, TESTING, PLAN, compose, Dockerfile,
Terraform, CI, scripts, backend) for any treatment of `worker/fake_worker.py` as
a production client. **None found; nothing to correct.** The compose service is
behind a `fake-worker` profile documented as end-to-end-only, and the Dockerfile
target says so. `RUNBOOK.md` says "**Never use `worker/fake_worker.py` for
this**", and README, TESTING.md and `scripts/dev.py` say MAGPIE is the only
contributor. The prior audit (prior K19) reached the same result. `fake_worker.py`
needs no change for the new seed field, because it does not read one.

---

## 11. Tests added or changed

| Test | What it pins |
|---|---|
| `admin_api::a_purge_waits_for_a_submission_in_flight_before_counting_contributions` | R1: purge waits for in-flight submissions, and their credit is handed back |
| `admin_api::a_games_job_may_pit_a_static_player_against_a_simmer` | B1: either seat, and differing simmers still refused |
| `admin_api::a_completed_job_is_not_exported_until_its_claims_have_landed` | B3 |
| `admin_api::a_long_rating_history_is_thinned_but_keeps_its_ends` | Performance item 3: at most 501 points, both ends kept |
| `leave_gen::a_leave_task_carries_its_seed_and_a_reissue_replays_it` | M6 |
| `leave_gen::only_the_first_result_for_a_leave_task_is_folded` | B2 |
| `leave_gen::the_next_generations_universe_is_seeded_off_the_claim_path` (rewritten) | R3: the claim answers at once, the seeding completes, and work follows |
| MAGPIE `test_shared_settings_do_not_leak_between_tasks` | M4 |
| MAGPIE `test_opening_rack_analysis_uses_the_players_settings` | M1 |
| `admin_api::a_simming_player_config_is_bounded_by_iterations_not_time` | U3: a simmer needs an iteration budget and no time limit |
| `admin_api::a_capture_job_refuses_simmers_that_capture_would_change` | U2 |
| `admin::tests::leave_generation_runs_at_redundancy_one` | U4 |
| `worker_api::a_failed_task_is_handed_straight_back` | U5 |
| `leave_gen::generation_ones_universe_is_seeded_by_the_first_claim_too` | U9 |
| `admin_api::a_job_with_history_can_be_deleted_and_its_census_survives` (updated) | U1: no audit rows for claims or submissions |
| MAGPIE contract key test (updated) | M6: the leave-generation fixture carries `seed` |
