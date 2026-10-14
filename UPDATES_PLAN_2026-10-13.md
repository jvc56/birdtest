# Updates plan — the 2026-10-13 batch

Source: `~/Dropbox/birdtest_todo.txt` (2026-10-13), nine items, numbered here
in the file's order, and two the owner added the same day (10, 11):

1. Active jobs come first on `/jobs`.
2. The Significance Test card's middle tile shows the player who is winning,
   not always player 1.
3. Player settings shows only the settings that differ, by default.
4. A job's Contributors card shows each contributor's movegens, and totals
   for movegens, compute time and tasks.
5. Rack lookup gets a **Random** button beside **Search**.
6. On an opening-rack job, the Progress and Opening racks cards are rethought
   to show the tasks and the redundancy.
7. When a job's players use a wordmap, word info table or rack info table,
   the contributor builds them as a best attempt for the storage it has,
   the ones that speed it up most first. No task ever requires one: without
   it the task runs slower, with the same results.
8. "Players", not "configs", on the ratings page and anywhere else a player
   config is meant.
9. The Time Left card's expected end date includes the year.
10. Every move list shows the move's leave, in the column after the move.
11. In a games or game-pairs job's move list, the played move loses its
    **PLAYED** tag and keeps its highlight.

**Branches.** birdtest: `updates-2026-10-13`, off `main` at `f6e4913`.
MAGPIE (item 7 only): a new branch off `origin/main`, which already contains
the pinned `537c90c9`. The local checkout is on `rit-cache-instance-id`, so
check it out fresh rather than building on that branch.

| Phase | Items | Repo | Deploy |
|---|---|---|---|
| 1 | 9, 2, 3, 5, 10, 11 | birdtest frontend | frontend only |
| 2 | 8 | birdtest frontend, backend messages, e2e, JOURNEYS | frontend and backend, no migration |
| 3 | 1, 4 | birdtest backend and frontend | backend and frontend, no migration |
| 4 | 6 | birdtest backend and frontend | backend and frontend, no migration |
| 5 | 7 | MAGPIE, then birdtest | a MAGPIE release, a contract change, a version floor, **migration `0002`** |

## Why this order

- **Phase 1 needs no server change.** Each of its items uses data the page
  already has: `test.mean` says who leads, the toggle between differing and
  all settings already exists, `GET /api/jobs/:id/rack-samples` already draws
  random racks, and the date is a formatting option. A move's leave is its
  rack less the tiles the move uses, both of which the page has.
- **The wording comes next, on its own.** It touches dozens of strings, the
  e2e specs that match them, and JOURNEYS.md. A commit with nothing else in
  it is easy to review and easy to revert.
- **Phase 3 is two small query changes.** One is the jobs list's `ORDER BY`,
  the other the contributor query's sums.
- **The opening-rack cards come after those.** Their layout is settled (D6).
  They need a split of the task counts and one index lookup, with no schema
  change.
- **The storage policy goes last.** It spans two repos, needs a benchmark
  before its order can be set (D8), and changes the contract.

## Rules carried over

