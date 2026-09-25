#!/usr/bin/env bash
# RUNBOOK.md §2.2: copy one job's rows from a scratch copy of the database back
# into production, after a mistaken purge or delete.
#
#   restore-job.sh <job-id>
#
# Run in the ops shell (scripts/prod-shell.sh), which writes this file to
# /tmp/restore-job.sh when its task starts, after §2.0 (the job stopped and
# what it did since the purge cleared) and §2.1 (the scratch copy restored):
#
#   SCRATCH_URL   the scratch copy; read from /tmp/restore.env if unset
#   DATABASE_URL  production, as the ops task has it
#   PG_RESTORE_LOG  the scratch restore's log (default /tmp/pg_restore.log),
#                 which must end in `pg_restore exit 0`: with -j4 each table
#                 commits on its own, and a copy taken mid-restore finds some of
#                 the job's tables empty and reports success
#   COPYBACK_DUMP_ONLY=1  dump the job's rows and print their sizes, load nothing
#   WORK_DIR      where the job's rows are dumped (default /tmp/restore)
#   BATCH_BYTES   the most text loaded per transaction (default 16 MiB): about
#                 400,000 progress rows, or a few hundred staged results
#   FREE_DUMP_DIR removed first, since its contents are in the scratch database
#                 now and would share the disk (default /tmp/dump; empty: none)
#
# It refuses to start when the scratch copy and production are one database,
# when production has the job active (RUNBOOK §2.0 stops it), or when the
# scratch copy holds nothing of the job -- a mistyped id, SCRATCH_URL pointing
# at production, or a copy taken after the mistake, each of which would
# otherwise "restore" nothing, say so, and be followed by the counter repair.
# While it loads it holds the job's merge lock, so the half-hourly merge cannot
# fold restored staged rows into restored progress rows mid-run.
#
# Each table's rows are loaded a batch at a time, through a temporary table and
# `INSERT ... ON CONFLICT DO NOTHING`. One statement per table, as the RUNBOOK
# first had it, queued a foreign-key check per row in one backend's memory,
# some 12 bytes a row: 780 MB for a 20-generation leave job's progress rows,
# more than a db.t4g.micro has (thirty-first audit). A batch is also a
# transaction of its own, so a run that stops part-way is resumed by running
# it again.
#
# Every batch checks that each of its rows is now in production exactly as it
# was dumped. A row that was already there with the same key and different
# contents -- a generation re-seeded, a task regenerated, because §2.0 was not
# done -- makes ON CONFLICT DO NOTHING drop the restored row silently; here it
# stops the run instead, with nothing of that batch loaded. A row already there
# identically is a re-run, and is fine. (A leave job's staged rows that a
# merge folded in between two runs also read as other contents: after a stop,
# a leave job may need §2.0 and a run from the start.)
set -uo pipefail

job=${1:-}
[[ "$job" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$ ]] \
  || { echo "usage: $0 <job-id>" >&2; exit 2; }
# As Postgres prints a uuid: the merge lock is keyed on that text.
job=${job,,}

if [[ -z "${SCRATCH_URL:-}" && -f /tmp/restore.env ]]; then
  # shellcheck disable=SC1091
  source /tmp/restore.env
fi
[[ -n "${SCRATCH_URL:-}" ]] || { echo "SCRATCH_URL is not set, and /tmp/restore.env does not set it (§2.1)" >&2; exit 2; }
[[ -n "${DATABASE_URL:-}" ]] || { echo "DATABASE_URL is not set" >&2; exit 2; }

log=${PG_RESTORE_LOG:-/tmp/pg_restore.log}
[[ -f "$log" ]] && [[ "$(tail -n 1 "$log")" == "pg_restore exit 0" ]] \
  || { echo "the scratch restore has not finished, or failed: $log does not end in 'pg_restore exit 0'" >&2; exit 1; }

free=${FREE_DUMP_DIR-/tmp/dump}
[[ -n "$free" ]] && rm -rf "$free"

work=${WORK_DIR:-/tmp/restore}
batch_bytes=${BATCH_BYTES:-16777216}
[[ "$batch_bytes" =~ ^[1-9][0-9]*$ ]] || { echo "BATCH_BYTES must be a positive number" >&2; exit 2; }

