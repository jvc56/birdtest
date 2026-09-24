#!/usr/bin/env bash
#
# Runs scripts/backup.sh and scripts/restore-drill.sh the way production does
# -- each on the official Postgres image, as `bash -c "<the script>"`, which is
# the Fargate tasks' entrypoint and command -- against the local compose
# stack, and checks what they leave behind:
#
#   docker compose up -d --wait postgres minio minio-init
#   (the schema applied to the database: the backend applies it on start, or
#    psql < backend/migrations/0001_initial.sql)
#   ./scripts/backup-drill-check.sh
#
# 1. A backup taken while another session keeps writing (audit_log rows, as
#    claims and admin actions do in production) succeeds and records an ok=true
#    `backups` row whose sha256 and row counts are the manifest's.
# 2. The restore drill of that backup passes: the restored row counts equal
#    the manifest's even though the database moved on while it was dumped.
#    The counts were once taken after pg_dump finished, outside its snapshot,
#    so any write during the dump failed every drill.
# 3. A backup that cannot upload exits non-zero and leaves an ok=false row.
#
# Everything it creates -- objects under its own prefix, the rows it wrote, the
# drill's database, its tools container -- is removed again, pass or fail. No
# AWS endpoint is contacted: S3 is the stack's MinIO, and CloudWatch metrics
# are off (BACKUP_METRICS=false).

set -Eeuo pipefail

COMPOSE="${COMPOSE:-docker compose}"
IMAGE="${BACKUP_CHECK_IMAGE:-postgres:16}"
BUCKET="${BACKUP_BUCKET:-birdtest-backups}"
# Each step's wall-clock bound. The first backup installs the AWS CLI.
STEP_TIMEOUT="${STEP_TIMEOUT:-600}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

RUN_ID="check-$(date -u +%Y%m%d%H%M%S)-${RANDOM}"
PREFIX="backup-check/${RUN_ID}"
BAD_PREFIX="backup-check/${RUN_ID}-bad"
DRILL_DB="birdtest_drill_${RUN_ID//-/_}"
TOOLS="birdtest-backup-check-${RUN_ID}"
WRITER_APP="backup-check-writer-${RUN_ID}"
DATABASE_URL="postgres://birdtest:birdtest@postgres:5432/birdtest"
S3="aws --endpoint-url http://minio:9000"

log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

tools() { docker exec "$@"; }
sql() {
  tools "${TOOLS}" psql "${DATABASE_URL}" --no-psqlrc -tA -q -v ON_ERROR_STOP=1 -c "$1"
}
# One of the two scripts, exactly as its Fargate task runs it.
run_script() {
  local script="$1"; shift
  local env_args=()
  for pair in "$@"; do env_args+=(-e "${pair}"); done
  timeout "${STEP_TIMEOUT}" docker exec "${env_args[@]}" "${TOOLS}" \
    bash -c "exec bash -c \"\$(cat /scripts/${script})\""
}

writer_pid=""
created_migrations=""
stop_writer() {
  sql "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
       WHERE application_name = '${WRITER_APP}'" >/dev/null 2>&1 || true
  if [[ -n "${writer_pid}" ]]; then wait "${writer_pid}" 2>/dev/null || true; fi
  writer_pid=""
}