- **Schema changes ship as numbered migrations** (owner, 2026-10-13:
  birdtest is deployed). Each change is a new `backend/migrations/0002_<name>.sql`,
  never an edit to `0001_initial.sql`, and there is no database reset. It is
  additive, so the previous image still runs on it: `db.rs` ignores
  migrations it does not know, for rollbacks (README, "After a schema
  change"). Old MAGPIE builds and existing rows must keep working across a
  deploy. Prefer designs that need no schema change: only phase 5 changes
  the schema.
- MAGPIE version: the floor went to 0.1.2 on 2026-10-12, and a contract
  change now comes with a bump. Phase 5 goes to 0.1.3, with a fixture
  recapture.
  - The `jobs` table's floor defaults now change in a migration
    (`ALTER COLUMN min_magpie_patch SET DEFAULT 3`).
  - `U-CFG-5`'s check (`every_copy_of_the_version_floor_agrees`) must read
    the default from the latest migration that sets it, not from `0001`.
  - Existing jobs keep their stored floor of 0.1.2.
- Builds stay light: cap `cargo`/`make` parallelism, no cppcheck or
  clang-tidy on MAGPIE, and ask before any Docker image build.
- No `cargo fmt`.

## Decisions

Each has a recommendation. Confirm or override before the phase that needs it.

| # | Question | Recommendation |
|---|---|---|
| D1 | Item 1: after the active jobs, how are the rest ordered? Statuses are `active`, `inactive` (0% allocation) and `completed`. | Active, then inactive, then completed, each newest first. The `status` filter and pagination keep working because the order is in SQL, not the page. |
| D2 | Item 2: whose score does the middle tile show before anyone leads (no games, or exactly 50%)? | Player 1's, as today. Otherwise the leader's: "{leader}'s score", `1 − mean` with the interval flipped for player 2, in the leader's colour. The bar below keeps its current orientation (player 1 on the left), so it does not jump around when the lead changes. |
| D3 | Item 3: what does the card show by default when the two players are identical, and for a one-player job? | Two players: differences by default. If none differ, a line "The players' settings are identical" in place of an empty table, with the same **All settings** button. One player: unchanged (key settings, with an **All settings** toggle), because nothing can differ. |
| D4 | Item 4: where do the totals come from? The card lists the top 50 contributors and "and N more". | A **Total** row at the foot of the table, over *all* contributors, so it is right when "and N more" is shown. As built: the job's own counters (`jobs.movegens`, `jobs.compute_ms`, the completed tasks), which every submission adds to alongside its contributor's figures, so no window sums are needed. It replaces the "N movegens for this job" line under the table. Movegens is a new column; on narrow screens it hides with Compute time. |
| D5 | Item 5: does **Random** fill the box and look the rack up, or only fill the box? | Fill and look up, as clicking a sample chip does today. It asks for one rack (`rackSamples(id, 1)`). The "Analysed racks to try" chips and **Shuffle** stay. |
| D6 | Item 6: what the two cards say. Options were mocked up at https://claude.ai/artifact/KZL6a53wyeo5uC8iuPPbYA. | **Decided (owner, 2026-10-13):** Progress **C2** and Opening racks **B**. C2 is a layered bar (settled, analysed, not yet) with a small table giving Done and Time left for analysed and for settled. B is a sentence saying where the consensus stands, plus four figures. Neither needs a schema change. See phase 4. |
| D7 | Item 8: do the URLs and API names change too (`/player-configs`, `/api/player-configs`, `player_config_ids`)? | No. Only user-facing text changes, including backend error messages. URLs and fields stay as they are: renaming them means churning routes, the contract and the e2e helpers for no gain a reader sees. The download filename `player-config-{id}.json` stays too. |
| D8 | Item 7: the priority order when not everything fits. No benchmark of the three tables exists. Only qualitative claims do: WMP "dramatically faster", RIT "further still", WIT "costs little". | Measure first (phase 5, step 1), then fix the order from the numbers. The order expected: WMP (179 MB), then WIT (122 MB), then RIT (1.9 GB). An RIT is built from a WMP, so it is never kept without one. |
| D9 | Item 7: how much space counts as available, and is anything evicted? | Available = free space on the data directory's filesystem (`statvfs`), minus a reserve, plus the bytes of tables this client built earlier that the task does not need. A new `contribute.txt` key `diskreserve` (in GB) sets the reserve, default 2 GB. When a table does not fit otherwise, evict tables the client built for other jobs, least recently used first, and only files listed in its own manifest. It never deletes a file it did not build. |
| D11 | Item 10: where does a move's leave come from? Neither move list shows one, and no API sends one. A saved position has `rack`; a rack lookup has the rack looked up. | Computed on the page: `leaveOf(rack, move)` in `lib/cgp.ts`. It reuses `parseMove`. A placement's leave is the rack less the tiles it puts down (a lowercase letter uses a `?`). Played-through squares and `.` squares use nothing. An exchange's leave is the rack less the exchanged tiles, and a pass's is the whole rack. Letters are shown in the rack's order. An empty leave (a bingo) shows "—". A move that cannot be read, or tiles the rack does not hold, also shows "—", rather than a wrong leave. No server or contract change. |
| D12 | Item 11: the **PLAYED** tag is also the row's only non-colour cue. | Drop the visible tag and keep the row's tint. Keep the `(played)` text for screen readers only (`sr-only`), and keep `data-played` for tests. The line above the board ("· played 8G HUH (24)") stays. |
| D10 | Item 7: does the server learn which tables a task skipped? | Yes, in the result: a `derived_skipped` list of roles, stored on the claim (a nullable column, migration `0002`) and shown on the admin's task view. Nothing depends on it for correctness, since results are identical with or without the tables. But a slow worker's timeouts count against the job and can set it aside, and an admin needs to see why. |

## Phase 1 — six page fixes (items 9, 2, 3, 5, 10, 11)

**Item 9: the year in Time Left.** `lib/jobProgress.ts:83-84`'s `when` adds
`year: 'numeric'`. That gives "Oct 14, 2026, 4:00 AM" for both "finished …"
and "about …". `jobProgress.test.ts:78` hard-codes the options, so update it
to match.

**Item 2: the leader's score (D2).** In `lib/components/MatchTestCard.svelte`,
the `test-score` tile (lines 63-78):
- picks `leader = test.mean > 0.5 ? 0 : test.mean < 0.5 ? 1 : 0`;
- shows `names[leader]`'s score, `leader === 1 ? 1 − mean : mean`, with the
  interval `[1 − upper, 1 − lower]` for player 2;
- colours it `text[leader]`.

Put the flip in a small helper in `lib/matchTest.ts` (`leaderScore(test)`)
and unit-test it, including the 50% case.

**Item 3: differences by default (D3).** In `lib/components/JobSettings.svelte`,
two players start in `'differences'`: invert the meaning of `toggled` for the
paired case, so the button reads **All settings** first. If `differing === 0`
in differences mode, show the identical line instead of an empty
`PlayerSettingsTable`. One player is unchanged. Update the component comment,
and the vitest and e2e checks that expect every row on first load (E-14 and
the games-job journeys).

**Item 5: Random.** In `routes/jobs/[id]/+page.svelte:177-188`, add a
`btn-secondary` **Random** beside **Search**. It calls
`api.rackSamples(jobId, 1)`, sets `rackQuery` to the rack, and calls
`lookupRack()`. It is disabled while a lookup is running, and shows the
lookup's error line if no rack comes back.

**Item 10: the leave after the move (D11).**
- `lib/cgp.ts` gains `leaveOf(rack, move)`. Unit tests in `cgp.test.ts`
  cover:
  - a placement;
  - a placement through tiles on the board;
  - a blank;
  - a bingo (empty leave);
  - an exchange and a pass;
  - a multi-letter tile (`[CH]`);
  - a move the rack cannot make ("—").
- In `lib/components/PositionPane.svelte` (the games and game-pairs move
  list), a **Leave** header goes right after **Move**, and each row gets a
  `font-mono` cell holding `leaveOf(position.rack, move.move)`.
- The opening-rack lookup table in `routes/jobs/[id]/+page.svelte` (around
  line 222) gets the same column, using the rack the lookup returned. Use the
  canonical spelling, not what was typed.
- Any other table listing ranked moves gets it too. Grep for the
  `<th>Move</th>` header to find them.

**Item 11: no PLAYED tag (D12).** In `PositionPane.svelte:121-125`, remove
the coloured `played` span and replace it with
`<span class="sr-only">(played)</span>`. The row's `PLAYED_TINTS` background
and `data-played` stay. Update any vitest or e2e check that looks for the
visible tag.

**Verify:** vitest, svelte-check, and tier 5 E-1, E-4, E-7, E-14, plus the
saved-positions journey (the move list).

## Phase 2 — "player", not "config" (item 8, D7)

Every user-visible "config" in the frontend means a player config, so every
one changes. The public nav and the Players page heading already say
"Players".

- **Public pages:**
  - `routes/ratings/+page.svelte`: the "Configs" header becomes "Players"; "anchor config" becomes "anchor player".
  - `routes/ratings/[id]/+page.svelte`: "Player config" headers, "All configs", "Add a player config", "Anchor player config", "Other player configs (joins the pool)", and the two refit sentences.
  - `routes/player-configs/+page.svelte`: "New player config", the load error, "No player configs yet."
  - `routes/player-configs/[id]/+page.svelte`: "a deleted config".
- **Admin:**
  - the tab label in `admin/+layout.svelte`;
  - `admin/player-configs/+page.svelte` and `new/+page.svelte`: headings and help text;
  - `admin/jobs/new/+page.svelte`: the "Player Config" label and five help sentences;
  - `admin/input-data`;
  - `admin/rating-pools/new`;
  - the shared load error, which appears in three places.
- **`lib/roundRobin.ts`'s four messages:** "Tick at least one player.", "At most 12 players: N are ticked.", "1 player → 1 self-play job", "N players → M jobs". Update `roundRobin.test.ts` to match.
- **Backend messages shown to users:**
  - `routes/public.rs` and `routes/ratings.rs`: "no such player", and similar;
  - `routes/admin.rs`: about twenty messages ("player config is invalid", "this static config records…", and so on).
  - **Keep** `admin.rs`'s "config fields do not match the requested job_type", which is about the job request's config block. Update the backend tests that match these strings.
- **Tests and docs:**
  - the e2e specs that match the old text: `e4`, `e7`, `e14`, `e16`, and `e2e/lib/api.ts` where it matches error text;
  - JOURNEYS.md's quoted strings (V-12, A-4, A-5, the round-robin steps, the ratings journeys);
  - section titles may stay ("Player configs" is the API's name) where they describe the endpoint.

**Verify:** vitest, svelte-check, backend nextest (`-j 2`), and tier 5 in
full, since the strings are spread across journeys. Afterwards, grep
`frontend/src` for `[Cc]onfig` in template text, and the only matches should
be identifiers.

## Phase 3 — the jobs list and the Contributors card (items 1, 4)

**Item 1 (D1).** `backend/src/routes/public.rs:143`'s
`ORDER BY j.created_at DESC, j.id DESC` becomes
`ORDER BY CASE j.status WHEN 'active' THEN 0 WHEN 'inactive' THEN 1 ELSE 2 END, j.created_at DESC, j.id DESC`.
Check that the jobs index still serves it; at today's row counts a sort is
fine either way. The admin jobs list should get the same order if it has its
own query. Test: three jobs created completed, active, inactive come back
active, inactive, completed.

**Item 4 (D4).**
- **Backend:** in `jobstats.rs`, `worker_contributions_on` adds
  `SUM(c.movegens)` to each row, and `WorkerContribution` gains `movegens`.
  The totals are the job's own counters: `movegens` and `tasks_completed`
  were already in the payload, and it gains `compute_seconds` from
  `jobs.compute_ms`. Each submission adds to those counters and to its
  contributor's figures together, so the column adds up when every
  contributor is shown. (As built; the first draft summed window totals.)
- **Test:** the totals equal the rows' sums for a job under 50 contributors,
  and exceed the visible rows' when there are more. Compare the movegens
  total with `jobs.movegens` and note any reason they could differ (a
  rejected result's movegens, for example). If they can differ, say which
  one the card shows.
- **Frontend:** `lib/api.ts` types; `WorkerTable.svelte` gets a **Movegens**
  column (`exactCount`) and an optional totals row (a `<tfoot>`, bold). The
  job page passes the totals and drops the line under the table. Other pages
  that use `WorkerTable` without totals are unchanged.

**Verify:** backend nextest (`-j 2`), vitest, svelte-check, tier 5 E-1 and
E-4.

## Phase 4 — the opening-rack cards (item 6, D6)

**What the cards say today.** Progress shows "Done: X of N racks settled", a
throughput in racks an hour, the time left, and a tasks line: "X in progress
· Y waiting to be reissued · Z done". The Opening racks card shows racks
analysed, and with consensus also racks settled, settled without a
consensus, and the rule ("each rack is analysed 3 to 7 times…").

What a reader cannot see from either:
- The tasks have two kinds. First-pass tasks cover the rack space once, in
  slices of `racks_per_batch`. Re-analysis tasks list unsettled racks again,
  for consensus.
- That the job is moving during the first pass. With consensus, no rack
  can settle before its `min_results_per_rack`-th analysis, so the bar
  (racks settled) sits at 0.0% for the whole first pass.
- Where the consensus stands: which analysis open racks are getting now,
  how many are waiting for agreement, and how many agreed.
- "Waiting to be reissued" means a task that was given back or timed out.
  That is a different thing from a consensus re-analysis, and the two words
  collide.

**What changes (D6: C2 and B).** Both cards change only on opening-rack
jobs, except the "to be handed out again" wording, which applies to every
job type. No schema change, and no database reset.

*Progress card (C2).*
- **Headline:** with consensus, "35.8% settled · 100.0% analysed". A job
  analysing each rack once keeps a single percentage.
- **Layered bar:**
  - settled racks are solid (`bg-primary`);
  - analysed but not settled racks are faint (`bg-primary/35`);
  - the rest is the track.
  - `aria-valuenow` stays the settled share, and `aria-valuetext` reads
    "35.8% settled, 100% analysed".
- **A small table** with columns Done and Time left. It replaces the Done
  and Time left figures.

  |  | Done | Time left |
  |---|---|---|
  | ▨ Analysed | 3,199,724 (of 3,199,724 · 100.0%) | — (every rack analysed) |
  | ■ Settled | 1,144,000 (of 3,199,724 · 35.8%) | 22.3h (about Oct 14, 2026, 1:20 PM) |

  - A job analysing each rack once has one row, **Analysed and settled**.
  - **Analysed time left is computed on the page.** Re-analysis starts only
    once the first pass covers the rack space, so until then every analysis
    goes to a new rack. The figure is racks not analysed divided by the
    current throughput. Once every rack is analysed it shows "—", with
    "every rack analysed".
  - **Settled time left** is today's server estimate (`eta_seconds`, via
    `timeLeftFigure`). A finished job shows "finished <date>".
  - Both rows use the existing idle wording ("paused while inactive", "no
    estimate yet").
- **Throughput and Allocation** stay as figures under the table.
  - Throughput is labelled **analyses/hour**: it counts tasks ×
    `racks_per_batch`, which is analyses, not distinct racks.
  - Change the opening-rack throughput unit in `jobstats.rs` to
    `analysis`, and make sure `throughputParts` gives the plural
    "analyses".
- **The tasks line:** "Tasks: 4,120 of 6,400 first pass · 9,000
  re-analysis · 40 in progress · 3 to be handed out again".
  - The first-pass total is `ceil(total_racks / racks_per_batch)`.
  - The re-analysis count appears only when `max_results_per_rack > 1`.
  - "To be handed out again" replaces "waiting to be reissued" on every job
    type.

*Opening racks card (B).*
- **One analysis per rack:** "Each rack is analysed once, by one
  contributor. No second analysis checks it." Then racks analysed of the
  total, as today.
- **With consensus:** first a bordered sentence saying where the consensus
  stands.
  - **First pass:** "First pass. 64.4% of racks have their first analysis.
    None can settle before its 3rd. Re-analysis starts when every rack has
    one." When `min_results_per_rack` is 1, a rack can settle at its first
    analysis, so the middle sentence goes.
  - **Re-analysis:** "Re-analysis. Every open rack has at least 2 analyses,
    and racks with the fewest are analysed first." This is phrased loosely
    on purpose. The reissue picks fewest first, but it also skips racks in
    flight and prefers racks the claimant has not analysed, within a window
    of 4 batches.
  - **Finished:** "Finished. Every rack is settled: 3,174,757 agreed, 24,967
    without consensus after 7 analyses."
- Then four figures:
  - **Waiting for agreement:** analysed − settled.
  - **Agreed:** settled − without consensus, with "x% of settled racks".
  - **Without consensus:** "split after 7 analyses".
  - **Racks analysed:** of the total.
- The rule sentence stays under them. The rack lookup is unchanged.

**Backend.** No schema change. Both reads go into the cached stats payload
(`jobstats.rs`).
- Split the completed-task counts by kind:
  - a first-pass task's seed is below `total_racks`, a re-analysis task's
    is at or above it;
  - one `FILTER` on the existing count, joined to the job's opening-rack
    config.
- `fewest_open_analyses`:
  - `SELECT MIN(results) FROM opening_rack_progress WHERE job_id = $1 AND NOT settled`;
  - one probe of the existing partial index `opening_rack_progress_unsettled_idx`;
  - read only once every rack is analysed and `max_results_per_rack > 1`,
    otherwise null.
- The payload's `opening_racks` gains `first_pass_tasks_done`,
  `first_pass_tasks_total`, `reissue_tasks_done` and
  `fewest_open_analyses`. The fields it already has (`racks_analyzed`,
  `racks_settled`, `racks_without_consensus`, `racks_total`) cover the
  rest.
- Tests:
  - the split adds up to `tasks_completed`;
  - `fewest_open_analyses` matches a direct query after a run of results,
    after a consensus change (`restate_racks`), and is null once every rack
    is settled.

**Frontend.**
- `lib/jobProgress.ts` gains `rackFronts(stats, now)`, giving the table's
  rows, and the layered fractions.
- `JobProgressCard.svelte` gets an opening-rack branch: the headline, the
  bar, the table, Throughput and Allocation.
- The Opening racks card in `routes/jobs/[id]/+page.svelte` takes the stage
  sentence and the figures. The sentence comes from a `consensusStage(stats,
  config)` in `lib/consensus.ts`.
- Unit tests cover: first pass, re-analysis, nearly done, finished, one
  analysis per rack, inactive, and `min_results_per_rack = 1`.
- Update the e2e checks that read the Progress card's figures or "waiting to
  be reissued" (E-1, E-4, the opening-rack journeys).

**Verify:**
- backend nextest (`-j 2`);
- vitest and svelte-check;
- tier 5 (E-1, E-4, the opening-rack journeys);
- tier 6 with an opening-rack job at `min 2 / max 3`. The stage sentence
  must move from the first pass to re-analysis to finished, and end with 0
  waiting for agreement.

## Phase 5 — tables as storage allows (item 7, D8–D10)

What exists today:
- A worker never downloads a wordmap, word info table or rack info table. It
  builds each one locally and checks it against the SHA-256 the claim pins.
- All three are optional accelerators. MAGPIE's tests show identical moves
  with and without them, and movegens count calls to the move generator, so
  the count does not change either. Skipping one is safe for results and for
  accounting.
- There is no disk check, size cap or eviction (README.md:633-638 says so and
  proposes this fix).
- A full disk during an RIT build ends the worker (`fwrite_or_die`, KL-51).
- A local write failure is counted, not remembered (KL-52).
- So today a player's `use_wordmap`, `use_wit` or `use_rit` behaves like a
  requirement. A contributor that cannot build the table declines the task
  or crashes; it never runs the task without the table.

**What changes in meaning.** These settings become "use the table if the
contributor can". A contributor short of space runs the task without the
tables that do not fit. That is slower, but the moves and the movegen counts
are the same. Nothing about a job asks for a contributor with more storage,
and every contributor stays eligible for every job.

1. **Benchmark (D8).** A script in MAGPIE's `scripts/` plays a fixed set of
   CSW24 games with a static player and a 2-ply simmer, under each of: none,
   WMP, WMP+WIT, WMP+RIT, all three. It reports movegens/s and seconds a
   game. Record the numbers and the resulting order in birdtest's PLAN.md,
   beside the sizes in README.md's table. `taskset -c 0-3`, short runs.
2. **The server sends sizes.** `derived_data.bytes` is already stored. Add
   `bytes` to `ExpectedDerived` (`derived.rs`, sent in the claim's
   `expected_data.derived`), so a worker knows how big a table will be before
   it builds it.
3. **MAGPIE: the policy.**
   - One function, `contribute_choose_tables`, takes the tables the task's
     players use (both players') with their sizes, the free space, the reserve
     (`diskreserve`), and the manifest. It returns the tables to use, in the
     priority order from step 1, never an RIT without its WMP.
   - It replaces the `contribute_wants_*` calls (`config.c:8430-8455`) in the
     three task types (games around `config.c:9928`, opening rack around
     10297, leave generation around 10500). The flags are set before the
     lexicon loads.
   - Eviction (D9) runs in the `ensure_*` functions just before a build, under
     the existing `.lock`. A manifest `contribute_tables.txt` in the data
     directory lists what the client built, with its last use.
   - Builds check free space first, so a full disk skips the table instead of
     crashing. The RIT writer returns an error instead of `fwrite_or_die`,
     which closes KL-51.
   - A log line names what was skipped and why: "skipping the rack info
     table for CSW24.CSW24: 1.9 GB needed, 0.8 GB free".
   - Tests: the chooser on its own (every combination of space and requests,
     including the RIT-needs-WMP rule), and a build that hits a simulated
     full disk and declines gracefully.
4. **The result says what was skipped (D10).**
   - `derived_skipped` joins the result body.
   - The server accepts the field missing (a result from a 0.1.2 build) and
     stores null.
   - birdtest stores it on the claim. Migration
     `0002_claim_derived_skipped.sql` does
     `ALTER TABLE task_claims ADD COLUMN derived_skipped TEXT[]`. The column
     is nullable and additive, so the previous image runs on it unchanged.
   - Shown on the admin's task and worker views.
5. **The contract.** `MAGPIE_VERSION` / `MIN_MAGPIE_VERSION` go to 0.1.3.
   - The same migration sets the `jobs` floor defaults to 0.1.3
     (`ALTER COLUMN ... SET DEFAULT`), and `U-CFG-5` reads them from it.
   - Existing jobs keep their floor of 0.1.2, so 0.1.2 builds still claim
     them. Check that a 0.1.2 build accepts the new `bytes` key in
     `expected_data.derived`. If it refuses unknown keys there, send `bytes`
     only to claims reporting 0.1.3 or later (the claim body carries
     `magpie_version`).
   - Fixtures: the hand-written `assignment-*.json` gain `bytes` on derived
   entries. Recapture the captured ones with `scripts/e2e_magpie_native.sh
   --cases capture --capture-out contract-fixtures` and copy them to MAGPIE's
   `test/birdtest_contract/`.
   - Pin birdtest to the merged MAGPIE commit.
6. **Docs.**
   - README.md's contributor-cost section: the policy and `diskreserve`. Also
     correct the stale "WIT off by default" and the inaccurate "a full disk
     gives a declined task".
   - PLAN.md: close KL-51, and note whether KL-52 still applies.
   - TESTING.md: the new tests.

**Verify:**
- MAGPIE: the new tests and the `contribute` suite on
  `BUILD=test_no_pgo_release`, plus a compile check on `BUILD=dev`, with
  `-j3` and `taskset -c 0-3`.
- birdtest: backend nextest and the contract-fixture test.
- The migration: it applies to a database at `0001` that already holds jobs
  and claims, and the previous image (`main` before this phase) still starts
  and serves against the migrated schema, which is the rollback case.
- Tier 6, plus a case run with a small filesystem (a loop mount or tmpfs as
  the data directory) where the RIT cannot fit. The job must complete and the
  admin view must show the RIT skipped.

## Verification summary

| Phase | Unit | Backend | e2e (tier 5) | MAGPIE | Tier 6 |
|---|---|---|---|---|---|
| 1 | vitest, svelte-check | — | E-1, E-4, E-7, E-14, saved positions | — | — |
| 2 | vitest, svelte-check | nextest | full | — | — |
| 3 | vitest, svelte-check | nextest | E-1, E-4 | — | — |
| 4 | vitest, svelte-check | nextest | E-1, E-4, opening-rack journeys | — | an opening-rack consensus job |
| 5 | — | nextest, contract, migration and rollback | full | new and contribute | full, plus a small-disk case |

## Not in this plan

- Memory as a constraint on tables: an RIT holds 1.9 GB of RAM while it
  plays. Item 7 is about storage only. A RAM-aware chooser would extend the
  same function.
- Renaming `/player-configs` and the API's `player_config` names (D7).
