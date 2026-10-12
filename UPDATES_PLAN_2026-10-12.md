# Updates plan — a per-move movegen budget for players (2026-10-12)

Source: the owner, 2026-10-12. A player config gets a setting that caps the
move generations it may spend choosing one move. Agreed already:

- it may be set on any player, and says it only affects simulation, inference
  and solving (a static player generates once a move, so it never binds);
- when the budget runs out, the player plays its best move so far, even short
  of the simulation's minimum iterations;
- everything spent on the move counts: simulation, the inference before it,
  and endgame and pre-endgame solving.

**Branches.** MAGPIE: commits on `~/MAGPIE`'s `birdtest-contribute`, on top of
`c8d35894`. birdtest: commits on `better_scorecards`, on top of `fbcd1c4`.

## Why a budget, when simmers already have an iteration budget

A simmer's `max_iterations` caps rollouts, but a rollout's cost depends on its
plies and the position: 1,000 iterations of a 4-ply simmer are about twice
the work of 1,000 at 2 plies. And the iteration budget says nothing about the
inference before a simulation, or about endgame and pre-endgame solving,
which have only time limits. birdtest already refuses a time limit on a
simmer (`admin.rs`: "a time limit makes results depend on the contributor's
hardware"), so today nothing hardware-independent bounds a move's whole
search. A movegen budget does: two players can be given the same budget and
compared at equal work, whatever their plies, and any machine does the same
work for it.

## How it works in MAGPIE

The pieces exist (see `-movegenlimit`, `c8d35894`):

- Each thread counts its own generations (`gen->movegen_count`). A new
  `gen_get_thread_movegen_count()` reads the calling thread's count, so a
  worker can measure what one unit of work cost it.
- A simulation already stops on a status check: `bai_result_set_and_get_status`
  (`src/ent/bai_result.c`) sets `BAI_RESULT_STATUS_TIMEOUT` when its time limit
  passes. A new `BAI_RESULT_STATUS_MOVEGEN_BUDGET` sits beside it.
- Inference (`inference.c`), the endgame solver (`endgame.c`, its soft and hard
  time limits) and the pre-endgame solver (`peg.c`, `peg_should_stop`) each
  check their own stop condition in their worker loops.

New: a small `MovegenBudget` (an atomic count spent and a limit), created for
each move a player chooses and passed to whatever searches for it. Each worker
charges it the generations one unit of work cost (a rollout, an inferred rack,
a batch of solver nodes) and every stop check also asks whether it is spent.
Charged per unit rather than per generation, so the hot path stays a local
increment: a move may run up to one unit per thread past the budget, never
short of it.

Where a move is chosen: autoplay's turn for a player (`autoplay.c`, which fills
each player's simulation settings from `config->p1_*`/`p2_*` and runs the
solvers in `autoplay_solvers.c`), the time-control chooser
(`play_chooser_choose_move`), and an opening-rack task's analysis of one rack.
Each gets one budget for the whole move: inference, simulation and solving draw
on the same count.

## Decisions

Each has a recommendation. Confirm or override before the phase that needs it.

| # | Question | Recommendation |
|---|---|---|
| D1 | MAGPIE's argument names. `-movegenlimit` is already the whole-command cap, and per-player settings are `<name>1`/`<name>2` with a bare form for both (`-tl1`, `-tl2`, `-tlim`). | `-movebudget <n>` for both players, `-mb1 <n>` and `-mb2 <n>` for one. Help text: "the move generations a player may spend choosing one move: its inference, simulation and solving together; 0 is no budget". |
| D2 | birdtest's name for it: the column, the JSON key MAGPIE reads, and the settings row's label. | Column and key `movegens_per_move` (nullable, positive); label **Movegens per Move**. Null means no budget, as `max_iterations` null means MAGPIE's default. |
| D3 | A static player that never solves cannot spend more than one generation a move. Accept a budget on it anyway (as agreed), refuse it as birdtest refuses simulation settings on a static player, or accept it and show it grey? | Accept it, and show the row grey for a player whose budget can never bind (static and no endgame solving), as the card already greys settings a job does not read. |
| D4 | Does the budget replace the iteration budget birdtest requires of a simmer (`admin.rs`: "a simming player must set an iteration budget")? | No: both apply, whichever ends the simulation first. A simmer still needs an iteration budget, so nothing changes for configs without a movegen budget. |
| D5 | Does a budget make games repeat exactly? | No, and the help text should not suggest it: with several threads the order rollouts finish in still varies. It makes the work the same on any machine, which is what birdtest needs. |

## Phase 1 — MAGPIE: the budget (D1, D5)

1. `src/ent/movegen_budget.h`: `MovegenBudget` with `create(limit)`,
   `charge(n)` (atomic add), `spent()`, `exhausted()`. A limit of 0 is never
   exhausted. Plus `gen_get_thread_movegen_count()` in `move_gen.c`.