cleanup() {
  local status=$?
  stop_writer
  if docker inspect "${TOOLS}" >/dev/null 2>&1; then
    tools "${TOOLS}" bash -c "${S3} s3 rm --recursive --only-show-errors s3://${BUCKET}/${PREFIX}/" \
      >/dev/null 2>&1 || log "could not remove s3://${BUCKET}/${PREFIX}/"
    sql "DELETE FROM backups WHERE s3_key LIKE 'backup-check/${RUN_ID}%';
         DELETE FROM audit_log WHERE action = 'backup_check.write' AND reason = '${RUN_ID}'" \
      >/dev/null 2>&1 || log "could not delete this check's rows"
    if [[ -n "${created_migrations}" ]]; then
      sql "DROP TABLE IF EXISTS _sqlx_migrations" >/dev/null 2>&1 || true
    fi
    tools "${TOOLS}" psql "${DATABASE_URL%/*}/postgres" --no-psqlrc -q \
      -c "DROP DATABASE IF EXISTS \"${DRILL_DB}\" WITH (FORCE)" >/dev/null 2>&1 || true
    # -v: the postgres image declares a data VOLUME, which an explicit rm of
    # even a --rm container leaves behind unless asked.
    docker rm -f -v "${TOOLS}" >/dev/null 2>&1 || true
  fi
  if (( status == 0 )); then log "backup drill check passed"; else log "backup drill check FAILED"; fi
  exit "${status}"
}
trap cleanup EXIT

# --- The stack ---------------------------------------------------------------
pg_container="$(${COMPOSE} ps -q postgres)"
[[ -n "${pg_container}" ]] || fail "no postgres service -- run: ${COMPOSE} up -d --wait postgres minio minio-init"
network="$(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' "${pg_container}" | awk '{print $1}')"
log "stack network ${network}; run ${RUN_ID}"

# A container of the production image to run everything from: the scripts,
# the concurrent writer, and the checks. The first backup installs the AWS
# CLI into it, as every production run does into its own.
docker run -d --rm --name "${TOOLS}" --network "${network}" --memory 1g \
  -e DATABASE_URL="${DATABASE_URL}" \
  -e BACKUP_BUCKET="${BUCKET}" \
  -e AWS_S3_ENDPOINT=http://minio:9000 \
  -e AWS_ACCESS_KEY_ID=birdtest \
  -e AWS_SECRET_ACCESS_KEY=birdtestbirdtest \
  -e AWS_DEFAULT_REGION=us-east-1 \
  -e BACKUP_METRICS=false \
  -v "${REPO}/scripts:/scripts:ro" \
  "${IMAGE}" sleep infinity >/dev/null

for _ in $(seq 60); do
  sql "SELECT 1" >/dev/null 2>&1 && break
  sleep 1
done
[[ "$(sql "SELECT to_regclass('public.backups') IS NOT NULL")" == t ]] \
  || fail "no schema in the database -- apply backend/migrations/0001_initial.sql first"

# backup.sh records which migrations the dump can be read by, from the table
# sqlx keeps. A schema applied with psql rather than by the backend has none,
# so the check records the one migration the way sqlx would, and removes the
# table again afterwards.
if [[ "$(sql "SELECT to_regclass('public._sqlx_migrations') IS NULL")" == t ]]; then
  checksum="$(sha384sum "${REPO}/backend/migrations/0001_initial.sql" | cut -d' ' -f1)"
  sql "CREATE TABLE _sqlx_migrations (
         version BIGINT PRIMARY KEY, description TEXT NOT NULL,
         installed_on TIMESTAMPTZ NOT NULL DEFAULT now(), success BOOLEAN NOT NULL,
         checksum BYTEA NOT NULL, execution_time BIGINT NOT NULL);
       INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
       VALUES (1, 'initial', true, '\\x${checksum}', 0)" >/dev/null
  created_migrations=1
fi

# --- 1. A backup under concurrent writes ----------------------------------
# Committed one row at a time, every 10 ms, for as long as the backup runs
# (bounded, should the terminate below never come).
log "starting the concurrent writer"
docker exec -e PGAPPNAME="${WRITER_APP}" "${TOOLS}" \
  psql "${DATABASE_URL}" --no-psqlrc -q -c "
    DO \$\$ BEGIN
      FOR i IN 1..60000 LOOP
        INSERT INTO audit_log (action, reason) VALUES ('backup_check.write', '${RUN_ID}');
        COMMIT;
        PERFORM pg_sleep(0.01);
      END LOOP;
    END \$\$" >/dev/null 2>&1 &
