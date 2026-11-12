-- 2026-11-11: the share-of-analyses consensus rule's columns go
-- (OPENING_RACK_NARROWING_PLAN.md). 0003 put every opening-rack job on the
-- lead rule (`win_margin`, `reissue_top_moves`), and no image since reads
-- `consensus_pct` or `min_results_per_rack`. Shipped a deploy after 0003, so
-- the image running while this applies is one that never names them.
--
-- Dropping `min_results_per_rack` takes 0001's CHECK that the most analyses
-- are at least the fewest with it, and with that check its bound on the most,
-- which is put back on its own.

ALTER TABLE job_opening_rack_config
    DROP COLUMN consensus_pct,
    DROP COLUMN min_results_per_rack,
    ADD CONSTRAINT job_opening_rack_config_max_results_range
        CHECK (max_results_per_rack BETWEEN 1 AND 100);

COMMENT ON COLUMN job_opening_rack_config.max_results_per_rack IS
    'The most analyses a rack gets, its first included. A rack settles once '
    'its most common rank-1 move leads the runner-up by win_margin analyses, '
    'or at this many, without a consensus. 1 is one analysis per rack.';