sql() { psql "$1" -X -tA -v ON_ERROR_STOP=1 -c "$2"; }
# The same server started at the same moment, and the same database on it.
fingerprint="SELECT pg_postmaster_start_time() || '/' || current_database()"
here=$(sql "$DATABASE_URL" "$fingerprint") || { echo "stopped: cannot reach DATABASE_URL" >&2; exit 1; }
there=$(sql "$SCRATCH_URL" "$fingerprint") || { echo "stopped: cannot reach SCRATCH_URL" >&2; exit 1; }
[[ "$here" != "$there" ]] || { echo "stopped: SCRATCH_URL is production itself" >&2; exit 1; }
status=$(sql "$DATABASE_URL" "SELECT status FROM jobs WHERE id = '$job'") \
  || { echo "stopped: cannot read the job in production" >&2; exit 1; }
[[ "$status" != active ]] || { echo "stopped: the job is active in production; deactivate it and clear it first (RUNBOOK §2.0)" >&2; exit 1; }

# One run at a time: a second would empty the first's work directory under it.
mkdir -p "$work" || { echo "stopped: cannot create $work" >&2; exit 1; }
exec 9> "$work.lock"
flock -n 9 || { echo "stopped: another restore-job.sh is running on $work" >&2; exit 1; }

TASKS="SELECT id FROM tasks WHERE job_id = '$job'"
RECORDS="SELECT id FROM position_analysis_records WHERE job_id = '$job'"
MOVES="SELECT id FROM position_analysis_moves WHERE record_id IN ($RECORDS)"

# Order matters: tasks, then what hangs off a task, then claims, then what
# hangs off a claim.
TABLES=(
  "tasks|job_id = '$job'"
  "opening_rack_requests|task_id IN ($TASKS)"
  "game_requests|task_id IN ($TASKS)"
  "leave_requests|task_id IN ($TASKS)"
  "task_claims|task_id IN ($TASKS)"
  "worker_data_gaps|job_id = '$job'"
  "game_results|job_id = '$job'"
  "leave_records|task_id IN ($TASKS)"
  "position_analysis_records|job_id = '$job'"
  "position_analysis_moves|record_id IN ($RECORDS)"
  "position_analysis_plies|move_id IN ($MOVES)"
  "leave_rack_progress|job_id = '$job'"
  "leave_rack_staging|job_id = '$job'"
  "leave_generation_progress|job_id = '$job'"
  "leave_selection_cursors|job_id = '$job'"
  "leave_generation_artifacts|job_id = '$job'"
  "leave_generation_transitions|job_id = '$job'"
)

