# Opening racks: narrowed reanalysis plan

**Goal:** after an opening-rack job's first pass, reanalyse each rack over only the top X moves its first analysis found, not the player's full candidate set (`num_plays`, N). Keep reanalysing until one move leads every other by 2 first-place finishes, or until the rack reaches the job's maximum number of passes.
**Status:** plan only. Nothing is implemented yet.
**Written:** 2026-11-11, against `main` at 9395521 and MAGPIE `main`.

**Decided in review:**
- The first pass's result counts toward the 2-vote lead.
- X defaults to 10, is set per job, and can be changed while the job runs.
- The new rule replaces the percentage rule and applies to existing jobs, including one that is still on its first pass.

## How it works today

- **First pass.** `next_request` (`backend/src/jobs/opening_rack.rs`) splits the rack space into slices of `racks_per_batch`. Each task sends its racks to MAGPIE, which generates every move for the rack (`config_contribute_generate_for_rack`) and, for a simming player, simulates the top `num_plays` of them. The ranked moves come back, capped at `num_plays_recorded`, and are stored in `position_analysis_records` and `position_analysis_moves`.
- **Reissues.** If `max_results_per_rack > 1`, `next_reissue` hands out unsettled racks once the space is covered, fewest analyses first, as a task that lists its racks in `opening_rack_requests.racks`. **Every reissue analyses the rack from scratch with all N candidates.**
- **Settling.** `record_consensus` counts each move's rank-1 finishes across a rack's analyses and calls `standing`. A rack is settled once it has at least `min_results_per_rack` analyses and the top move holds at least `consensus_pct`% of them, or once it reaches `max_results_per_rack`. `restate_racks` applies the same rule in SQL when an admin edits the settings, and an edit that unsettles racks of a completed job reopens it.

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
| `min_results_per_rack`, `consensus_pct` | existing | No longer read. Dropped from the API and UI. The columns stay, because a migration must not drop what the previous image reads, and a later migration can remove them. | n/a |

The existing simmer requirement for `max > 1` (`validate_opening_rack_player`) still applies. A static player would return the same order on every pass.

## Applying it to existing jobs, including the running one

- **Migration.** Migration 0003 adds `reissue_top_moves` and `win_margin` as `NOT NULL` with defaults 10 and 2, so every job, old and new, is on the new rule from the moment it deploys. There is no legacy branch in the code.
- **Restating.** Racks keep the settled flags they have until something restates them. A submission recomputes the standing of the racks it touches, and a consensus edit restates the whole job under its existing locks and hold. The migration does not restate in place: restating a full English job rewrites millions of rows, and the edit path already does that safely.
- **The running job (first pass).** Every rack it has analysed has one result, so `runner_up_count` 0 is already correct and nothing needs backfilling. After deploy, set the job's max passes, for example 7, in the consensus editor. That edit restates the racks: every rack with one vote becomes unsettled, because 1–0 is not a lead of 2, and reissues follow as soon as the first pass is covered. If the first pass finishes before the deploy and the job completes, the same edit reopens it, as it does today. Either way, no analysis is lost or repeated.
- **Effective X on that job.** Candidates can only come from moves that were stored. Check its player's `num_plays_recorded` before deploying. If it is below 10, its reissues use that many candidates, and there is no way to recover moves that were never stored.
- **Other unfinished jobs.** Any job already in its reissue phase should get a consensus edit after deploy, to restate it under the new rule. Before deploying, list the opening-rack jobs that are not completed. Completed jobs are left as they are unless an edit reopens one.
- **During the deploy.** Pause opening-rack jobs (set them inactive) for the rollout. The old image doesn't write `runner_up_count`, so a reissue it accepted during the rollout window could leave that column stale on a rack with more than one result.

## Changes

### MAGPIE (first, as its own PR and release)

