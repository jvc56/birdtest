# Opening racks: narrowed reanalysis plan

**Goal:** after an opening-rack job's first pass, reanalyse each rack over only the top X moves its first analysis found, not the player's full candidate set (`num_plays`, N). Keep reanalysing until one move leads every other by 2 first-place finishes, or until the rack reaches the job's maximum number of passes.
**Status:** plan only. Nothing is implemented yet.
**Written:** 2026-11-11, against `main` at 9395521 and MAGPIE `main` (latest release `v0.1.4`).

**Decided in review:**
- The first pass's result counts toward the 2-vote lead.
- X defaults to 10, is set per job, and can be changed while the job runs.
- The new rule applies to existing jobs, including one that is still on its first pass.
- The old percentage consensus rule is removed: from the code, the API, the UI and, in a follow-up migration, the schema.
- An admin can change any job's minimum MAGPIE version, whatever its type or status. This is a general edit, not something specific to opening-rack jobs.

## How it works today

- **First pass.** `next_request` (`backend/src/jobs/opening_rack.rs`) splits the rack space into slices of `racks_per_batch`. Each task sends its racks to MAGPIE, which generates every move for the rack (`config_contribute_generate_for_rack`) and, for a simming player, simulates the top `num_plays` of them. The ranked moves come back, capped at `num_plays_recorded`, and are stored in `position_analysis_records` and `position_analysis_moves`.
- **Reissues.** If `max_results_per_rack > 1`, `next_reissue` hands out unsettled racks once the space is covered, fewest analyses first, as a task that lists its racks in `opening_rack_requests.racks`. **Every reissue analyses the rack from scratch with all N candidates.**
- **Settling.** `record_consensus` counts each move's rank-1 finishes across a rack's analyses and calls `standing`. A rack is settled once it has at least `min_results_per_rack` analyses and the top move holds at least `consensus_pct`% of them, or once it reaches `max_results_per_rack`. `restate_racks` applies the same rule in SQL when an admin edits the settings, and an edit that unsettles racks of a completed job reopens it.
- **Version floor.** A job's minimum MAGPIE version (`jobs.min_magpie_major/minor/patch`) can only be set at creation. The scheduler's `candidate_jobs` reads it on every claim, so it takes effect as soon as it is written.

The cost is in the reissues. A rack's first pass already ranks its N candidates, and almost all of them are clearly worse than the leaders. Simulating them again on every pass spends most of the iteration budget on moves that cannot win.

## The new behaviour

1. **Pass 1 is unchanged.** Every rack is analysed once with the full N candidates.
2. **Passes 2 and later.** A reissue sends each rack with its **candidates**: the top X moves, by rank, of that rack's first-pass analysis, which is already stored. MAGPIE simulates only those moves, and its rank-1 move is that pass's vote.
3. **Settling.** After each pass, the rack is settled if its leading move has at least `win_margin` (2) more rank-1 finishes than the runner-up, or if it has had `max_passes` analyses. The second case is "settled without consensus", as it is today, and records the leading move (the alphabetically first one if tied).
4. **The first pass votes.** Its rank-1 move is always one of the candidates. A rack therefore settles after two passes if the first reissue agrees with the first pass.

Example with X = 10, margin 2 and max 7 passes. Pass 1 ranks 8H AEINRST first. Pass 2 also ranks it first (2–0), so the rack settles after two analyses. On a split rack the votes might go A, B, A, B, A, A (4–2), and it settles after six. A rack still within 1 after seven passes settles without consensus.

**Candidates are derived when a reissue is dispatched, not stored on the rack.** Because they come from the rack's stored first-pass analysis, racks analysed before this change get candidates just as new ones do, with no backfill. Each reissue task records the candidates it sent, so a reclaim gets the same list and the submission is checked against it.

## Settings

