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
#                 170,000 progress rows, or a few hundred staged results
#   FREE_DUMP_DIR removed first, since its contents are in the scratch database
#                 now and would share the disk (default /tmp/dump; empty: none)
#
# It refuses to start when the scratch copy and production are one database,
# when production has the job active (RUNBOOK §2.0 stops it) or completed (it
# completed again since the mistake, or §2.3 has already put it back), when
# production holds an export of the job (made since; §2.0 deletes it), when the
# scratch
# copy holds nothing of the job (a mistyped id, or SCRATCH_URL pointing at
# production), and when it already holds the audit row of the job's last purge
# or delete -- a copy taken after the mistake, which may hold rows of a job that
# went on running, or only the generation-0 artifact a leave job's purge writes
# back. (That row is written in the purge's own transaction, so its presence is
# exact; a comparison of timestamps was not.) Each would otherwise "restore"
# the wrong rows, say so, and be followed by the counter repair.
#
# A deleted job is restored whole: before its rows, whichever of the input-data
# rows, player configs, jobs row (made inactive) and config row it needs are
# missing from production. They are part of every run, not only the first, so a
# run stopped after the jobs row resumes with the rest.
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
# A purge leaves a completed job inactive, so one completed now has completed
# again since, on the results §2.0 deletes: its status and verdict are theirs.
[[ "$status" != completed ]] || { echo "stopped: the job is completed in production. If §2.3 has put it back -- it was completed before the mistake -- the restore is done: do not run §2.0 again. Otherwise it completed again since the mistake, on the results §2.0 deletes: RUNBOOK §2.0 returns it to inactive with its verdict cleared" >&2; exit 1; }
exports=$(sql "$DATABASE_URL" "SELECT count(*) FROM job_exports WHERE job_id = '$job'") \
  || { echo "stopped: cannot read the job's exports in production" >&2; exit 1; }