2. Simulation: `BAIOptions` gains `MovegenBudget *movegen_budget`; each worker
   charges a rollout's cost after it; `bai_result_set_and_get_status` returns
   the new status when the budget is spent. `sim_args_fill` takes it.
3. Inference, endgame and pre-endgame solving: each takes the budget, charges
   per unit of work, and stops on it as on its deadline. A solve stopped by the
   budget returns its best so far, as one stopped by time does.
4. The per-player setting: `p1_movegen_budget`/`p2_movegen_budget` in the
   config, the arguments of D1 in `config.c`'s five places (token, name, help
   text in a help group, settings string, loading), and the default 0.
5. The move's budget: created where a move is chosen (autoplay's turn, the
   play chooser, an opening-rack analysis), from that player's setting, and
   passed to inference, simulation and solving.
6. Tests: a simmer with a small budget stops at it (its generations for the
   move at least the budget and at most one rollout per thread past it), and
   plays a legal move; a budget of 0 changes nothing (a fixed-seed static run
   unchanged); a solver stops on the budget and returns a move.

**Verify:** the new tests and the existing `sim`, `infer`, `endgame`, `peg`,
`autoplay` and `contribute` suites on `BUILD=test_no_pgo_release` (asserts on); a compile check
with `BUILD=dev`; the include-cycle lint. `-j3`, `taskset -c 0-3`, 10 minutes
a suite.

## Phase 2 — the contract: MAGPIE reads the player's budget (D2)

1. `contribute.c`: read `movegens_per_move` from each player in an assignment,
   as `time_limit_secs` is read (`config.c` around line 9200): null or absent
   is no budget, a positive integer is the budget, anything else is the
   server's error. Into `-mb1`/`-mb2` for the task.
2. The key joins the list a player must state ("a player must state every
   setting", `test_a_player_must_state_every_setting`).
3. Fixtures: the hand-written `assignment-*.json` in birdtest's
   `contract-fixtures/` gain the key (null in all but one, which sets a budget);
   the captured ones are recaptured with `scripts/e2e_magpie_native.sh --cases
   capture --capture-out contract-fixtures` and copied to MAGPIE's
   `test/birdtest_contract/`.
4. `MAGPIE_VERSION` / `MIN_MAGPIE_VERSION` stay at 0.1.1 (pre-launch): an old
   build refuses the new key, which is what "must state every setting" is for.

**Verify:** MAGPIE's `contribute` suite against the new fixtures.

## Phase 3 — birdtest backend (D2, D3, D4)

1. `backend/migrations/0001_initial.sql`: `player_configs.movegens_per_move
   BIGINT NULL CHECK (movegens_per_move > 0)`, edited in place. **The database
   is reset on deploy** (and `dev.py --reset-db` locally).
2. `models/job.rs`, `jobs/handler.rs` (the player spec), `routes/public.rs` (the
   config read), `routes/admin.rs` (create: bind it; refuse 0 or a negative; no
   simming requirement, per D3).
3. The assignment carries it to MAGPIE under the key of D2.
4. Tests: create with and without a budget; 0 and -1 refused; the assignment
   carries it; the contract-fixture test against phase 2's fixtures.

**Verify:** backend tests (`cargo nextest`, throwaway Postgres and MinIO, `-j 2`).

## Phase 4 — birdtest frontend and docs (D2, D3)

1. `lib/api.ts`'s `PlayerSettings` gains the field; the new-player-config form
   a "Movegens per Move" input (blank is none), with the help text of D1.
2. `lib/jobSettings.ts`: a **Movegens per Move** row ("—" when unset) after
   **Maximum Total Iterations**, and greyed (D3) for a player that can never
   spend it; `playerSummary` adds "N movegens a move" when set.
3. Docs: PLAN.md's player settings table and the assignment schema,
   SETTINGS_COMPARISON.md's player rows, JOURNEYS.md (the config form and the
   settings card), TESTING.md (the new tests).

**Verify:** vitest, svelte-check, tier 5 (E-14 reads a config's settings), and
tier 6 with a new case: a games job between a simmer with a budget and one
without, whose budgeted player's tasks report fewer movegens a game.

## Verification summary

| Phase | Unit | Backend | e2e (tier 5) | MAGPIE | Tier 6 |
|---|---|---|---|---|---|
| 1 | — | — | — | new + sim, infer, endgame, peg, autoplay, contribute | — |
| 2 | — | — | — | contribute with new fixtures | capture |
| 3 | — | nextest | — | — | — |
| 4 | vitest, svelte-check | — | full | — | full, plus the new case |

## Not in this plan

- A movegen budget for a whole task (the job-level option from
  2026-10-11's D5).
- Showing each move's movegens on a saved position.