- **Request.** Each `opening_rack` task request may carry an optional `candidates` field: an array parallel to `racks`, where element *i* is the list of move strings for rack *i*. It is present only on reissues.
- **Analysis.** In `config_contribute_generate_for_rack`, when a rack has candidates, skip move generation. Build the move list from the strings with the same path `addmoves` uses (`validated_moves_create` → `validated_moves_add_to_sorted_move_list`), then simulate as normal. `num_moves` reports how many candidates were simulated.
- **Refusals.** A request with candidates for a static player, an array whose length doesn't match `racks`, an empty candidate list, or a move that doesn't validate on the empty board with that rack is an `ERROR_STATUS_CONTRIBUTE_SERVER_ERROR`. That is the same class as an unusable rack.
- **Round trip.** The move strings birdtest stores are the ones MAGPIE wrote (`autoplay_results_write_ranked_plays_json`). Add a test that every kind it writes, including placements, exchanges and blank designations, parses back through `validated_moves_create` and produces the same move. **This is the main technical risk.** If any format fails to round-trip, fix it in MAGPIE before anything else.
- **Contract fixture.** Add `test/birdtest_contract/assignment-opening-rack-candidates.json`.
- **Version.** Ship as a feature release, 0.1.5 (0.1.4, the current release, has no candidates support). No floor changes: `DEFAULT_MIN_MAGPIE_VERSION` stays at 0.1.2 under the floor policy, and no job's own `min_magpie_version` is raised, the running job's included. A job floor of 0.1.5 would lock 0.1.2–0.1.4 workers out of its remaining first pass, which needs nothing new from MAGPIE. Only reissues need 0.1.5, so gating happens per task instead (next section). The gate's version is a constant in the backend (`CANDIDATES_MIN_MAGPIE_VERSION`), not a setting.

### Birdtest backend

- **Migration `0003_opening_rack_narrowing.sql`** (additive):
  - `job_opening_rack_config`: add `reissue_top_moves INT NOT NULL DEFAULT 10 CHECK (reissue_top_moves BETWEEN 2 AND 100)` and `win_margin INT NOT NULL DEFAULT 2 CHECK (win_margin BETWEEN 1 AND 100)`.
  - `opening_rack_progress`: add `runner_up_count INT NOT NULL DEFAULT 0`. Backfill it only for rows with `results > 1`, from their rank-1 tallies. These are bounded by the reissues already made, and are zero for the running job.
  - `opening_rack_requests`: add `candidates JSONB NULL`, an array of string arrays parallel to `racks`, set only on reissue tasks.
- **`models/job.rs`.** `OpeningRackConfig` and `ConsensusSettings` replace `consensus_pct` and `min_results_per_rack` with `reissue_top_moves` and `win_margin`. Update `ConsensusSettings::load` to match.
- **`standing`.** Compute `runner_up_count` (the second-highest rank-1 count, or 0). `agreed = top_count - runner_up_count >= win_margin`, and `settled = agreed || results >= max_passes`. A rack whose candidate list has only one move also counts as agreed, because no other move can win it. Update `restate_racks` to the same rule, and replace its old tests and `standing`'s with ones for the margin.
- **`record_consensus`.** Also write `runner_up_count`. Otherwise unchanged.
- **Dispatch.**
  - `next_reissue` still chooses the racks, unchanged.
  - `next_request` then derives their candidates in one query, bounded by the batch: each rack's record whose task has `seed < total_racks` (the first-pass task), with its moves where `rank <= X`, in rank order. It puts them in `OpeningRackRequest.candidates: Option<Vec<Vec<String>>>`, marked `skip_serializing_if = "Option::is_none"` so first-pass tasks send exactly what they send today.
  - `insert_request` writes them to `opening_rack_requests.candidates`, and `load_request` reads them back from there.
- **Per-task version gate.** A worker below 0.1.5 can still claim a job's first-pass tasks but not its reissues. Pass the claim's `WorkerCapabilities` into `registry::acquire`. For a worker below 0.1.5, `next_request` creates no reissue, and re-dispatching an available task skips tasks whose request carries candidates. The worker then gets `NoWork` for this job, and the scheduler moves on to other jobs.
- **Submissions.** In `process_response` and `check_batch_against_task`, a reissue's analysis must return only that rack's candidates, and `num_moves` must not exceed the number of candidates. `check_batch_against_task` already reads the request row inside the task's row lock, so this adds one column to that read, not another query.
- **Job creation and edits (`routes/admin.rs`).**
  - Creation accepts `reissue_top_moves` (default 10) and `win_margin` (default 2), and stops accepting `consensus_pct` and `min_results_per_rack`. To keep scripts working for one release, sending them is accepted and ignored.
  - `PATCH …/consensus` edits all three settings: `reissue_top_moves`, `win_margin` and `max_results_per_rack`. Changing X alone writes the setting without a restate, because it doesn't change any rack's standing. Changing the margin or max passes restates the job, and can reopen a completed one, as today.
  - `consensus_problems` checks the new ranges.
