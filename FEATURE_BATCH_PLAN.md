# Feature batch plan — September 2026

Implementation plan for the September 2026 feature batch. All decisions below
are settled; the [Decisions](#decisions) section records them.

**Out of scope for now:** rewriting the home page intro header and the
Contribute card.

## Conventions that apply to every phase

- **Schema changes edit `backend/migrations/0001_initial.sql` in place** (there
  is one migration until release — see README "After a schema change"). Dev
  databases need a reset after phases 2, 5 and 6.
- Each phase updates `PLAN.md`, `JOURNEYS.md` and `TESTING.md` (IDs and test
  counts) alongside the code.
- New or changed routes must be added to the route table in
  `backend/tests/authz.rs`, or `the_route_table_is_every_route_the_router_serves`
  fails.
- Destructive admin actions write their audit record in the same transaction
  (`backend/src/audit.rs`), and get a case in
  `backend/tests/audit.rs::every_destructive_admin_action_writes_exactly_its_record`.
- Verify with the local recipe, with capped build parallelism. Don't run
  `cargo fmt`: this code isn't rustfmt-formatted.

## Decisions

| Topic | Decision |
|---|---|
| Home header / Contribute card | Leave as is for now |
| Match score box | W–L–D, score (W + ½D), win %, plus average score and spread per player (small backend addition); the pentanomial stays in the SPRT card |
| Stats row | Status moves from the header badge into the row; the admin job page also gets the row and the match score box |
| SPRT-off scope | Games **and** game pairs; SPRT off by default |
| Empty-default selects | Letter distribution and layout, on the job form and the rating-pool form |
| Contributor ranking | By compute time (claim → submit), using running counters |
| Leavegen player limits | Static play only (`num_plies = 0`), equity sort, no rack info table |
| Old MAGPIE builds | Not supported across the leavegen change (pre-release); the MAGPIE change and both contract fixtures land together |
| Rating pool edit | Anchor and anchor rating; older rating history stays on the old scale, so the history chart shows a step at the change |
| Board | Highlight the previous move; click-a-move-to-preview is a follow-up; the positions UI is random position + rack search only (the latest-positions feed goes) |
| Settings tables | Player settings **and** job-level settings groups become tables |

---

## Phase 1 — Small, independent fixes

### 1.1 "Create new …" buttons for admins

- `frontend/src/routes/jobs/+page.svelte` → link to `/admin/jobs/new`.
- `frontend/src/routes/player-configs/+page.svelte` → link to
  `/admin/player-configs/new`.
- Copy the existing pattern in `frontend/src/routes/ratings/+page.svelte`
  (`$: isAdmin = $session?.is_admin ?? false`, a `btn-primary` link in a
  `flex justify-between` header).
- No backend change; the create routes are already admin-only.
- Docs: `JOURNEYS.md` visibility checks (non-admins see no button).

### 1.2 Consistent register errors

The cause: the email input is `type="email" required`, so the browser blocks
submit with its own popup. The username error is a server field error shown as
red `.field-error` text.

- `frontend/src/routes/register/+page.svelte`: `novalidate` on the form, plus
  client-side checks that mirror the server's rules and fill `fields.*`, so
  every error renders the same way:
  - username: 3–32 characters after trimming (`backend/src/routes/auth.rs`
    `register`);
  - email: the rule in `is_bare_address`. The browser accepts `a@b`, which the
    server rejects;
  - password: required. The server still has the final say on strength
    (zxcvbn).
- The server keeps validating; client checks only surface errors earlier.
- Apply the same treatment to `reset-password/+page.svelte`, which has the same
  native popup, and make `login` show errors the same way.
- Tests: e2e `e2-register-and-api-key.spec.ts`, `e9-password-reset.spec.ts`.

### 1.3 "No new data to insert."

- `frontend/src/routes/admin/input-data/+page.svelte`: when
  `newRows.length + collisions.length === 0`, show "No new data to insert." and
  no Insert button.
- Backend (`backend/src/inputdata.rs` `run_import_within`): when staging finds
  nothing new or changed, move the import straight to a terminal state rather
  than leaving it `staged` for 24h until `expire_unconfirmed_imports` cancels
  it. Audit it (e.g. `input_data.import_nothing_new`).
- Tests: `backend/tests/input_data.rs`
  (`a_second_import_of_the_same_tarball_is_a_no_op`), `importWatch.test.ts`.

### 1.4 Derived data page updates by itself

The cause: `admin/derived-data/+page.svelte` polls every 3s, but only while a
row is pending or building. Once everything is idle it stops, so rows queued
later never appear.

- Keep polling at all times: 3s while anything is pending or building, about
  10s when idle, and paused while the tab is hidden (`visibilitychange`).
- Extract the poller into a small `$lib` helper modelled on `importWatch.ts`,
  guarded by a generation counter, and use it for
  `DerivedDataStatus.svelte` too.
- No SSE: the builder (`backend/src/bin/build-derived.rs`) is a separate
  process on a 5-minute schedule, so push would need Postgres LISTEN/NOTIFY
  for little gain.

### 1.5 Letter distribution and layout start empty

- `frontend/src/routes/admin/jobs/new/+page.svelte`: stop pre-filling
  `letterdistId` and `layoutId` with `firstOfRole(...)` in `loadChoices()`.
  Add a `<option value="" disabled selected>Choose…</option>` to each
  `required` select.
- In `submit()`, refuse an empty id with an error that names the field (today
  an empty id would reach the server as a generic JSON-shape 400).
- Same change on `frontend/src/routes/admin/rating-pools/new/+page.svelte`.
- No backend change.
- e2e e4, e7 and e12 already pick these explicitly; check any that relied on
  the default.

### 1.6 Stats row → status, allocation, tasks completed, estimated time left

- `frontend/src/routes/jobs/[id]/+page.svelte` (the 4-card row near line 105):
  replace Redundancy and Results accepted with Status (`JobStatusBadge`, moved
  out of the header) and Tasks completed (`stats.tasks_completed`).
- Redundancy remains visible in Settings → All settings.
- Add the same row to `frontend/src/routes/admin/jobs/[id]/+page.svelte`.
- `e10-phone-width.spec.ts` asserts exactly 4 `.grid > .card`. Keep the row at
  4 cards, and keep it the only matching grid on the page.

---

## Phase 2 — Games and game pairs

### 2.1 SPRT optional, off by default

Without SPRT, a job plays to its existing target: `max_games` / `max_pairs`.
Today that's labelled "Hard cap".

**Schema**

- `job_game_config` and `job_game_pair_config`: add
  `sprt_enabled BOOLEAN NOT NULL DEFAULT FALSE`.
- `jobs.sprt_decided_status` CHECK: add a non-SPRT completion status (e.g.
  `reached_target`). Alternatively, leave `sprt_decided_*` NULL for non-SPRT
  jobs and record the reason in the audit log. Pick whichever keeps
  `jobstats::Completion` simplest.

**Backend**

- `models/job.rs`: `GameConfig`, `GamePairConfig` and `SprtParams` gain the
  flag.
- `routes/admin.rs` `JobTypeConfig::Game` / `GamePair`:
  `#[serde(default)] sprt_enabled`, and `min_*` becomes optional. In
  `validate_job_body` (the `sprt` closure), skip the α/β/Elo/min checks when
  SPRT is disabled. Update `insert_job_config`.
- `stats/sprt.rs` / `jobstats::build_game_stats`: `GameStats.sprt` becomes
  `Option<…>`; no evaluation when disabled.
- `routes/worker.rs` `finish_condition_met`: a disabled job completes when
  `units_completed >= max_units`. Keep the idle-job check path
  (`finish_idle_job`) working for it.
- `jobs/mod.rs` `complete_unless_purged`: record the right completion reason.
- `routes/public.rs` `GamesSettings`: expose `sprt_enabled`.

**Frontend**

- Job form: an "Run an SPRT" checkbox, off by default. α, β, Elo bounds and
  "Min before SPRT" show only when it is on. "Hard cap" reads "Games to play"
  (or "Pairs to play") when it is off.
- `api.ts`: `GameStats.sprt` optional.
- Job page: no SPRT card when it is disabled. The Progress label drops
  "(hard cap)".
- Admin Progress line: no SPRT summary when it is disabled.
- `format.ts` (`sprtState`, `completionText`) and `jobSettings.ts`
  (`jobGroups`): handle no-SPRT jobs.

**Tests and seeds** (anything that omits the flag would silently become no-SPRT)

- Pass `sprt_enabled: true` explicitly where SPRT behaviour is under test:
  `scripts/seed.py`, `scripts/e2e_magpie.py`, `e2e/lib/api.ts` `activeJob`,
  the e5/e7/e11 payloads, and the raw INSERT in
  `scripts/restore-job-check.sh`.
- `backend/tests/stats.rs` (`gated_games_job` and the llr/cap completion
  tests), `finish.rs`, `jobs.rs`, and the `admin.rs` unit tests.
- Add tests for:
  - a no-SPRT job completing at its target;
  - no-SPRT being the default;
  - SPRT fields ignored or refused when SPRT is disabled.
- Frontend: `format.test.ts` (F-FMT-5, 5b, 12), `jobSettings.test.ts`.
- e2e: e1, e8, e10 and e12 (SPRT card and "Min before SPRT"), and e4 (admin
  "SPRT running — LLR" line).
- Worker contract fixtures are unaffected: `GameRequest` carries no SPRT
  fields.

### 2.2 Match score box

Placed after Settings and before the SPRT card, for `games` and `game_pairs`
jobs, on both the public and admin job pages.

- **Contents:**
  - Player 1's wins–losses–draws.
  - Score W + ½D out of the games played, and win %.
  - Average score per player and average spread.
- **Moved out of the SPRT card:** the W/L/D line and `OutcomeChart` (currently
  inside the SPRT card on `jobs/[id]/+page.svelte`).
- **Stays in the SPRT card:** the pentanomial. With SPRT off, this box is the
  job's result.
- **Backend addition (average score and spread):**
  - `game_results` stores per-batch `p1_score_mean` / `p2_score_mean`. Add
    games-weighted averages, `SUM(games * p1_score_mean) / SUM(games)`, to both
    stats queries in `jobstats.rs` (`plain_game_stats`, `game_pair_stats`).
  - Both queries go over `FIRST_GAME_RESULT_PER_TASK`.
  - Add the fields to `GameStats`, `build_game_stats` and the TS `GameStats`.
- **Admin page layout:** it has no Settings or SPRT box today. Add
  `<JobSettings>` (via `api.jobConfig`), then the match score box, then a
  proper SPRT box, replacing the one-line SPRT summary in Progress.
- **Tests:**
  - Extend `backend/tests/stats.rs` for the averages.
  - e2e: look the box up by its heading, and keep it out of the 4-card grid
    selector.

---

## Phase 3 — Settings shown as tables

Today there are three styles:

- a label/value grid (`<dl>`) of key settings on `/player-configs/[id]`;
- one-line `playerSummary` strings in `JobSettings.svelte`;
- single-column and comparison tables inside "All settings".

**Target:**

- **One `PlayerSettingsTable` component**, with columns for one player or two
  (comparison).
  - By default it shows the key rows (`keySettings` in
    `frontend/src/lib/jobSettings.ts`).
  - "All settings" expands to all `PLAYER_ROWS`.
  - Rows where two players differ are bold, as today.
  - JSON download stays.
- **Used on:**
  - `/player-configs/[id]`, replacing the `<dl>` grid and the separate
    table;
  - `JobSettings.svelte` (public and admin job pages), replacing the summary
    lines. It becomes a comparison table for games and pairs.
  - the leavegen job page (phase 6).
- **Job-level groups** (`jobGroups`: Job, Games and the test, Opening racks,
  Leave generation) are rendered as a table too, with key rows by default and
  the rest under "All settings".
- `playersLine` in the job header and the list-page summary columns can stay
  as one-line summaries.
- **Tests:** `jobSettings.test.ts` (F-SET-1), `e2e/tests/e14-player-configs.spec.ts`
  (currently reads the first `dd`).
- **Docs:** `JOURNEYS.md`, `PLAN.md`.

---

## Phase 4 — Saved positions on a board

### 4.1 Backend

- **Random position**: `GET /api/jobs/:id/positions/random`, signed-in only,
  games and pairs jobs only, like the existing positions endpoint.
  - Don't use `ORDER BY random()`: a capturing job holds millions of rows.
  - Instead: pick a random seed in `[1, max seed]`, take the next task through
    `tasks_seed_unique_idx (job_id, seed)`, then pick a random
    `(game_index, turn_number)` of that task through
    `position_analysis_records_in_game_idx`. Retry a bounded number of times if
    a task has no positions.
  - Return the same `SavedPosition` shape as today, with its moves.
- **Board data**: expose the job's layout and letter distribution. Their bytes
  are already in `input_data.content`; today only their names reach the
  frontend.
  - Add them to `GET /api/jobs/:id/config`, or a small
    `GET /api/jobs/:id/board`.
  - Return the layout parsed (15×15 square types plus the start square) and
    the letter → tile-score map.
  - Reuse the parsers: `layout_problem` in `routes/admin.rs` and
    `LetterDistribution::parse` in `jobs/racks.rs`.
- Remove the paged latest-positions feed from the endpoint (or leave it unused).
  Keep the rack search.
- **Tests:** `backend/tests/public_api.rs`
  (`captured_positions_are_searchable_when_signed_in`), plus a new
  random-position test covering the retry and empty-job cases. Add the new
  route to `authz.rs`.

### 4.2 Frontend

- **CGP parser in `$lib`, with vitest tests.** CGP is board rows, then
  `rackA/rackB`, then the scores, then the count of consecutive zero-score
  turns.
  - Racks are in fixed player order; the position's `rack` field says whose
    turn it is.
- **`Board.svelte`:**
  - Draws the grid from the layout, with premium squares coloured and tiles
    showing their letters and scores; blanks are lower case.
  - Also shows both racks and the scores.
  - Highlights the tiles of the previous move (notation like `8G HUH`; an
    exchange or pass highlights nothing).
- **`SavedPositions.svelte`:** one position at a time, with a "Random
  position" button, the board, and the ranked moves table. Keep the rack
  search, which shows one of that rack's positions at a time.
- **Follow-up, not in this batch:** click a ranked move to preview it on the
  board.
- **`worker/fake_worker.py`** always sends an empty board. Make it send
  varied, valid positions so e2e renders tiles.
- **Tests:** rewrite `e2e/tests/e12-saved-positions.spec.ts` (it asserts
  pages of 10 and 20 and "Load more").
- **Docs:** `JOURNEYS.md` V-10.

---

## Phase 5 — Admin data management

### 5.1 Rating pools: delete, and edit the anchor

No schema change. Members, runs, ratings and residuals already cascade from
`rating_pools`.

- **`DELETE /api/admin/rating-pools/:id`** (`backend/src/routes/ratings.rs`):
  - Take `lock_pool_fit` (make it `pub(crate)`) so the delete waits for a fit
    in flight.
  - Write `rating_pool.deleted` plus a `rating_pool.deleted.census`
    (`name=… members=… runs=…`) via `audit::log_detail`, then delete.
  - Deleting frees the anchor and member configs, and the pool's input data,
    for their own deletes.
- **`PATCH /api/admin/rating-pools/:id`** with `anchor_player_config_id` and/or
  `anchor_rating`. In one transaction:
  1. Take `lock_pool_fit`.
  2. Validate the new anchor (reuse `unknown_config`). Check the rating
     against `MAX_ABS_ANCHOR_RATING`.
  3. Add the anchor to `rating_pool_members` if it isn't a member.
  4. Update the pool.
  5. Audit `rating_pool.anchor_changed`, with old → new anchor and rating in
     the reason.
  6. Refit in the request, with a new `Trigger::Anchor`. The trigger column
     is TEXT, so this needs no migration. The periodic sweep would never
     notice an anchor change, because it compares only evidence and members.
- **Fix assumptions that pools are never deleted:**
  - `add_member` checks the pool with a plain `SELECT 1`. Use
    `FOR KEY SHARE`, or map the `rating_pool_members_pool_id_fkey` violation
    to a 404.
  - `recompute_stale` should treat a pool deleted mid-sweep as a quiet skip,
    not a logged failure.
- **`remove_member`'s anchor refusal message** ("A pool's anchor is fixed…")
  now tells the admin to change the anchor first.
- **Frontend** `frontend/src/routes/ratings/[id]/+page.svelte`, admin only:
  - an anchor `<select>` of the members plus other configs, and an
    anchor-rating input;
  - a Delete button that confirms, then goes to `/ratings`;
  - `api.ts`: `updateRatingPool`, `deleteRatingPool`.
- **Tests:**
  - `backend/tests/ratings.rs`:
    - delete cascades and returns 404 afterwards;
    - delete frees the anchor config;
    - an anchor change refits, sets `is_anchor` and auto-adds the member;
    - a bad anchor gets a 400;
    - the sweep skips a deleted pool;
    - update `removing_the_anchor_is_refused_with_the_fix_named`.
  - Also `authz.rs`, `audit.rs`, and e2e `e7-ratings.spec.ts`.
- **Docs:** close KL-41 in `PLAN.md`, update the API table, `JOURNEYS.md`
  A-13, and `TESTING.md`.

### 5.2 Export available at any time

**Fix first: the cache bug.** `exports::newest_ready()` returns the newest
ready export whatever the job's state was when it was built.
`job_results_stream` redirects a completed job's stream to it, and
`get_export` offers it. An export built mid-run would later be served as the
completed job's full corpus.

- **Schema:** `job_exports` gains a `final BOOLEAN` or snapshot marker, set
  when the export was built from a completed job.
- `newest_ready` and the stream redirect consider only final exports. A
  completed job with only snapshot exports builds a fresh one.

**Then:**

- **`exports::start`:** drop the "completed only" 409 and the "no claims in
  flight" 409 for jobs that aren't completed. An active job always has claims
  in flight. Keep the one-running-export-per-job index and the
  two-builds-at-once cap.
- **Consistency:** `build` scans results, then `finish_build` scans positions,
  each on its own connection and snapshot. On a live job the positions file
  could reference results missing from the results file. Run both scans in
  one `REPEATABLE READ` transaction on one connection.
- **Leave generation:** `settle()` merges staged rows only for completed jobs,
  so a running leavegen export reflects the last merge, which can be up to
  30 minutes old. Say so in the UI rather than forcing a merge.
- **Frontend** `admin/jobs/[id]/+page.svelte`: the Export card is always
  shown. A snapshot export is labelled "Snapshot as of <time> — job still
  running".
- **Tests:**
  - `backend/tests/admin_api.rs` `only_a_completed_job_can_be_exported`
    becomes "a running job exports a snapshot".
  - `backend/tests/exports.rs`: the redirect ignores snapshot exports, and one
    export runs at a time.
- **Docs:** `PLAN.md` "Exports" (the "completed only" invariant), the module
  docs in `exports.rs`, the schema comment on `job_exports`, and
  `JOURNEYS.md` A-10.

---

## Phase 6 — Contributors and leave generation

### 6.1 Contributor metrics, ranked by compute time

MAGPIE reports no CPU or thread counts. Ranking everyone by grouping
`task_claims` is a full scan, so use running counters.

- **Schema:** add counters on both `users` and `anonymous_workers`:
  - `compute_seconds` (the sum of `completed_at - claimed_at` over accepted
    claims);
  - `games_played` (from `game_results.games`, plus the games of leave
    tasks);
  - `racks_analyzed` (opening-rack racks and leave racks);
  - alongside the existing `tasks_completed`.
  - Partial indexes to rank by `compute_seconds`, like
    `users_worker_rank_idx`.
- **Submit path** (`routes/worker.rs`, where `tasks_completed` is
  incremented): bump the new counters with this claim's units. The existing
  progress delta counts only the first accepted result per task, so compute
  these units per claim.
- **Purge and delete** (`admin.rs` `Contributions::count` / `give_back`):
  reverse them. The RUNBOOK recount SQL covers them too.
- **Backfill:** in the migration, from `task_claims` joined to the result
  tables.
- **API:** `worker_page` in `routes/public.rs` returns the new fields and
  takes a `sort` parameter; the default is `compute_seconds`.
- **Frontend:** `frontend/src/routes/workers/+page.svelte` shows compute time
  (human-readable), games, racks, tasks and last result. Columns are
  sortable, ranked by compute time. Keep it readable at phone width by hiding
  secondary columns.
- **Tests:**
  - `backend/tests/public_api.rs`
    (`contributor_lists_paginate_and_leak_no_credentials`, ties);
  - `worker_api.rs` (the counters bump on submit);
  - purge tests (the counters are reversed).

### 6.2 Leavegen per-generation rack targets (birdtest only)

MAGPIE's `leavegen 100,200,500,1000,1000,1000 …`: one minimum rack count per
generation, and the list's length is the number of generations. Workers never
receive a target, so this is server-only.

- **Schema:** `job_leave_config` replaces `generation_count` and
  `target_rack_count` with
  `target_rack_counts INT[] NOT NULL`, with a CHECK that the cardinality is at
  least 1 and every element is at least 1.
- **`LeaveConfig`:** `target_rack_counts: Vec<i32>`, `generation_count()` =
  its length, `target_for(generation)`.
- **Per-generation reads in `backend/src/jobs/leave_gen.rs`.** Every read of
  the single target becomes per-generation:
  - `refresh_summary`, where the SQL uses `c.target_rack_counts[p.generation]`;
  - `any_rack_below_target`;
  - `racks_after`;
  - `furthest_below_target`.
  - The generation count used by `current_generation`, `close_generation` and
    completion comes from the list length.
- **API** `JobTypeConfig::Leave`: `target_rack_counts: Vec<i32>`.
  - Validation: non-empty, every value at least 1, with a sane maximum
    length.
- **Display:**
  - `jobstats::LeaveGenStats` reports the current generation's target and the
    whole list;
  - `public.rs` `LeaveSettings` and `jobSettings.ts` show the list;
  - the job page reads "Generation 3 of 6 — target 500 occurrences per rack".
- **Form:** `admin/jobs/new/+page.svelte` has one text field for a
  comma-separated list, parsed and validated client-side.
- **Tests and seeds:**
  - backend: `backend/tests/{leave_gen,leave_generation,magpie_leave,scheduler,jobs,stats,public_api}.rs`,
    including a test that generation 2 closes at its own target;
  - seeds: `scripts/seed.py` and `scripts/e2e_magpie.py`.

### 6.3 Leavegen runs with a player config (birdtest + MAGPIE)

Today MAGPIE's contribute leavegen (`config_contribute_leave_gen`,
`src/impl/config.c`) resets both seats to default static settings, so a player
sent by the server would be ignored.

