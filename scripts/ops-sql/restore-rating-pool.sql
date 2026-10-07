-- RUNBOOK.md §2.6: copy a deleted rating pool and its members back from a
-- copy taken before the deletion, in one transaction. Run it in the shell
-- whose task holds §2.1's scratch copy (its /tmp/restore.env names
-- SCRATCH_URL), or a task where SCRATCH_URL reaches a PITR scratch instance:
--
--   scripts/prod-psql.sh --task <that task's arn> -v pool=<the pool's id> \
--     scripts/ops-sql/restore-rating-pool.sql
--
-- It refuses, copying nothing, if a pool of the same name has been made since
-- (rename that one first), if its anchor or a member config has been deleted
-- since, or -- a foreign-key error on rating_pools_letterdist_id_fkey or
-- rating_pools_layout_id_fkey -- if its letter distribution or layout has
-- (RUNBOOK §2.6 says what then).
--
-- prod-psql: needs pool
-- prod-psql: writes
\getenv scratch SCRATCH_URL
\getenv production DATABASE_URL
\if :{?scratch}
\else
DO $$ BEGIN RAISE EXCEPTION 'no SCRATCH_URL: run this in the task that holds the scratch copy (RUNBOOK 2.1)'; END $$;
\endif
SELECT :'pool'::uuid AS pool \gset

\connect :scratch
COPY (SELECT * FROM rating_pools WHERE id = :'pool') TO STDOUT (FORMAT csv) \g /tmp/pool.csv
COPY (SELECT * FROM rating_pool_members WHERE pool_id = :'pool') TO STDOUT (FORMAT csv) \g /tmp/pool_members.csv
SELECT (SELECT count(*) FROM rating_pools WHERE id = :'pool') AS pools,
       (SELECT count(*) FROM rating_pool_members WHERE pool_id = :'pool') AS members \gset
\echo 'from the scratch copy:' :pools 'pool,' :members 'members'
SELECT :pools = 1 AS found \gset
\if :found
\else
DO $$ BEGIN RAISE EXCEPTION 'the scratch copy has no such pool: a copy from after the deletion, or a mistyped id'; END $$;
\endif

\connect :production
BEGIN;
\copy rating_pools FROM '/tmp/pool.csv' CSV
\copy rating_pool_members FROM '/tmp/pool_members.csv' CSV
COMMIT;
\echo 'copied: now Recompute on the pool page (/ratings/<id>), or POST /api/admin/rating-pools/:id/recompute'
