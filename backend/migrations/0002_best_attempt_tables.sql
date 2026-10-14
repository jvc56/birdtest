-- 2026-10-13: wordmaps, word info tables and rack info tables are a best
-- attempt, and MAGPIE 0.1.3 is the floor that knows it.
--
-- The first migration after deploy (UPDATES_PLAN_2026-10-13, "Schema changes
-- ship as numbered migrations"). Additive, so the image before it still runs
-- on the schema it leaves: a column that image never names, an index, and two
-- column defaults it never relies on (create_job writes a job's floor
-- explicitly).

-- The derived files a completed claim's worker ran without for want of
-- space -- 'wmp', 'wit', 'rit', sorted and once each -- or NULL when it ran
-- with every one its players use (and for a build before 0.1.3, which does
-- not say). The moves are the same without them, so nothing decides anything
-- from this; an admin reads it to see why a job's tasks run slow.
ALTER TABLE task_claims ADD COLUMN derived_skipped TEXT[]
    CONSTRAINT task_claims_derived_skipped_roles CHECK (
        derived_skipped <@ ARRAY['wmp', 'wit', 'rit']::text[]
        AND cardinality(derived_skipped) > 0
    );

-- The admin job page's count of skipped tables reads only the claims that
-- skipped one, which is what this indexes and nothing else.
CREATE INDEX task_claims_derived_skipped_idx
    ON task_claims (job_id) WHERE derived_skipped IS NOT NULL;

-- The floor the server writes for a new job (MIN_MAGPIE_VERSION), kept equal
-- here so a row written any other way (a restore, a hand insert) does not
-- floor a job below the server. 0001's defaults said 0.1.2.
ALTER TABLE jobs ALTER COLUMN min_magpie_major SET DEFAULT 0;
ALTER TABLE jobs ALTER COLUMN min_magpie_minor SET DEFAULT 1;
ALTER TABLE jobs ALTER COLUMN min_magpie_patch SET DEFAULT 3;