**MAGPIE** (`/home/josh/MAGPIE`):

- Replace the reset loop with `config_contribute_apply_player_settings` for
  both seats, as the opening-rack path does. Read `use_wordmap` from the
  player.
- Leavegen keeps loading the server's KLV, not the player's leaves.
- Update `test/contribute_test.c` (the leave-key list) and
  `test/birdtest_contract/assignment-leave-generation.json`.

**birdtest:**

- **Schema:** `job_leave_config.player_config_id UUID NOT NULL REFERENCES
  player_configs(id)`. The lexicon (`kwg_id`) and `use_wordmap` now come from
  the player.
- **API and validation** (`routes/admin.rs`):
  - `JobTypeConfig::Leave` gains `player_config_id`.
  - Refuse a player with `num_plies > 0`, `sort_strategy` other than equity,
    or `use_rit = true`.
  - Check kwg role and `lex_ld_compat` against the player's kwg. Skip klv
    compatibility, because the player's leaves aren't used.
- **Dispatch:**
  - `dispatch::JobKind::LeaveGeneration` carries a `PlayerSpec`.
  - `handler::LeaveRequest` gains `player: PlayerSpec`, and `lexicon` /
    `use_wordmap` derive from it.
- **Where a job's players are listed:** add the leave config's player to
  `jobs::expected_data`, `derived::NEEDS_CTE`, the `delete_player_config`
  guard and the `jobstats` lexicon query.
  - Exclude the player's klv and win% files from the data a leave job needs,
    since leavegen never loads them.
