# Pre-endgame and endgame play in games and game-pairs jobs

**Goal.** A games or game-pairs job can have its players solve the
pre-endgame (PEG) and the endgame. Today every turn is played by static
equity or by a simulation. This document describes what has to change in
MAGPIE and in birdtest.

MAGPIE references are to `src/…` at `cfd04b86` on `birdtest-contribute`, the
commit [docker/Dockerfile:6](docker/Dockerfile#L6) pinned when this plan was
written; the implementation is `79100bf1` (see
[Implementation notes](#implementation-notes)).

## Decisions

These shape everything below.

1. **The settings belong to the player config, not to the job.** Whether a
   player solves endgames changes how strong it is. Rating pools rate player
   configs, so a config must play the same way in every job it appears in.
   This also matches MAGPIE, which chooses a strategy per player
   (`PlayChooserStrategy`), and the way birdtest already stores simulation
   settings.
2. **Only static players are deterministic.** MAGPIE almost always runs
   multithreaded, and multithreaded sims, PEG solves and endgame solves can't
   be made to replay identically. A player that simulates or solves gives a
   sample, not a fixed answer. Seeding each solve consistently makes results
   more consistent, but nothing in this plan depends on two runs agreeing.
3. **Work is bounded by depth and candidate counts, not by time.** Birdtest
   already requires `time_limit_secs = 0` for simmers, so that a config is
   equally strong on a fast machine and a slow one. The solvers follow the
   same rule: a fixed endgame depth and a fixed PEG schedule, with no
   wall-clock limit. Results still vary from run to run (decision 2); they
   just don't vary with the hardware.
4. **Each solve uses the worker's threads, set directly.** PEG and endgame
   solves are called with the worker's thread count (`config->num_threads`).
   Autoplay's multi-threading mode (`pgp`/`igp`) decides how autoplay spreads
   games and sims across threads. It has nothing to do with the solvers, and
   this plan leaves it as it is.
5. **The endgame switch also controls PEG.** `endgame_plies = 0` turns off
   both solvers, because PEG needs endgame solving. A player can have the
   endgame without PEG, but not PEG without the endgame.
6. **The phase decides the evaluator, per turn.** For a player with
   `endgame_plies > 0`:
   - when the bag is empty, it solves the endgame;
   - when the bag holds 1 to `peg_max_bag` tiles, it solves the pre-endgame;
   - on every other turn it plays as it does today, statically or by
     simulation.

   This works for static players as well as simmers. Neither solver needs a
   win% model.
7. **Capturing positions doesn't change how a player plays.** On a PEG or
   endgame turn, the captured analysis is what the deciding solve produced.
   Capture never asks for more search.

## What exists today

| Piece | Where | Why it can't be used as-is |
|---|---|---|
| Endgame solver | `src/impl/endgame.c`, `endgame_solve` | Usable as it is. A `seed` of 0 seeds from the current time (`endgame.c:3979`), so the caller should pass one. |
| PEG solver | `src/impl/peg.c`, `peg_solve` | Usable as it is, except that several of its knobs fall back to bag-size-dependent built-ins instead of stated values. |
| PlayChooser, which runs both solvers inside autoplay | `src/impl/play_chooser.c`, `play_chooser_choose_move` (`:775`) | Every budget is wall-clock time (`:380`, `:548`, `:694`). An untimed player still gets a default time budget, so how strong a player is depends on the hardware. `contribute` turns PlayChooser off for every task (`config.c:8172`). |
| Autoplay's per-turn move choice | `src/impl/autoplay.c:750`, `game_runner_get_best_move` | Uses PlayChooser if one is configured; otherwise a static top move or a simulation. There is no depth-bounded solver branch. |
| PlayChooser's thread count | `config.c:3933` | PlayChooser takes the thread count autoplay computes for sims (`num_worker_threads_per_sim`). The new solver path doesn't copy that; it passes the worker's thread count. |
| Positions recorder | `src/ent/autoplay_results.c:2128`, `autoplay_results_add_move` | Records a static move list or simulation results only. |

The existing CLI options `eplies`, `etopk`, `etlim`, `pegtopk`, `pegtlim`,
`pegstride`, `pnoprune`, `pegpess` and `pegnested` configure the interactive
`endgame` and `peg` commands. Autoplay never reads them.

## How each solve is run

| Choice | Why |
|---|---|
| No wall-clock limit: endgame soft and hard limits 0, PEG `time_budget_seconds` 0, no external deadline. | Decision 3. Both solvers already treat 0 as unbounded. |
| A fixed endgame depth and a fixed PEG stage schedule. | These replace time as the bound on work. Both are already supported. |
| A nonzero seed for each solve, derived from the game seed, turn number and player index. | Seed 0 means "the current time". A seed derived from the task keeps results as consistent as multithreading allows (decision 2). |
| `num_threads` is the worker's thread count, for both solvers (decision 4). | PEG on few threads is far too slow. Determinism isn't a goal, so nothing has to be held back. |
| Transposition tables as MAGPIE already sizes them, as a fraction of the machine's RAM. The endgame shares one table across the run's solves, as PlayChooser shares one across concurrent solves (`play_chooser.c:312`). PEG allocates its own per call, as it does today. | Nothing needs tables to be the same size on every machine: their size affects speed, and results already vary (decision 2). |
| Every PEG setting stated in the request, none left to a built-in default. | This is birdtest's existing rule for every setting (`magpie_defaults.rs`): a config should mean the same thing on every MAGPIE build. Today the nested-lookahead stride defaults by bag size (`peg.c:1367`), and the nested candidate caps live in `config.c:3525`. |

## The settings

Each new player setting has a column, a request key, MAGPIE per-player CLI
options, and the default birdtest writes at creation. As with today's
settings, the values are written into the row from MAGPIE's defaults when the
config is created, so a config means the same on every MAGPIE release.

| Setting | Column (`player_configs`) | Request key | New MAGPIE options (per player) | Default when on | Notes |
|---|---|---|---|---|---|
| Endgame depth | `endgame_plies INT NOT NULL DEFAULT 0` | `endgame_plies` | `eplies1` / `eplies2` | 6 (MAGPIE's `eplies` default) | 0 = no endgame and no PEG. Valid range 1–25 (`MAX_VARIANT_LENGTH`). |
| PEG at bag ≤ | `peg_max_bag INT NOT NULL DEFAULT 0` | `peg_max_bag` | `pegbag1` / `pegbag2` | 4 (`PEG_MAX_BAG`) | 0 = no PEG. Valid range 1–4, and only when `endgame_plies > 0`. Lowering it is the main way to control cost. |
| PEG stage schedule | `peg_stage_top_k INT[]` | `peg_stage_top_k` | `pegtopk1` / `pegtopk2` | `{32,16,8,4,2}` | 1–16 entries (`CONFIG_PEG_MAX_STAGES`), each ≥ 2 and non-increasing. The exhaustive `all`/`0` form is refused. |
| PEG scenario stride | `peg_scenario_stride INT` | `peg_scenario_stride` | `pegstride1` / `pegstride2` | 1 (full enumeration) | ≥ 1. Never 0: 0 means "the solver's default". |
| PEG opponent model | `peg_opp_model TEXT` | `peg_opp_model` | `pegpess1` / `pegpess2` | `'rational'` | `'rational'` or `'pessimistic'`. |
| Nested lookahead | `peg_nested BOOLEAN` | `peg_nested` | `pegnested1` / `pegnested2` | true (the CLI default) | |
| Nested candidate caps | `peg_nested_cand_caps INT[]` | `peg_nested_cand_caps` | global `pegncaps` | `{8,4,2}` | Only when nested lookahead is on. |
| Nested depth | `peg_nested_max_depth INT` | `peg_nested_max_depth` | global `pegndepth` | 1 (`PEG_NESTED_DEFAULT_DEPTH`) | Only when nested lookahead is on. |
| Nested strides by bag | `peg_nested_strides INT[]` | `peg_nested_strides` | global `pegnstrides` | `{1,1,5,7}` (bags 1–4) | Replaces today's bag-dependent built-in. Only when nested lookahead is on. |

Notes on the table:

- **The settings nest.** All three levels follow the same all-or-nothing
  pattern as the simulation settings and their CHECK:
  - `endgame_plies = 0` requires `peg_max_bag = 0`.
  - `peg_max_bag = 0` requires every other PEG setting to be NULL, and
    `peg_max_bag > 0` requires them all to be set.
  - The three nested settings are set exactly when `peg_nested` is true.
- **The endgame depth only applies to empty-bag turns.** PEG's own endgame
  solves, inside each scenario, use the depth its stage schedule sets.
  `endgame_plies` doesn't limit them.
- **Option names are suggestions.** Whatever MAGPIE chooses, each per-player
  pair needs a global form that sets both players (as `plies` does for
  `pl1`/`pl2`). That way a CLI `autoplay` run can use the same settings as a
  birdtest task.
- **`num_plays` doesn't limit PEG.** PEG generates and ranks every candidate
  itself.
- **`use_inference` only applies to simulation turns.** PEG enumerates every
  unseen tile, so it has nothing to infer.
- **Adding the options grows `arg_token_t`.** The suggested names add 15
  tokens (6 per-player pairs plus 3 global nested options).
  [SETTINGS_COMPARISON.md](SETTINGS_COMPARISON.md) has to be updated to
  match. The existing global options `eplies`, `pegtopk`, `pegstride`,
  `pegpess` and `pegnested` become the global forms of the new per-player
  options. `etopk`, `etlim`, `pegtlim`, `pnoprune` and the display options
  stay unstored: a time limit breaks decision 3, and a task has no
  hand-picked move list.

## MAGPIE changes

### M1. Solver entry points for autoplay

1. **PEG nesting.** Add `nested_strides_by_bag[PEG_MAX_BAG + 1]` to
   `PegArgs`, used whenever it's set. Extend `peg_args_fill` to cover the new
   fields, since its comment requires every field to be stated at each call
   site.
2. **A wrapper for autoplay and `contribute`.** Add
   `autoplay_solve_endgame(...)` and `autoplay_solve_peg(...)` in a new
   `src/impl/autoplay_solvers.c`. Each takes a per-player settings struct, the
   game, the thread count and a seed. They:
   - fill `EndgameArgs` / `PegArgs` from the struct, with no time limits and
     `num_threads` set to the worker's full thread count;
   - return the chosen move plus the ranking for capture;
   - return an error if the solve produced no move. They never fall back
     silently. An unbounded solve always produces a move, so "no move" means
     a bug, and it should fail the task.
3. **The seed for each solve.** Derive it as
   `hash(game seed, turn number, player index)`, never 0.

No transposition-table changes are needed. An earlier draft of this plan had
them, but only to make results deterministic, which decision 2 drops.

### M2. Autoplay

1. **Settings.** Add `AutoplaySolverSettings` with endgame plies, PEG max bag,
   stage schedule, stride, opponent model and the nested settings. Add
   `p1_solver` and `p2_solver` fields to `AutoplayArgs`
   (`src/impl/autoplay.h`).
2. **Threads.** `AutoplayArgs` carries the worker's thread count for the
   solvers. Nothing in autoplay's `pgp`/`igp` handling
   (`config.c:3843–3852`) changes.
3. **Scratch.** Each `AutoplayWorker` owns an endgame context, reused across
   turns and games. The run owns one endgame table, shared by every worker.
4. **Choosing a move.** In `game_runner_get_best_move`
   (`autoplay.c:750`), before the static/simulation branch:

   ```c
   const AutoplaySolverSettings *s = &args->solver[player_on_turn_index];
   const int bag = bag_get_letters(game_get_bag(game));
   if (s->endgame_plies > 0) {
     if (bag == 0) return solve_endgame(...);
     if (bag >= PEG_MIN_BAG && bag <= s->peg_max_bag) return solve_peg(...);
   }
   // existing PlayChooser / static / sim path
   ```

   Two details are worth recording in code comments:
   - PEG's effective bag counts the opponent's rack as unseen
     (`peg_compute_bag_size`, `peg_compute_unseen`), so a PEG turn never sees
     the opponent's actual tiles.
   - The endgame starts only once the bag is empty, when the opponent's rack
     can be deduced from the tiles already seen. Neither solver cheats.
5. **Leave generation is unaffected.** Its games end when the bag drops below
   `RACK_SIZE` (`autoplay.c:654`), so it never reaches either branch.
6. **Optional per-game counters** for cost monitoring: endgame turns, PEG
   turns, endgame nodes, and PEG scenarios. These go into the game recorder's
   output, not into any result that decides anything.

### M3. Positions recorder

When positions are captured, `autoplay_results_add_move` gets a new argument:
the solver's ranking. It writes an `analysis` field on each position, one of
`static`, `sim`, `peg` or `endgame`.

- **PEG turns:**
  - Moves come from the graded ranking (`PegResult.graded_cands`, falling back
    to `top_cands`), capped at `num_plays_recorded`.
  - Each move carries `score`, the move's static `equity`, `win_percentage` =
    `win_pct`, `mean_spread`, and `fidelity_plies` (the depth of the tier it
    reached).
  - `num_moves` is the number of candidates entered at stage 0.
- **Endgame turns:**
  - One move: the solve's best move. It carries `score`, static `equity`,
    `mean_spread` (the solved spread) and `fidelity_plies` (the depth
    reached).
  - `num_moves` is the number of root moves.
  - Recording more moves would require asking the solver for its top *k*
    lines, which changes the search. That would break decision 7.
- **Equity on solver moves:** `small_move_to_move` doesn't carry equity. Work
  out static equity for each recorded move the same way move generation does,
  so the `equity` column stays meaningful and `NOT NULL`.

### M4. `contribute`

1. **The allowed keys.** Add the request keys from the table to
   `src/def/contribute_defs.h`:
   - `endgame_plies` and `peg_max_bag` join `contribute_required_player_keys`
     (`config.c:8195`), so every player states them, if only as 0.
   - Add a `contribute_required_peg_keys` list, required when
     `peg_max_bag > 0`, and a nested list, required when `peg_nested` is
     true.
2. **Reset and apply.**
   - `config_contribute_reset_player_settings` (`config.c:8144`) resets both
     players' solver settings to off.
   - `config_contribute_apply_player_settings` (`config.c:8240`) reads and
     range-checks each key, with the same limits birdtest enforces. Any value
     out of range is a `CONTRIBUTE_SERVER_ERROR`, and so is
     `peg_max_bag > 0` with `endgame_plies = 0`.
3. **The games executor** (`config_contribute_games`, `config.c:8601`) passes
   the settings and the worker's thread count to `AutoplayArgs`.
4. **The other executors.** The opening-rack executor ignores the settings: an
   opening rack never reaches a small bag. The leave executor also ignores
   them, and birdtest refuses them for leave jobs anyway.
5. **The output.** Captured positions include the new fields from M3.

### M5. Tests

- `test/contribute_test.c`: a games task between two static players with
  `endgame_plies` 0 still produces byte-identical JSON at 1, 2 and 8 threads.
  This is the determinism birdtest relies on, and the new code must not
  disturb it.
- `test/contribute_test.c`: a games task with a PEG + endgame player
  completes. Its captured PEG and endgame positions have the shape M3
  describes.
- `test/contribute_test.c`: these requests are refused:
  - one leaving out each required key;
  - `peg_max_bag > 0` with `endgame_plies = 0`;
  - PEG keys without `peg_max_bag > 0`.
- `test/autoplay_test.c`: with `endgame_plies` 0, a player never reaches
  either solver, whatever `peg_max_bag` says.
- `test/autoplay_test.c`: each PEG and endgame solve is given the worker's
  full thread count, under both `pgp` and `igp`.

`MAGPIE_VERSION` stays as it is. Nothing is in production, so there are no
older workers or servers to keep apart from this contract.

## birdtest changes

Schema changes edit
[backend/migrations/0001_initial.sql](backend/migrations/0001_initial.sql)
in place, as [PLAN.md](PLAN.md) prescribes until release.

### B1. Schema

1. **`player_configs`** gets the columns in the [settings table](#the-settings)
   and a CHECK, `player_configs_solver_settings`, next to the existing
   simulation CHECK ([L617](backend/migrations/0001_initial.sql#L617)):
   - `endgame_plies BETWEEN 0 AND 25` and `peg_max_bag BETWEEN 0 AND 4`;
   - `endgame_plies > 0 OR peg_max_bag = 0`;
   - when `peg_max_bag = 0`, every other PEG column is NULL;
   - when `peg_max_bag > 0`, `peg_stage_top_k`, `peg_scenario_stride`,
     `peg_opp_model` and `peg_nested` are NOT NULL;
   - the three nested columns are NOT NULL exactly when `peg_nested`.

   Update the column comment block to say the solver settings apply to games
   and game-pairs jobs only, and that `endgame_plies = 0` turns PEG off too.
2. **`position_analysis_records`**
   ([L1124](backend/migrations/0001_initial.sql#L1124)) gets
   `analysis TEXT NOT NULL CHECK (analysis IN ('static','sim','peg','endgame'))`.
   Opening-rack records are `static` or `sim`.
3. **`position_analysis_moves`**
   ([L1222](backend/migrations/0001_initial.sql#L1222)) gets two nullable
   columns, both NULL for static and sim rows:
   - `mean_spread DOUBLE PRECISION`: PEG's mean spread, or the endgame's
     solved spread.
   - `fidelity_plies SMALLINT`: the depth the move was ranked at.

   Update the `win_percentage` comment: it is now set for sim and PEG rows,
   and NULL for static and endgame rows.

### B2. Backend

| File | Change |
|---|---|
| [magpie_defaults.rs](backend/src/magpie_defaults.rs) | Constants for the "default when on" column, each citing its MAGPIE source. |
| [models/job.rs](backend/src/models/job.rs#L101) `PlayerConfig` | The new fields. |
| [routes/admin.rs](backend/src/routes/admin.rs#L501) `CreatePlayerConfigBody` / [`create_player_config`](backend/src/routes/admin.rs#L546) / [`validate_player_config_body`](backend/src/routes/admin.rs#L796) | Accept the fields. With `endgame_plies` 0 (the default), the player is unchanged and states no PEG settings. When PEG is on, fill unstated PEG settings from the defaults. Refuse `peg_max_bag > 0` with field `peg_max_bag` when `endgame_plies` is 0 ("PEG needs endgame solving: set an endgame depth"). Likewise refuse PEG settings without `peg_max_bag > 0`, and nested settings without `peg_nested`, as simulation settings without plies are refused today. Validate the ranges in the settings table. |
| [routes/admin.rs](backend/src/routes/admin.rs#L1699) `validate_leave_player` | Refuse `endgame_plies > 0`, which also covers PEG. Leave games end before either solver is reached, and this function's rule is to refuse a setting it won't honour rather than drop it quietly. |
| [jobs/handler.rs](backend/src/jobs/handler.rs#L60) `PlayerSpec` | The new keys, with `None` for an unused PEG or nested key. MAGPIE needs them null, not absent, to tell the difference. |
| [jobs/mod.rs](backend/src/jobs/mod.rs#L509) `insert_position_analyses` | Store `analysis`, `mean_spread` and `fidelity_plies`. Keep at most `num_plays_recorded` moves per position, as today. |
| [jobs/plausibility.rs](backend/src/jobs/plausibility.rs) | `analysis` must be one of the four values. A PEG move has `win_percentage` in [0, 1] and a finite `mean_spread` within the score bounds. An endgame position has exactly one move. `fidelity_plies` is between 0 and 25. |
| [exports.rs](backend/src/exports.rs) | Positions export lines carry the new fields. |
| [routes/public.rs](backend/src/routes/public.rs#L280) | Player config output and the positions API carry the new fields. |
| Clone onto newer data (`cloned_from_id`) | Copies the solver settings too. A clone is meant to play the same way. |

Opening-rack jobs keep accepting these configs. A config is reusable, and the
settings are simply never read there. The job page marks them unused (see
B3), just as it does for inference after SETTINGS_COMPARISON.md's Fix 2.

**Comments that overstate determinism.** Several comments say redundant
claims "replay the same deterministic work" or "identical games":
[jobs/registry.rs:380](backend/src/jobs/registry.rs#L380),
[jobs/registry.rs:619](backend/src/jobs/registry.rs#L619),
[jobs/mod.rs:506](backend/src/jobs/mod.rs#L506) and
[jobs/mod.rs:721](backend/src/jobs/mod.rs#L721). That's only true between two
static players. It's already false for simmers, and solver players are no
different.

The behaviour itself is still right: only the first accepted result adds to
a job's totals and stores its captured positions. Correct the comments to say
so. For a job with a simulating or solving player, a redundant claim is a
second sample, not a check.

### B3. Frontend

1. **[admin/player-configs/new](frontend/src/routes/admin/player-configs/new/+page.svelte)**:
   an "Endgame and pre-endgame" section.
   - Controls: an endgame checkbox with a depth field, and a PEG checkbox with
     a bag-size field. The PEG checkbox is disabled until the endgame
     checkbox is ticked, and unticking the endgame clears PEG.
   - A collapsed "Advanced PEG" area holding the schedule, stride, opponent
     model, nested lookahead and the nested settings.
   - A hint that cost grows steeply with bag size and schedule.
2. **[lib/jobSettings.ts](frontend/src/lib/jobSettings.ts)**:
   - Add the new fields to `PlayerSettings` and to the player rows.
   - `playerSummary` ends with the solvers, for example
     `static, by equity · 6-ply endgame · PEG ≤4`.
   - `unusedPlayerSettings` marks every solver row unused for opening-rack
     jobs.
3. **Position pages:** show the analysis kind, and on PEG and endgame
   positions show `win%`, spread and depth instead of the simulation columns.
4. **[lib/api.ts](frontend/src/lib/api.ts)**: the types.

The player rows extend the order in
[SETTINGS_COMPARISON.md](SETTINGS_COMPARISON.md#player-settings-in-display-order).
Two rows join the key rows, right after row 11 (Plays recorded):

- Endgame (`6-ply` or `off`)
- Pre-endgame (`bag ≤ 4` or `off`; always `off` when the endgame is off)

The other PEG settings go under "All settings", after row 21 (Move-gen
margin), in this order:

- PEG schedule
- PEG stride
- PEG opponent
- Nested lookahead
- Nested caps
- Nested depth
- Nested strides

### B4. Contract, tests and docs

- [contract-fixtures/](contract-fixtures/): regenerate
  `assignment-games.json` and `assignment-game-pairs.json` so they carry the
  new keys. Add a captured result with `peg` and `endgame` positions, using
  the capture command in the fixtures' README.
- [scripts/e2e_magpie_native.sh](scripts/e2e_magpie_native.sh): a case with a
  PEG + endgame player at `games_per_batch` 2. It checks that the task's
  result is accepted and that captured PEG and endgame positions land. It
  doesn't compare redundant claims; see decision 2.
- Backend unit tests:
  - config validation: each range, PEG refused without an endgame depth, the
    all-or-nothing rules, and leave-job refusal;
  - plausibility checks on the new position fields.
- Docs: new test IDs in [TESTING.md](TESTING.md), a journey in
  [JOURNEYS.md](JOURNEYS.md), and the SETTINGS_COMPARISON.md update from
  [The settings](#the-settings).

## Cost

Neither solver is cheap, and with no time limit (decision 3) a task can't cut
a solve short. `peg.c` itself notes that leaf endgames at the deep stages
"legitimately run for minutes". Before choosing defaults or limits:

- **Phase 0 measures.** Use `test/benchmark_endgame_test.c` and
  `test/benchmark_peg_test.c`, and autoplay runs on the e2e lexicon. Measure
  the seconds per game and peak memory for each combination:
  - endgame off, 4-ply, 6-ply and 8-ply;
  - PEG off, `peg_max_bag` 1, 2 and 4;
  - the default schedule, with nested lookahead on and off.
- **The form's defaults follow the numbers.** Defaults stay MAGPIE's own
  values (decision 1 of `magpie_defaults.rs`). If PEG at bag 4 turns out to
  cost minutes per game, the form should *pre-fill* a smaller `peg_max_bag`.
  It should show the estimate next to the field, and still allow 4.
- **Heartbeats are not the limit.** `contribute` sends heartbeats from its own
  thread, so a long task stays claimed. The limit is how long a contributor
  waits for one task. Recommend a smaller `games_per_batch` for solver jobs
  (keeping it even), and say so on the job form.
- **Concurrent solves.** Autoplay plays several games at once, so several
  games can be solving at the same time, each with the worker's full thread
  count. Each concurrent PEG call also allocates its own tables. Phase 0
  should measure CPU and memory with games solving concurrently. If PEG's
  per-call tables add up to too much, MAGPIE can let PEG use the run's shared
  endgame table instead.

## Phases

| Phase | Work | Done when |
|---|---|---|
| 0. Measure | Benchmarks from [Cost](#cost). | Per-game time, and CPU and memory with concurrent solves, written down. |
| 1. MAGPIE solvers | M1. | Autoplay can call both solvers, with all threads and no time limits. |
| 2. MAGPIE autoplay and contribute | M2–M5, on `birdtest-contribute`. | M5's tests pass, including the static-player determinism test. |
| 3. birdtest schema and backend | B1, B2. Bump `MAGPIE_COMMIT` in [docker/Dockerfile](docker/Dockerfile#L6). | A job with solver players runs end to end and its captured positions land. |
| 4. birdtest frontend and docs | B3, B4. | The form, job page and position pages show the settings and the solver analyses. |

MAGPIE requires `endgame_plies` and `peg_max_bag` from Phase 2 on, so the
`MAGPIE_COMMIT` bump and birdtest sending both keys have to land together.
Nothing is deployed, so that is the only ordering to respect.

## Rejected alternatives

- **Reusing PlayChooser with a fixed time per move.** A player's strength
  would then depend on the contributor's hardware, which is the reason
  birdtest already requires simmers to have no time limit.
- **Job-level toggles.** One config would then play differently in different
  jobs, and the rating pools that rate it would be comparing different
  players under one name.
- **PEG without the endgame.** PEG needs endgame solving, so `endgame_plies`
  is the switch for both (decision 5).
- **Capturing the endgame's top *k* lines.** That means asking the solver for
  more than it needs to choose a move, so turning capture on would change how
  the player plays. That breaks decision 7.

## Implementation notes

What was built, and where it departs from the plan above.

- **The existing global options stay separate.** `eplies`, `pegtopk`,
  `pegstride`, `pegpess` and `pegnested` still configure only the interactive
  `endgame` and `peg` commands. They do not set both autoplay players, as the
  plan proposed. MAGPIE saves `eplies` in `settings.txt`, so making it a
  global form would have turned on endgame solving in the autoplay of anyone
  who had once run the `endgame` command. The per-player options (`eplies1`,
  `pegbag1`, `pegtopk1`, `pegstride1`, `pegpess1`, `pegnested1` and their
  `2` forms) and the nested options `pegncaps`, `pegndepth` and `pegnstrides`
  are new, as planned.
- **PEG shares the run's endgame table.** `PegArgs.shared_endgame_tt` is the
  refinement the [Cost](#cost) section held in reserve, built straight away:
  without it every concurrent pre-endgame solve allocated its own tables, up
  to a quarter of RAM per call.
- **A pre-endgame position's `num_moves` is every play the position has.**
  That is the field PEG's first stage scores; its graded ranking holds only
  the plays a later stage kept. MAGPIE counts the plays with a move
  generation.
- **An endgame play's equity is looked up.** The solve hands back a play with
  no static equity, so MAGPIE finds the same play in a full move generation
  for the position and takes its equity from there.
- **Measured cost (Phase 0).** NWL23, a static player against a static
  player, 2 threads, release build, wall time for a batch of 2 games:

  | Endgame | Pre-endgame | Schedule | Nested | Per batch |
  |---|---|---|---|---|
  | 2 plies | bag ≤ 2 | `[2]` | no | 0.8–1.8 s |
  | 2 plies | bag ≤ 4 | `[2]` | no | 1.8 s to over 60 s |
  | 2 plies | bag ≤ 4 | `[2]` | yes, caps `[2]` | 1.6 s to over 15 minutes |
  | 6 plies | bag ≤ 4 | MAGPIE's defaults | yes | over 2 minutes |

  The variation is between positions: a few pre-endgames at a bag of 4 cost
  far more than the rest. So the form pre-fills a bag of 2 and warns about
  the cost. MAGPIE's tests and tier 6's `M-12` and `capture` use a bag of 2
  with no nested lookahead.
