#!/usr/bin/env bash
#
# Prove scripts/restore-job.sh (RUNBOOK.md §2.2) against a real Postgres: two
# jobs' rows are purged from one database and copied back from a copy of it
# taken before, through its failure and re-run cases. Run it against the local
# docker compose stack:
#
#   docker compose up -d postgres
#   ./scripts/restore-job-check.sh
#
# or any Postgres 16 container, with PG_EXEC="docker exec -i <container>". It
# runs entirely inside the container, as the ops shell runs the script, and
# works in two databases of its own, which it drops afterwards.
#
# The cases:
#   - it refuses to start before the scratch restore has finished, against a
#     scratch copy that is production itself, for a job production has active,
#     for a job the scratch copy holds nothing of, and against a copy taken
#     after the purge from a job that went on running;
#   - a deleted job comes back whole: its jobs row (inactive), its config, and
#     the player config and input data deleted with it;
#   - COPYBACK_DUMP_ONLY loads nothing;
#   - a row production holds under a restored row's key with other contents --
#     a generation re-seeded because §2.0 was skipped -- stops it, with that
#     batch not loaded and another job untouched;
#   - after §2.0, the same run finishes what the stopped one began;
#   - a job loaded in several batches comes back row for row, and loading it
#     again changes nothing;
#   - it holds the job's merge lock while it loads;
#   - position analyses come back through their parents' ids, and the
#     BIGSERIAL sequences are moved forward past the restored ids.
set -Eeuo pipefail

EXEC="${PG_EXEC:-docker compose exec -T postgres}"
PGUSER_="${PGUSER_:-birdtest}"
PROD=restorejob_prod
SCRATCH=restorejob_scratch
LATE=restorejob_late
HERE="$(cd "$(dirname "$0")" && pwd)"

psql_() { ${EXEC} psql -U "${PGUSER_}" -X -q -v ON_ERROR_STOP=1 "$@"; }
val() { ${EXEC} psql -U "${PGUSER_}" -X -tA -v ON_ERROR_STOP=1 -d "$1" -c "$2"; }

