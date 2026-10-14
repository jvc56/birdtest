-- 2026-10-13: wordmaps, word info tables and rack info tables are a best
-- attempt (MAGPIE 0.1.3; the floor stays 0.1.2, so builds before it still
-- contribute and say nothing of what they skip).
--
-- The first migration after deploy (UPDATES_PLAN_2026-10-13, "Schema changes
-- ship as numbered migrations"). Additive, so the image before it still runs
-- on the schema it leaves: a column that image never names, and an index.

-- The derived files a completed claim's worker ran without for want of disk
-- or memory -- 'wmp', 'wit', 'rit', sorted and once each -- or NULL when it ran
-- with every one its players use (and for a build before 0.1.3, which does
-- not say). Without them a static player plays the same moves, and a simming
-- one equally valid games that need not be the same; nothing decides
-- anything from this. An admin reads it to see why a job's tasks run slow.
ALTER TABLE task_claims ADD COLUMN derived_skipped TEXT[]
    CONSTRAINT task_claims_derived_skipped_roles CHECK (
        derived_skipped <@ ARRAY['wmp', 'wit', 'rit']::text[]
        AND cardinality(derived_skipped) > 0
    );

-- The admin job page's count of skipped tables reads only the claims that
-- skipped one, which is what this indexes and nothing else.
CREATE INDEX task_claims_derived_skipped_idx
    ON task_claims (job_id) WHERE derived_skipped IS NOT NULL;
