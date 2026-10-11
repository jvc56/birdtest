# Updates plan — the 2026-10-10 batch

Source: `~/Dropbox/birdtest_todo.txt` (2026-10-10). Supersedes `UPDATES_PLAN.md`,
whose seven phases all shipped (PR #21).

**Branch:** no new branch. Every phase is one or more further commits on the
existing `todo-batch-2026-10-09`, on top of `d9aa01d`, and goes out with that
branch's PR.

The eleven items are regrouped into six phases. Each is its own commit (or run
of commits) and leaves the branch green, so the branch can be deployed after
any one of them.

## Why the order changed

- **The access reversal goes first** (todo 5). It decides *who* every later page
  is written for. Two of the later phases — the saved-position pane and the
  ratings cards — are layout and visibility work whose audience is the
  question; settling it first means they are written once rather than revised
  when the audience changes.
- **One page, one pass.** Four items (1, 8, 3, 10) all land on the job page and
  the two modules behind it (`JobSettings.svelte`, `lib/jobSettings.ts`).
  Shipped separately they are three passes over the same file and three rounds
  of e2e repair.
- **The saved-position pane comes after the card shuffle**, so the page around
  it is already in its final order when the grid is retuned.
- **The two ratings items ship together** (6, 9). Item 9 paints cells that item
  6's reordering moves, and both touch the same amber "predicted badly"
  marker — done apart, the second one re-does the first one's CSS.
- **The two schema items go last and adjacent** (4+11, then 7). Phases 1–4 are
  deploys with no migration; phases 5 and 6 each edit `0001_initial.sql` and so
  need a database reset, and put last they can share one.
- **The rename rides with the totals that justify it** (11 with 4). "Contributions"
  is only the right name once the page leads with the site's totals.

## Rules carried over

- Pre-launch still: edit `0001_initial.sql` in place, no compat shims, warn
  before the deploy that resets the database.
- Commit per phase on `todo-batch-2026-10-09`; no new branch, no force-push.
- `MAGPIE_VERSION` / `MIN_MAGPIE_VERSION` stay at 0.1.1. **No MAGPIE change and
  no fixture recapture in this batch**: the per-job time limit (phase 6) keeps
  the assignment's `max_task_seconds` field exactly as it is on the wire, so
  `contract-fixtures/assignment-*.json` stay at 3600.
- Verification per tier at the end of each phase (see "Verification").

## Decisions

Settled 2026-10-10. D3 overrides the earlier recommendation; the rest are taken
as recommended.

| # | Question | Decision |
|---|---|---|
| D1 | Phase 2 replaces the Movegens card. The figure then has nowhere on the job page — the last batch added it deliberately. Keep it or drop it? | Keep it as a line under the job's **Contributors** table ("N movegens for this job"), where the contributor figures already are. |
| D2 | What does the Player settings card show by default once the toggle is "All settings"/"Different settings only"? | **Different settings only**, when the job has two players. A one-player job (opening-rack, leave) and the player-config page have no differences, so they keep today's key/all toggle. |
| D3 | Does `settings.max_task_seconds` survive as the **default for new jobs**, or does `/admin/settings` go away entirely (it holds nothing else)? | **`/admin/settings` goes away entirely**: the page, its nav link, `GET`/`PUT /api/admin/settings`, the `settings` table and the `settings.changed` audit action. A new job's limit defaults to 3600, the `jobs` column's own default, and the creation form pre-fills it. |
| D4 | Can a job's limit be changed after creation? | Yes, on the job's Manage page, like the consensus patch. A limit you cannot lower after watching tasks time out is the setting the job page already tells you to fix. |
| D5 | Item 9: which cells count as "even"? | The one on screen: a cell displaying `50.0%` gets no tint, whatever `actual` is to fifteen places. Compare the rounded figure, not the float. |
| D6 | Item 10: better terms than IGP/PGP? | Spell them out as asked: **"Intra-game parallelism (all threads on one game)"** and **"Per-game parallelism (one game per thread)"**. The stored values stay `igp`/`pgp`. |
| D7 | Item 11: how far does the rename reach? | Nav link, `<h1>` and prose only. The URL stays `/workers` (no redirect to maintain), and the per-job **Contributors** card keeps its name — it lists contributors, not contributions. |

---

## Phase 1 — Anyone may see what a user sees (todo 5)

The reversal: an account's only privilege is making API keys. Saved positions
are the one thing a signed-out visitor is refused today.

**1a. Backend** (`backend/src/routes/public.rs`)
- Drop `_user: crate::auth::CurrentUser` from `job_positions` (:1442) and
  `random_position` (:1524). They are the only two non-admin handlers in the
  public router that take it; nothing else in `routes/public.rs`,
  `routes/ratings.rs` or `jobstats.rs` gates on a session.
- Nothing else moves: `/api/me/*` (`routes/account.rs`) stays exactly as it is.
  That is the account privilege, and after this it is the only one.

**1b. Frontend** (`routes/jobs/[id]/+page.svelte:171-193`)
- Delete the `{#if $session}` / `{:else if $session === null}` fork. Render
  `<SavedPositions …>` whenever `config?.games?.capture_positions`.
- Remove the "Sign in to search them" card with it.
- `SavedPositions.svelte`'s doc comment (:5) says "Signed-in users only — the
  routes refuse anyone else"; rewrite.

**1c. Tests**
- `backend/tests/authz.rs:308-309`: both rows move from `Session` to the public
  class. The table is checked against the router's source, so this is required,
  not optional.
- `e2e/tests/e12-saved-positions.spec.ts:64-69`: the visitor context should now
  assert it *sees* a position (`getByTestId('saved-position')`), not the sign-in
  line. Rename the test — it is no longer "a signed-in user draws…".
- `backend/tests/public_api.rs`: add a case reading `/positions/random` with no
  cookie and expecting 200.

**1d. Docs**
- `PLAN.md:1082` — drop "Signed-in users only; a signed-out visitor is told to
  sign in."
- `JOURNEYS.md:1047` — the `curl` expecting `401` now expects `200`.
- `TESTING.md` — the `A-AUTHZ` prose and any positions row in the route table.
- Wherever the site says what an account is *for*, make it one sentence: an API
  key, so your contributions carry your name. `routes/+page.svelte:90-93`
  already says this and needs no change.

---

## Phase 2 — The job page's cards (todo 1, 8, 3, 10)

### 2a. Active Contributors replaces Movegens (todo 1)

**Backend** (`backend/src/jobstats.rs`)
- Add `active_contributors: i64` to `JobStats` (:26-55).
- Compute it in `compute_inner` (:462): distinct identities holding an open
  claim on this job that is still alive — `task_claims` joined to `tasks`,
  `state = 'claimed'`, `COALESCE(last_heartbeat_at, claimed_at) > now() -
  heartbeat_timeout`, counting `COALESCE(claimed_by_user_id::text,
  claimed_by_anon_uuid::text)` distinctly. The timeout is
  `cfg.heartbeat_timeout` (`config.rs:299`, 300 s), the same clock reclamation
  uses — so "active" here means what it means everywhere else on the site.
- It rides the same stats payload, so the SSE stream carries it with no new
  endpoint.

**Frontend** (`lib/components/JobStatsRow.svelte:30-33`)
- Third card becomes **Active contributors**, a plain count.
- Its doc comment (:5-11) names movegens as one of the four figures; rewrite.
- The comment at :15 ("E-10 counts `.grid > .card` for these four") still holds —
  four cards, same grid.
- Per D1, add the movegens line under the Contributors table on both job pages
  (`routes/jobs/[id]/+page.svelte:387-395`,
  `routes/admin/jobs/[id]/+page.svelte:633-637`).

**Tests** — a `jobstats` integration test for the count (claimed and
heartbeating = 1; the same worker twice = 1; a stale heartbeat = 0). The four
card names are asserted in `e2e/tests/e1-anonymous-browsing.spec.ts:49` —
"Movegens" becomes "Active contributors" there.

### 2b. Settings cards go last (todo 8)

- `routes/jobs/[id]/+page.svelte`: move the `{#if config}<JobSettings …>` block
  (:162-164) from above Saved positions to below the Contributors card
  (after :395).
- `routes/admin/jobs/[id]/+page.svelte`: same move, :590-592 to after the
  Contributors card (:633-637).
- `JobSettings.svelte` renders **two** cards (Job settings, Player settings);
  moving the component moves both, which is what the item asks.
- Check `e2e/tests/e10-phone-width.spec.ts` and `e11` for card-order or
  nth-card selectors.

### 2c. Player settings: differences or all (todo 3)

This reverses last batch's 1g. Three removals and one relabel.

- `lib/components/PlayerSettingsTable.svelte`
  - Remove `showShared` / `folded` (:32-36) and the "Show N shared settings" /
    "Hide shared settings" button (:106-117).
  - Remove the amber `Differences (N)` group header row (:77-85) and the
    `bg-warning/10` / `border-l-warning` tinting on the difference rows
    (:87-91). Differing rows become ordinary rows.
  - Replace the `all` boolean with `mode: 'differences' | 'key' | 'all'`
    (per D2), and render one `<tbody>`: `differences` for `'differences'`,
    `keySettings` for `'key'`, `playerRows` for `'all'`.
  - Keep the muted-`unused` note (:40-44), the colour dots and the `data-setting`
    attributes — `e2e` and the component test use them.
- `lib/jobSettings.ts:460-481` — `settingBlocks` loses its reason to exist in
  its present shape. Either return the three lists, or export
  `differingSettings(players, unused)` and let the card pick. The latter is
  smaller: `playerRows(...).filter(r => r.differs)` is already the body.
- `lib/components/JobSettings.svelte:43-50` — the top-right button becomes
  `{mode === 'all' ? 'Different settings only' : 'All settings'}`, with
  `aria-expanded` dropped (it is not a disclosure any more) in favour of
  `aria-pressed`.
- `routes/player-configs/[id]/+page.svelte:54-59` — one player, no differences:
  passes `mode={all ? 'all' : 'key'}` and keeps its own label.
- An empty state is now reachable: two identical configs in
  "different settings only". Say so in the card — "These players' settings are
  identical." — rather than showing an empty table.
- **Tests**: `PlayerSettingsTable.test.ts` and `jobSettings.test.ts` (the
  `settingBlocks` cases); `JOURNEYS.md:165` and `:384`-adjacent prose;
  `TESTING.md`'s description of the card.

### 2d. IGP and PGP spelt out (todo 10)

Per D6, "Intra-game parallelism (all threads on one game)" and "Per-game
parallelism (one game per thread)", in four places. The stored values do not
change:
- `lib/jobSettings.ts:232-234` (`threadingText`) — the Job settings card's row.
- `routes/admin/jobs/new/+page.svelte:574-580` — the two `<option>`s and the
  help text under them.
- `backend/src/routes/admin.rs:1457-1458` — the doc comment.
- `lib/jobSettings.test.ts:119-120`, and the prose in `PLAN.md:6027`,
  `TESTING.md:887-888`, `JOURNEYS.md:732-737`.

---

## Phase 3 — The saved-position pane (todo 2)

The pane was built when two move lists sat side by side. One shows at a time
now, so the right-hand column is far wider than it needs to be and the board is
smaller than it could be. `saved_pos.png` is the symptom.

**3a. The grid** (`lib/components/PositionPane.svelte:73`)
- Today: `grid gap-4 xl:grid-cols-[27rem_minmax(0,1fr)]` — the board is pinned
  to 27rem at every width above `xl`, and everything left over goes to a move
  table that does not need it.
- Change to a proportional split that still has a floor:
  `lg:grid-cols-[minmax(24rem,1fr)_minmax(0,1fr)]`. Equal shares, the board
  never under 24rem, and the breakpoint drops from `xl` (1280px) to `lg`
  (1024px) so a laptop gets the two-column layout at all.
- `items-start` on the grid, so the short column does not stretch.
- The board's own `max-w-xl` (`Board.svelte:83`) is what caps it at 36rem;
  raise to `max-w-2xl` (42rem) so a wide screen actually spends the extra room
  on the board.

**3b. Square rack tiles** (`Board.svelte:199-216`, `:262-274`)
- Cause: the tile row is a flex container at default `align-items: stretch`
  inside `min-h-8`, so each tile's *height* is stretched to the line while its
  *width* shrinks below the `2rem` basis — `aspect-ratio: 1` loses to an
  explicit stretch. Hence rectangles.
- Fix: `items-start` (or `items-center`) on the row at :201, and give the tile
  `flex: 0 0 auto; width: 2rem; height: 2rem` with the aspect ratio kept as
  belt and braces. With the board column wider, seven tiles at 2rem fit each
  half comfortably; below that, let the tiles scale with
  `width: clamp(1.4rem, 100%, 2rem)` rather than deform.
- Keep `data-testid="rack"`, the score superscript and the blank colouring —
  `e12` asserts on them.

**3c. The inference table** (`PositionPane.svelte:143`)
- `w-full` is why it spans the whole right column. Make it `w-auto` like the
  move table above it, so both are as wide as their content and the column's
  slack is simply slack — or, better, cap it: `max-w-md`. Its three columns
  (Leave, Draws, Equity) need nothing like half a screen.

**3d. Narrow screens**
- Unchanged in kind: one column, board first. Verify at 393px that the move
  table still scrolls inside its own box and the page does not widen
  (`e2e/tests/e10-phone-width.spec.ts`).

**Tests** — `e10` (phone width, no horizontal overflow), `e12` (board, rack,
played-move markers), `e17` (pair divergences, the toggle). Add a width
assertion only if one is cheap; this is mostly a look-at-it change, so take a
screenshot at 1440px and at 393px and compare against `saved_pos.png`.

---

## Phase 4 — The ratings page (todo 6, 9)

**4a. Cross table first, All configs admin-only** (todo 6)
- `routes/ratings/[id]/+page.svelte`: move the Cross table card (:362-427) to
  directly after the Ratings dot plot (:180-183), and wrap the All configs card
  (:185-306) in `{#if isAdmin}`.
- The Anchor card (:308-360) is already admin-only and stays last of the admin
  cards. Final order: header → **Ratings** → **Cross table** → *(admin)* All
  configs → *(admin)* Anchor.
- What a visitor loses with the card: the exact rating to a decimal, ± SE as a
  number, and pairs played per config. The dot plot keeps the interval visually
  and the cross table keeps the rating column, which is the redundancy the item
  is about. The "not yet rated" members listed under the plot stay visible.
- The WESPA-scale note at :259-263 lives inside the hidden card — move it under
  the cross table so every reader still gets it.
- **Tests**: `e2e/tests/e7-ratings.spec.ts:69` locates the All configs card as
  an admin, which still works; add a signed-out assertion that the card is
  absent and the cross table is the first card after the plot.
  `JOURNEYS.md:384-387` describes both cards and their order.

**4b. Green and red cells** (todo 9)
- `routes/ratings/[id]/+page.svelte:396-404`: the cell already carries
  `class:text-warning={missed.has(...)}`. Add a background tint from the record:
  `actual > 0.5` → `bg-success/10`, `< 0.5` → `bg-destructive/10`, even → none.
  Both tokens exist (`tailwind.config.js:17-19`).
- Per D5 the comparison is on the **displayed** figure. Put it in
  `lib/ratingPool.ts` beside `winText` as `recordSide(cell): 'win' | 'loss' |
  'even'`, computed from the same rounding `scorePct` applies, and unit-test it
  in `ratingPool.test.ts` (50.04% → even; 50.1% → win).
- Keep the amber marker readable on the tint: amber stays the *text* colour, the
  record is the *background*. A cell can be both (a win the ratings predict
  badly) and must still read as both — check that pairing explicitly.
- Colour is not the only carrier: the percentage itself is in the cell and the
  hover (`cellTitle`) spells it out, so this adds nothing a colour-blind reader
  depends on. Keep it that way — do not drop the ± SE text to make room.
- `JOURNEYS.md`'s cross-table step gains the green/red expectation.

---

## Phase 5 — Contributions (todo 4, 11) — **first schema change, needs a reset**

### 5a. Totals, then the breakdown (todo 4)

Reading of the item: the page leads with three site totals — movegens, compute
time, tasks — and under them the same three broken down by job type; a
contributor's own dropdown shows one **row** per job type with those three
figures, not the current 2×2 grid of movegens alone.

**Schema** (`backend/migrations/0001_initial.sql`)
- Add `compute_ms BIGINT NOT NULL DEFAULT 0 CHECK (compute_ms >= 0)` to `jobs`,
  beside `movegens` (:556). `jobs` already carries `movegens` and
  `tasks_completed`; compute time is the one of the three it lacks.
- Extend `task_claims_user_idx` / `task_claims_anon_idx` (:1926-1929) to
  `INCLUDE (movegens, claimed_at)`. They already key on
  `(identity, job_id, completed_at)`, so with `claimed_at` alongside, a
  contributor's compute time and completed-task count are still an index-only
  walk — which `public_api::a_contributors_breakdown_reads_only_their_index`
  requires.
- The purge/delete paths that give a contributor's totals back
  (`routes/admin.rs:3249-3305`) must zero `jobs.compute_ms` the same way they
  zero `movegens`.

**Backend** (`backend/src/routes/worker.rs`, `routes/public.rs`)
- `worker.rs:1130-1145`: add `compute_ms = compute_ms + $8` to the single
  `UPDATE jobs` the submit transaction already makes, binding the `compute_ms`
  it computed at :1043. No new statement, no new lock.
- `site_movegens` (:2129-2136) becomes one grouped query over `jobs` —
  `SELECT job_type, SUM(movegens), SUM(compute_ms), SUM(tasks_completed)
   FROM jobs GROUP BY job_type` — and the totals are the sum of its rows. One
  scan of a small table gives both halves of the card.
- `contributor_movegens_query` (:2149-2175) gains, per job type,
  `SUM(c.compute_ms_expr) FILTER (WHERE c.completed_at IS NOT NULL)` and
  `COUNT(*) FILTER (WHERE c.completed_at IS NOT NULL)`. **Use a FILTER, not a
  `WHERE state = 'completed'`** — the existing comment explains that naming
  `state` invites the fleet-wide index instead of the identity one, and the
  compute expression (`CLAIM_COMPUTE_MS`, `worker.rs:883`) is NULL rather than 0
  for an open claim, so it cannot simply be summed.
- `MovegensByType` (:2098-2120) becomes a per-type triple. Rename it —
  `ContributionsByType` — since it is no longer movegens; the endpoint paths can
  stay (`/api/workers/movegens`) or move to `/contributions` with the rename.
  Keep the "every type always present, at 0" property: the page lists four rows
  whatever the data.

**Frontend** (`routes/workers/+page.svelte`, `lib/movegens.ts`)
- The card at :151-167 becomes: a totals row of three figures
  (`exactCount(movegens)`, `computeTime(seconds)`, `tasks.toLocaleString()`),
  then a four-row table — job type × the same three columns.
- The per-contributor dropdown (:262-270) drops `sm:grid-cols-2` for a small
  table: one row per job type, three columns, the same headers as the card.
- The two testids are read by the journeys — `site-movegens`
  (`e1:62`, and `e10:44` navigates by the card) and `contributor-movegens`
  (`e1:76`) — and `e10:147,151` holds the **Movegens** column header on screen
  at phone width. Renaming either testid means editing those four places;
  keeping them is cheaper and costs only a stale name in the markup.
- `lib/movegens.ts:22-28` — `movegensLines` returns the triple per type; keep
  `MOVEGEN_TYPES` as the fixed order.
- The intro paragraph (:144-149) explains the three figures already; fold the
  totals into it.

### 5b. The page is Contributions (todo 11)

Per D7: `routes/+layout.svelte:15` label, `routes/workers/+page.svelte:143`
heading and the prose around it. The route stays `/workers`; the job pages'
**Contributors** card keeps its name — which makes the rename a *split*, so the
e2e selectors have to be read one by one rather than swept:
- `e1-anonymous-browsing.spec.ts:56,58` — nav link and page heading: rename both.
- `e10-phone-width.spec.ts:106,146` — the header link list and the tap target:
  rename both.
- `e1:53` and `e4-live-dashboard.spec.ts:136` — the *job page's* Contributors
  card: leave alone.

---

## Phase 6 — The task time limit is a job's, not the site's (todo 7)

Per D3, the site-wide setting goes and nothing replaces it: the limit is a
job's, set at creation and changed on the job's Manage page. `/admin/settings`
holds nothing else, so the page, its API and its table go with it.

**Schema** (`0001_initial.sql`)
- `jobs` gains `max_task_seconds INT NOT NULL DEFAULT 3600 CHECK
  (max_task_seconds BETWEEN 600 AND 86400)` — the same bounds and the same
  reasoning as the settings column (:1841-1854), whose comment moves with it.
  The column default is the default for new jobs; there is no other.
- Drop `CREATE TABLE settings` and its `INSERT INTO settings DEFAULT VALUES`
  (:1835-1860). It holds the limit and who last changed it, and nothing else.
- Rewrite the two comments that cite `settings.max_task_seconds` —
  `jobs.time_limit_*` (:490) and `task_claims.deadline_at` (:1010) — to cite
  `jobs.max_task_seconds`.

**Claim path** (`backend/src/scheduler.rs:1357-1363`)
- The deadline comes from the job instead of the settings row: the `INSERT …
  SELECT … FROM settings s` becomes `… FROM jobs j WHERE j.id = $6` — the job id
  is already bound to that statement. One statement still writes the deadline
  and returns the limit, so assignment and deadline cannot disagree; keep that
  property and its comment. The job's row lock is taken later, by the
  `UPDATE jobs` at :1422, and does not move.
- Nothing on the wire changes: the assignment still carries `max_task_seconds`
  (`routes/worker.rs:242`, :409), so MAGPIE, the fixtures and tier 4 are
  untouched.

**Admin API** (`backend/src/routes/admin.rs`)
- Delete `SettingsView`, `SettingsBody`, `read_settings`, `get_settings` and
  `put_settings` (~:72-140) and their two routes. Keep `MIN_TASK_SECONDS` /
  `MAX_TASK_SECONDS` (:69-70) — the job routes use them now.
- `CreateJobBody` (:1298-1325) gains `max_task_seconds: Option<i32>`; omitted
  means the column default (3600), and anything outside
  `MIN_TASK_SECONDS..=MAX_TASK_SECONDS` is a `400` on the field, with the
  wording `put_settings` uses today.
- Per D4, add `PATCH /api/admin/jobs/:id/time-limit` beside the consensus patch
  (:26). Its audit row takes over `settings.changed`'s shape under the name
  `job.time_limit_changed`, after `job.consensus_changed`: "max_task_seconds
  3600 -> 1800", and nothing written when the value is unchanged. Claims
  already made keep their deadlines; say so in the response and the UI, as the
  settings page does today.
- `authz.rs`'s route table: remove the two `/api/admin/settings` rows
  (:236-237), add the `PATCH` row.

**Frontend**
- Delete `routes/admin/settings/` and the **Settings** link in
  `routes/admin/+layout.svelte:17`. In `lib/api.ts`, delete the `Settings` type
  (:578), `settings` and `updateSettings` (:857-859); add the time-limit patch
  beside the consensus one.
- `routes/admin/jobs/new/+page.svelte`: a **Task Time Limit (Seconds)** field in
  the per-type section, pre-filled with 3600, with the settings page's help
  text (:81-87) moved to it.
- `routes/admin/jobs/[id]/+page.svelte`: the limit in the Controls card, with a
  Save, next to where the job already reports its time-limit declines.
- `lib/jobSettings.ts:175-231` (`jobSettings`): a **Task Time Limit** row, so
  the job's limit is visible on the public job page like every other setting.
- `JobStatusCard.svelte:43-47` already shows the time-limit notice; its text
  (`lib/format.ts`, `timeLimitNotice`) should now point at the job's own limit.

**Tests and docs**
- `backend/tests/admin_routes.rs:878-926` (the settings round-trip) becomes its
  job-scoped equivalent: a job created without a limit has 3600; the patch
  refuses 599 and 86,401 with a `400` on the field, and the column's CHECK
  refuses a direct `UPDATE jobs SET max_task_seconds = 599`; a change writes
  one `job.time_limit_changed` row, from what to what; the same value again
  writes none. Creation with an out-of-range limit is a `400` too.
- `backend/tests/worker_routes.rs:1371` sets `UPDATE settings SET
  max_task_seconds = 900`; make it `UPDATE jobs … WHERE id = $1` on its job.
- `scheduler.rs` tests that a claim's deadline comes from its job — two jobs
  with different limits, claimed in one run.
- Comments: `routes/worker.rs:627` names `/admin/settings` as the remedy for
  time-limit declines; it is the job's Manage page now.
  `contract-fixtures/README.md:48` calls the assignment's `max_task_seconds`
  "the server's setting (`/admin/settings`)"; it is the job's. The fixtures
  themselves do not change.
- `PLAN.md`: `:75-77` and `:3310` (site-wide, read at each claim from
  `settings`); the `settings.changed` audit row (:5586) becomes
  `job.time_limit_changed`; the two `/api/admin/settings` API rows
  (:5698-5699) go and the `PATCH` joins the job routes beside `:5675`; the
  `/admin/settings` page row (:6039) goes; the schema copy (:8201) follows
  `0001_initial.sql`.
- `README.md:1096` sends the reader to **Admin → Settings** for the time
  limit; point it at the job's Manage page.
- `JOURNEYS.md:951` (audit action list), `:958-972` and `:1171-1176` (the
  settings `curl`s become the job patch); `TESTING.md:3109`, `:3361-3365`,
  `:3746`, `:3790` likewise.

---

## Verification

Per phase, the lowest tier that proves it (TESTING.md, "Pick the lowest tier"):

| Phase | Tiers |
|---|---|
| 1 Access | 3 (`authz`, `public_api`), 5 (`e12`) |
| 2 Job page | 1F (`jobSettings`, `PlayerSettingsTable`), 2 (the active-contributor count), 5 (`e4`, `e10`, `e11`) |
| 3 Position pane | 5 (`e10`, `e12`, `e17`) + eyes at 1440px and 393px |
| 4 Ratings | 1F (`ratingPool`), 5 (`e7`) |
| 5 Contributions | 2 (the jobs counters), 3 (`public_api`, including the index-only plan test), 5 (`e1`, `e10`) |
| 6 Time limit | 2 (`scheduler`), 3 (`admin_api`, `authz`, `worker_routes`), 4 unchanged (assert it), 5 (`e11`) |

Phases 5 and 6 each reset the database on deploy; warn first, and if they ship
together, reset once. Before the phase 6 commit, grep `backend/`, `frontend/src/`,
`e2e/`, `contract-fixtures/` and the docs for `/admin/settings`, `FROM settings`,
`UPDATE settings` and `settings.changed`; none should be left.