cleanup() {
  local status=$?
  for db in "$PROD" "$SCRATCH" "$LATE"; do
    psql_ -d postgres -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null 2>&1 || true
  done
  ${EXEC} rm -rf /tmp/restore-job-check /tmp/restore-job-check.lock /tmp/restore-job-check.log >/dev/null 2>&1 || true
  if (( status == 0 )); then echo "restore-job check passed"; else echo "restore-job check FAILED" >&2; fi
  exit "$status"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Runs restore-job.sh in the container for job $1, with any further
# VAR=value arguments in its environment; prints its output, returns its
# status.
restore() {
  local job=$1; shift
  ${EXEC} env \
    SCRATCH_URL="postgresql:///$SCRATCH?user=$PGUSER_" \
    DATABASE_URL="postgresql:///$PROD?user=$PGUSER_" \
    PG_RESTORE_LOG=/tmp/restore-job-check.log \
    WORK_DIR=/tmp/restore-job-check FREE_DUMP_DIR= BATCH_BYTES=16000 "$@" \
    bash -s -- "$job" < "$HERE/restore-job.sh" 2>&1
}

TABLES_OF_A_JOB=(tasks task_claims game_results position_analysis_records leave_rack_progress
                 leave_rack_staging leave_generation_progress leave_selection_cursors
                 leave_generation_artifacts)
# Every row of job $2 in database $1, per table, as one digest.
digest() {
  local db=$1 job=$2 out="" table
  for table in "${TABLES_OF_A_JOB[@]}"; do
    out+="$table:$(val "$db" "SELECT count(*) || '/' || coalesce(md5(string_agg(x::text, '|' ORDER BY x::text)), '-')
                             FROM $table x WHERE job_id = '$job'") "
  done
  out+="moves:$(val "$db" "SELECT count(*) || '/' || coalesce(md5(string_agg(m::text, '|' ORDER BY m::text)), '-')
                          FROM position_analysis_moves m JOIN position_analysis_records r ON r.id = m.record_id
                          WHERE r.job_id = '$job'") "
  out+="plies:$(val "$db" "SELECT count(*) || '/' || coalesce(md5(string_agg(p::text, '|' ORDER BY p::text)), '-')
                          FROM position_analysis_plies p JOIN position_analysis_moves m ON m.id = p.move_id
                          JOIN position_analysis_records r ON r.id = m.record_id WHERE r.job_id = '$job'")"
  echo "$out"
}

for db in "$PROD" "$SCRATCH" "$LATE"; do
  psql_ -d postgres -c "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null
done
psql_ -d postgres -c "CREATE DATABASE $PROD" >/dev/null
psql_ -d "$PROD" < "$HERE/../backend/migrations/0001_initial.sql" >/dev/null

# Four jobs: a games job with 500 tasks (five batches of a hundred), a leave
# job in its tail, a third that is never purged, and a fourth, with a player
# config of its own, that is deleted.
psql_ -d "$PROD" <<'SQL'
BEGIN;
INSERT INTO users (username, email, password_hash, is_admin)
VALUES ('restorer', 'restorer@example.invalid', 'x', true);
INSERT INTO input_data (path, role, name, sha256, bytes, tarball_date, content)
VALUES ('letterdistributions/check.csv', 'letterdist', 'check', repeat('a', 64), 3, '20260101', '\x00ff00'::bytea),
       ('layouts/check.txt', 'layout', 'check', repeat('b', 64), 3, '20260101', '\x010203'::bytea);
INSERT INTO jobs (id, job_type, status, allocation, redundancy, variant,
                  letterdist_id, layout_id, created_by, bingo_bonus, sim_cutoff)
SELECT j.id::uuid, j.kind::job_type, 'inactive', 100, 1, 'classic',
       (SELECT id FROM input_data WHERE role = 'letterdist'),
       (SELECT id FROM input_data WHERE role = 'layout'),
       (SELECT id FROM users), 50, 0.1
FROM (VALUES ('00000000-0000-0000-0000-00000000000a', 'games'),
             ('00000000-0000-0000-0000-00000000000b', 'leave_generation'),
             ('00000000-0000-0000-0000-00000000000c', 'games')) j(id, kind);
INSERT INTO tasks (job_id, seed, state, accepted_count)
SELECT j::uuid, s, 'completed', 1
FROM unnest(ARRAY['00000000-0000-0000-0000-00000000000a', '00000000-0000-0000-0000-00000000000c']) j,
     generate_series(1, 500) s;
INSERT INTO task_claims (task_id, job_id, claim_token, state, claimed_by_user_id, completed_at)
SELECT t.id, t.job_id, gen_random_uuid(), 'completed', (SELECT id FROM users), now() FROM tasks t;
INSERT INTO game_results (task_claim_id, task_id, job_id, games, wins, losses, ties,
                          p1_score_mean, p1_score_sd, p2_score_mean, p2_score_sd)
SELECT c.id, c.task_id, c.job_id, 10, 6, 4, 0, 412.5, 55.25, 398.0, 61.5 FROM task_claims c;
INSERT INTO position_analysis_records (task_claim_id, task_id, job_id, rack, num_moves)
SELECT c.id, c.task_id, c.job_id, 'AEINRST', 2 FROM task_claims c WHERE c.job_id = '00000000-0000-0000-0000-00000000000a' LIMIT 20;
INSERT INTO position_analysis_moves (record_id, rank, move, score, equity)
SELECT r.id, k, 'H8 RETAINS', 70 + k, 40.5 + k FROM position_analysis_records r, generate_series(1, 3) k;
INSERT INTO position_analysis_plies (move_id, ply, bingo_percentage, average_score)
SELECT m.id, p, 12.5, 30.25 FROM position_analysis_moves m, generate_series(1, 2) p;
INSERT INTO leave_rack_progress (job_id, generation, rack, occurrence_count)
SELECT '00000000-0000-0000-0000-00000000000b', 1, 'R' || lpad(i::text, 5, '0'), i % 7
FROM generate_series(1, 250) i;
INSERT INTO leave_generation_progress (job_id, generation) VALUES ('00000000-0000-0000-0000-00000000000b', 1);
INSERT INTO leave_selection_cursors (job_id, generation, cursor_rack)
VALUES ('00000000-0000-0000-0000-00000000000b', 1, NULL);
INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256, builder)
VALUES ('00000000-0000-0000-0000-00000000000b', 0, 'leaves/check/generation-0.klv2', repeat('0', 64), 'klv-1');
INSERT INTO leave_rack_staging (job_id, generation, task_id, racks, counts, equity_sums)
SELECT '00000000-0000-0000-0000-00000000000b', 1, gen_random_uuid(), ARRAY['R00001'], ARRAY[3::bigint], ARRAY[1.5]
FROM generate_series(1, 3);
INSERT INTO input_data (id, path, role, name, sha256, bytes, tarball_date)
VALUES ('00000000-0000-0000-0000-0000000000e1', 'lexica/CHECK.kwg', 'kwg', 'CHECK', repeat('e', 64), 3, '20260101'),
       ('00000000-0000-0000-0000-0000000000e2', 'lexica/CHECK.klv2', 'klv', 'CHECK', repeat('f', 64), 3, '20260101');
