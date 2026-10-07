-- RUNBOOK.md §2.0: clear what a job purged by mistake has generated since,
-- before §2.2 copies the old rows back. Deactivate the job first (admin
-- page or API); this refuses an active one.
--
--   scripts/prod-psql.sh -v job=<the job's id> scripts/ops-sql/clear-job.sql
--
-- prod-psql: needs job
-- prod-psql: writes
\if :{?job}
\else
DO $$ BEGIN RAISE EXCEPTION 'run with -v job=<the job id>'; END $$;
\endif

BEGIN;
-- Which job, so the log says what was cleared; and the refusals.
SELECT id, name, job_type, status FROM jobs WHERE id = :'job'::uuid;
SELECT EXISTS (SELECT 1 FROM jobs WHERE id = :'job'::uuid) AS job_exists,
       EXISTS (SELECT 1 FROM jobs WHERE id = :'job'::uuid AND status = 'active') AS job_active \gset
\if :job_active
DO $$ BEGIN RAISE EXCEPTION 'the job is active: deactivate it first (admin page or API), then run this again'; END $$;
\endif
\if :job_exists
\else
\echo 'no jobs row: a deleted job has nothing to clear (2.2 restores it whole)'
\endif
-- A purged job that completed again since -- a small job, or a force-complete --
-- cannot be deactivated from the admin page, and its verdict is from the
-- results about to be deleted: back to inactive, with no verdict.
UPDATE jobs SET status = 'inactive', test_decided_status = NULL,
                test_decided_lower = NULL, test_decided_upper = NULL,
                test_decided_units = NULL
 WHERE id = :'job' AND status = 'completed';
-- And an export of those results: once §2.3 completes the job again it would be
-- served as the restored job's corpus. (A purge deletes exports for this
-- reason; the objects go with the bucket's lifecycle rule.)
DELETE FROM job_exports WHERE job_id = :'job';
DELETE FROM task_claims c USING tasks t WHERE c.task_id = t.id AND t.job_id = :'job';
DELETE FROM tasks WHERE job_id = :'job';
DELETE FROM opening_rack_progress       WHERE job_id = :'job';
DELETE FROM leave_rack_progress         WHERE job_id = :'job';
DELETE FROM leave_rack_staging          WHERE job_id = :'job';
DELETE FROM leave_generation_progress   WHERE job_id = :'job';
DELETE FROM leave_selection_cursors     WHERE job_id = :'job';
DELETE FROM leave_generation_artifacts  WHERE job_id = :'job';
DELETE FROM leave_generation_transitions WHERE job_id = :'job';
COMMIT;
