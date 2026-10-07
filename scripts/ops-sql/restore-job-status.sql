-- RUNBOOK.md §2.3, the end: put back a restored job's status (and, for a
-- purged job, its match-test verdict) from the scratch copy's `jobs` row,
-- after repair-job-counters.sql.
--
--   scripts/prod-psql.sh -v job=<id> -v old_status=completed -v old_verdict=... \
--     -v old_lower=... -v old_upper=... -v old_units=... scripts/ops-sql/restore-job-status.sql
--
-- Each value from the scratch copy's row
--   SELECT status, test_decided_status, test_decided_lower, test_decided_upper,
--          test_decided_units
--   FROM jobs WHERE id = '<the job id>';
-- and the empty string where it is NULL (a job an admin completed has no
-- verdict): :'var' always quotes, so NULLIF is what turns empty back into NULL.
--
-- prod-psql: needs job old_status old_verdict old_lower old_upper old_units
-- prod-psql: writes
\if :{?old_units}
\else
DO $$ BEGIN RAISE EXCEPTION 'run with -v job, old_status, old_verdict, old_lower, old_upper and old_units'; END $$;
\endif
UPDATE jobs SET status = :'old_status',
               test_decided_status = NULLIF(:'old_verdict', ''),
               test_decided_lower  = NULLIF(:'old_lower', '')::float8,
               test_decided_upper  = NULLIF(:'old_upper', '')::float8,
               test_decided_units  = NULLIF(:'old_units', '')::bigint
 WHERE id = :'job'
RETURNING id, status, test_decided_status;