INSERT INTO player_configs (id, name, recorder_type, sort_strategy, kwg_id, klv_id, num_plies, num_plays,
                            num_plies_recorded, num_plays_recorded, use_wordmap, use_rit, movegen_margin, created_by)
VALUES ('00000000-0000-0000-0000-0000000000f1', 'check-static', 'best', 'equity',
        '00000000-0000-0000-0000-0000000000e1', '00000000-0000-0000-0000-0000000000e2', 0, 100, 2, 10,
        false, false, 5, (SELECT id FROM users));
INSERT INTO jobs (id, job_type, status, allocation, redundancy, variant,
                  letterdist_id, layout_id, created_by, bingo_bonus, sim_cutoff)
SELECT '00000000-0000-0000-0000-00000000000d', 'games', 'active', 100, 1, 'classic',
       (SELECT id FROM input_data WHERE role = 'letterdist'), (SELECT id FROM input_data WHERE role = 'layout'),
       (SELECT id FROM users), 50, 0.1;
INSERT INTO job_game_config (job_id, player1_config_id, player2_config_id, games_per_batch, min_games, max_games)
VALUES ('00000000-0000-0000-0000-00000000000d', '00000000-0000-0000-0000-0000000000f1',
        '00000000-0000-0000-0000-0000000000f1', 10, 100, 1000);
INSERT INTO tasks (job_id, seed, state, accepted_count)
SELECT '00000000-0000-0000-0000-00000000000d', s, 'completed', 1 FROM generate_series(1, 50) s;
INSERT INTO task_claims (task_id, job_id, claim_token, state, claimed_by_user_id, completed_at)
SELECT t.id, t.job_id, gen_random_uuid(), 'completed', (SELECT id FROM users), now()
FROM tasks t WHERE t.job_id = '00000000-0000-0000-0000-00000000000d';
COMMIT;
SQL

GAMES=00000000-0000-0000-0000-00000000000a
LEAVE=00000000-0000-0000-0000-00000000000b
OTHER=00000000-0000-0000-0000-00000000000c
DELETED=00000000-0000-0000-0000-00000000000d
psql_ -d postgres -c "CREATE DATABASE $SCRATCH TEMPLATE $PROD" >/dev/null
want_games=$(digest "$SCRATCH" "$GAMES")
want_leave=$(digest "$SCRATCH" "$LEAVE")
want_other=$(digest "$PROD" "$OTHER")
want_deleted=$(digest "$SCRATCH" "$DELETED")
want_deleted_job=$(val "$SCRATCH" "SELECT j::text FROM (SELECT id, job_type, allocation, letterdist_id, layout_id, created_by FROM jobs WHERE id = '$DELETED') j")