- **Read paths.** `jobstats.rs` (the job settings shown), `exports.rs` (the `consensus` object gains `runner_up_count` and loses nothing), and `routes/public.rs` where they expose consensus settings.

### Frontend

- **`lib/consensus.ts`.** Change `consensusFields`, `consensusProblem` and `analysesPerRack` to the new rule, for example "up to 7 passes, first to lead by 2, reissues over the top 10".
- **`admin/jobs/new`.** Replace the minimum and share boxes with "Reanalyse top X moves" (default 10) and "Win by" (default 2). Relabel "most analyses" as "Max passes". Warn when X is larger than the player's `num_plays_recorded`.
- **`ConsensusEditor.svelte`.** Edit X, the margin and max passes. Note beside X that a change applies to reissues from then on.
- **`jobs/[id]`.** On a rack sample, show the current lead, for example "A 3 – B 1". On a reissued analysis, also show the candidates it ran over.

### Worker stub, fixtures, tests, docs

- **`worker/fake_worker.py`.** When `candidates` is present, rank only those moves, with a deterministic rank-1 that can be forced to split, so the e2e tests can exercise both a margin settle and a max-passes settle.
- **Contract fixtures.** Add `contract-fixtures/` request and response pairs for a reissue with candidates. The existing first-pass fixtures don't change.
- **Unit tests** (`opening_rack.rs`):
  - Settling by margin (2–0, 3–1, 4–2), and no settle at 1–0 or 2–1.
  - A tie at max settles without consensus.
  - A single candidate settles.
  - `restate_racks` agrees with `standing`.
- **Integration tests** (`backend/tests/scheduler.rs`, `worker_api.rs`):
  - A first-pass task sends no candidates.
  - A reissue sends exactly the first-pass analysis's top X, capped at the stored moves.
  - A reclaim gets the same candidates, even after an edit to X.
  - A job created before the migration gets candidates for racks analysed before it.
  - A submission containing a move outside the candidates is refused with a 400.
  - A 0.1.4 worker can still claim first-pass tasks but no reissues.
  - A consensus edit on a first-pass job unsettles its 1–0 racks.
  - The same edit reopens a completed job.
- **e2e.** Update `e18-opening-rack-consensus` to the new settings, end to end.
- **Docs.** Update the job description in `README.md`, the admin steps in `RUNBOOK.md` (including the post-deploy edit for unfinished jobs), and the rows in `SETTINGS_COMPARISON.md`.

## Order of work

1. MAGPIE: candidates in the request, the round-trip test and the contract fixture. Release 0.1.5.
2. Birdtest: migration 0003, the models, and `standing`/`restate_racks`, with unit tests.
3. Dispatch, the version gate, the submission checks, the fake worker and integration tests.
4. Admin API and frontend, then e2e.
5. Before deploying, list the unfinished opening-rack jobs and check the running job's `num_plays_recorded`.
6. Deploy with opening-rack jobs paused. Then set max passes on the running job (and edit any other unfinished job) and reactivate them. Watch the running job's first reissues: candidate counts, the time per rack, and racks settling at 2–0.

Steps 2–4 are one birdtest PR. It can deploy before 0.1.5 workers are running: the running job keeps doing its first pass on any worker, and its reissues wait for a 0.1.5 worker.

## Open questions

1. **Is the margin counted in votes, or in equity or win%?** The plan uses rank-1 votes, as the request describes. A simulated win% gap would settle sooner but needs a statistical threshold, so it is left out.
2. **Is X = 10 wide enough?** Once the running job has reissued a sample of racks, measure how often a full-N reanalysis puts a move outside the first pass's top 10 in first place. If that happens often, raise X for that job; the edit takes effect on the next reissues.
