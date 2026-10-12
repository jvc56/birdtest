# Updates plan — the 2026-10-11 batch

Source: `~/Dropbox/birdtest_todo.txt` (2026-10-11), five items.

**Branches.** The birdtest items are further commits on `better_scorecards`,
on top of `e840e20` (five commits ahead of `origin/better_scorecards`: the
Progress, Player settings, Ratings, board and status-pill changes). The MAGPIE
items are commits on `~/MAGPIE`'s `birdtest-contribute`, on top of `4908a0c8`.

The five items are grouped into four phases. Each is its own commit and leaves
its branch green.

| Phase | Items | Repo | Deploy |
|---|---|---|---|
| 1 | 4: fishtest, not fishnet | birdtest | none (docs and a comment) |
| 2 | 3: drop the test sentence; 1: link player configs in rating pools | birdtest | frontend only, no migration |
| 3 | 5: `magpie contribute` says how long each task took | MAGPIE | a MAGPIE build; no contract change |
| 4 | 2: a MAGPIE argument that caps a command's move generations | MAGPIE | a MAGPIE build; no contract change unless D5 says otherwise |

## Why this order

- **The wording fix goes first.** It touches only prose (README, PLAN, one
  Rust doc comment), so it cannot break anything. It also has the one passage
  that needs a decision before it is rewritten (D3).
- **The two frontend items ship together.** Both are small, both are on pages
  the last commits already changed, and one round of e2e covers them.
- **The MAGPIE items go last, smallest first.** Phase 3 is one print line and
  its test. Phase 4 is a new argument wired into the move generator and the
  thread-control status, and its meaning is still open (D4, D5).

## Rules carried over

- Pre-launch: no compat shims. No item here touches the schema, so no
  database reset.
- `MAGPIE_VERSION` / `MIN_MAGPIE_VERSION` stay at 0.1.1 (memory: nothing in
  prod). No fixture recapture: neither MAGPIE phase changes what crosses the
  wire, unless D5 decides birdtest sends the cap in its tasks.
- Builds stay light: cap `cargo`/`make` parallelism, no cppcheck or clang-tidy
  on MAGPIE, and ask before any Docker image build.
- No `cargo fmt` (it rewrites 75 files).

## Decisions

Each has a recommendation. Confirm or override before the phase that needs it.

| # | Question | Recommendation |
|---|---|---|
| D1 | Item 1: which config names on a pool's page become links? Names appear in the cross table's rows, the dot plot's labels, the admin's All configs table and the members list. You asked for the cross table to look as it does. | All four, each linking to `/player-configs/:id`. In the cross table and the dot plot the link keeps today's colour and is underlined only on hover, so neither looks different at rest. The two admin tables use the site's ordinary link colour. |
| D2 | Item 3: the sentence goes. Nothing else calls `testLabel` / `TEST_LABELS` in `lib/format.ts`, and `JobStatusCard`'s `test` variable goes with it. Remove them, or keep `testLabel` for later? | Remove them, and their `F-FMT-5` tests. The Significance Test card says where the test stands with its own pill and verdict tile. |
| D3 | Item 4: two passages, PLAN.md's "Why impossibility, and not per-worker anomaly detection" and `backend/src/jobs/plausibility.rs`'s module comment, say per-worker chi-square is "the one fishnet uses", justified by fishnet's replicated analysis. The chi-square residual check is fishtest's: fishtest flags workers whose results stray from the rest of the run. Renaming alone would leave a wrong rationale, since fishtest's check does not rest on replication. | Rewrite both passages around fishtest's per-worker residual check. State what it does, and why birdtest still declines it: every birdtest worker plays different seeds, and a residual cannot tell a broken worker from seeds that favoured one side. Keep the conclusion. If, on rewriting, the argument no longer holds against fishtest's version (fishtest has the same seed variety and uses the check anyway), say so in the plan's follow-ups rather than silently weakening the doc. |
| D4 | Item 2: what is the movegen cap for? A budget that makes a run's cost deterministic (for benchmarks or reproducible comparisons), or a safety stop? And what happens when it is reached? | A deterministic work budget for any command. New argument `-movegenlimit <n>` (`0`, the default, means no limit). When the command's move generations since it started reach `n`, the command stops as it does at a time limit: the run halts through `thread_control_set_status` with a new status, keeps what it has, and says it stopped at its movegen limit. Counted across all threads. |
| D5 | Item 2: does birdtest use the cap? It could put a per-job movegen budget in a task, as it does `max_task_seconds`. | Not in this batch. The argument is MAGPIE-side only, so nothing crosses the wire and no fixture changes. A task budget in movegens is its own item, with its own schema and contract change. |
| D6 | Item 5: what does the line say, and where? | Elapsed wall time from claim to submission, in the units birdtest's `computeTime` uses: `[14:03:07] finished task #3: dev pairs in 2m 13s (5 started, 4 completed)`. The same on the "stopped … at its limit" line, which already names the limit, and on "task failed". |

## Phase 1 — fishtest, not fishnet (item 4)

Every place that says birdtest is modelled after Fishnet says Fishtest, and
each description of what the model is matches it. Fishtest is Stockfish's
distributed testing framework: volunteers play engine-vs-engine test matches,
judged by a sequential test on game pairs. That is birdtest's own shape, where
Fishnet distributes Lichess's position analysis.

1. `README.md:3`: "modelled after Fishnet" → "modelled after Fishtest".
2. `PLAN.md:7`: "modeled after Fishnet (which crowdsources chess game analysis
   for Lichess)" → "modeled after Fishtest (which crowdsources test matches
   between Stockfish versions)".
3. `PLAN.md` "Why impossibility, and not per-worker anomaly detection" (around
   line 122) and `backend/src/jobs/plausibility.rs:5-20`: rewrite per D3.