# The purge, for both jobs, and a sequence behind the ids it removed.
psql_ -d "$PROD" <<SQL
BEGIN;
DELETE FROM tasks WHERE job_id IN ('$GAMES', '$LEAVE');
DELETE FROM leave_rack_progress WHERE job_id = '$LEAVE';
DELETE FROM leave_rack_staging WHERE job_id = '$LEAVE';
DELETE FROM leave_generation_progress WHERE job_id = '$LEAVE';
DELETE FROM leave_selection_cursors WHERE job_id = '$LEAVE';
DELETE FROM leave_generation_artifacts WHERE job_id = '$LEAVE';
SELECT setval('leave_rack_staging_id_seq', 1);
SELECT setval('position_analysis_records_id_seq', 1);
SELECT setval('position_analysis_moves_id_seq', 1);
INSERT INTO audit_log (action, target_type, target_id, job_id)
VALUES ('job.purged', 'job', '$GAMES', '$GAMES'), ('job.purged', 'job', '$LEAVE', '$LEAVE'),
       ('job.deleted', 'job', '$DELETED', NULL);
DELETE FROM jobs WHERE id = '$DELETED';
DELETE FROM player_configs WHERE id = '00000000-0000-0000-0000-0000000000f1';
DELETE FROM input_data WHERE id = '00000000-0000-0000-0000-0000000000e2';
COMMIT;
SQL
# A copy taken after the purge: of a job that went on dispatching, and of a
# leave job whose purge wrote its generation-0 artifact back and nothing else.
psql_ -d postgres -c "CREATE DATABASE $LATE TEMPLATE $PROD" >/dev/null
val "$LATE" "INSERT INTO tasks (job_id, seed, state) VALUES ('$GAMES', 9001, 'available')" >/dev/null
val "$LATE" "INSERT INTO leave_generation_artifacts (job_id, generation, artifact_key, sha256, builder)
             VALUES ('$LEAVE', 0, 'leaves/check/generation-0.klv2', repeat('0', 64), 'klv-1')" >/dev/null
empty=$(digest "$PROD" "$GAMES")

echo "-- refuses before the scratch restore has finished"
${EXEC} rm -f /tmp/restore-job-check.log
if out=$(restore "$GAMES"); then fail "ran with no restore log: $out"; fi
[[ "$out" == *"has not finished"* ]] || fail "no reason given: $out"
${EXEC} sh -c 'printf "restoring\npg_restore exit 0\n" > /tmp/restore-job-check.log'

echo "-- refuses production as its scratch copy, an active job, and a job it holds nothing of"
if out=$(restore "$GAMES" SCRATCH_URL="postgresql:///$PROD?user=$PGUSER_"); then fail "restored from production: $out"; fi
[[ "$out" == *"SCRATCH_URL is production itself"* ]] || fail "not told why: $out"
val "$PROD" "UPDATE jobs SET status = 'active' WHERE id = '$GAMES'" >/dev/null
if out=$(restore "$GAMES"); then fail "restored into an active job: $out"; fi
[[ "$out" == *"the job is active in production"* ]] || fail "not told why: $out"
val "$PROD" "UPDATE jobs SET status = 'inactive' WHERE id = '$GAMES'" >/dev/null
if out=$(restore 00000000-0000-0000-0000-0000000000ff); then fail "restored nothing and said so: $out"; fi
[[ "$out" == *"holds no job"* ]] || fail "not told why: $out"
for purged in "$GAMES" "$LEAVE"; do
  if out=$(restore "$purged" SCRATCH_URL="postgresql:///$LATE?user=$PGUSER_"); then fail "restored from a copy taken after the purge: $out"; fi
  [[ "$out" == *"it was taken after the mistake"* ]] || fail "not told why: $out"
done

echo "-- dump only loads nothing"
out=$(restore "$GAMES" COPYBACK_DUMP_ONLY=1) || fail "dump only failed: $out"
[[ "$(digest "$PROD" "$GAMES")" == "$empty" ]] || fail "dump only loaded rows"

echo "-- a row re-seeded since the purge stops the run"
val "$PROD" "INSERT INTO leave_rack_progress (job_id, generation, rack, occurrence_count)
             VALUES ('$LEAVE', 1, 'R00100', 0)" >/dev/null
if out=$(restore "$LEAVE"); then fail "loaded over a re-seeded row: $out"; fi
[[ "$out" == *"are not in leave_rack_progress as dumped"* ]] || fail "not told why: $out"
[[ "$(val "$PROD" "SELECT count(*) FROM leave_rack_progress WHERE job_id = '$LEAVE'")" == 1 ]] \
  || fail "the stopped batch was loaded"