| Setting | Where | Rule | Editable while running? |
|---|---|---|---|
| `reissue_top_moves` (X) | `job_opening_rack_config`, new, default 10 | 2 ≤ X ≤ 100. A rack's candidates are its first `min(X, stored moves)` moves, so the effective X is capped at the player's `num_plays_recorded`. The admin form warns when X is larger than that. | Yes. The new X applies to reissues dispatched after the edit. Votes already cast stay. A move that falls outside a smaller X keeps its votes but cannot gain more. |
| `win_margin` | `job_opening_rack_config`, new, default 2 | 1 ≤ margin ≤ 100 | Yes, and the edit restates the job's racks. |
| `max_passes` | the existing `max_results_per_rack`, relabelled in the UI | Unchanged: 1 to 100. 1 means one analysis per rack and no reissues. | Yes, as today. |
| `min_results_per_rack`, `consensus_pct` | existing | **Removed.** No code reads them after migration 0003. Migration 0004, in the next deploy, drops the columns. | n/a |
| Minimum MAGPIE version | `jobs`, existing | Any strict version. A value below the fleet floor (0.1.2) is allowed but has no effect, because the fleet floor applies first. | **Yes, new:** any job type, any status. |

The existing simmer requirement for `max > 1` (`validate_opening_rack_player`) still applies. A static player would return the same order on every pass.

**Reissue guard.** An opening-rack job with `max_passes > 1` must have a minimum version of at least 0.1.5. Creating such a job, or making a consensus edit that leaves one with `max_passes > 1`, is refused with a `400` naming `min_magpie_version` if the floor is lower. The floor is never raised automatically: the admin raises it, either in the creation form (which pre-fills 0.1.5 when max passes is above 1) or with the new floor edit. Without this guard, a worker below 0.1.5 would claim reissues it cannot run, and every one would come back as a failed task.

## Editing a job's minimum MAGPIE version

`PATCH /api/admin/jobs/:id/min-magpie-version` with `{ "min_magpie_version": "0.1.5" }`, modelled on `PATCH /jobs/:id/time-limit`:

- **Scope.** Any job type (opening rack, games, game pairs, leave generation) and any status (active, inactive, completed). It never changes the job's status: setting the floor on a completed job does not reopen it.
- **Validation.** The value must pass `Version::parse_strict`, as at creation. Lowering is allowed, but not when it would leave an opening-rack job with `max_passes > 1` below 0.1.5 (the reissue guard).
- **Effect.** The floor applies to claims from then on. Open claims keep their tasks and their submissions are accepted, because the worker was eligible when it claimed. A returned or reclaimed task goes through the claim filter again, like any other.
- **Locking.** It runs under the job's row lock, which the claim path's `UPDATE jobs` also takes, so a claim reads the old floor or the new one, never a mix. A request that changes nothing gets a `200` and writes nothing.
- **Audit.** One `job.min_magpie_version_changed` row, recording the old and new versions.
- **UI.** An editor on the admin job page (`admin/jobs/[id]`), next to the time limit. The public job page already shows "requires MAGPIE ≥ …".

This edit doesn't depend on MAGPIE 0.1.5 or on anything else in this plan, so it can ship first, as its own PR.

## Removing the old consensus rule

The percentage rule is deleted rather than kept for older jobs. Every reference goes:

- **Backend.**
  - In `models/job.rs`, the fields of `OpeningRackConfig`/`ConsensusSettings` and the `load` query.
  - The percentage branch of `standing` and `restate_racks`, and their tests.
  - The checks in `consensus_problems`.
  - The fields in the create and `PATCH …/consensus` request bodies, both in `routes/admin.rs`. Requests that still send them are refused as unknown fields (`400`), not ignored, so a stale script fails loudly.
  - The `job.consensus_changed` audit text, which becomes "max 5 -> 7, margin 2 -> 3, top 10 -> 12; N racks unsettled".
  - `jobstats.rs` and `routes/public.rs`.
