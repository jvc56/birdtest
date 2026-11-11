# Opening racks: narrowed reanalysis plan

**Goal:** after an opening-rack job's first pass, reanalyse each rack over only the top X moves its first analysis found, not the player's full candidate set (`num_plays`, N). Keep reanalysing until one move leads every other by 2 first-place finishes, or until the rack reaches the job's maximum number of passes.
**Status:** plan only. Nothing is implemented yet.
**Written:** 2026-11-11, against `main` at 9395521 and MAGPIE `main`.

## How it works today

- **First pass.** `next_request` (`backend/src/jobs/opening_rack.rs`) splits the rack space into slices of `racks_per_batch`. Each task sends its racks to MAGPIE, which generates every move for the rack (`config_contribute_generate_for_rack`) and, for a simming player, simulates the top `num_plays` of them. The ranked moves come back, capped at `num_plays_recorded`, and are stored in `position_analysis_records` and `position_analysis_moves`.
- **Reissues.** If `max_results_per_rack > 1`, `next_reissue` hands out unsettled racks once the space is covered, fewest analyses first, as a task that lists its racks in `opening_rack_requests.racks`. **Every reissue analyses the rack from scratch with all N candidates.**
- **Settling.** `record_consensus` counts each move's rank-1 finishes across a rack's analyses and calls `standing`. A rack is settled once it has at least `min_results_per_rack` analyses and the top move holds at least `consensus_pct`% of them, or once it reaches `max_results_per_rack`. `restate_racks` applies the same rule in SQL when an admin edits the settings.

The cost is in the reissues. A rack's first pass already ranks its N candidates, and almost all of them are clearly worse than the leaders. Simulating them again on every pass spends most of the iteration budget on moves that cannot win.

## The new behaviour

1. **Pass 1 is unchanged.** Every rack is analysed once with the full N candidates.
2. **Fixing the candidates.** When a rack's first analysis is recorded, its top X ranked moves become the rack's **candidates**. They are stored once and never change.
3. **Passes 2 and later.** A reissue sends each rack with its candidates. MAGPIE simulates only those moves, and its rank-1 move is that pass's vote.
4. **Settling.** After each pass, the rack is settled if its leading move has at least `win_margin` (default 2) more rank-1 finishes than the runner-up, or if it has had `max_passes` analyses. The second case is "settled without consensus", as it is today, and records the leading move (the alphabetically first one if tied).
5. **Which passes vote.** The first pass votes. Its rank-1 move is always one of the candidates, so the rule stays "first to lead by 2". The fastest settle is two passes, the first plus one reissue that agrees with it. See open question 1.

Example with X = 5, margin 2 and max 7 passes. Pass 1 ranks 8H AEINRST first. Pass 2 also ranks it first (2–0), so the rack settles after two analyses. On a split rack the votes might go A, B, A, B, A, A (4–2), and it settles after six. A rack still within 1 after seven passes settles without consensus.

## Settings

| Setting | Where | Rule | Editable after creation? |
|---|---|---|---|
| `reissue_top_moves` (X) | `job_opening_rack_config`, new | 2 ≤ X ≤ player's `num_plays_recorded`. Only the stored moves can become candidates. | No. Candidates are fixed at each rack's first analysis, and changing X partway would give racks different candidate sets. |
| `win_margin` | `job_opening_rack_config`, new, default 2 | 1 ≤ margin ≤ `max_passes`. The UI suggests 2. | Yes, through the existing consensus editor, and `restate_racks` restates the racks. |
| `max_passes` | the existing `max_results_per_rack`, relabelled in the UI | Unchanged: 1 to 100. 1 means one analysis per rack, with no reissues and no candidates. | Yes, as today. |
| `min_results_per_rack`, `consensus_pct` | existing | Not used by jobs on the new rule. | n/a |

The existing simmer requirement for `max > 1` (`validate_opening_rack_player`) still applies. A static player would return the same order on every pass.

### Existing jobs

The project is deployed, so the schema changes go in additive migrations. Both new columns are nullable. A job with `reissue_top_moves IS NULL` keeps the current rule (full reissues and a percentage consensus) unchanged, so every job created before this change behaves as it does now. New jobs created through the admin form get the new rule. The API still accepts the old fields for one release, so scripts and e2e helpers that create jobs keep working (open question 3).

