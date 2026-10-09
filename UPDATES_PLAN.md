# Updates plan — October 2026 batch

Source: `~/Dropbox/birdtest_todo.txt` (2026-10-09). Branch: `todo-batch-2026-10-09`.

The items are regrouped into phases so that each phase can ship on its own. Why the order changed:

- **The reset fix ships before the first schema change.** Phases 3 onward edit `0001_initial.sql`, and each such deploy needs a prod reset. That reset should already clear the S3 dumps.
- **Split deploys ship early.** Every later phase deploys at least once, so they all benefit.
- **The allocation-only model comes before round-robin creation.** A round robin creates many jobs at once, and they have to be created in the final "inactive = 0%" model.
- **The MAGPIE changes go in one pin bump.** Contribute output, IGP and the task time limit change MAGPIE and the claim contract together: one MAGPIE change, one pin, one fixture recapture.
- **The movegens audit comes after the MAGPIE bump**, so it checks the build that will actually run, including the IGP sim path.
- **Cross tables come after round robins.** A round robin is what fills a cross table, and its e2e test creates one.

Pre-launch rules still hold (see memory `nothing-in-prod`):
- Edit `0001` in place, with no compat shims.
- `MAGPIE_VERSION` / `MIN_MAGPIE_VERSION` stay at 0.1.1, but `MAGPIE_COMMIT` gets bumped.
- Warn before any deploy that resets the database.

Builds stay capped as in `keep-builds-light`. Verification follows `birdtest-local-verification`.

Decisions taken on 2026-10-09:
- Settings differences are shown **"differences first"**.
- The cross-table spread figure is the **standard error of the win %**.
- A reset also deletes the **S3 nightly dumps**. RDS point-in-time backups are left to expire.
- Glicko is **not** implemented. The write-up in Phase 7 explains why.
- Pool ratings move to a WESPA-comparable scale: 250 points per logit, as WESPA's k (7c, approved).
- The MAGPIE branch takes `main` by **merge, not rebase** (5a).

---

## Phase 1 — UI fixes (frontend only, no schema change)

**1a. Mobile header** (`frontend/src/routes/+layout.svelte:39-76`)
- Problem: the header takes three rows on a phone (brand / links / Sign in+Register).
- Fix: put the auth block on the brand row, right-aligned (`ml-auto`). The links keep their own row, which already fits on one line at 393px.
- Result: two rows, and every link stays visible, so `e10`'s in-viewport check keeps passing.

**1b. contribute.txt table on mobile** (`frontend/src/routes/+page.svelte:87-115`)
- Problem: the "What it is" column scrolls off the side.
- Fix below `sm`: render each setting as a stacked block — name and default on one line, the description underneath (a `<dl>`, or a `sm:hidden` card list beside a `hidden sm:table` table).
- Test: add `expectTableFits` (or a no-overflow check on the list) for the home page to `e2e/tests/e10-phone-width.spec.ts`.

**1c. Saved positions on mobile** (`lib/components/SavedPositions.svelte`, `PositionPane.svelte`, `Board.svelte`)
- Card padding `p-3 sm:p-5`, so the board gets the width.
- The rack input becomes `flex-1 min-w-0` so Random/rack/Search fit on one or two tidy lines.
- The "Showing:" toggle becomes a two-segment control:
  - full width, labelled by the player name with its colour dot
  - "'s move" dropped on phones
  - `data-testid="pair-toggle"` kept for `e17`
- Check the move list and the inference table at 393px.

**1d. Misaligned racks on desktop** (`Board.svelte:181-209`)
- Cause: the "to move" badge wraps under a long name, which pushes that player's tiles down.
- Fix: make the racks grid two rows (`grid-rows-[auto_auto]`), with each player's block a `row-span-2 grid-rows-subgrid`. Both name lines then share a height and the tiles line up.
- Also: let the name truncate (`min-w-0 truncate`, full name in `title`), so the badge doesn't wrap.
- `e12`'s rack/badge assertions are unaffected.

