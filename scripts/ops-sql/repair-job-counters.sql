-- RUNBOOK.md §2.3: repair a restored job's counters, after §2.2's copy, in
-- one transaction.
--
--   scripts/prod-psql.sh -v job=<the job's id> scripts/ops-sql/repair-job-counters.sql
--
-- prod-psql: needs job
-- prod-psql: writes
\if :{?job}
\else
DO $$ BEGIN RAISE EXCEPTION 'run with -v job=<the job id>'; END $$;
\endif

BEGIN;

UPDATE tasks t
   SET accepted_count     = actual.accepted,
       active_claim_count = actual.active
  FROM (
    SELECT t2.id,
           count(*) FILTER (WHERE c.state = 'completed')::int AS accepted,
           count(*) FILTER (WHERE c.state = 'claimed')::int   AS active
      FROM tasks t2
      LEFT JOIN task_claims c ON c.task_id = t2.id
     WHERE t2.job_id = :'job'
     GROUP BY t2.id
  ) actual
 WHERE t.id = actual.id
   -- Only rows that are wrong: every task of a large job rewritten was
   -- seconds of writes and as many dead tuples, for rows already right.
   AND (t.accepted_count, t.active_claim_count) IS DISTINCT FROM (actual.accepted, actual.active);

-- State and completed_at follow from the counters, exactly as the submit
-- path computes them: a task has one slot, so an accepted result completes
-- it and a live claim holds it.
UPDATE tasks t
   SET state = CASE
         WHEN t.accepted_count > 0 THEN 'completed'::task_state
         WHEN t.active_claim_count > 0 THEN 'claimed'::task_state
         ELSE 'available'::task_state
       END,
       completed_at = CASE WHEN t.accepted_count > 0
                           THEN COALESCE(t.completed_at, now()) ELSE NULL END
 WHERE t.job_id = :'job'
   AND t.state IS DISTINCT FROM CASE
         WHEN t.accepted_count > 0 THEN 'completed'::task_state
         WHEN t.active_claim_count > 0 THEN 'claimed'::task_state
         ELSE 'available'::task_state
       END;

-- The job's own counters. claims_issued is the scheduler's deficit numerator,
-- measured from claims_baseline; the statement after this one puts the
-- restored job level with the jobs beside it, as activation and purge do
-- (scheduler::join_at_parity), so it neither owes nor is owed a backlog. The
-- rest are the dashboard's progress totals; they
-- are maintained one task at a time in the claim and submit paths, so a row
-- copy leaves them describing the results the job had before. Each is
-- recomputed here exactly as the read it replaced computed it.
-- The claims are read once, for all of their columns: a second subquery for
-- last_completed_at was a second pass over them. movegens is what the submit
-- path added, each accepted claim's own.
UPDATE jobs j
   SET claims_issued = cl.issued,
       last_completed_at = cl.last,
       movegens = cl.movegens,
       tasks_total = (SELECT count(*) FROM tasks t WHERE t.job_id = j.id),
       tasks_completed = (SELECT count(*) FROM tasks t
                           WHERE t.job_id = j.id AND t.state = 'completed'),
       games_completed = (SELECT COALESCE(sum(r.games), 0)
                            FROM game_results r WHERE r.job_id = j.id),
       racks_analyzed = (SELECT count(DISTINCT p.rack)
                           FROM position_analysis_records p
                          WHERE p.job_id = j.id AND p.game_index IS NULL),
       -- An opening-rack job's settled racks are its settled progress rows:
       -- every opening-rack job keeps one per rack, and one wanting a single
       -- analysis per rack settles each at its first.
       racks_settled = (SELECT count(*) FROM opening_rack_progress p
                         WHERE p.job_id = j.id AND p.settled),
       racks_without_consensus = (SELECT count(*) FROM opening_rack_progress p
                                   WHERE p.job_id = j.id AND p.without_consensus)
  FROM (SELECT count(*) AS issued,
               max(c.completed_at) FILTER (WHERE c.state = 'completed') AS last,
               COALESCE(sum(c.movegens) FILTER (WHERE c.state = 'completed'), 0) AS movegens
          FROM task_claims c JOIN tasks t ON t.id = c.task_id
         WHERE t.job_id = :'job') cl
 WHERE j.id = :'job';

-- Level with the lowest of the jobs being *served* -- those that issued a
-- claim within the heartbeat timeout (300 s unless HEARTBEAT_TIMEOUT_SECONDS
-- says otherwise) of the latest claim of any of them -- or, when none ever
-- has, the highest on offer: scheduler::join_at_parity's rule.
WITH others AS (
  SELECT (o.claims_issued - o.claims_baseline)::float8 / o.allocation AS ratio,
         o.last_claimed_at,
         MAX(o.last_claimed_at) OVER () AS latest
    FROM jobs o
   WHERE o.status = 'active' AND o.allocation > 0 AND o.id <> :'job'
)
UPDATE jobs j
   SET claims_baseline = j.claims_issued - floor(
         COALESCE((SELECT MIN(ratio) FROM others
                    WHERE last_claimed_at > latest - interval '300 seconds'),
                  (SELECT MAX(ratio) FROM others), 0)
         * COALESCE(j.allocation, 0))::bigint
 WHERE j.id = :'job';

COMMIT;