(( exports == 0 )) || { echo "stopped: production holds an export of the job made since the mistake, which would be served as the restored job's corpus; RUNBOOK §2.0 deletes it" >&2; exit 1; }
mistake=$(sql "$DATABASE_URL" "SELECT max(id) FROM audit_log
                                WHERE action IN ('job.purged', 'job.deleted') AND target_id = '$job'") \
  || { echo "stopped: cannot read production's audit log" >&2; exit 1; }
if [[ -n "$mistake" ]]; then
  after=$(sql "$SCRATCH_URL" "SELECT count(*) FROM audit_log WHERE id = $mistake
                               AND action IN ('job.purged', 'job.deleted') AND target_id = '$job'") \
    || { echo "stopped: cannot read the scratch copy's audit log" >&2; exit 1; }
  (( after == 0 )) || { echo "stopped: the scratch copy already holds job $job's purge or delete (audit row $mistake): it was taken after the mistake; use an earlier dump or point in time" >&2; exit 1; }
else
  echo "warning: production's audit log records no purge or delete of job $job" >&2
fi

# One run at a time: a second would empty the first's work directory under it.
mkdir -p "$work" || { echo "stopped: cannot create $work" >&2; exit 1; }
exec 9> "$work.lock"
flock -n 9 || { echo "stopped: another restore-job.sh is running on $work" >&2; exit 1; }

TASKS="SELECT id FROM tasks WHERE job_id = '$job'"
RECORDS="SELECT id FROM position_analysis_records WHERE job_id = '$job'"
MOVES="SELECT id FROM position_analysis_moves WHERE record_id IN ($RECORDS)"

# Order matters: tasks, then what hangs off a task, then claims, then what
# hangs off a claim.
PLAYERS="SELECT unnest(ARRAY[player1_config_id, player2_config_id]) FROM job_game_config WHERE job_id = '$job'
         UNION SELECT unnest(ARRAY[player1_config_id, player2_config_id]) FROM job_game_pair_config WHERE job_id = '$job'
         UNION SELECT player_config_id FROM job_opening_rack_config WHERE job_id = '$job'
         UNION SELECT player_config_id FROM job_leave_config WHERE job_id = '$job'"
INPUTS="SELECT letterdist_id FROM jobs WHERE id = '$job' UNION SELECT layout_id FROM jobs WHERE id = '$job'
        UNION SELECT unnest(ARRAY[kwg_id, klv_id, winpct_id]) FROM player_configs WHERE id IN ($PLAYERS)"

# What a deleted job needs back before its rows: what it names first, then the
# job, then its config. For a purged job all of it is there, and nothing loads.
PRELUDE=(
  "input_data|id IN ($INPUTS)"
  "player_configs|id IN ($PLAYERS)"
  "jobs|id = '$job'"
  "job_game_config|job_id = '$job'"
  "job_game_pair_config|job_id = '$job'"
  "job_opening_rack_config|job_id = '$job'"
  "job_leave_config|job_id = '$job'"
)
# Changes to a table's rows before they go in: the job comes back inactive (a
# restored job dispatching before its counters are repaired is what §2.0
# prevents), and a reference to an account deleted since is cleared.
declare -A ADJUST=(
  [input_data]="UPDATE restoring r SET imported_by = NULL WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id = r.imported_by);"
  [player_configs]="UPDATE restoring r SET created_by = NULL WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id = r.created_by);
                    UPDATE restoring r SET cloned_from_id = NULL WHERE NOT EXISTS (SELECT 1 FROM player_configs p WHERE p.id = r.cloned_from_id) AND NOT EXISTS (SELECT 1 FROM restoring c WHERE c.id = r.cloned_from_id);"
  [jobs]="UPDATE restoring SET status = 'inactive'; UPDATE restoring r SET created_by = NULL WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id = r.created_by);"
)
# Rows whose production copy may rightly differ from the dump -- shared rows
# another job may use (an object key filled in since), and the job's own row
# (its counters, reset by a purge and repaired in §2.3) -- are checked for their
# key alone.
declare -A BY_KEY_ONLY=([input_data]=1 [player_configs]=1 [jobs]=1)

TABLES=(
  "${PRELUDE[@]}"
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
  "opening_rack_progress|job_id = '$job'"
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
[[ -s "$work/jobs" ]] || { echo "stopped: the scratch copy holds no job $job -- a wrong id, or SCRATCH_URL pointing at production" >&2; exit 1; }
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
# The coprocess lets go of this script's flock (fd 9), or a failed run left it
# held by a psql still queued on the lock.
coproc HOLD { exec 9>&-; exec psql "$DATABASE_URL" -X -q -tA -v ON_ERROR_STOP=1; }
# Idle for the whole load, so never timed out for it, whatever the parameter
# group says.
echo "SET idle_session_timeout = 0; SET idle_in_transaction_session_timeout = 0;
SET lock_timeout = '60s';
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
  by_key_only=false
  [[ -z "${BY_KEY_ONLY[$table]:-}" ]] || by_key_only=true

  rm -f "$work/$table".batch.*
  # By bytes, whole lines each: a staged leave result is tens of kilobytes.
  # `split -C` breaks a line longer than its size, so a batch is at least the
  # longest line.
  longest=$(LC_ALL=C awk '{ if (length($0) > m) m = length($0) } END { print m + 1 }' "$work/$table")
  chunk=$(( batch_bytes > longest ? batch_bytes : longest ))
  split -C "$chunk" -d -a 6 "$work/$table" "$work/$table.batch." \
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
${ADJUST[$table]:-}
WITH ins AS (INSERT INTO $table SELECT * FROM restoring ON CONFLICT DO NOTHING RETURNING 1)
SELECT count(*) FROM ins;
DO \$check\$
DECLARE differing bigint;
BEGIN
  SELECT count(*) INTO differing
    FROM restoring r LEFT JOIN $table t ON ($t_key) = ($r_key)
   WHERE t.$first IS NULL OR ($by_key_only IS NOT TRUE AND row(t.*) IS DISTINCT FROM row(r.*));
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
# Its result sets are empty or one number, and say nothing a reader needs.
psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 > /dev/null <<'SQL' || { echo "stopped: could not check the sequences" >&2; exit 1; }
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
# Every rating pool refits at the next sweep, on the rows now back. A sweep
# that ran while they were loading may have fitted part of them and recorded
# the job's counter as seen -- a deleted job's row comes back with it -- and
# after that nothing would move it again (thirty-first audit). The statement
# and its locks are `ratings::mark_every_pool_for_refit`'s.
psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 > /dev/null <<'SQL' || { echo "the rows are restored, but the rating pools could not be marked for a refit: run POST /api/admin/rating-pools/:id/recompute for each pool, then repair the counters (RUNBOOK §2.3, §2.3b)" >&2; exit 1; }
BEGIN;
SELECT pg_advisory_xact_lock(2, hashtext(id::text)) FROM rating_pools ORDER BY id;
UPDATE rating_runs r SET evidence_games = NULL
  FROM rating_pools p
  CROSS JOIN LATERAL (SELECT x.id FROM rating_runs x WHERE x.pool_id = p.id
                      ORDER BY x.computed_at DESC, x.id DESC LIMIT 1) newest
 WHERE r.id = newest.id AND r.evidence_games IS NOT NULL;
COMMIT;
SQL
echo "restored; now repair the counters (RUNBOOK §2.3, §2.3b)"