4. Search again after editing: `grep -rni fishnet` over the repo (excluding
   `node_modules`, `target`, `.git`) returns nothing, and MAGPIE's checkout has
   none today.

The two existing mentions of fishtest (`PLAN.md:9816`, the "Possible Future
Improvements (from fishtest)" heading) are already right.

**Verify:** `cargo check` on the backend (a comment changed; `-j` capped), and
read the two rewritten passages.

## Phase 2 — the job and ratings pages (items 3 and 1)

### Item 3: drop the Status card's test sentence

`frontend/src/lib/components/JobStatusCard.svelte:38-40` prints "Its
significance test is … (see the Significance Test card)." beside an inactive
or set-aside job's status. Remove it.

- Per D2, remove `testLabel` and `TEST_LABELS` from `lib/format.ts`, their
  `F-FMT-5` tests in `format.test.ts` (and the `testLabel(testState(...))`
  assertion in the `testState` tests, which keep their other assertions), and
  the `test` variable and its imports in `JobStatusCard.svelte`.
- Docs: `JOURNEYS.md:150-156` (the Status card's inactive wording),
  TESTING.md's `F-FMT-5` entry and the frontend unit count, and PLAN.md
  wherever it quotes the sentence (the `/jobs/[id]` row says the inactive
  status card adds "where its significance test stands").

### Item 1: link player configs in the rating pools

Config pages are public (`GET /api/player-configs/:id`, `/player-configs/[id]`),
and every rated row and member carries its `player_config_id`. On
`routes/ratings/[id]/+page.svelte`, per D1:

- Cross table: each row's name (`<th scope="row">`) becomes a link that keeps
  today's look. The column heads are shortened labels and stay unlinked.
- Ratings dot plot (`RatingDotPlot.svelte`): each label becomes an SVG `<a>`
  to the config, with the same look at rest.
- All configs (admin): the name links.
- Members (admin): the name links.

**Verify:** `npx vitest run` and `npx svelte-check`; e2e tier 5 for E-7 (the
ratings journey). Add one assertion there that a config's name links to its
page, and update E-1's or E-7's status-card expectations if either quotes the
removed sentence.

## Phase 3 — `magpie contribute` says how long a task took (item 5)

In `~/MAGPIE/src/impl/contribute.c`:

1. Record the task's start when it is claimed, beside `state->deadline =
   contribute_deadline_start(...)`, from a monotonic clock, in a new
   `ContributeState` field.
2. A small formatter for the elapsed time in birdtest's `computeTime` units:
   "45s", "2m 13s", "1h 5m 2s".
3. `print_task_line` callers: "finished" gets " in <elapsed>" before its
   counts (D6). "stopped … at its N-second limit" and "task failed" get the
   same.
4. `test/contribute_test.c`: assert the finished line's shape and the
   formatter's units at 0 s, 59 s, 60 s and 3,661 s.
5. birdtest's contributor instructions quote no task lines today; check
   `routes/contribute*` and the F-DOCS tests anyway. `UPDATES_PLAN.md:207-208`
   records the old line format as history and stays as it is.

**Verify:** MAGPIE's contribute tests (`make` with `-j` capped), then a short
local `magpie contribute` run against the throwaway stack (the memory's
verification recipe) to see the line.

## Phase 4 — a MAGPIE argument that caps move generations (item 2)

The pieces exist. Each move generator counts its own generations
(`gen->movegen_count`, `move_gen.c:4000` and `:4020`). `gen_get_movegen_count()`
sums them across threads, and `config_contribute_execute` already measures a
task by its before-and-after difference. Commands stop mid-run through
`thread_control_set_status`, which autoplay, sim, inference and analyse all
check.

Per D4:

1. **The argument.** A new `ARG_TOKEN_MOVEGEN_LIMIT` in `config.c`'s argument
   table, `-movegenlimit <n>`, parsed into the config, with usage text and the
   same validation as the other non-negative integers. `0` means no limit.
2. **The count while running.** The per-generator counts are unsynchronized,
   so reading their sum mid-run is racy. Add a process-wide atomic counter of
   generations since the command started: each generator adds to it in
   batches (every 64 generations, say), so the hot path stays a local
   increment. A generator that takes it to the limit sets the new status.
3. **The stop.** A new thread-control status (e.g.
   `THREAD_CONTROL_STATUS_MOVEGEN_LIMIT`) beside the time-limit one. Every
   command that already halts on a time limit halts on it and reports it the
   same way: autoplay keeps the games it finished, sim keeps its current
   ranking, inference its leaves so far.
4. **Reset.** The counter is zeroed when a command starts, so a session's
   commands each get their own budget.
5. **Tests.** A command with a small limit stops with the new status, and its
   movegen count is at least the limit and at most the limit plus one batch per
   thread. A limit of 0 changes nothing.
6. **Docs.** MAGPIE's command help. birdtest's PLAN.md gets one line under
   MAGPIE's arguments only if D5 changes.

**Verify:** the new MAGPIE tests and the existing contribute and autoplay
tests (with `-j` capped), then tier 6 (the MAGPIE smoke run) once, since the
change touches the move generator every task runs through.

## Verification summary

| Phase | Unit | e2e (tier 5) | MAGPIE | Tier 6 |
|---|---|---|---|---|
| 1 | `cargo check` | — | — | — |
| 2 | vitest, svelte-check | E-1, E-7 | — | — |
| 3 | — | — | contribute tests | a local `magpie contribute` run |
| 4 | — | — | new and existing tests | yes |

## Follow-ups (not in this batch)

- A per-job movegen budget sent in tasks (D5), with its own schema, contract
  and fixture change.
- Revisit the worker-integrity argument if D3's rewrite finds that fishtest's
  residual check does transfer to birdtest.