- **Contract fixtures:** `contract-fixtures/assignment-leave-generation.json`,
  in lockstep with MAGPIE's copy.
- **Form:** a player select for leavegen (reuse the opening-rack one),
  filtered to or validated against the limits above.
- **Tests and seeds:**
  - `backend/tests/{leave_gen,magpie_leave,worker_routes}.rs`;
  - `scripts/seed.py`, `scripts/e2e_magpie.py`;
  - the native MAGPIE e2e (`scripts/e2e_magpie_native.sh`).

### 6.4 The leavegen job page shows its player settings

Today `public.rs::job_config` sends an empty `players` list for leave jobs.

- Push `PlayerSettings::new("player", …)` in the LeaveGeneration arm.
  Phase 3's `PlayerSettingsTable` renders it.
- Drop Lexicon and Wordmap from the Leave generation group, because they now
  come from the player. Mark settings leavegen doesn't use (leaves, recorder,
  movegen margin) as unused.

---

## Order and dependencies

1. **Phase 1** is independent; do it first.
2. **Phase 2:** 2.1 (SPRT optional) comes before 2.2 (match score box),
   because the box takes over the W/L/D display from the SPRT card.
3. **Phase 3** comes before 6.4, which reuses its table component.
4. **Phases 4 and 5** are independent of each other and of phase 3.
5. **Phase 6:** 6.2 is birdtest-only and can come any time. 6.3 needs the
   MAGPIE change, landed together with the contract fixtures. 6.4 follows 6.3
   and phase 3.