[[ "$(digest "$PROD" "$OTHER")" == "$want_other" ]] || fail "another job was touched"

echo "-- after §2.0 the same run finishes, holding the job's merge lock while it loads"
val "$PROD" "DELETE FROM leave_rack_progress WHERE job_id = '$LEAVE'" >/dev/null
# Batches smaller than a line: each is one line, not a line broken in two.
restore "$LEAVE" BATCH_BYTES=20 > /tmp/restore-job-check.out &
running=$!
held=
for _ in $(seq 100); do
  if [[ "$(val "$PROD" "SELECT count(*) FROM leave_rack_progress WHERE job_id = '$LEAVE'")" -gt 0 ]]; then
    held=$(val "$PROD" "SELECT NOT pg_try_advisory_lock(3, hashtext('$LEAVE'))")
    break
  fi
  sleep 0.1
done
wait "$running" || fail "the re-run failed: $(cat /tmp/restore-job-check.out)"
out=$(cat /tmp/restore-job-check.out); rm -f /tmp/restore-job-check.out
[[ "$held" == t ]] || fail "a merge could have run mid-load (held: '$held')"
[[ "$(digest "$PROD" "$LEAVE")" == "$want_leave" ]] || fail "the leave job came back different: $out"

echo "-- five batches come back row for row, and again changes nothing"
out=$(restore "$GAMES") || fail "the restore failed: $out"
[[ "$out" == *"tasks: 500 rows, 500 loaded now"* ]] || fail "unexpected summary: $out"
[[ "$(digest "$PROD" "$GAMES")" == "$want_games" ]] || fail "the games job came back different"
out=$(restore "$GAMES") || fail "running it again failed: $out"
[[ "$out" == *"tasks: 500 rows, 0 loaded now, 500 already there as dumped"* ]] || fail "unexpected re-run: $out"
[[ "$(digest "$PROD" "$GAMES")" == "$want_games" ]] || fail "running it again changed the job"
[[ "$(digest "$PROD" "$OTHER")" == "$want_other" ]] || fail "another job was touched"

echo "-- a deleted job comes back whole, inactive, and a stopped one resumes"
# The first run stops once the jobs row is in: the config is refused.
val "$PROD" "CREATE FUNCTION refuse() RETURNS trigger LANGUAGE plpgsql AS \$\$
             BEGIN RAISE EXCEPTION 'refused'; END \$\$;
             CREATE TRIGGER refuse BEFORE INSERT ON job_game_config FOR EACH ROW EXECUTE FUNCTION refuse()" >/dev/null
if out=$(restore "$DELETED"); then fail "the refused config did not stop the run: $out"; fi
[[ "$(val "$PROD" "SELECT count(*) FROM jobs WHERE id = '$DELETED'")" == 1 ]] || fail "the jobs row was not in before the stop"
val "$PROD" "DROP TRIGGER refuse ON job_game_config; DROP FUNCTION refuse()" >/dev/null
out=$(restore "$DELETED") || fail "the deleted job's restore failed: $(tail -5 <<<"$out")"
[[ "$(digest "$PROD" "$DELETED")" == "$want_deleted" ]] || fail "the deleted job's rows came back different: $out"
[[ "$(val "$PROD" "SELECT j::text FROM (SELECT id, job_type, allocation, letterdist_id, layout_id, created_by FROM jobs WHERE id = '$DELETED') j")" == "$want_deleted_job" ]] \
  || fail "its jobs row came back different"
[[ "$(val "$PROD" "SELECT status FROM jobs WHERE id = '$DELETED'")" == inactive ]] || fail "it came back dispatching"
[[ "$(val "$PROD" "SELECT count(*) FROM job_game_config c JOIN player_configs p ON p.id = c.player1_config_id
                    JOIN input_data k ON k.id = p.klv_id WHERE c.job_id = '$DELETED'")" == 1 ]] \
  || fail "its config, player config or input data did not come back"

echo "-- the sequences are past the restored ids"
for table in leave_rack_staging position_analysis_records position_analysis_moves; do
  [[ "$(val "$PROD" "SELECT last_value >= (SELECT max(id) FROM $table) FROM ${table}_id_seq")" == t ]] \
    || fail "${table}_id_seq is behind its table"
done