writer_pid=$!
for _ in $(seq 100); do
  written="$(sql "SELECT count(*) FROM audit_log WHERE action = 'backup_check.write' AND reason = '${RUN_ID}'")"
  (( written > 0 )) && break
  sleep 0.1
done
(( written > 0 )) || fail "the concurrent writer never wrote"

log "backup.sh"
run_script backup.sh "BACKUP_PREFIX=${PREFIX}" || fail "backup.sh exited $?"
stop_writer

read -r backups ok sha during < <(sql "
  SELECT count(*), bool_and(ok), max(b.sha256),
         (SELECT count(*) FROM audit_log a
           WHERE a.action = 'backup_check.write' AND a.reason = '${RUN_ID}'
             AND a.created_at BETWEEN min(b.started_at) AND max(b.finished_at))
  FROM backups b WHERE b.s3_key LIKE '${PREFIX}/%'" | tr '|' ' ')
[[ "${backups}" == 1 && "${ok}" == t ]] || fail "expected one ok=true backups row, got ${backups} (ok=${ok})"
(( during > 0 )) || fail "nothing was written while the backup ran, so it proved nothing"
log "the writer committed ${during} rows while the backup ran"

manifest="$(tools "${TOOLS}" bash -c "${S3} s3 ls s3://${BUCKET}/${PREFIX}/" | awk '$NF ~ /\.manifest\.json$/ {print $NF}')"
[[ -n "${manifest}" ]] || fail "no top-level manifest under s3://${BUCKET}/${PREFIX}/"
tools "${TOOLS}" bash -c "${S3} s3 cp --only-show-errors s3://${BUCKET}/${PREFIX}/${manifest} /tmp/manifest.json"
counts="$(sql "SELECT row_counts::text FROM backups WHERE s3_key LIKE '${PREFIX}/%'")"
docker exec -i "${TOOLS}" python3 - "${sha}" "${counts}" <<'PY' || fail "the backups row does not match the manifest"
import json, sys
manifest = json.load(open("/tmp/manifest.json"))
sha, counts = sys.argv[1], json.loads(sys.argv[2])
assert manifest["sha256"] == sha, f"manifest sha256 {manifest['sha256']} != row's {sha}"
assert manifest["table_row_counts"] == counts, "the row's row_counts are not the manifest's"
assert manifest["table_row_counts"]["audit_log"] > 0, "audit_log was dumped empty"
print(f"backups row matches the manifest: sha256 {sha}, audit_log {counts['audit_log']} rows")
PY

# --- 2. The drill of that backup -------------------------------------------
log "restore-drill.sh"
run_script restore-drill.sh "BACKUP_PREFIX=${PREFIX}" "DRILL_DB=${DRILL_DB}" \
  || fail "restore-drill.sh exited $?"
[[ "$(sql "SELECT count(*) FROM pg_database WHERE datname = '${DRILL_DB}'")" == 0 ]] \
  || fail "the drill left ${DRILL_DB} behind"

# --- 3. A backup that cannot upload ------------------------------------------
log "backup.sh to a bucket that does not exist (expected to fail)"
# Its log names every file the upload refused; only the end of it is shown.
status=0
bad_log="$(mktemp)"
run_script backup.sh "BACKUP_PREFIX=${BAD_PREFIX}" "BACKUP_BUCKET=${BUCKET}-missing-${RANDOM}" \
  2>"${bad_log}" || status=$?
tail -n 3 "${bad_log}" >&2
rm -f "${bad_log}"
(( status != 0 )) || fail "backup.sh exited 0 with nowhere to upload"
(( status != 124 )) || fail "backup.sh timed out"
read -r failed_rows failed_ok < <(sql "
  SELECT count(*), bool_or(ok) FROM backups WHERE s3_key LIKE '${BAD_PREFIX}/%'" | tr '|' ' ')
[[ "${failed_rows}" == 1 && "${failed_ok}" == f ]] \
  || fail "expected one ok=false backups row for the failed run, got ${failed_rows} (ok=${failed_ok})"
log "the failed backup exited ${status} and recorded ok=false"