**1e. Contribute steps order** (`+page.svelte:65-143`)
- New order: Install → **(Optional) Create an account** → contribute.txt → Run.
- Rewrite the account step: "copy your API key; you'll put it in contribute.txt in the next step".
- Make the apikey row say "(step 2)".
- Remove the "create the file if you don't have one" aside from the account step.
- Update `JOURNEYS.md` V-2.

**1f. Remove "Raw results: …"** (`routes/jobs/[id]/+page.svelte:383-387`)
- Remove the paragraph only. The endpoint stays, because rack lookup uses it.

**1g. Player settings: differences first** (`lib/components/PlayerSettingsTable.svelte`, `lib/jobSettings.ts`)
- At the top: a **Differences (N)** block with an amber tint and left accent bar, holding the differing rows. Each player's column header carries its colour dot.
- Below it: the shared settings, collapsed behind "Show N shared settings". They are expanded when there is only one player (the player-config page) or no differences.
- Replace the "Settings the players differ in are in bold" note.
- Tests: update `jobSettings.test.ts`, and add a component-level test that differing rows render in the differences block. Update `JOURNEYS.md:165`.

**1h. /workers page** (`routes/workers/+page.svelte`, `lib/format.ts`)
- **Auto-refresh:** use `createPoller` every 30 s. It pauses while the tab is hidden. Keep the current page and sort, and keep the stale-answer `generation` guard.
- **Exact movegens:** a new `exactCount` gives grouped digits (`1,234,567,890`) and replaces `bigCount` here.
- **Compute time down to seconds:**
  - A new formatter gives `2m 13s`, `5h 20m 13s`, `3d 4h 5m 6s`, dropping zero units.
  - Use it on `/workers`, `WorkerTable` and `/admin/workers`.
  - The ETA and backup-age `duration` stay as they are.
- Tests: F-FMT-16 and F-FMT-19 get siblings; check `e10` column widths at phone size; update `JOURNEYS.md:392-394`.

**1i. Opening-rack "Analysed racks to try" are random**
- Today the page takes the first 10 distinct racks of the newest 50 records (`routes/jobs/[id]/+page.svelte:38-49`). Those are the tail of one batch, which `scatter` makes a fixed stride apart in enumeration order — hence alphabetically close.
- New endpoint: `GET /api/jobs/:id/rack-samples?n=10`.
  - It makes 10 random index probes: `WHERE job_id=$1 AND id >= $r ORDER BY id LIMIT 1`, with `$r` uniform in the job's `[min(id), max(id)]`.
  - It de-duplicates the racks. The cost is a handful of index lookups, whatever the job's size.
  - Confirm `position_analysis_records` has an index usable for `(job_id, id)` and add one if not.
- Optional "Shuffle" button.
- Update `e13` (it waits on `per_page=50`).

**1j. Long JSON key names in downloads (answer, plus one small change)**
- **At rest:** Postgres stores results in typed columns. Keys like `blended_utility` are only produced when serialising (`exports.rs:79-155`), so they cost nothing in storage.
- **Exports:** gzip NDJSON. Measured on the contract fixtures:

  | Fixture | Raw | Gzipped | Keys shortened, raw | Keys shortened, gzipped |
  |---|---|---|---|---|
  | `result-games.json` | 30.1 KB | 5.19 KB | 22.5 KB | 4.87 KB |

  The long names cost about 34% uncompressed but only about 6% compressed, so gzip takes care of it. Renaming isn't worth the churn.
- **Where they do cost:** the uncompressed paths — the paginated `/results` JSON and the live NDJSON stream. Neither the backend nor nginx compresses today.
- **Change:** turn on nginx `gzip` for `application/json` and `application/x-ndjson`. Not `text/event-stream`, because SSE must stay unbuffered. Check `nginxConfig.test.ts`.

## Phase 2 — Ops: resets clear S3 dumps; split deploys