## Changes

### MAGPIE (first, as its own PR and release)

- **Request.** Each `opening_rack` task request may carry an optional `candidates` field: an array parallel to `racks`, where element *i* is the list of move strings for rack *i*. It is present only on narrowed reissues.
- **Analysis.** In `config_contribute_generate_for_rack`, when a rack has candidates, skip move generation. Build the move list from the strings with the same path `addmoves` uses (`validated_moves_create` → `validated_moves_add_to_sorted_move_list`), then simulate as normal. `num_moves` reports how many candidates were simulated, which is at most X.
- **Refusals.** A request with candidates for a static player, an array whose length doesn't match `racks`, an empty candidate list, or a move that doesn't validate on the empty board with that rack is an `ERROR_STATUS_CONTRIBUTE_SERVER_ERROR`. That is the same class as an unusable rack.
- **Round trip.** The move strings birdtest stores are the ones MAGPIE wrote (`autoplay_results_write_ranked_plays_json`). Add a test that every kind it writes, including placements, exchanges and blank designations, parses back through `validated_moves_create` and produces the same move. **This is the main technical risk.** If any format fails to round-trip, fix it in MAGPIE before anything else.
- **Contract fixture.** Add `test/birdtest_contract/assignment-opening-rack-candidates.json`.
- **Version.** Ship as a feature release, 0.1.4. Per the floor policy, `DEFAULT_MIN_MAGPIE_VERSION` stays at 0.1.2. Narrowed jobs set the **per-job** `min_magpie_version` to 0.1.4 instead (next section).

### Birdtest backend

- **Migration `0003_opening_rack_narrowing.sql`** (additive):
  - `job_opening_rack_config`: add `reissue_top_moves INT NULL CHECK (reissue_top_moves >= 2)` and `win_margin INT NULL CHECK (win_margin >= 1)`, with a check that both are null or both are set.
  - `opening_rack_progress`: add `candidates TEXT[] NULL` (non-empty when set) and `runner_up_count INT NOT NULL DEFAULT 0`. Backfill `runner_up_count` for existing rows in the migration, so a legacy job switched to the margin rule later would restate correctly. That switch is not offered, but the column should not hold wrong data.
- **`models/job.rs`.** `OpeningRackConfig` and `ConsensusSettings` gain `reissue_top_moves: Option<i32>` and `win_margin: Option<i32>`. `ConsensusSettings::load` reads both. Add `ConsensusSettings::narrowed()`.
- **`standing`.** Compute `runner_up_count` (the second-highest rank-1 count, or 0). On the new rule, `agreed = top_count - runner_up_count >= win_margin`. A rack with only one candidate also counts as agreed: no other move can win it. Otherwise the rule is unchanged. Update `restate_racks` to match. It must continue to implement the same rule as `standing`, and the existing tests that keep the two in step should cover the new branch.
- **`record_consensus`.** When a rack has no progress row yet (its first analysis) and the job is narrowed, take the first X moves of that analysis by rank and write them to `candidates`, along with `runner_up_count`. This is the one point where the candidates are chosen.
- **Dispatch.** `next_reissue` does not change: it still selects racks, and fewest-analyses-first already works through the racks pass by pass. `OpeningRackRequest` gains `candidates: Option<Vec<Vec<String>>>` with `skip_serializing_if = "Option::is_none"`, so first-pass tasks and legacy jobs send exactly what they send today. Both `next_request` and `load_request` (which rebuilds the request on a reclaim) fill it for a listed-racks task on a narrowed job, from `opening_rack_progress.candidates`, in the order of `racks`. The candidates never change, so a reclaim reproduces the original request exactly.
- **Submissions.** In `process_response` and `check_batch_against_task`, a narrowed reissue's analysis must return only that rack's candidates, and `num_moves` must not exceed the number of candidates. That requires the task's candidates, so read them alongside `task_racks` in `check_batch_against_task`, inside the same row lock and with one extra column, not an extra query.
- **Job creation (`routes/admin.rs`).** Accept `reissue_top_moves` and `win_margin`. Validate X against the player's `num_plays_recorded` and `num_plays`. When `max_results_per_rack > 1` and X is set, raise the job's `min_magpie_version` to at least 0.1.4. Extend `consensus_problems` for the margin. `PATCH …/consensus` accepts `win_margin` and `max_results_per_rack`, and refuses `reissue_top_moves`.
- **Read paths.** `jobstats.rs` (the job settings shown), `exports.rs` (the `consensus` object gains `runner_up_count`, and `candidates` for narrowed jobs), and `routes/public.rs` where they expose consensus settings.