- **Frontend.** `lib/consensus.ts`, `lib/jobSettings.ts`, `lib/api.ts`, `ConsensusEditor.svelte`, `admin/jobs/new`, `jobs/[id]`, and the tests `consensus.test.ts`, `jobSettings.test.ts` and `jobProgress.test.ts`.
- **Tests and scripts.** `backend/tests/` (`jobs.rs`, `authz.rs`, `public_api.rs`, `audit.rs`, `worker_api.rs`), `scripts/seed.py` and `scripts/e2e_magpie.py`.
- **Docs.** The consensus journeys in `JOURNEYS.md` and the rows in `SETTINGS_COMPARISON.md`. Older plan files (`PLAN.md`, `UPDATES_PLAN_*.md`) are historical records and stay as written.
- **Schema, in two steps.** Migrations are additive while the previous image is still running.
  - **0003, this deploy:** leaves both columns in place. They are `NOT NULL` with defaults, so the new image's inserts, which don't name them, still succeed.
  - **0004, the next deploy:** once no image reads them, drop `consensus_pct` and `min_results_per_rack`. Dropping `min_results_per_rack` also drops the `max_results_per_rack >= min_results_per_rack AND max_results_per_rack <= 100` check, so re-add `CHECK (max_results_per_rack BETWEEN 1 AND 100)`. Update the `job_opening_rack_config` comment in the same migration.

## Applying it to existing jobs, including the running one

