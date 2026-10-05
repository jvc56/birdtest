# Feature batch plan: settings wording, editable consensus, richer move lists, and a match test (October 2026)

This is the implementation plan for the third feature batch:
- consistent settings wording;
- a simming opening-rack dev job;
- opening-rack consensus settings that can be edited after a job is created;
- win %, sim ply stats and inference on saved move lists;
- replacing the SPRT with a test that answers "is one player better, at X%
  confidence?".

The requests are grouped into phases by dependency. The
[Request → phase](#request--phase) table maps each one back.

**Status: implemented (October 2026)**, with every decision under
[Decisions](#decisions) taken. Departures from the text below:

- **The match test's tuning has a planning variance.** The paper's ρ is
  tuned for a score of variance 1. Used as written on per-game scores (at
  most ¼) and pair scores (about 1/20), the interval's tightest point fell
  20 times past n\*, and early in a job it was too wide to decide even 90
  wins in 100 games. ρ² is now divided by `n*·v`, with v fixed in advance at
  ¼ for a game and 1/16 for a pair, which keeps the guarantee
  (`stats/match_test.rs`).
- **`templates.forget` is not called by the consensus edit.** Nothing reads
  the consensus settings from the cached template any more: claims and
  submissions read `ConsensusSettings::load` under locks that the edit holds.
- **An inference can list no leaves.** At the default inference margin of 0,
  a simmer's move is often one that no rack makes the static best. Such a
  position keeps an inference of 0 leaves, saying so. Tier 6's `M-16`
  checks this against a real MAGPIE.
- **The inference fixture is hand-written.** It is
  `contract-fixtures/result-games-inference.json`, in MAGPIE's key layout,
  and MAGPIE's own test checks that its output carries every key in it.
  This replaces adding a captured sim position to `result-games.json`.

## Conventions that apply to every phase

- **Schema changes edit `backend/migrations/0001_initial.sql` in place.**
  There is one migration until release. Dev databases need
  `dev.py --reset-db` after phases 3, 4 and 5.
- **Nothing is in prod.** That means no version bumps, floors or compatibility
  shims. The inference change to the worker contract (phase 4) lands in MAGPIE
  and birdtest together, along with both copies of the contract fixtures:
  `contract-fixtures/` and MAGPIE's `test/birdtest_contract/`.
- Each phase updates `PLAN.md`, `JOURNEYS.md` and `TESTING.md` (IDs and test
  counts) alongside the code.
- New routes go in the route table in `backend/tests/authz.rs`. Otherwise
  `the_route_table_is_every_route_the_router_serves` fails.
- Admin writes record an audit row in the same transaction
  (`backend/src/audit.rs`). New ones are added to
  `every_destructive_admin_action_writes_exactly_its_record`
  (`backend/tests/audit.rs`).
- Verify with the local recipe, with build parallelism capped. Don't run
  `cargo fmt`. Phase 4 changes MAGPIE too, so it also needs the MAGPIE suite
  and tier 6 (`scripts/e2e_magpie_native.sh`).

## Request → phase

| # | Request | Phase |
|---|---|---|
| 1 | dev.py's opening-rack job uses a simming player | 2.1 |
| 2 | Saved move lists show win %, up to two plies' stats, and inference | 4 |
| 3 | Opening-rack jobs show min and max analyses per rack and consensus % | 1.8 |
| 4 | Admins can edit those three after creation; the job starts or stops to match | 3 |
| 5 | Remove the "Racks in all" and "Rack size" rows | 1.8 |
| 6 | No spinners on number inputs | 1.1 |
| 7 | Rack info tables on by default for every job | 2.2 |
| 8 | "Plies recorded" is "—" for a static player | 1.5 |
| 9 | "Move-gen margin" is "—" unless the recorder keeps moves within an equity margin | 1.5 |
| 10 | Job types capitalised ("Leave Generation") | 1.6 |
| 11 | Every job and player setting name in Title Case | 1.3 |
| 12 | Remove the "A pair played identically comes out even…" sentence | 1.7 |
| 13 | Rename 18 settings | 1.3 |
| 14 | yes/no instead of on/off for flags | 1.4 |
| 15 | Replace the SPRT with a test that answers "A is better at X% confidence" | 5 |

---

## Phase 1: Settings wording and display (frontend only)

The changes are almost all in `frontend/src/lib/jobSettings.ts`, `format.ts` and
`app.css`, plus the two create forms.

### 1.1 No spinners on number inputs (#6)

There are 35 `type="number"` inputs, and one global rule in
`frontend/src/app.css` covers them all:

```css
input[type='number']::-webkit-inner-spin-button,
input[type='number']::-webkit-outer-spin-button { -webkit-appearance: none; margin: 0; }
input[type='number'] { appearance: textfield; -moz-appearance: textfield; }
```

The arrow keys still step the value, and `min`, `max` and `step` validation is
unchanged.

### 1.2 Give rows an id that isn't their label

Today a row's display label is also its identity:
- `SOLVER_ROWS`, `LEAVE_UNUSED`, `OPENING_RACK_UNUSED` and `UNCAPTURED_UNUSED`
  match on label text.
- `playerSummary` checks `unused.has('Inference')` and `unused.has('Endgame')`.
- `jobSettings.test.ts` asserts the label lists.

Give `PlayerRowSpec` and `JobSetting` a stable `id`, such as `num_plays`,
`endgame` or `max_units`. Key the "unused" sets and `playerSummary` on that id.
`SettingRow` carries both `id` and `label`. Do this first: it turns every
rename in 1.3 into a one-line change, and tests that check behaviour no longer
break when the wording changes.

### 1.3 Renames and Title Case (#11, #13)

**The requested renames:**

| Now | New |
|---|---|
| Cap (pairs) / Cap (games) | Maximum Pairs / Maximum Games |
| Games to play / Pairs to play (no test) | Games To Play / Pairs To Play |
| Records positions | Position Recorder |
| Fewest pairs / games before the test is acted on | Minimum Pairs / Minimum Games |
| Plays considered | Moves Generated |
| Sort | Sorted By |
| Iterations (most) | Maximum Total Iterations |
| Inference | Uses Inference |
| Recorder | Move Recorder |
| Plays recorded | Moves Recorded |
| Endgame | Uses Endgame |
| Pre-endgame | Uses Preendgame |
| Iterations per play (fewest) | Minimum Iterations per Play |
| Threshold | Stopping Threshold Rule |
| Utility weight: win % | Win % Utility Weight |
| Utility weight: spread | Spread Utility Weight |
| Utility spread scale | Spread Utility Scale |
| Time limit (s) | Time Limit (Seconds) |
| Move-gen margin | Movegen Margin |

**Every other row in Title Case:**
- Letter Distribution, Bingo Bonus, Sim Cutoff, Win % Model, Stopping %,
  Sampling Rule, Inference Margin.
- PEG Schedule, PEG Stride, PEG Opponent, Nested Lookahead, Nested Caps,
  Nested Depth, Nested Strides.
- Rack Info Table, Word Info Table, Plies Recorded.
- Games Per Task, Racks Per Task, Generations, Target Per Rack, Oldest MAGPIE.
- Type, Variant, Board, Lexicon, Leaves and Plies are single words and don't
  change.
- The SPRT rows (SPRT, SPRT α, SPRT β) are left alone here. Phase 5 replaces
  them.

**The create forms use the same wording (decision 4).** The labels on
`admin/jobs/new` and `admin/player-configs/new` take the same names. A field's
help text can still explain the setting in a sentence. `JobSettings.svelte`'s
"Settings in grey…" and "differ in bold" notes keep their current wording.

**Tests:**
- `frontend/src/lib/jobSettings.test.ts` (label lists).
- `format.test.ts`.
- e2e specs that match the old text: e1, e4, e7, e8, e10, e12, e14, e16, e17.
  Run `grep` again before starting; this list is from planning.

### 1.4 yes/no for every flag (#14)

`show()` already prints `yes` / `no`. These are the cases that still say "off"
or "none":
- `endgameText`: "6-ply endgame" or "off" becomes **"yes (6 plies)"** or
  **"no"**.
- `preEndgameText`: "bag ≤ 2" or "off" becomes **"yes (bag ≤ 2)"** or **"no"**.
- Position Recorder: `show(capture_positions)` already says yes/no; first
  divergences become **"yes (first divergences)"**.
- The SPRT row's "none" goes away with phase 5. Its replacement, "Significance Test",
  is yes/no.

`playerSummary`'s one-line summary ("2-ply sim, … · 6-ply endgame") is not a
settings row and keeps its wording. The SPRT *status* `off` ("not run: the job
plays to its target") is a status, not a setting, and is replaced in phase 5.

### 1.5 "—" where a setting doesn't apply (#8, #9)

- **Plies Recorded:** "—" when `num_plies === 0`. A static player records no
  plies.
- **Movegen Margin:** "—" unless `recorder_type === 'equity'`. The margin bounds
  only the recorder that keeps "every move within the equity margin".

These go in the row's `value` function, not in `show()`. A 0 is still shown as
0 wherever it means something.

### 1.6 Job types capitalised (#10)

`JOB_TYPE_LABELS` (`format.ts:64`) becomes "Opening Rack Analysis", "Games",
"Game Pairs" and "Leave Generation". It is used everywhere: the Type row, the
jobs list, and the default name for an unnamed job (`format.ts:306`). So the
wording is the same across the site. Update `format.test.ts` and any e2e text
match.

### 1.7 Remove the identical-pairs sentence (#12)

In `MatchScore.svelte:42`, the divergent table's note becomes just "Over N
games: both games of the M pairs whose games did not play identically." The
sentence after it goes.

### 1.8 Opening-rack rows (#3, #5)

`jobSettings()`'s opening-rack block loses "Racks in all" and "Rack size". Its
single "Analyses per rack" row (`analysesPerRack`) is replaced by three key
rows:
- Minimum Analyses Per Rack: `min_results_per_rack`.
- Maximum Analyses Per Rack: `max_results_per_rack`.
- Consensus %: `consensus_pct`, or "—" when the maximum is 1, because one
  analysis per rack seeks no consensus.

"Racks Per Task" stays under "All settings". `analysesPerRack` stays in
`consensus.ts` only if something else still uses it; otherwise remove it and
its tests.

---

## Phase 2: Seed and dev changes

### 2.1 dev.py's opening-rack job sims (#1)

In `scripts/seed.py`, the `opening_rack` branch of `create_dev_jobs` (≈:570)
currently makes `static-equity-all`. It changes to a simming player that copies
the sim games and pairs jobs:
- `sim-2ply-rack` (with the `ab-` prefix on english_ab), sorted on equity.
- recorder `all`, `num_plays` 10, `num_plays_recorded` 10.
- `num_plies` 2, `max_iterations` 200, `time_limit_secs` 0.
- the main data's win % model. english_ab has none, as the sim games jobs
  already find.
- If the data has no win % model, raise `SeedError`, as the sim jobs do.

The job's settings:
- **Consensus:** `min_results_per_rack: 2`, `max_results_per_rack: 5`,
  `consensus_pct: 80`, so the dev job exercises phase 3. Job creation allows
  `max > 1` because the player sims.
- **Batch:** `racks_per_batch` drops from 500 to **20** on english (2 on
  english_ab). A 2-ply, 200-iteration sim per rack makes a 500-rack claim take
  far too long.
- Update dev.py's `DEV_JOBS` description to "an opening-rack job, a 2-ply simmer
  ranking every rack's plays, each rack analysed until 80% agree (2–5
  analyses)".
- The old `static-equity-all` player config is still made by
  `job_config`/`--job opening_rack` (`seed.py:751`). Leave it there.

### 2.2 Rack info tables on by default (#7)

The server default (`admin.rs:915`, `use_rit.unwrap_or(true)`) and the
new-config form (`useRit = true`) are already on. The remaining offs are in
the seed:
- `create_dev_jobs`: `rit = args.rit and not on_small`. The english_ab players
  get a table too, which means `rit = args.rit`. The comment saying "one on
  eight racks saves nothing" goes.
- **Leave generation stays without one (decision 1).** `validate_leave_player`
  refuses a rack info table because the table holds leave values, and every
  generation plays a new KLV. The leave job's player keeps `rit=False`. Change
  its names (`static-equity-no-rit` / `static-equity`) only if english_ab's
  `static-equity` now needs a table: in that case name the leave player
  `ab-static-equity-no-rit` so the two configs don't collide.
- `--no-rit` (dev.py and seed.py) still turns tables off for every job.
- e2e: `e2e/lib/api.ts:87`'s `createStaticConfig` sets `use_rit: false` to
  skip a build in tests. That stays.

---

## Phase 3: Opening-rack consensus editable after creation (#4)

### 3.1 Always keep progress rows (decision 3)

Today `opening_rack_progress` rows exist only when `max_results_per_rack > 1`:
- `record_consensus` (`opening_rack.rs:435-438`) returns early otherwise.
- `seeks_consensus()` (`models/job.rs:184`) keys on `max > 1`.

That makes editing across the 1 boundary unsafe:
- **1 → more than 1:** there are no rows for reissues to work from.
- **More than 1 → 1:** the plain counting path counts every result as a new
  rack, so reissued racks that are still in flight are counted twice in
  `racks_analyzed` and `racks_settled`.

The change:
- `record_consensus` always upserts progress rows and always derives the
  counters from them. The early return goes.
- Only *reissuing* depends on max > 1: `next_request` calls `next_reissue` only
  then. Rename `seeks_consensus()` to `reissues()` so it says what it does.
- With max 1, a rack is settled after its first analysis, as now. `standing()`
  already gives that: `results >= max`.
- **Cost:** one row per rack for every opening-rack job, the same as any
  consensus job pays today. The index on unsettled racks (`0001_initial.sql:1100`)
  stays small for a max-1 job, because every row is settled.
- `exports.rs` keeps the `consensus` field only for jobs whose maximum is above
  1, read from the config. A max-1 export is unchanged.

### 3.2 Read the settings fresh, not from the template cache

`JobTemplate` (`jobs/dispatch.rs`) caches `OpeningRackConfig` for the life of
the process. Claims (`scheduler.rs:801-833`) and submissions
(`worker.rs:699-705`) take that `Arc` before their transaction. So even with
`templates.forget(id)` after an edit, a claim or submission already in flight
would use the old settings.

The fix:
- `record_consensus` reads `min_results_per_rack`, `max_results_per_rack` and
  `consensus_pct` from `job_opening_rack_config ... FOR SHARE` inside the
  submission's transaction.
- `next_request` / `next_reissue` read them under the dispatch lock.
- The edit updates that row `FOR UPDATE`. This serializes against both.
- The edit also calls `templates.forget(id)`, and the comment calling `forget`
  "for deleted jobs only" (`dispatch.rs:196-201`) is updated. So is the "config
  rows have no update path" note in `PLAN.md` (≈:2420).

### 3.3 The endpoint

`PATCH /api/admin/jobs/:id/consensus`, admin only. The body is partial:
`{ min_results_per_rack?, max_results_per_rack?, consensus_pct? }`.

The shape follows `ratings.rs::update_pool`: only the fields given change, and
nothing changed means an early `200` with no audit row. In one transaction:

1. **Refuse when it doesn't apply.**
   - `refuse_while_purging`.
   - 404 for an unknown job.
   - 400 for a job that isn't an opening-rack job.
2. **Lock** in the purge's order: `lock_job_dispatch` → `lock_open_claims` →
   `load_job_for_update`. Then lock the config row.
3. **Validate the merged settings.** Pull the range checks out of
   `validate_job_body` (`admin.rs:1628-1662`) into
   `validate_consensus(min, max, pct)`, shared by creation and the edit. Then
   re-run `validate_opening_rack_player` against the job's player: a static
   player can't have max > 1. Field errors use the creation form's names.
4. **Update** `job_opening_rack_config`.
5. **Restate every rack** with one UPDATE using `standing()`'s formula:
   - agreed = `results >= min AND top_count*100 >= pct*results`;
   - settled = agreed OR `results >= max`;
   - `without_consensus` = settled AND NOT agreed.

   Then reset `jobs.racks_settled` and `racks_without_consensus` from the row
   counts. Raising min or max un-settles racks; lowering them settles racks.
6. **Start or stop the job.**
   - **Completed, and now has unsettled racks: reopen.**
     - Take the `birdtest.activate` lock and check that active allocations plus
       this job's still fit in 100%.
     - If they fit, set status `active` with `activated_at`, run
       `join_at_parity` and `request_derived_data` as `activate_job` does, and
       write `job.activated`.
     - If they don't fit, set status **`inactive`**, write `job.deactivated`,
       and say why in the response. The admin then frees allocation and
       activates it.
     - Either way, delete its `is_final` `job_exports` rows, as the purge does
       (`admin.rs:3113-3118`). Otherwise `exports::newest_ready` would serve
       the old corpus as final once the job completes again.
   - **Active, and now fully settled:** after the commit, run the finish check:
     `finish_condition_met`, then `complete_finished`. Claims still in flight
     are accepted as usual; the "every task completed" rule already waits for
     them.
   - **Inactive:** the status doesn't change. If it is now fully settled, it
     completes when next activated, through the existing `rearm_idle` path.
7. **Audit:** `log_detail("job.consensus_changed", "min 1 → 3, max 1 → 5,
   consensus 100% → 80%")`, plus the status-change row from step 6 when there
   is one.
8. **After the commit:** `templates.forget(id)`, `finish_checks.rearm_idle(id)`
   when the job is active, and `push_after_change` so SSE viewers see the new
   settings and counters.

The response is `{ config: OpeningRackConfig, status, reopened: bool,
reopened_inactive_reason?: string }`.

### 3.4 Admin UI

The admin job page (`frontend/src/routes/admin/jobs/[id]/+page.svelte`) gets a
**Consensus** card for opening-rack jobs, beside Controls:
- It has three inputs (Minimum Analyses Per Rack, Maximum Analyses Per Rack,
  Consensus %), reusing `admin/jobs/new`'s fieldset and its `consensusProblem`
  check. Consensus % is disabled while the maximum is 1.
- **Save** goes through `run()` with a notice: "Saved. The job reopened",
  "…stayed inactive: allocations are full", or "…completed: every rack is
  settled". Field errors from `ApiError.fields` show under their inputs.
- After saving, set `config = null` so the page reads it again. The comment
  that config is "fixed once the job exists" (≈:46) is updated.
- `api.ts` gets `updateOpeningRackConsensus(id, body)`, using the `patch`
  helper.

### 3.5 Tests

**Backend:**
- `jobs.rs` (next to :286): the edit's field errors, a static player refused
  max > 1, a non-opening-rack job refused, and a no-op that writes no audit
  row.
- `worker_api.rs` (next to :327), using its harness:
  - raise max on a running job → unsettled racks are reissued and settle;
  - lower min or pct → it completes;
  - max 1 → 3 on a job with results → reissues start from the existing
    progress rows;
  - max 3 → 1 → counters stay exact, with no double counting of in-flight
    reissues.
- `finish.rs`:
  - a completed job reopened by an edit completes again;
  - with allocations full, it reopens as inactive;
  - its final export is gone.
- `admin_routes.rs:376`-style response shape, plus a read that agrees with it.
- The `audit.rs` table and the `authz.rs` route table.

**Frontend unit:** `jobSettings.test.ts` for the three rows.

**e2e:** a new `e18-opening-rack-consensus.spec.ts`. It creates a simming
config, then an opening-rack job with max 3. It edits the settings from the
admin page and sees the notice and the new rows on the public page.

---

## Phase 4: Win %, ply stats and inference on saved move lists (#2)

**What's already there:**
- **Win %:**
  - MAGPIE sends it for sim and PEG positions, and for opening racks when the
    player sims.
  - The server stores it in `position_analysis_moves.win_percentage`.
  - Saved positions show it.
  - The opening-rack lookup drops it.
- **Ply stats:**
  - MAGPIE sends `plies: [{ply, average_score, bingo_percentage}]` for every
    simmed move.
  - The server keeps the first `num_plies_recorded` in
    `position_analysis_plies`.
  - No public endpoint returns them; only exports and the admin stream do.
- **Inference:** none of it exists yet, on either side.

### 4.1 Win % and ply columns

**What P1 and P2 mean.** P1 and P2 are *plies*, not players. MAGPIE's own sim
table labels `P%d-S` / `P%d-BP` with ply index + 1
(`MAGPIE/src/str/sim_string.c:148-153`):
- P1 is the reply after the move (wire `ply: 0`).
- P2 is the turn after that (wire `ply: 1`).
- S is the average score and BP the bingo percentage.

**Backend (`backend/src/routes/public.rs`):**
- `saved_positions` (:1129-1178) selects the plies too: a jsonb subquery
  `plies` per move, ordered by ply, `[{ply, average_score,
  bingo_percentage}]`, as `exports.rs:72-103` does. It returns plies 0 and 1
  only. The page never shows more, and a 10-ply job would otherwise send five
  times the rows.
- `rack_lookup` (:1033-1086) adds `win_percentage` and the same `plies` to each
  row.

**Frontend:**
- `api.ts`'s `SavedPosition` move type gains `plies: {ply: number;
  average_score: number; bingo_percentage: number}[]`.
- The lookup rows get a type (`RackLookupRow`) in place of
  `Record<string, unknown>`.
- **`PositionPane.svelte`:**
  - Shows `Win %` as now, when any move has one.
  - Adds `P1-S`, `P1-BP`, `P2-S`, `P2-BP` for as many plies as the moves carry,
    up to 2: n = the largest `plies.length`, capped at 2. That gives fewer
    columns when the job recorded fewer plies, and none for a static player.
  - Each header has a `title` ("Ply 1: the reply's average score").
  - S is shown to one decimal place, BP as one decimal place with a % sign.
- **The opening-rack lookup table** (`routes/jobs/[id]/+page.svelte:257-280`)
  gets the same columns, by the same rule, for each analysis.
- **Rename the existing "Plies" column to "Solved Plies".** On PEG and endgame
  positions it is the solver's `fidelity_plies`, and it would read as one of
  the new ones.
- **Width:** both tables sit inside `overflow-x-auto`. The phone-width e2e
  check (e12b, e10) passes because the table scrolls inside its box.

### 4.2 Inference

**Where it is shown:**
- **It describes a position, not a move.** MAGPIE infers the opponent's leave
  once per turn, from their previous move, before the sim. So it is stored per
  position and shown once, above the move table.
- **Opening racks never infer.** MAGPIE turns it off for them
  (`config.c:9031-9034`: no previous play), so the lookup shows no inference.
- **It applies when** the player uses inference, it is past turn 0, and the
  previous move wasn't a pass. This is the condition the board printer checks
  (`autoplay.c:952-956`).
- **It shows the opponent's leave (decision 2):** the tiles they kept, which is
  what MAGPIE's leave list holds.
- **The display**, in `PositionPane`'s summary: "Inferred from O6 BHUT: 143
  possible leaves, average equity 12.4". Under that is a compact table of the
  10 most common leaves: Leave | Draws (%) | Equity.

**MAGPIE:**
- **`config.c:4139-4149`:** autoplay builds each seat's `InferenceArgs` with a
  leave list capacity of 0, so `inference_results_reset` makes no list
  (`inference_results.c:116-122`) and `inference.c` skips the inserts. Pass a
  capacity of **10** when the positions recorder is on, and leave it at 0
  otherwise, so nothing else changes. Measure the cost on a tier-6 run. The
  list is a bounded heap of 10, so the cost should be small.
- **`autoplay_results.c`:**
  - Add `const InferenceResults *inference_results` to `RecorderArgs` (:43-71),
    and pass `AutoplayWorker.inference_results` from `autoplay.c:901-920`, or
    NULL when the turn didn't infer.
  - `positions_data_add_move` copies a fixed-size summary into
    `CapturedPosition`:
    - `num_leaves`:
      `stat_get_num_unique_samples(inference_results_get_equity_values(r, INFERENCE_TYPE_LEAVE))`;
    - `total_draws`: `stat_get_num_samples(...)`;
    - `average_equity`: `stat_get_mean(...)`;
    - up to 10 `{leave, draws, equity}` from `inference_results_get_leave_rack_list`,
      which is already sorted by draws, read with
      `leave_rack_get_leave/_draws/_equity`.
  - `inference_string.c:287-391` is the template for reading the list. Write
    each leave in the job's letter distribution, as racks are written.
- **`write_captured_position`** (:1950-2002) writes the summary under new
  `contribute_defs.h` keys:

  ```json
  "inference": {"num_leaves": 143, "total_draws": 52011, "average_equity": 12.41,
                "leaves": [{"leave": "AEINST", "draws": 812, "equity": 30.2}, ...]}
  ```

  It is absent when the turn didn't infer.
- **Tests:** `test/contribute_test.c` and `test/birdtest_contract/`. MAGPIE's
  contract tests read the same fixtures (`contribute_test.c:417-441`).

**birdtest backend:**
- **Wire type:** `CapturedPosition` (`jobs/handler.rs:403-427`) gains
  `inference: Option<InferenceSummary>`.
- **Plausibility checks** (`plausibility.rs`, next to `check_analysis`):
  - only on a `sim` position past turn 0;
  - `num_leaves >= leaves.len()`, at most 10 leaves, each leave's draws ≤
    `total_draws`;
  - leaves in descending draws order, finite equities;
  - each leave a valid multiset in the job's distribution, no longer than a
    rack.
- **Schema:** a new table in `0001_initial.sql`:

  ```sql
  CREATE TABLE position_analysis_inference (
      record_id      BIGINT PRIMARY KEY REFERENCES position_analysis_records(id) ON DELETE CASCADE,
      num_leaves     INTEGER NOT NULL CHECK (num_leaves >= 0),
      total_draws    BIGINT  NOT NULL CHECK (total_draws >= 0),
      average_equity DOUBLE PRECISION NOT NULL,
      leaves         JSONB   NOT NULL   -- [{leave, draws, equity}], ≤ 10, by draws descending
  );
  ```

- **Inserted** in `insert_position_analyses` (`jobs/mod.rs:511-671`), next to
  the plies.
- **Returned** by `saved_positions` as `inference: {...} | null` per position.
  Pairs return it per game, through `partner` too.
- **Restore and backup:** `scripts/restore-job.sh` (:169 lists the plies table)
  and `restore-job-check.sh` add the new table. Check `scrub.sql` and
  `exports.rs` (exports gain `inference` per position, the way they carry
  plies).

**Fake worker and fixtures:**
- `worker/fake_worker.py`:
  - `_synthetic_position` adds an `inference` block on turns > 0 when the
    player uses inference.
  - The opening-rack branch (:480-517) adds `win_percentage` and
    `blended_utility`. Today it sends plies without win %, so the server
    stores those racks as `static`.
- `contract-fixtures/result-games.json`: add an in-game `sim` position with
  plies and an `inference` block, update `contract-fixtures/README.md`, and
  copy it to `MAGPIE/test/birdtest_contract/`.

### 4.3 Tests

**Backend:**
- `public_api.rs:697-707` (lookup rows) and `:787-792` (saved-position moves)
  assert exact shapes. Extend them with `win_percentage`, `plies` and
  `inference`.
- New cases:
  - plies cut to 2 in the API;
  - inference returned for a sim position past turn 0, and null otherwise;
  - inference refused on a static or turn-0 position, and refused with 11
    leaves.
- `routes/worker.rs` C-3 to C-5 (the fixture acceptance tests).
- `handler.rs` U-WIRE-3/4.

**Frontend unit:** a helper for the ply-column count (`plyColumns(moves)`) and
the inference summary text.

**e2e:**
- `e12-saved-positions.spec.ts` asserts the P1-S…P2-BP headers on a simmed job,
  and the inference block.
- `e13-opening-rack-samples.spec.ts` asserts Win % and the ply headers in the
  lookup.
- Tier 6 runs real MAGPIE with a 2-ply inferring player and checks the stored
  inference rows.

---

## Phase 5: Replace the SPRT with a match test (#15)

### 5.1 Why

The SPRT tests H0: Elo = `elo_low` against H1: Elo = `elo_high`, −10 and +10 by
default. Its error rates α and β hold only *at those two Elo values*. Its
verdict says which of the two the data favours. It does not say whether
player 1 is better. Two problems follow:
- **Equal players get a winner half the time.** By symmetry, two equal players
  "pass" about half the time, and the badge says "passed (H1 accepted)".
- **It asks for settings the question doesn't have.** To use it, an admin has
  to pick an Elo margin, α and β, none of which appear in "is A better, at X%
  confidence?". The normal-approximation LLR also overstates |LLR| when nearly
  every unit scores alike (PLAN KL-87), which is a common case for pairs that
  play identically.

The replacement answers the question directly. It keeps a **confidence
interval for player 1's score** that stays valid however often it is checked,
and stops as soon as the interval excludes 50%.

### 5.2 The test

**The unit and its score.** These are unchanged from today, and the module
doc's reasoning for them is kept:
- A games job scores each game 1, ½ or 0.
- A pairs job scores each pair i/4 from its pentanomial bucket i, including
  pairs played identically, which score exactly ½ and lower the variance.

Let n be the number of units. From the counts the server already stores
(`Tally` and `Pentanomial`), compute:
- μ̂, the mean score;
- σ̂², the sample variance of the unit scores.

**The interval: an asymptotic confidence sequence.** This is the Robbins
normal-mixture boundary with the empirical variance, from Waudby-Smith,
Arbour, Sinha, Kennedy & Ramdas, *Time-uniform central limit theory and
asymptotic confidence sequences* (2021). With α = 1 − confidence/100:

```
half_width(n) = sqrt( 2·(n·σ̂²·ρ² + 1) / (n²·ρ²) · ln( sqrt(n·σ̂²·ρ² + 1) / α ) )
interval      = [ μ̂ − half_width, μ̂ + half_width ]

ρ = sqrt( (−2·ln α + ln(−2·ln α + 1)) / n* )      // tightest at n = n*
```

**Why this method:**
- **It holds under repeated looks.** The interval contains the true score at
  every n at once, with probability about X%. So checking after every
  submission, as `should_check_finish` does every 8, and stopping the moment
  it decides is valid. A plain fixed-n confidence interval would not be.
- **It needs only stored sums.** It uses n, μ̂ and σ̂², all derivable from the
  stored counts. It is order-free and stateless: it is recomputed on each read
  as the SPRT is now, with no new columns per result and no per-unit history.
  An exact, nonasymptotic betting confidence sequence would need each unit's
  outcome in order, which batches don't keep.
- **It stays well behaved when nearly every pair ties.** If every pair ties,
  σ̂² → 0 and the half-width shrinks to `sqrt(2·ln(1/α)/(n²ρ²))`, which is
  still positive. The KL-87 overstatement goes away.

**The guard:** "asymptotic" means valid once n is large. `min_units` is kept
and is required, so nothing is acted on before then. It defaults to 1,000
games or 500 pairs.

**Tuning point:** n* = √(min_units · max_units), the geometric mean of the
earliest point the test acts and its cap. The boundary is fairly flat around
n*, so this costs little at either end. It is one function (`tuning_units`),
so it is easy to change.

**The decision**, made only once n ≥ `min_units`:
- lower bound > 0.5 → **player 1 better**;
- upper bound < 0.5 → **player 2 better**;
- n ≥ `max_units` with neither → **inconclusive**. The page then reports the
  interval, for example "player 1 scores 49.6%–50.5%: no difference larger
  than about ±3 Elo at 95%". For equal players this is the right outcome, and
  the SPRT could not give it.

**Error rate:** if the players are truly equal, the chance of ever declaring a
winner is at most about α, split between the two sides.

**Elo, for display:** `elo(s) = −400·log10(1/s − 1)`. It is applied to μ̂ and
to both bounds, and clamped at ±1000 when a bound reaches 0 or 1. The scores
are already per game for both job types (a pair's i/4 is player 1's per-game
score), so the Elo is per game too.

### 5.3 Settings

Per job, in both `job_game_config` and `job_game_pair_config`:

| Now | New |
|---|---|
| `sprt_enabled` | `test_enabled` (BOOLEAN, default FALSE) |
| `sprt_alpha`, `sprt_beta`, `elo_low`, `elo_high` | **removed** |
| (none) | `confidence_pct` DOUBLE PRECISION NOT NULL DEFAULT 95, CHECK `> 50 AND < 100` |
| `min_games` / `min_pairs`, `max_games` / `max_pairs` | unchanged in meaning |

**Validation** (`admin.rs:1568-1625`):
- `confidence_pct` and `min_*` are refused unless `test_enabled`.
- `min_*` is required with it, and must be `<= max_*`.
- `confidence_pct` is finite and in (50, 100).

**The decided verdict** on `jobs` changes:
- `sprt_decided_status` / `_llr` / `_units` become `test_decided_status`
  (`'player1_better' | 'player2_better' | 'inconclusive'`),
  `test_decided_lower`, `test_decided_upper` and `test_decided_units`, all
  null together.
- `complete_unless_purged` (`jobs/mod.rs:305-344`) and `Finish::Sprt` become
  `Finish::Test`.
- `SPRT_CHECK_EVERY` (`state.rs:40`) becomes `TEST_CHECK_EVERY`, still 8.

### 5.4 Backend

- **Modules.** `backend/src/stats/sprt.rs` becomes `stats/match_test.rs`.
  - `Tally`, `Pentanomial` and `Sample` move to `stats/outcomes.rs`, because
    `plausibility.rs` and `ratings.rs` use them too.
  - `match_test.rs` holds `Sample::{mean, variance}`, `half_width`,
    `tuning_units`, `elo` and `evaluate(sample, confidence_pct, min, max) ->
    TestResult`.
  - `TestResult` is `{ mean, lower, upper, elo, elo_lower, elo_upper,
    confidence_pct, status }`, where status is `Running | Player1Better |
    Player2Better | Inconclusive`.
- **Stats.** `jobstats.rs` swaps `sprt: Option<SprtResult>` for `test:
  Option<TestResult>`, and `decided` takes the new shape. Nothing else in
  `GameStats` changes, and the pentanomial is still reported.
- **Ratings** are unaffected; they read pentanomials, not the test. Update the
  comments in `ratings.rs` and `main.rs` that name the SPRT.
- **Unit tests in `match_test.rs`:**
  - the half-width matches values computed by hand for a few (n, σ̂², α, n*);
  - all-ties pairs never decide, and end inconclusive;
  - a 55% player decides "player 1 better" well before the cap.
  - **A simulation test:** 2,000 runs of equal players, checked after every
    batch up to `max_units`, give a false winner rate ≤ α + a tolerance.
  - The same at 53% gives a power figure that is recorded in `TESTING.md`.
- **The existing SPRT tests** in `backend/tests/stats.rs`, `finish.rs`,
  `worker_api.rs`, `jobs.rs`, `admin_api.rs`, `public_api.rs` and
  `common/mod.rs` move to the new settings and verdicts.
  `worker/fake_worker.py`'s player-1 bias (:886) still drives a chosen
  verdict.

### 5.5 Frontend

- **`SprtCard.svelte` becomes `MatchTestCard.svelte`:**
  - It has a status badge.
  - It has one sentence: "Player 1 scores 53.1% per game (95% interval
    51.2%–55.0%), about +22 Elo (+8 to +35). Player 1 is better at 95%
    confidence."
  - It has a bar: the interval drawn on a score axis centred on 50% with the
    50% line marked, scaled to the interval's extent.
  - It explains, in one short paragraph, that the interval stays valid
    however often it is checked, so the job stops the moment it excludes 50%,
    or at its cap.
  - The pentanomial table is kept for pairs jobs.
- **`format.ts`:** `sprtLabel` / `sprtState` become `testLabel` / `testState`
  with the states running, paused, undecided (force-completed), player1_better,
  player2_better, inconclusive and off. `JobStatusCard.svelte` and both job
  pages follow.
- **Settings rows** (`jobSettings.ts`):
  - Significance Test (yes/no);
  - Confidence % (key);
  - Minimum Pairs/Games and Maximum Pairs/Games (from 1.3).
  - The SPRT, SPRT α and SPRT β rows go.
- **Create form** (`admin/jobs/new` :44-50, :133-169, :443-476): a "Match
  Test" checkbox, then Confidence % (default 95), Minimum and Maximum. The
  α/β/Elo inputs go.
- **e2e:** `e8-pentanomial.spec.ts`, plus every spec that matches SPRT text:
  e1, e4, e5, e7, e10, e11, e12. `e2e/lib/api.ts` job bodies use
  `test_enabled` and `confidence_pct`.

### 5.6 Scripts and docs

- `scripts/seed.py`'s `job_config` and `create_dev_jobs`, `e2e_magpie.py`, and
  `restore-job-check.sh:155` use `test_enabled` and `confidence_pct`.
- `PLAN.md` (KL-87 closes), `README.md`, `RUNBOOK.md`, `JOURNEYS.md`,
  `SETTINGS_COMPARISON.md` and `TESTING.md` replace the SPRT with the match
  test.
- Help text on the admin users page (`admin/users/+page.svelte:44`) changes
  "SPRT" to "match tests".

---

## Decisions

All five were settled while planning:

1. **Rack info tables and leave generation:** leave-generation jobs stay
   without one. Every generation plays a new KLV, and the table holds leave
   values.
2. **Inference's top 10:** the opponent's *leave*, the tiles kept, as MAGPIE
   stores it. Not the full rack.
3. **Editing across max 1:** progress rows are always kept, so any edit is
   safe (3.1), rather than refusing edits that take the maximum across 1.
4. **Create forms:** they use the same setting names as the settings tables.
5. **The SPRT:** replaced in this batch by the match test in phase 5.

## Suggested order and resets

1. **Phase 1:** frontend only, with no reset.
2. **Phase 2:** seed only. Fresh dev jobs need `dev.py --reset-db`, or a new job
   via `--new-job`.
3. **Phase 3:** schema change (progress rows always kept), so reset.
4. **Phase 5:** schema change, independent of 3 and 4. It goes before 4 because
   it is server and frontend only.
5. **Phase 4:** MAGPIE and birdtest together, with a schema change, so reset.
   Run the MAGPIE suite, tier 5 and tier 6.