### Frontend

- **`lib/consensus.ts`.** Add the margin rule to `consensusFields`, `consensusProblem` and `analysesPerRack`, for example "up to 7 passes, first to lead by 2, reissues over the top 5".
- **`admin/jobs/new`.** Replace the minimum and share boxes with "Reanalyse top X moves" and "Win by" (default 2). Relabel "most analyses" as "Max passes". Show the per-job version floor that will be applied.
- **`ConsensusEditor.svelte`.** Edit the margin and max passes. Show X as read-only.
- **`jobs/[id]`.** On a rack sample, show the candidates and the current lead, for example "A 3 – B 1".

### Worker stub, fixtures, tests, docs

- **`worker/fake_worker.py`.** When `candidates` is present, rank only those moves, with a deterministic rank-1 that can be forced to split, so the e2e tests can exercise both a margin settle and a max-passes settle.
- **Contract fixtures.** Add `contract-fixtures/` request and response pairs for a narrowed reissue. The existing fixtures don't change.
- **Unit tests** (`opening_rack.rs`): settling by margin (2–0, 3–1, 4–2); no settle at 2–1; a tie at max settles without consensus; a single candidate settles; the legacy rule is unchanged for null settings; `restate_racks` matches `standing` for both rules.
- **Integration tests** (`backend/tests/scheduler.rs`, `worker_api.rs`): the first pass sends no candidates; a reissue sends exactly the stored top X; a reclaim gets the same candidates; a submission containing a move outside the candidates is refused with a 400; a narrowed job's floor keeps a 0.1.3 worker from claiming its reissues while it can still claim legacy jobs.
- **e2e.** Extend `e18-opening-rack-consensus` to cover a narrowed job end to end.
- **Docs.** Update the job description in `README.md`, the admin steps in `RUNBOOK.md`, and the rows in `SETTINGS_COMPARISON.md`.

## Order of work

1. MAGPIE: candidates in the request, the round-trip test and the contract fixture. Release 0.1.4.
2. Birdtest: migration 0003, the models, `standing`/`restate_racks` and `record_consensus`, with unit tests.
3. Dispatch and submission checks, the fake worker and integration tests.
4. Admin API and frontend, then e2e.
5. Deploy. Create a small narrowed job (rack size 2 or 3) on prod to check it end to end before running a full-size one.

Steps 2–4 are one birdtest PR, deployable whenever it's ready. Until 0.1.4 workers are running, a narrowed job's reissues just wait for them, and legacy jobs are unaffected.

## Open questions

1. **Does the first pass vote?** The plan says yes: it is a real analysis, its winner is a candidate, and counting it allows a two-pass settle. The case against is that it simulated N moves with the same budget, so each candidate got fewer iterations and its vote is noisier. The alternative is to tally reissues only, which means at least two reissues per rack and roughly one extra pass per rack. That is a one-line difference in `record_consensus` and the read paths, so it can be decided during review.
2. **Is the margin counted in votes, or in equity or win%?** The plan uses rank-1 votes, as the request describes. A simulated win% gap would settle sooner but needs a statistical threshold, so it is left out.
3. **Retire the percentage rule?** The plan keeps it for existing jobs only. Once none are running, a later change can drop `min_results_per_rack` and `consensus_pct` from the API and UI, and the columns in a later migration.
4. **Default X.** The UI default could be 5, or `min(5, num_plays_recorded)`. This is worth measuring on a small job by checking how often pass 1's winner lies outside the top X of a later full reanalysis. If the true best move often falls outside the top 5, X should be larger.
