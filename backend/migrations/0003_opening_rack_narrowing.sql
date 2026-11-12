-- 2026-11-11: opening racks are reanalysed over their first pass's leaders,
-- and settle when one move leads by a margin (OPENING_RACK_NARROWING_PLAN.md).
--
-- After a rack's first analysis, every later one ranks only the top
-- `reissue_top_moves` moves of that first analysis (MAGPIE 0.1.5's
-- `candidates`), and the rack settles once its most common rank-1 move has
-- `win_margin` more rank-1 finishes than the runner-up, or once it has
-- `max_results_per_rack` analyses. That rule replaces the share-of-analyses
-- one: `consensus_pct` and `min_results_per_rack` are read by nothing from
-- this release, and a later migration drops them, once no image that reads
-- them is running.
--
-- Additive, so the image before it still runs on the schema it leaves.

-- X, and the lead that settles a rack. Every job, old and new, is on the rule
-- from here: a job's racks restate under it at its next consensus edit, and
-- each as a submission touches it.
ALTER TABLE job_opening_rack_config
    ADD COLUMN reissue_top_moves INT NOT NULL DEFAULT 10
        CONSTRAINT job_opening_rack_config_reissue_top_moves_range
        CHECK (reissue_top_moves BETWEEN 2 AND 100),
    ADD COLUMN win_margin INT NOT NULL DEFAULT 2
        CONSTRAINT job_opening_rack_config_win_margin_range
        CHECK (win_margin BETWEEN 1 AND 100);

-- No longer written, and still under 0001's CHECK that the most analyses are
-- at least the fewest: at 1, any most an edit sets passes it.
UPDATE job_opening_rack_config SET min_results_per_rack = 1 WHERE min_results_per_rack <> 1;

-- How many analyses ranked the runner-up first: the second-highest count of
-- a rack's rank-1 moves, 0 while one move holds them all. The lead that
-- settles a rack is `top_count - runner_up_count`.
ALTER TABLE opening_rack_progress
    ADD COLUMN runner_up_count INT NOT NULL DEFAULT 0
        CONSTRAINT opening_rack_progress_runner_up_count_range
        CHECK (runner_up_count >= 0 AND runner_up_count <= top_count);

-- A rack with one analysis has no runner-up, so only those analysed again
-- need theirs counted: the reissues already made, none for a job still on
-- its first pass.
WITH firsts AS (
    SELECT a.job_id, a.rack, m.move, COUNT(*)::int AS n
    FROM opening_rack_progress p
    JOIN position_analysis_records a
      ON a.job_id = p.job_id AND a.rack = p.rack AND a.game_index IS NULL
    JOIN position_analysis_moves m ON m.record_id = a.id AND m.rank = 1
    WHERE p.results > 1
    GROUP BY a.job_id, a.rack, m.move
), ranked AS (
    SELECT job_id, rack, n,
           row_number() OVER (PARTITION BY job_id, rack ORDER BY n DESC, move) AS place
    FROM firsts
)
UPDATE opening_rack_progress p
SET runner_up_count = r.n
FROM ranked r
WHERE r.place = 2 AND p.job_id = r.job_id AND p.rack = r.rack;

-- The moves a reissue sent for each of its racks, parallel to `racks`: each
-- rack's first analysis's top `reissue_top_moves` when it was dispatched.
-- Kept on the request so a reclaim sends the same moves, and a submission is
-- checked against what was asked, whatever X is by then. NULL on a task
-- covering a range of the first pass, and on a reissue made before this.
ALTER TABLE opening_rack_requests
    ADD COLUMN candidates JSONB
        CONSTRAINT opening_rack_requests_candidates_shape
        CHECK (
            candidates IS NULL
            OR (racks IS NOT NULL
                AND jsonb_typeof(candidates) = 'array'
                AND jsonb_array_length(candidates) = rack_count)
        );