- **Migration.** Migration 0003 adds `reissue_top_moves` and `win_margin` as `NOT NULL` with defaults 10 and 2, so every job, old and new, is on the new rule from the moment it deploys.
- **Restating.** Racks keep the settled flags they have until something restates them. A submission recomputes the standing of the racks it touches, and a consensus edit restates the whole job under its existing locks and hold. The migration does not restate in place: restating a full English job rewrites millions of rows, and the edit path already does that safely.
- **The running job (first pass).**
  1. Every rack it has analysed has one result, so `runner_up_count` 0 is already correct and nothing needs backfilling.
  2. After deploy, once its first pass is covered (so older workers aren't cut off from the last racks), raise its minimum version to 0.1.5 with the new floor edit.
  3. Then set its max passes, for example 7, in the consensus editor. That edit restates the racks: every rack with one vote becomes unsettled, because 1–0 is not a lead of 2, and reissues begin.
  4. If the first pass finishes and the job completes before you do this, the consensus edit reopens it, as it does today. Either way, no analysis is lost or repeated.
- **Effective X on that job.** Candidates can only come from moves that were stored. Check its player's `num_plays_recorded` before deploying. If it is below 10, its reissues use that many candidates, and there is no way to recover moves that were never stored.
- **Other unfinished jobs.** Any opening-rack job already in its reissue phase needs its floor raised to 0.1.5 and a consensus edit to restate it under the new rule. Before deploying, list the opening-rack jobs that are not completed. Completed jobs are left as they are unless an edit reopens one.
- **During the deploy.** Pause opening-rack jobs (set them inactive) for the rollout. The old image doesn't write `runner_up_count`, so a reissue it accepted during the rollout window could leave that column stale on a rack with more than one result.

## Changes

### MAGPIE (first, as its own PR and release)

- **Request.** Each `opening_rack` task request may carry an optional `candidates` field: an array parallel to `racks`, where element *i* is the list of move strings for rack *i*. It is present only on reissues.
- **Analysis.** In `config_contribute_generate_for_rack`, when a rack has candidates, skip move generation. Build the move list from the strings with the same path `addmoves` uses (`validated_moves_create` → `validated_moves_add_to_sorted_move_list`), then simulate as normal. `num_moves` reports how many candidates were simulated.
- **Refusals.** A request with candidates for a static player, an array whose length doesn't match `racks`, an empty candidate list, or a move that doesn't validate on the empty board with that rack is an `ERROR_STATUS_CONTRIBUTE_SERVER_ERROR`. That is the same class as an unusable rack.
- **Round trip.** The move strings birdtest stores are the ones MAGPIE wrote (`autoplay_results_write_ranked_plays_json`). Add a test that every kind it writes, including placements, exchanges and blank designations, parses back through `validated_moves_create` and produces the same move. **This is the main technical risk.** If any format fails to round-trip, fix it in MAGPIE before anything else.
- **Contract fixture.** Add `test/birdtest_contract/assignment-opening-rack-candidates.json`.
- **Version.** Ship as a feature release, 0.1.5. The current release, 0.1.4, has no candidates support. The fleet-wide floor `DEFAULT_MIN_MAGPIE_VERSION` stays at 0.1.2, under the floor policy. Each job that reissues gets 0.1.5 through its own floor, which the admin sets. The reissue guard's version is a backend constant, `CANDIDATES_MIN_MAGPIE_VERSION`, not a setting.

### Birdtest backend

- **Migration `0003_opening_rack_narrowing.sql`** (additive):
  - `job_opening_rack_config`: add `reissue_top_moves INT NOT NULL DEFAULT 10 CHECK (reissue_top_moves BETWEEN 2 AND 100)` and `win_margin INT NOT NULL DEFAULT 2 CHECK (win_margin BETWEEN 1 AND 100)`.
  - `opening_rack_progress`: add `runner_up_count INT NOT NULL DEFAULT 0`. Backfill it only for rows with `results > 1`, from their rank-1 tallies. These are bounded by the reissues already made, and are zero for the running job.
  - `opening_rack_requests`: add `candidates JSONB NULL`, an array of string arrays parallel to `racks`, set only on reissue tasks.
- **Migration `0004_drop_consensus_pct.sql`** (next deploy): see "Removing the old consensus rule".
- **`models/job.rs`.** `OpeningRackConfig` and `ConsensusSettings` replace `consensus_pct` and `min_results_per_rack` with `reissue_top_moves` and `win_margin`. Update `ConsensusSettings::load` to match.
- **`standing`.** Compute `runner_up_count` (the second-highest rank-1 count, or 0). `agreed = top_count - runner_up_count >= win_margin`, and `settled = agreed || results >= max_passes`. A rack whose candidate list has only one move also counts as agreed, because no other move can win it. `restate_racks` implements the same rule in SQL.
- **`record_consensus`.** Also write `runner_up_count`. Otherwise unchanged.
- **Dispatch.**
  - `next_reissue` still chooses the racks, unchanged.
  - `next_request` then derives their candidates in one query, bounded by the batch: each rack's record whose task has `seed < total_racks` (the first-pass task), with its moves where `rank <= X`, in rank order. It puts them in `OpeningRackRequest.candidates: Option<Vec<Vec<String>>>`, marked `skip_serializing_if = "Option::is_none"` so first-pass tasks send exactly what they send today.
  - `insert_request` writes them to `opening_rack_requests.candidates`, and `load_request` reads them back from there.
- **Submissions.** In `process_response` and `check_batch_against_task`, a reissue's analysis must return only that rack's candidates, and `num_moves` must not exceed the number of candidates. `check_batch_against_task` already reads the request row inside the task's row lock, so this adds one column to that read, not another query.
- **Job creation and edits (`routes/admin.rs`).**
  - Creation accepts `reissue_top_moves` (default 10) and `win_margin` (default 2), and applies the reissue guard.
  - `PATCH …/consensus` edits all three settings: `reissue_top_moves`, `win_margin` and `max_results_per_rack`. Changing X alone writes the setting without a restate, because it doesn't change any rack's standing. Changing the margin or max passes restates the job, and can reopen a completed one, as today. It also applies the reissue guard.
  - New: `PATCH …/min-magpie-version` (see "Editing a job's minimum MAGPIE version").
  - `consensus_problems` checks the new ranges.
- **Read paths.** `jobstats.rs` (the job settings shown), `exports.rs` (the `consensus` object gains `runner_up_count`), and `routes/public.rs` where they expose consensus settings.

### Frontend

- **`lib/consensus.ts`.** Rewrite `consensusFields`, `consensusProblem` and `analysesPerRack` for the new rule, for example "up to 7 passes, first to lead by 2, reissues over the top 10".
- **`admin/jobs/new`.** Replace the minimum and share boxes with "Reanalyse top X moves" (default 10) and "Win by" (default 2). Relabel "most analyses" as "Max passes". Warn when X is larger than the player's `num_plays_recorded`. Pre-fill the minimum MAGPIE version with 0.1.5 when max passes is above 1.
- **`ConsensusEditor.svelte`.** Edit X, the margin and max passes. Note beside X that a change applies to reissues from then on. If the job's floor is below 0.1.5, point the admin to the floor editor before they save max passes above 1.
- **`admin/jobs/[id]`.** A minimum-MAGPIE-version editor next to the time-limit editor, for every job.
- **`jobs/[id]`.** On a rack sample, show the current lead, for example "A 3 – B 1". On a reissued analysis, also show the candidates it ran over.

### Worker stub, fixtures, tests, docs

- **`worker/fake_worker.py`.** When `candidates` is present, rank only those moves, with a deterministic rank-1 that can be forced to split, so the e2e tests can exercise both a margin settle and a max-passes settle.
- **Contract fixtures.** Add `contract-fixtures/` request and response pairs for a reissue with candidates. The existing first-pass fixtures don't change.
- **Unit tests** (`opening_rack.rs`):
  - Settling by margin (2–0, 3–1, 4–2), and no settle at 1–0 or 2–1.
  - A tie at max settles without consensus.
  - A single candidate settles.
  - `restate_racks` agrees with `standing`.
- **Integration tests** (`backend/tests/scheduler.rs`, `worker_api.rs`, `jobs.rs`, `audit.rs`):
  - **Dispatch:**
    - A first-pass task sends no candidates.
    - A reissue sends exactly the first-pass analysis's top X, capped at the stored moves.
    - A reclaim gets the same candidates, even after an edit to X.
    - A job created before the migration gets candidates for racks analysed before it.
  - **Submissions:** one containing a move outside the candidates is refused with a 400.
  - **Consensus edits:**
    - An edit on a first-pass job unsettles its 1–0 racks.
    - The same edit reopens a completed job.
  - **Removed fields:** sending `consensus_pct` or `min_results_per_rack` gets a 400.
  - **Floor edit:**
    - Works on every job type and on active, inactive and completed jobs.
    - Doesn't reopen a completed job.
    - A 0.1.4 worker can't claim from a job raised to 0.1.5, while its open claim still submits.
    - An unchanged value writes nothing.
    - A malformed version gets a 400.
    - The edit writes one audit row.
  - **Reissue guard:**
    - Creating a job, or making a consensus edit, with max passes above 1 and a floor below 0.1.5 gets a 400.
    - Lowering the floor below 0.1.5 on such a job gets a 400.
- **e2e.** Update `e18-opening-rack-consensus` to the new settings and the floor edit, end to end.
- **Docs.** Update the job description in `README.md`, the admin steps in `RUNBOOK.md` (the floor edit, and the post-deploy steps for unfinished jobs), `JOURNEYS.md` and `SETTINGS_COMPARISON.md`.

## Order of work

1. **Birdtest PR A, the floor edit.** The endpoint, the audit row, the UI and tests. It is independent of everything else, so it can merge and deploy first.
2. **MAGPIE:** candidates in the request, the round-trip test and the contract fixture. Release 0.1.5.
3. **Birdtest PR B, narrowed reanalysis:** migration 0003, the models, `standing`/`restate_racks`, dispatch, the submission checks, the reissue guard, removal of the old rule from code, UI and tests, then the fake worker, integration tests and e2e.
4. **Before deploying B,** list the unfinished opening-rack jobs and check the running job's `num_plays_recorded`.
5. **Deploy B with opening-rack jobs paused.** Then, for the running job once its first pass is covered: raise its floor to 0.1.5, set its max passes and reactivate it. Do the same for any other unfinished job. Watch the running job's first reissues: candidate counts, the time per rack, and racks settling at 2–0.
6. **Birdtest PR C,** in the next deploy: migration 0004, which drops `consensus_pct` and `min_results_per_rack`.

PR B can deploy before 0.1.5 workers are running. The running job keeps its current floor until you raise it, so its first pass continues on any worker.

## Open questions

1. **Is the margin counted in votes, or in equity or win%?** The plan uses rank-1 votes, as the request describes. A simulated win% gap would settle sooner but needs a statistical threshold, so it is left out.
2. **Is X = 10 wide enough?** Once the running job has reissued a sample of racks, measure how often a full-N reanalysis puts a move outside the first pass's top 10 in first place. If that happens often, raise X for that job; the edit takes effect on the next reissues.