**2a. A reset also deletes the S3 nightly dumps** (`scripts/lib/ops.sh` `ops_reset_database`, `reset-prod-db.sh`, `deploy.sh --reset-db`, `rollback.sh --reset-db`)
- **Already cleared:** `audit_log` and the `backups` table are dropped with the schema.
- **New step:** delete every object version under `pg/` in the backups bucket **and** the DR bucket. Version-specific deletes don't replicate, so each bucket is done explicitly. Each delete carries `x-amz-bypass-governance-retention` (both buckets use a GOVERNANCE Object Lock).
- **IAM:** the operator needs `s3:ListBucketVersions`, `s3:DeleteObjectVersion` and `s3:BypassGovernanceRetention` on both buckets, plus `kms:Decrypt` if the listing needs it. Add these in `infra/` (`onboard-deployer.sh` / deployer policy). The backup task's role stays write-only.
- **Prompts:** the typed-hostname confirmation now names both buckets and the dump count. Remove "The nightly dumps … are kept".
- **Not touched:** RDS PITR. It expires in 30 days; say so in the banner and in RUNBOOK.
- **Tests:** add `aws s3api list-object-versions` / `delete-objects` stand-ins to `ops-scripts-check.sh`, and run `runbook-check.sh`.

**2b. Split deploys: the frontend deploys without downtime**
- **Why a frontend change goes down today:**
  - One ECS service runs both containers with min-healthy 0%, so the old task stops before the new one starts.
  - `deploy.sh` rebuilds and retags all three images every time.
- **Infra:**
  - Move the nginx/frontend container to its **own ECS service and task definition**. It is stateless, so it can roll normally: `desired_count = 2`, min-healthy 100% and max 200% (or 1 task with 100/200).
  - The ALB already routes `/api/*` and `/health` to the backend target group. Check the listener rule priorities, and make the frontend's `BACKEND_UPSTREAM` irrelevant in prod (or point it at the ALB) so nginx no longer needs `127.0.0.1:8080`.
  - The backend stays single-instance, for the in-process state documented in `ecs.tf:454-466`.
