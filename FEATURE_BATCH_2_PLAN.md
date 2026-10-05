# Feature batch plan — October 2026

Implementation plan for the second feature batch: job-page clarity, the
positions board, removing redundancy, opening-rack consensus, bulk allocation,
and word info tables on by default. The requests are reordered into phases by
dependency; [Request → phase](#request--phase) maps each one back.

**Status: implemented (October 2026)**, with every recommendation under
[Decisions to confirm](#decisions-to-confirm) taken. Two departures from the
text below: the pair-outcome table has three rows per player (won both, won
one and drew one, even) rather than five, since player 2's "lost both" is
player 1's "won both" and a loss row would make the higher count the worse
one; and an opening-rack export line carries its rack's consensus standing
rather than a separate progress file.

## Conventions that apply to every phase

- **Schema changes edit `backend/migrations/0001_initial.sql` in place** (one
  migration until release). Dev databases need `dev.py --reset-db` after 1.3
  and phases 3, 4 and 5.
- **Nothing is in prod:** no version bumps, floors or compatibility shims. A
  change to the worker contract lands in MAGPIE and birdtest together, with
  both contract fixtures (`contract-fixtures/` and MAGPIE's
  `test/birdtest_contract/`).
- Each phase updates `PLAN.md`, `JOURNEYS.md` and `TESTING.md` (IDs and test
  counts) alongside the code.
- New or changed routes go in the route table in `backend/tests/authz.rs`, or
  `the_route_table_is_every_route_the_router_serves` fails.
- Admin writes record an audit row in the same transaction
  (`backend/src/audit.rs`).
- The e2e suite pins the stats row: `e1-anonymous-browsing.spec.ts:38` expects
  the `.grid > .card` labels `Status, Allocation, Tasks completed, Estimated
  time left`, and `e10-phone-width.spec.ts:68` expects exactly four of them.
  Phase 2 changes both.
- Verify with the local recipe, with capped build parallelism. Don't run
  `cargo fmt`.

## Request → phase

| # | Request | Phase |
|---|---|---|
| 1 | Make "Available / Claimed / Completed" clear | 2.2 |
| 2 | Status card on its own row, with context and the completion message | 2.1 |
| 3 | Leave-generation targets list is ambiguous | 1.2 |
| 4 | Split Settings into job and player cards | 2.3 |
| 5 | Remove redundancy from games and game pairs | 3 |
| 6 | Show the move played from a position on the board | 5.1 |
| 7 | Remove the CGP text under the board | 1.1 |
| 8 | Match score as a two-player table | 2.4 |
| 9 | One board for a game pair's position | 5.2 |
| 10 | Explain the SPRT's LLR and bounds | 2.5 |
| 11 | Pentanomial table with players as columns | 2.6 |
| 12 | Opening-rack consensus percentage instead of redundancy | 4 |
| 13 | Admin page to set several jobs' allocations at once | 6 |
| 14 | Word info tables on by default | 1.3 |

---

## Phase 1 — Small, independent changes

### 1.1 Remove the CGP text under the board (#7)

- `frontend/src/lib/components/PositionPane.svelte:58-60`: delete the
  `<p … title="CGP">{position.position}</p>` block, and fix the doc comment at
  :3 that mentions it.
- The CGP stays in the API response and the exports; the board still parses it.
- Docs: `JOURNEYS.md` V-10 says "the CGP under the board" (around :268).
  Reword it.

### 1.2 Leave-generation targets that don't read as one list of numbers (#3)

`'100, 200, 500, 1,000, 1,000, 1,000'` uses the comma both as the thousands
separator and as the list separator.

- **Settings row** (`frontend/src/lib/jobSettings.ts:196-201`, "Target per
  rack"): join with arrows, `100 → 200 → 500 → 1,000 → 1,000 → 1,000`. The
  order is the order of the generations, which the arrow says.
- **Leave card** (`frontend/src/routes/jobs/[id]/+page.svelte:248-287`):
  replace the "Targets by generation: …" sentence (:255-261) with a small table
  — Generation | Target per rack | State (closed / current / to come) — built
  from `target_rack_counts` and `current_generation`. That also gives the card
  per-generation context it lacks today.
- **Creation form**: `parseTargetRackCounts` (`frontend/src/lib/format.ts`
  ~:229) splits on commas, so typing `1,000` is ambiguous there too. Accept
  spaces as separators as well, refuse a group of exactly three digits after a
  comma ("did you mean 1000?"), and change the placeholder to `100 200 500 1000`.
- Tests: `jobSettings.test.ts` (the target row), a `format.test.ts` case for
  the parser.

### 1.3 Word info tables on by default (#14)

`use_wit` is a **player-config** setting, not a job setting. "On for all jobs"
means every new player config asks for one unless it opts out.

- Schema: `player_configs.use_wit BOOLEAN NOT NULL DEFAULT true`
  (`0001_initial.sql:596`).
- `backend/src/routes/admin.rs:933-936`: `body.use_wit.unwrap_or(true)`, and
  update the comment (it currently explains why it is opt-in).
- Frontend form: `frontend/src/routes/admin/player-configs/new/+page.svelte:40`,
  `let useWit = true`.
- `scripts/seed.py` `player_config()`: send `use_wit` explicitly (true), as it
  sends `use_wordmap` and `use_rit`.
- No constraint refuses it: leave generation is allowed one
  (`backend/src/derived.rs:108-138`). The server builds one table per lexicon
  before any job that uses it dispatches (about 3 s and 122 MB for CSW24), so
  the derived-file builder becomes part of every fresh dev start.
- Tests: `e2e/tests/e4-live-dashboard.spec.ts:7,19` asserts the box starts
  unchecked; flip it. `e2e/lib/api.ts:88` sends `use_wit: false` and can stay,
  or drop the field to exercise the default. Check `backend/tests/derived.rs`
  for configs that assumed no table.

---

## Phase 2 — Job page layout (frontend, plus one stats field)

Pages: public `frontend/src/routes/jobs/[id]/+page.svelte` and admin
`frontend/src/routes/admin/jobs/[id]/+page.svelte`. Shared components live in
`frontend/src/lib/components/`.

### 2.1 Status on its own row, with its context (#2)

Today `JobStatsRow.svelte` is one grid of four cards (Status, Allocation, Tasks
completed, Estimated time left), and `CompletionNote.svelte` renders a tinted
box *above* that grid (public :102/:104, admin :406/:407).

- New `JobStatusCard.svelte`: one full-width card holding the badge and one or
  two sentences of context. It absorbs `CompletionNote.svelte`, which is then
  deleted.
  - **active:** "Workers are being offered its tasks: N% of claims."
  - **inactive:** "Paused: no tasks are offered. An admin can activate it."
  - **completed:** "Finished {date}: {completionText(stats)}." (the existing
    `completionText`, `frontend/src/lib/format.ts:183-221`).
  - Games and pairs jobs with a test add the SPRT's state in a few words
    ("the SPRT is still running"), linking to the SPRT card.
- `JobStatsRow.svelte` keeps three cards: Allocation, Tasks completed,
  Estimated time left.
- Both pages: `JobStatusCard` then `JobStatsRow`.
- e2e: `e1` (:38) now expects three labels plus a separate status card;
  `e10` (:68) counts three cards. `data-testid="job-status"` moves to the new
  card.

### 2.2 Make the task counts clear (#1)

"Available / Claimed / Completed" (public page Progress card,
`jobs/[id]/+page.svelte:135-140`; admin one-liner at :630-633) count **task
rows**, not games, pairs or racks (`backend/src/jobstats.rs:454-464`). Tasks
are created on demand when a worker asks, so "Available" is not the work left.

- Put them under a "Tasks" subheading and state the unit in it: "A task is one
  batch of N games / N pairs / N racks / one leave-generation batch handed to a
  worker." (The batch size is in the config.)
- Rename and explain each:
  - **Waiting to be reissued** (was Available): "created, then given back — its
    worker's claim lapsed or was declined — and offered to the next worker
    before any new task is made."
  - **In progress** (was Claimed): "held by a worker now."
  - **Done** (was Completed): "returned and accepted."
  - Drop **Created**, or keep it as "Made so far", since it is the sum of the
    three.
- After phase 3 a task has one slot, so these meanings are exact; until then
  "Waiting" also includes tasks with redundancy left to fill. Write the text for
  the phase 3 meaning and land 2.2 after phase 3 if the gap matters.
- Admin page: the same wording on its one line.

### 2.3 Settings as two cards: job, then players (#4)

Today `JobSettings.svelte` is one card with one "All settings" toggle that
expands both the job table and `PlayerSettingsTable`.

- Split into **Job settings** and **Player settings** cards, job first. Each
  card has its own "All settings / Key settings only" toggle, so either can be
  fully expanded on the page.
- The "Download every setting as JSON" link goes under the job card.
- Player names already link to `/player-configs/{id}`
  (`PlayerSettingsTable.svelte:49`). Keep them, and add "Open config →" under
  each column header so the link is obvious.
- `jobSettings.ts` already separates `jobSettings()` from `PLAYER_ROWS`; this is
  a presentation change only.
- The player config page (`frontend/src/routes/player-configs/[id]/+page.svelte:59`)
  reuses `PlayerSettingsTable`, so check it still renders with no role header.

### 2.4 Match score as a table (#8)

Today `MatchScore.svelte` shows a `<dl>`, an average-score line, the
`OutcomeChart` bar chart, and a W/L/D percentage line.

- Replace everything with one table: a column per player (the name, linking to
  the config), and these rows:
  - Wins
  - Draws (equal for both, so never coloured)
  - Score (W + ½D) / games
  - Score % (`m.scorePct`, and its complement for player 2)
  - Average score per game
  - Average spread (player 2's is the negation)
- Per row, the higher value is green and the lower red. Equal values stay
  neutral. Use colour plus weight or an arrow, so the result doesn't depend on
  colour alone.
- Pairs jobs keep the note "per game, over both games of every pair".
- Delete `OutcomeChart.svelte`, `Bars.svelte` and `AxisY.svelte` if nothing
  else uses them. `layercake` is used only by `OutcomeChart.svelte`, so remove
  it from `frontend/package.json` too.
- Factor the "two players, higher green, lower red" table into a component
  (`PlayerCompareTable.svelte`) that 2.6 reuses.

### 2.5 Explain the SPRT's LLR and bounds (#10)

`SprtCard.svelte` prints "LLR x, bounds [lo, hi]" with no explanation.

- Add a short paragraph under the figures, worded for the job's unit, with the
  job's own numbers filled in:
  - "The test weighs two hypotheses: player 1 is {elo_low} Elo stronger (H0)
    or {elo_high} Elo stronger (H1). Each completed pair is one observation,
    scored by player 1's result across its two games (0, ½, 1, 1½ or 2)."
  - "The LLR (log-likelihood ratio) is the evidence so far: positive favours
    H1, negative favours H0."
  - "The test passes when the LLR reaches the upper bound {hi} =
    ln((1−β)/α) and fails when it reaches the lower bound {lo} = ln(β/(1−α)),
    with α = {α} and β = {β} the accepted error rates."
  - Games jobs use the per-game version (each game scores 0, ½ or 1).
- A progress bar from lo to hi with a marker at the LLR shows where the test
  stands at a glance (plain markup, not a chart library).
- Keep it collapsible ("What do these numbers mean?") so the card stays short
  for people who know.

### 2.6 Pentanomial table with players as columns (#11)

Today the table (`SprtCard.svelte:57-92`, rows from
`frontend/src/lib/charts/pentanomial.ts:10-16`) has columns Pair outcome |
Pairs | Share, with player 1's view in the labels.

- Use `PlayerCompareTable` from 2.4: a column per player, each cell
  "count (share%)", rows from that player's side:
  - Won both
  - Won one, drew one
  - Even (split 1-1, or drew both)
  - Lost one, drew one
  - Lost both
- Player 2's column is player 1's mirrored (P1 lost both = P2 won both).
- Per row, higher green, lower red, equal neutral. The "Even" row is always
  equal.
- `pentanomial.ts` gains the mirrored rows. Update its unit tests.

---

## Phase 3 — Remove redundancy (#5)

Recommended scope: **drop `jobs.redundancy` entirely, for every job type.**
Games, pairs and leave generation would no longer use it, and phase 4 replaces
it for opening racks with per-rack consensus. A task then has exactly one slot,
and completes when its one result is accepted.

**What is lost:** PLAN.md:162 planned to use redundant claims as an integrity
cross-check for static jobs, by comparing the copies for equality. Removing the
column removes that future option. Record the decision in PLAN.md; it could
come back later as sampled audit tasks.

### Backend

- Schema: drop `jobs.redundancy` (`0001_initial.sql:407-408`). Rewrite the
  comments at :493-495, :884-888, :1157, :1221-1236 and :1339-1343.
  - Keep `tasks.accepted_count` / `active_claim_count` for now; they become
    0/1.
  - Simplify later only if it is cheap. The four state formulas would all
    become "accepted → completed".
- The four places that compute task state switch from `j.redundancy` to `1`:
  - `scheduler.rs:486-487` (lapse)
  - `scheduler.rs:1217` (claim)
  - `scheduler.rs:1355-1356` (release)
  - `routes/worker.rs:811-831` (submit)
  - Keep them in step, as the comment at scheduler.rs:1324 asks.
- `routes/admin.rs`:
  - Remove `CreateJobBody.redundancy` (:1176-1177) from the INSERT (:1404,
    :1410), and the validation at :1527-1535.
  - Unit tests :3930-3932 and :4010-4020.
  - A body that still states `redundancy` is refused as an unknown field, if
    the body denies unknown fields; otherwise it is ignored. Check which.
- `models/job.rs:33`, `jobstats.rs` (`JobSummary.redundancy` :87/:519, and
  the ETA's `/ redundancy` at :1016-1043), `routes/public.rs` (:64, :90,
  :172, :238, :506).
- "First result per task" logic (`jobstats.rs:12-31`, `ratings.rs:120-130`,
  `registry.rs:620-658`, `leave_gen.rs:150-160` `credit_claim`) becomes
  trivially true. Leave it in place or simplify it; don't rewrite the
  aggregates in this phase.
- `registry.rs` `next_available` (:154-186) keeps its same-identity exclusion,
  which now matters only for lapsed tasks.

### Frontend

- New-job form: remove the field (`admin/jobs/new/+page.svelte:18, :112-113,
  :241-242`).
- `jobSettings.ts:215` row and :69 type; `jobSettings.test.ts:31, 80, 112, 117`.
- Jobs list column (`routes/jobs/+page.svelte:63, :84`).
- `api.ts:211, 231, 307`.

### Tests, scripts, docs

- `backend/tests/common/mod.rs:433` `games_job(redundancy, …)` and :455
  `bare_job(job_type, redundancy, …)` have ~265 call sites. Drop the argument
  mechanically (a scripted edit). Then delete or rewrite the tests that exist
  to exercise redundancy above 1:
  - `scheduler.rs` :148-192, :536-579 (I-SCHED-13), :608-612 (I-SCHED-14),
    :750-762
  - `worker_api.rs` :94-101, :284-289, :320-331, :380-394, :439, :954-1006,
    :1149-1154
  - `submissions.rs` :219-223
  - `stats.rs` :522-528
  - `boundaries.rs` :830-854
  - `ratings.rs` :415-487
  - `leave_gen.rs` :922-929
  - `jobs.rs` :168, :192, :1000
  - `derived.rs` :53
  - Renumber in TESTING.md.
- `scripts/seed.py:456, :696` (`--redundancy`); `scripts/restore-roundtrip.sh:84`,
  `scripts/restore-job-check.sh:106, :149` (raw INSERTs);
  `e2e/lib/api.ts:104`; `worker/fake_worker.py:15`.
- PLAN.md (key passage :102, ~60 mentions), TESTING.md (~25), RUNBOOK.md,
  JOURNEYS.md, SETTINGS_COMPARISON.md.

---

## Phase 4 — Opening-rack consensus (#12)

A rack keeps being analysed until enough analyses agree on its best move.

- **Consensus** for a rack = (number of its results whose rank-1 move is the
  most common rank-1 move) ÷ (number of its results).
- A rack is **settled** once it has at least `min_results_per_rack` results
  and its consensus is at least `consensus_pct`, or once it reaches
  `max_results_per_rack`. In the second case it is settled *without* consensus,
  and the page counts those separately.
- A settled rack is never issued again.

### Job settings

- `job_opening_rack_config` (`0001_initial.sql:684-698`) gains:
  - `consensus_pct NUMERIC` (50 < x ≤ 100)
  - `min_results_per_rack INT` (≥ 1)
  - `max_results_per_rack INT` (≥ min)
- With min = max = 1 the job is today's job: one analysis per rack. That is
  the default, and the only setting allowed for a **static** player, whose
  analyses are deterministic and always agree. Job creation refuses a static
  player with min > 1 (see [Decisions to confirm](#decisions-to-confirm)).
- `admin.rs` `CreateJobBody`/validation (~:1624-1634) and the new-job form;
  `jobSettings.ts` rows; `seed.py`'s dev opening-rack jobs (a simming one could
  use 80% / 3 / 7).

### Per-rack state

- New `opening_rack_progress(job_id, rack TEXT, results INT, top_move TEXT,
  top_count INT, settled BOOL, settled_without_consensus BOOL, PRIMARY KEY
  (job_id, rack))` and a pick index on `(job_id, settled, results)`.
  - Keyed by rack text, as `leave_rack_progress` is. There is no rack → index
    inverse in `RackIndex` (`backend/src/jobs/racks.rs:361-469`).
  - It needs per-move counts to find the top move, so either keep a child table
    `opening_rack_move_counts(job_id, rack, move, count)`, or recompute from
    `position_analysis_records` ⋈ `position_analysis_moves WHERE rank = 1`
    for the racks a submission touched (bounded by `max_results_per_rack`
    rows per rack, so cheap).
- Update it in the submit transaction for the batch's racks. A task touches
  `racks_per_batch` rows; if that becomes a hot spot under load, switch to the
  leave-generation pattern (`leave_rack_staging` + `merge_staged`,
  `leave_gen.rs:358-547`).

### Dispatch: a tiled first pass, then a tail of reissued racks

1. **First pass (unchanged):** contiguous slices of the scattered rack space,
   `seed` = index of the first rack, cursor `MAX(seed) + racks_per_batch`
   (`opening_rack.rs:209-251`). Every rack gets its first result here.
2. **Tail:** once the cursor reaches `total_racks`, `next_request` picks up to
   `racks_per_batch` unsettled racks that are not in flight, fewest results
   first. It issues them as an **explicit rack list** with a fresh random seed
   (`rack i` uses `seed + i`, as today, so the worker contract is unchanged:
   the request already carries `racks`).
   - `opening_rack_requests` (`0001_initial.sql:994-1009`) gains
     `racks TEXT[]` (NULL for a range task), like `leave_requests.forced_racks`.
   - `load_request` and `check_batch_against_task` (`opening_rack.rs:17-51,
     144-190`) read the list when present.
   - Tail tasks need seeds outside the tiled range, because of the
     `(job_id, seed)` unique index (`0001_initial.sql:875`). Use a separate
     tail counter starting at `total_racks`, or random seeds with a retry on
     conflict.
   - With `min_results_per_rack = 1` and consensus met trivially, the tail is
     empty and the job behaves as today.
3. **Same-worker exclusion:** today's exclusion is per task. A tail task must
   not hand a worker a rack it has already analysed, or consensus counts one
   worker twice. Filter candidates with `NOT EXISTS` over that rack's records
   joined to `task_claims` for this identity. If that is too slow, accept
   per-task exclusion and record the limitation.
4. **Finish condition** (`routes/worker.rs:1168-1186`): the cursor is
   exhausted, every rack is settled, and nothing is in flight.

### Stats and UI

- `jobs.racks_analyzed` keeps meaning "racks with at least one result".
  `OpeningRackStats` (`jobstats.rs:804-824`) adds `racks_settled`,
  `racks_settled_without_consensus`, and the results-per-rack distribution.
- The progress bar becomes "racks settled / total". The ETA uses the remaining
  unsettled racks × the expected results per rack.
- Opening racks card: per rack, show the consensus ("TOP MOVE in 4 of 5
  analyses, 80%"). The `?rack=` lookup (`public.rs:1053-1064`) already returns
  every record for a rack.
- Exports: add `opening_rack_progress` to the export, so a rack's settled
  answer doesn't have to be recomputed.

### Tests

- Tests for: consensus reached at min; consensus never reached, so the rack
  stops at max; a static player refused above 1; tail tasks never repeat a
  rack to the same identity; the finish condition; and a lapsed tail task is
  reissued with the same rack list.

---

## Phase 5 — Positions: the move played, and one board per pair

### 5.1 Show the move that was played (#6)

The move played *from* a position is not stored anywhere today.
`position_analysis_records` has `previous_move` only (`0001_initial.sql:1196-1199`),
and with first-divergence capture the next turn's row (which would hold it) is
never kept. MAGPIE has the move in hand when it captures
(`/home/josh/MAGPIE/src/ent/autoplay_results.c:1735ff`,
`positions_data_add_move`, "args->move has been chosen but not yet applied"),
but never writes it out.

- **MAGPIE**:
  - Add `played_move` / `played_move_score` to `CapturedPosition`
    (`autoplay_results.c:1532-1553`).
  - Fill them in `positions_data_add_move` with `move_get_string(args->move,
    …)`, as `previous_move` is filled.
  - Write them in `write_captured_position` (:1934-1977).
  - Add the new keys in `src/def/contribute_defs.h`.
  - Update the contract fixtures (`test/birdtest_contract/result-games.json`,
    `result-game-pairs.json`).
  - Opening-rack records don't need it, since an opening analysis plays
    nothing.
- **Backend**:
  - `CapturedPosition` (`backend/src/jobs/handler.rs:403-420`): add a
    **required** `played_move` / `played_move_score` (not
    `#[serde(default)]`; nothing in prod).
  - Columns on `position_analysis_records`.
  - `PositionAnalysis` (:502-519), `game.rs:59-82`, the INSERT in
    `jobs/mod.rs:541-552`.
  - Plausibility: same check as `previous_move` in `plausibility.rs:187-207`.
    The played move should normally appear in `moves`; don't require it,
    because a simmer's pick isn't always rank 1.
  - `public.rs` `POSITION_COLUMNS` / `saved_positions()` (:1100-1173), and the
    `SavedPosition` type (`frontend/src/lib/api.ts:118-140`).
- **Board** (`frontend/src/lib/components/Board.svelte`): two highlights.
  - **Previous move** (already on the board): keep today's fill and stroke
    (:84-86), and give it the legend entry "previous move".
  - **Move played** (not yet on the board): draw its newly placed tiles on the
    empty squares as distinct tiles, with a different fill and a dashed or
    contrasting outline, and the legend entry "played here".
    `parseMove(text).squares` (`frontend/src/lib/cgp.ts:133-190`) already gives
    the row, column and letter. Skip `through` squares; those tiles are already
    there.
  - An exchange or pass draws nothing; the summary line says "played
    (exch …)" / "passed".
  - Update the aria-label (:64).
- **Moves table** (`PositionPane.svelte:63-95`): mark the played move's row
  ("played" badge, row tint), matched by move text.
- **Summary line** (`PositionPane.svelte:44-47`): "after PREV (n) · played MOVE
  (n)".

### 5.2 One board for a game pair's position (#9)

At a first-divergence position the two games have the same board and the same
rack. The server already checks this (`backend/src/jobs/game_pair.rs:139-198`);
only the racks and scores in the CGP are in each game's own seat order.

- `SavedPositions.svelte:162-172`: when the two positions' boards match (always
  for first-divergence capture), render **one** board. Beside it, render the
  two ranked move lists side by side, headed by player name, each marking its
  played move.
- The board draws both played moves, one colour per player and keyed to the
  column headers. If the two moves overlap squares, show one at a time with a
  "Show: P1's play / P2's play / both" toggle, defaulting to both when they
  don't overlap.
- Racks and scores are shown once, labelled by player name rather than by
  seat. Work out the mapping from `game_index` (the seats swap between a pair's
  games).
- A pairs job that saves *every* turn can have different boards at the same
  turn (the games diverged earlier). Fall back to two boards there, as today.
- Captions: "Turn N of a game pair, where the players first chose differently"
  stays.
- Docs: JOURNEYS V-10 (the two-board description around :262-275).

Depends on 5.1: without the played move there is nothing to show on the board
beyond the previous move.

---

## Phase 6 — Set several jobs' allocations at once (#13)

Today the only way to change an allocation is `POST
/api/admin/jobs/:id/activate {allocation}` (`backend/src/routes/admin.rs:2196-2293`).
It refuses any value that would put the active total over 100, so moving 20%
from job A to job B takes two steps in the right order.

- **Route**: `PUT /api/admin/jobs/allocations` with body `{ "allocations": [{
  "job_id", "allocation" }] }`.
  - Under the same advisory lock (`hashtext('birdtest.activate')`, :2237), it
    applies every change and validates the **resulting** total of active jobs
    (≤ 100).
  - It refuses completed jobs, unknown ids and values outside 0..=100, and
    reports every problem at once with per-row field errors.
  - Each changed job gets `scheduler::join_at_parity` (:2271), as activation
    does.
- **Semantics** (recommended): the page edits active jobs. An inactive job
  listed with a value above 0 is activated by the same request. 0 leaves a job
  inactive, or deactivates it. That makes the page the one place to rebalance.
- **Audit**: one `job.allocation_changed` row per changed job, with the old and
  new allocation in `reason` (as `rating_pool.anchor_changed` does). Today
  `job.activated` doesn't record the value at all. Add a case to
  `backend/tests/audit.rs`.
- **Tests**:
  - `authz.rs` route table entry: admin only, plus the CSRF pair.
  - Swap 60/40 to 40/60 in one request.
  - Refuse a request totalling 110.
  - Refuse a request that includes a completed job.
  - Two concurrent requests serialise under the lock.
- **Page**: `frontend/src/routes/admin/allocation/+page.svelte`, linked from the
  admin index and from each job's Controls card.
  - One row per non-completed job: name, type, status, current allocation, a
    number input.
  - A live total ("85% of 100%") that turns red above 100, with Save disabled
    while it is above 100.
  - A "Share equally" helper.
  - Saving sends everything in one request.
- `api.ts`: `setAllocations(rows)`. `seed.py` could use it to share out new dev
  jobs in one call instead of computing the free share (`scripts/seed.py:608-619`).

---

## Decisions to confirm

| Topic | Recommendation | Why it matters |
|---|---|---|
| Consensus settings | `consensus_pct` **plus** `min_results_per_rack` and `max_results_per_rack`. The request read "instead of redundancy, min redundancy and max redundancy"; no min/max redundancy exists today | Without a minimum, a rack's first result is 1/1 = 100% and nothing is ever reissued. Without a maximum, a rack whose simmers split can be reissued forever |
| Static opening-rack players | Only min = max = 1 | Static analysis is deterministic, so more results always agree and only cost compute |
| Redundancy scope | Drop the column for every job type (phase 3), with opening racks moving to consensus | The request says redundancy only makes sense for opening racks, and #12 replaces it there too. Dropping it gives up the planned integrity cross-check (PLAN.md:162) |
| Bulk allocation and inactive jobs | The page can activate (value > 0) and deactivate (0) as well as rebalance | Otherwise rebalancing toward a paused job still takes two pages |
| Word info table default | On for new player configs; existing configs untouched (reset dev DBs) | It is a player setting, not a job setting |
| Pair board with overlapping plays | A toggle between the two players' plays | Two moves on the same squares can't both be drawn |

## Suggested order and resets

1. Phase 1 (independent; no reset except for 1.3's schema default).
2. Phase 2 (frontend only, plus e2e updates).
3. Phase 3 (schema; **reset**).
4. Phase 4 (schema; **reset**). Depends on phase 3.
5. Phase 5 (schema plus the MAGPIE contract; **reset**, rebuild MAGPIE and the
   backend image together).
6. Phase 6 (new route; independent, and can go any time after phase 3 so the
   page never shows redundancy).