rm -rf "${work:?}"/*
for entry in "${TABLES[@]}"; do
  table=${entry%%|*} filter=${entry#*|}
  psql "$SCRATCH_URL" -X -q -v ON_ERROR_STOP=1 -c \
    "COPY (SELECT * FROM $table WHERE $filter) TO STDOUT" > "$work/$table" \
    || { echo "stopped: could not dump $table from the scratch copy" >&2; exit 1; }
done
du -ch "$work"/* | sort -h | tail -6   # the job's own rows, largest last
total=$(cat "$work"/* | wc -l)
(( total > 0 )) || { echo "stopped: the scratch copy holds no row of job $job -- a wrong id, SCRATCH_URL pointing at production, or a copy taken after the mistake" >&2; exit 1; }
if [[ "${COPYBACK_DUMP_ONLY:-}" == 1 ]]; then
  echo "dumped only; compare the sizes above with FreeStorageSpace (RUNBOOK §2.2)"
  exit 0
fi

# The job's merge lock, held by a session of its own until this script ends:
# the sweep's merge skips a job whose lock is taken. (An admin's "merge
# progress now" waits for it.)
# It ends, and the lock with it, when this script does and its input closes:
# nothing signals it (inside the ops container, a pid is as likely a server
# process's as psql's).
coproc HOLD { psql "$DATABASE_URL" -X -q -tA -v ON_ERROR_STOP=1; }
# Idle for the whole load, so never timed out for it, whatever the parameter
# group says.
echo "SET idle_session_timeout = 0; SET idle_in_transaction_session_timeout = 0;
SELECT 'held' FROM (SELECT pg_advisory_lock(3, hashtext('$job'))) taken;" >&"${HOLD[1]}"
IFS= read -r -t 60 held <&"${HOLD[0]}"
[[ "$held" == held ]] || { echo "stopped: could not take the job's merge lock" >&2; exit 1; }

for entry in "${TABLES[@]}"; do
  table=${entry%%|*}
  [[ -f "$work/$table" ]] || { echo "stopped: $work/$table has gone" >&2; exit 1; }
  rows=$(wc -l < "$work/$table")
  if (( rows == 0 )); then
    echo "$table: nothing to restore"
    continue
  fi
  key=$(psql "$DATABASE_URL" -X -tA -v ON_ERROR_STOP=1 -c \
    "SELECT string_agg(quote_ident(a.attname), ',' ORDER BY k.n)
       FROM pg_index i
       CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, n)
       JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
      WHERE i.indrelid = '$table'::regclass AND i.indisprimary") \
    || { echo "stopped: could not read $table's primary key" >&2; exit 1; }
  [[ -n "$key" ]] || { echo "stopped: $table has no primary key to check its rows by" >&2; exit 1; }
  t_key="t.${key//,/, t.}" r_key="r.${key//,/, r.}" first=${key%%,*}

  rm -f "$work/$table".batch.*
  # By bytes, whole lines each: a staged leave result is tens of kilobytes.
  split -C "$batch_bytes" -d -a 6 "$work/$table" "$work/$table.batch." \
    || { echo "stopped: could not split $table" >&2; exit 1; }
  rm -f "$work/$table"   # the batches hold it now, on the same disk
  inserted=0
  for batch in "$work/$table".batch.*; do
    out=$(psql "$DATABASE_URL" -X -q -tA -v ON_ERROR_STOP=1 <<SQL
BEGIN;
-- The check below probes the key's index per row: a hash join read the whole
-- table for every batch, into temporary files on the production volume.
SET LOCAL enable_hashjoin = off;
SET LOCAL enable_mergejoin = off;
CREATE TEMP TABLE restoring (LIKE $table) ON COMMIT DROP;
\copy restoring FROM '$batch'
WITH ins AS (INSERT INTO $table SELECT * FROM restoring ON CONFLICT DO NOTHING RETURNING 1)
SELECT count(*) FROM ins;
DO \$check\$
DECLARE differing bigint;
BEGIN
  SELECT count(*) INTO differing
    FROM restoring r LEFT JOIN $table t ON ($t_key) = ($r_key)
   WHERE t.$first IS NULL OR row(t.*) IS DISTINCT FROM row(r.*);
  IF differing > 0 THEN
    RAISE EXCEPTION '% of these rows are not in $table as dumped: another row holds their key, or one of their unique values', differing
      USING HINT = 'the job wrote rows since the purge (do RUNBOOK §2.0, then run this again); or, for a leave job run again after a stop, a merge folded its staged rows in since (do §2.0 and run it from the start)';
  END IF;
END
\$check\$;
COMMIT;
SQL
    )
    status=$?
    if (( status != 0 )); then
      # Every later table hangs off this one: carrying on buried the first
      # error under a foreign-key failure per table.
      echo "stopped: could not load $table (nothing of the failed batch was loaded; earlier ones were)" >&2
      exit 1
    fi
    inserted=$(( inserted + out ))
  done
  rm -f "$work/$table".batch.*
  echo "$table: $rows rows, $inserted loaded now, $(( rows - inserted )) already there as dumped"
done

# The BIGSERIAL ids came back as they were, from these sequences, which a
# selective restore does not rewind; each statement moves a sequence only
# forward, and only when a table is ahead of it. A plain setval(max(id)) could
# move it back below ids the fleet took since, and the next insert collided.
psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 <<'SQL' || { echo "stopped: could not check the sequences" >&2; exit 1; }
SELECT setval('position_analysis_records_id_seq', m)
  FROM (SELECT max(id) AS m FROM position_analysis_records) x
 WHERE m > (SELECT last_value FROM position_analysis_records_id_seq);
SELECT setval('position_analysis_moves_id_seq', m)
  FROM (SELECT max(id) AS m FROM position_analysis_moves) x
 WHERE m > (SELECT last_value FROM position_analysis_moves_id_seq);
SELECT setval('leave_rack_staging_id_seq', m)
  FROM (SELECT max(id) AS m FROM leave_rack_staging) x
 WHERE m > (SELECT last_value FROM leave_rack_staging_id_seq);
SQL
echo "restored; now repair the counters (RUNBOOK §2.3, §2.3b)"
