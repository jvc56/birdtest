-- RUNBOOK.md §4, checks 2 and 3: referential and counter sanity after a
-- restore. Reads only. Every count must be zero.
--
--   scripts/prod-psql.sh scripts/ops-sql/check-restore.sql
--
-- (Check 3b, the contributor counters, is in RUNBOOK.md §4 itself.) The
-- queries are §4's as written there; scripts/ops-scripts-check.sh fails if
-- the two drift apart.
BEGIN TRANSACTION READ ONLY;

-- 2. Referential sanity.
SELECT count(*) AS jobs_missing_data FROM jobs j
 WHERE NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.letterdist_id)
    OR NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.layout_id);

SELECT count(*) AS inputs_missing_content FROM input_data
 WHERE role IN ('letterdist','layout') AND content IS NULL;

-- 3. Counter sanity. Must be zero. Serial: in parallel, each worker builds
--    the whole grouped aggregate of the claims itself.
SET max_parallel_workers_per_gather = 0;
SELECT count(*) AS counter_disagreements
  FROM tasks t
  LEFT JOIN (
    -- One pass over the claims, grouped: a per-task probe (as a lateral
    -- join) was a random index read per task, hours on the drill's disk
    -- at tens of millions of tasks.
    SELECT c.task_id,
           count(*) FILTER (WHERE c.state = 'completed') AS accepted,
           count(*) FILTER (WHERE c.state = 'claimed')   AS active
      FROM task_claims c GROUP BY c.task_id
  ) actual ON actual.task_id = t.id
 WHERE t.accepted_count <> COALESCE(actual.accepted, 0)
    OR t.active_claim_count <> COALESCE(actual.active, 0);

COMMIT;