- **`deploy.sh`:**
  - Read the deployed commit (from the current task definitions' image tags) and `git diff --name-only` it against HEAD.
  - Rebuild and retag only what changed:
    - `frontend/**` → frontend
    - `backend/**`, `docker/**` (MAGPIE pin), `Cargo.*` → backend (+ derived-builder when the MAGPIE pin or `derived.rs` changed)
    - `infra/**` → terraform plan/apply
  - Each service keeps its own image variable, so a frontend deploy leaves the backend task untouched.
  - Keep `--all` to force everything.
  - A diff that touches `backend/migrations/0001_initial.sql` must refuse without `--reset-db`.
- **Optional:** shorten the backend's own gap with `deregistration_delay` 30 → 10 s. The backend gap itself stays, by design.
- **Docs and tests:** update RUNBOOK, README and the `ops-scripts-check.sh` stand-ins, and add a test for the path→service mapping.

## Phase 3 — Job control: allocation is the only switch; round-robin creation

**3a. Activation through Allocation only** (schema change → reset)
- **Invariant:** for a job that isn't completed, `status = 'active'` ⇔ `allocation > 0`.
  - `jobs.allocation` becomes `NOT NULL DEFAULT 0`.
  - Add a `CHECK ((status = 'active') = (allocation > 0))` for non-completed jobs.
  - Completed jobs are held at 0.
- **Backend** (`routes/admin.rs`):
  - Delete `activate_job` and `deactivate_job` and their routes (no compat shims).
  - `set_allocations` becomes the only path. 0% sets `inactive` **and** allocation 0 (today it keeps the old number). Above 0% activates.
  - Force-complete and purge set allocation 0.
  - Move the leave-artifact and derived-data pre-work that `activate_job` duplicated so only `set_allocations` does it (it already does).
  - Update the `job.activated` / `job.deactivated` audit entries.
- **Frontend:**
  - Remove the allocation input and the Activate/Deactivate buttons from `routes/admin/jobs/[id]/+page.svelte:419-463`.
  - Show the current allocation read-only, with a link to `/admin/allocation`.
  - Force complete, Purge and Delete stay.
  - Remove `activateJob` / `deactivateJob` from `api.ts`.
- **Also update:**
  - `scripts/seed.py`, `e2e/run.sh`, `scripts/e2e_magpie*.py`: anything that activates through the old endpoint.
  - The `e4` and `e11` specs.
  - `backend/tests/admin_api.rs` / `admin_routes.rs`.
  - PLAN.md "Job Lifecycle Controls": drop the "active at 0%" case; deactivated just means 0%.
  - JOURNEYS.

**3b. Round-robin games / game-pairs jobs**
- **Form** (`routes/admin/jobs/new/+page.svelte:412-426`): replace the Player 1/Player 2 selects with a checklist of player configs, and preview the matchups ("4 configs → 6 jobs").
  - Ticking one config creates one self-play job, which is what p1 = p2 allows today.
  - Ticking n ≥ 2 creates C(n,2) jobs.
- **API:** `POST /api/admin/jobs` takes `player_config_ids: [..]` for games/game_pairs, replacing `player1/2_config_id`. It returns the created job ids.
  - All jobs are inserted in one transaction, all inactive at 0%.
  - Names: "{name}: {A} vs {B}".
  - Seat order follows the order the configs are listed. Game pairs swap seats anyway; plain games batches are even, so starts alternate.
  - `validate_shared_player_options` runs for every pair before anything is inserted. A failure names the clashing pair.
  - Cap n (e.g. 12 → 66 jobs).
- **After creating:** redirect to `/admin/allocation`, showing the new jobs, rather than to one job page.
- **Tests:** a backend test for 3 → 3 and 4 → 6 jobs, all or none on validation failure; e2e create-flow update. Update `seed.py` `create_dev_jobs`.

## Phase 4 — Admin settings table and task time limit (backend half of the contract change)

**4a. Admin-editable settings**
- Nothing exists today; `set-setting.sh` edits Terraform variables.
- Add a single-row `settings` table in `0001`: `max_task_seconds INT NOT NULL DEFAULT 3600 CHECK (60..86400)`, `updated_by`, `updated_at`.
- `GET/PUT /api/admin/settings`, with an audit entry `settings.changed`.
- `/admin/settings` page (new admin tab).

**4b. Enforce the limit**
- The task assignment gains `max_task_seconds` (from the settings row at claim time) and `job_name` (for Phase 5's output).
- Each claim stores its deadline: `claimed_at + max_task_seconds`.
- The reclaim sweep (`scheduler.rs:459-499`) lapses claims past their deadline + 60 s grace, **even if they are still heartbeating**.
- A submission after deadline + grace is refused like a lapsed claim.
- A new decline reason, `time_limit`, is counted per job.
- **Job page:** show "N tasks hit the 1-hour limit — lower the batch size". A job whose single unit always exceeds the limit (e.g. a deep-sim game pair) would otherwise retry forever. After K consecutive time-limit declines with no completion, set the job aside (0%) and say why.
- **Contract:** update fixtures and README in `contract-fixtures/`.
- **Tests:** a reclaim test past the deadline; a claim test that the assignment carries the current setting.

## Phase 5 — MAGPIE: IGP default, quiet contribute output, deadline (one pin bump)

**Work in `~/MAGPIE` on `birdtest-contribute`.**

**5a. Bring in PR 756**
- **Merge** `origin/main` into `birdtest-contribute` — a merge commit, **not a rebase**. That keeps the branch's history and the pinned commits such as 856a6062 reachable. `origin/main` now includes PR 756, the IGP sim-determinism fix merged 2026-10-09; `birdtest-contribute` is 18+ commits behind.
- Fetch first: the local `origin/main` is stale (5b9d3be4 vs remote ee9e8877).
- After merging, resolve any conflicts in `config.c` (the contribute executors and `config_contribute_reset_shared_settings`). Then run MAGPIE's own tests, including the new `simdetigp` shard, before the 5b–5d changes go on top.

**5b. IGP by default**
- `config_contribute_reset_shared_settings` forces `pgp` today (`config.c:8944`). Make it take the mode from the task request instead.
- birdtest adds a job-level `threading_mode` (`igp` | `pgp`, default **igp**) to the job form and task request. It only has an effect when a player sims; static-only jobs behave identically.
- Update `SETTINGS_COMPARISON.md:240` (which also cites the wrong line).

**5c. Quiet contribute output**
- Print only:
  - derived-data creation at startup (the existing "building …" lines, plus matching "built …" lines for the wordmap and word info table)
  - `[time] started task #<n>: <job name>`
  - `[time] finished task #<n>: <job name> (<started> started, <completed> completed)`
- Remove the settings dump (`contribute.c:760-803`), the "completed N task(s)" line, the idle/retry chatter, and the "nothing to do" line.
- Errors that end the run or need the user to act still print to stderr. These are the shutdown/"update MAGPIE"/"run download_data.sh" messages, task failures and give-up. Assumption: "only print" doesn't mean hiding failures.
- `job_name` comes from the assignment (Phase 4).

**5d. Deadline**
- Stop a task at `max_task_seconds` using the existing stop path ("handing the task back unfinished") and decline it with reason `time_limit`.

**5e. Pin, fixtures, verification**
- Bump `MAGPIE_COMMIT` in `docker/Dockerfile`, recapture the contract fixtures (`scripts/capture_contract.py`), and run tier 5/6 e2e.
- A builder without the new fields fails at result time with a "rebuild MAGPIE" message, as before.

## Phase 6 — Movegens: audit and breakdown

**6a. Audit**
- Already confirmed: every job type submits `movegens` (`worker.rs:694-708`), and a test checks each fixture reports more than 0.
- Still to check in MAGPIE that the counter covers every generator used inside a task:
  - sim threads under both `igp` and `pgp`
  - inference
  - endgame/PEG small-move generation (`generate_small_moves_in_lanes` is counted)
  - leave-gen forced-rack games
  - opening-rack sims
  - generators created per thread and destroyed before the before/after diff (`config.c:9946/9962`) — make sure their counts aren't lost
- Write a MAGPIE test per job type comparing the reported count with an independent count (e.g. turns × sims).
- Check that the game-pairs fixture's 96 for 4 static games is right.

**6b. Breakdown**
- Add a running `jobs.movegens` counter (credited with the claim's movegens; subtracted on purge/delete like `Earned`).
- Expose it on the job page stats row.
- On `/workers`: totals by job type (opening rack / games / game pairs / leave gen), plus a per-contributor breakdown by type in an expandable row. That comes from `task_claims ⋈ tasks ⋈ jobs`, grouped by type.
- Exact numbers, as in 1h.

## Phase 7 — Rating pools: cross table; Glicko write-up; WESPA-comparable scale

**7a. Cross table** (`routes/ratings/[id]/+page.svelte`, `backend/src/ratings.rs`, `routes/ratings.rs`)
- **Layout:** an n×n matrix of the pool's members, ordered by rating, with the rating in the rightmost column.
- **Each cell holds:**
  - **Win %:** (W + ½D) / games from the row config's side.
  - **± standard error of that win %:** pools use game pairs, so it comes from the per-pair score variance in the summed pentanomial, shown in per-game % terms. For plain-games data it would come from W/L/D.
  - **Average spread difference:** Σ games·(p1_mean − p2_mean) / Σ games, from `game_results`.
- **Data source:** sum per (config A, config B) across the pool's game_pairs jobs, from the same rows `build_matrix` reads (`ratings.rs:79-170`). Cells mirror across the diagonal (100 − %, −spread).
- **Cost:** computed at fit time and stored per run, or computed on request; it is one grouped query either way.
- **Mobile:** sticky first column, horizontal scroll inside the card.
- **What happens to `ResidualMatrix`:** keep it below as "Model fit", or fold its residual into the cell's hover. Recommend hover.

**7b. Why not WESPA Glicko** (to go into PLAN.md's ratings section and the `bradley_terry.rs` module doc)
- **Strength doesn't change.** Glicko models a person whose strength drifts, with RD growing over idle weeks. A player config is a frozen set of flags with a fixed strength, so the time machinery has nothing to track.
- **Order matters.** Glicko updates one period at a time, so the same results in a different order give different ratings. Results arrive in whatever order volunteers finish them, and purges and reruns would reshuffle ratings. Bradley–Terry uses all the evidence at once and doesn't care about order.
- **The uncertainty never shrinks.** WESPA's RD floor (50–75 by band) is meant to keep human ratings responsive. For a bot with 100k games it means the rating keeps moving as if it were uncertain by ±50, whereas Bradley–Terry's standard error shrinks with the evidence.
- **Periods would be arbitrary.** There are no tournaments. Any choice of period (day, batch, job) is arbitrary and changes the numbers.
- **The calibration is for humans.** Newcomer seeding (5 virtual games at 1500), RD 300 and the RD bands are calibrated to the WESPA human population. Bot ratings would land in bands that depend on where the anchor sits.
- **Game pairs lose their advantage.** Glicko's updates are per game; feeding it pairs throws away the variance reduction game pairs exist for.

**7c. A WESPA-comparable rating scale (approved 2026-10-09)**
- **What it matches:** the rating *gap* ↔ win-expectation curve.
  - WESPA Glicko predicts E = 1 / (1 + e^(−gΔ/250)), with g ≈ 0.95–0.99 between established players.
  - Today's Bradley–Terry ratings use the Elo scale, 400/ln 10 ≈ 173.7 points per logit (`ELO_PER_LN`, `bradley_terry.rs:27`).
  - At **250 points per logit**, a gap of X in birdtest predicts the same win % as a gap of X between two established WESPA players (within ~1–5% from g). Example: 100 points ≈ 59.9% instead of 64.0%.
- **What it can't match:** the absolute level. Bot-vs-bot results say nothing about strength against humans, so the absolute number is only as meaningful as the anchor rating. The page says so in one line under the table.
- **Backend** (`backend/src/stats/bradley_terry.rs`):
  - Replace `ELO_PER_LN` with `POINTS_PER_LOGIT = 250.0` (WESPA's k).
  - Every rating, standard error and convergence check goes through it (lines 432, 456, 571).
  - The residuals' predicted score (line 243) hardcodes `10^(−Δ/400)`. It becomes `1 / (1 + e^(−Δ/250))`, using the same constant.
  - The priors are in logit units (`PRIOR_SCALE`, line 69), so they stay as they are.
  - Update the module doc (lines 1–24), which calls the scale Elo.
- **Default anchor:** keep 2000. On the WESPA scale it reads as a strong-club level, and the admin can still change it per pool.
- **Tests:**
  - Rewrite the scale-dependent ones in `bradley_terry.rs`: lines 680, 838, 1031 and 1114, which assume `400·log10`.
  - Add a test that a 0.75 score rate fits to a gap of 250·ln 3 ≈ 274.7.
  - Update any e2e or `ratings` test that asserts concrete rating numbers.
- **The per-job match test** (`stats/match_test.rs`) is a confidence interval on player 1's per-game score, not an SPRT; the SPRT was removed. It doesn't use a rating scale.
  - Its result still carries leftover `elo` / `elo_lower` / `elo_upper` fields (`match_test.rs:106-127`, serialized into job stats). No page reads them, and `matchTest.test.ts` asserts the sentence says "nothing in Elo".
  - **Remove** those fields, the `elo()` helper and `MAX_ELO`, along with their tests (`match_test.rs:297, 316, 321-326`). That leaves no Elo-scale number anywhere a WESPA-scale rating could be confused with.
  - `outcomes.rs` mentions Elo only in comments and one test's illustration (lines 25, 95, 170). Reword those in score terms, or leave them.
- **Fix the stale SPRT comment** at `routes/admin.rs:1703` while there: it describes "the SPRT then in use".
- **Frontend:**
  - Label the pool rating "rating (WESPA scale)".
  - `RatingDotPlot.svelte` and the ratings pages mention Elo; replace that wording.
- **Docs:** PLAN.md ratings section and JOURNEYS, wherever pool ratings are called Elo.
- **Deploy:** stored `rating_runs` are on the old scale. A refit of every pool after deploy (`mark_every_pool_for_refit`) is enough; prod is reset anyway.

## Docs to update as phases land

- **PLAN.md:** job lifecycle (3a), round robins (3b), settings and time limit (4), ratings and cross tables (7).
- **JOURNEYS.md:** V-2 (1e), :165 (1g), :392 (1h), :254 (1i).
- **RUNBOOK / README:** resets and deploys (2).
- **SETTINGS_COMPARISON.md:** mtmode (5b).
- **contract-fixtures/README.md** (4, 5).
