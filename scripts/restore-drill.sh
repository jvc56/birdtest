#!/usr/bin/env bash
#
# Automated restore drill (PLAN.md, "Drills"). Restores the most recent
# nightly dump into a throwaway database, runs the verification queries from
# 7.6 against it, and drops it again. Scheduled monthly by infra/backup.tf;
# runnable by hand the same way as scripts/backup.sh.
#
# This is the only check that catches a dump which has been silently producing
# unusable output: everything else verifies that a backup *ran*.
#
# By default the drill restores into a Postgres of its own, started inside
# this container -- the task runs the postgres image, whose major version is
# the dump's by construction, with restore_ephemeral_storage_gib (200 GiB by
# default) of ephemeral storage -- and never
# touches the production instance. (DRILL_TARGET=server restores into a second
# database on the server DATABASE_URL names instead, as it once always did.)
#
# It used to be the production instance, justified by `max_allocated_storage`
# covering a second copy. It does not reliably: RDS grows storage only after
# free space has sat under 10% for five minutes, by one step, then not again
# for six hours, and never shrinks it. A drill writes a whole corpus, its
# index builds' spill and their WAL in about an hour, so one near the free
# space could fill the instance mid-restore -- production down -- and one that
# survived ratcheted storage up for good, spent the burstable instance's CPU
# and I/O credits against live traffic, and put its WAL in the PITR archive.

set -Eeuo pipefail

DRILL_TARGET="${DRILL_TARGET:-local}"
if [[ "${DRILL_TARGET}" == server ]]; then
  : "${DATABASE_URL:?DATABASE_URL is required with DRILL_TARGET=server}"
fi
: "${BACKUP_BUCKET:?BACKUP_BUCKET is required}"
BACKUP_PREFIX="${BACKUP_PREFIX:-pg}"
METRIC_NAMESPACE="${METRIC_NAMESPACE:-birdtest/backup}"
WORKDIR="${WORKDIR:-/tmp/birdtest-drill}"
PGRESTORE_JOBS="${PGRESTORE_JOBS:-4}"
DRILL_DB="${DRILL_DB:-birdtest_drill_$(date -u +%Y%m%d%H%M%S)}"

# MinIO in a local stack, real S3 in production -- the same split the backend
# makes with S3_ENDPOINT. Unset everywhere but a developer's machine.
s3_args=()
if [[ -n "${AWS_S3_ENDPOINT:-}" ]]; then
  s3_args=(--endpoint-url "${AWS_S3_ENDPOINT}")
fi

# The CloudWatch metrics are production's alarm inputs and nothing else's. A
# run against a local stack has no CloudWatch to send them to -- its
# credentials are MinIO's, and sending them would reach real AWS -- so it
# skips them: BACKUP_METRICS=false skips them anywhere, =true sends them
# anywhere, and unset means "send them unless AWS_S3_ENDPOINT points at a
# stand-in object store".
if [[ -z "${BACKUP_METRICS:-}" ]]; then
  if [[ -n "${AWS_S3_ENDPOINT:-}" ]]; then BACKUP_METRICS=false; else BACKUP_METRICS=true; fi
fi

# A checksum of the dump's *contents*, independent of the file metadata that a
# download does not preserve: each file hashed under its relative path, then a
# hash of that listing. Hashing a tar of the directory instead would compare
# mtimes and ownership, and would report a mismatch for every dump that had
# merely made the round trip through S3.
dump_digest() {
  ( cd "$1" && find . -type f | LC_ALL=C sort | xargs -r sha256sum ) \
    | sha256sum | cut -d' ' -f1
}

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >&2; }

# Everything the drill's own server writes, in one directory the postgres user
# owns: the data directory, its socket and its log.
LOCAL_DIR="${WORKDIR}/local"
LOCAL_PGDATA="${LOCAL_DIR}/pgdata"
LOCAL_SOCKET="${LOCAL_DIR}/socket"
if [[ "${DRILL_TARGET}" == local ]]; then
  # Unix socket only, trust auth: nothing outside this container can reach it.
  ADMIN_URL="postgresql:///postgres?host=${LOCAL_SOCKET}&user=postgres"
  DRILL_URL="postgresql:///${DRILL_DB}?host=${LOCAL_SOCKET}&user=postgres"
else
  # The admin URL is the same server, different database. Postgres cannot drop
  # a database from a connection to it, so CREATE/DROP go through this one.
  # The query string is carried across: `?sslmode=require` belongs on every
  # one of these connections, not just the original.
  BASE_URL="${DATABASE_URL%%\?*}"
  QUERY=""
  if [[ "${DATABASE_URL}" == *\?* ]]; then
    QUERY="?${DATABASE_URL#*\?}"
  fi
  ADMIN_URL="${BASE_URL%/*}/postgres${QUERY}"
  DRILL_URL="${BASE_URL%/*}/${DRILL_DB}${QUERY}"
fi

# As the postgres user, which the server refuses to run as root without.
as_postgres() {
  if [[ "$(id -u)" == 0 ]]; then gosu postgres "$@"; else "$@"; fi
}

cleanup() {
  local status=$?
  if [[ "${DRILL_TARGET}" == local ]]; then
    if [[ -f "${LOCAL_PGDATA}/postmaster.pid" ]]; then
      as_postgres pg_ctl -D "${LOCAL_PGDATA}" -m immediate stop >/dev/null 2>&1 \
        || log "could not stop the drill's own server"
    fi
  else
    log "dropping ${DRILL_DB}"
    psql "${ADMIN_URL}" --no-psqlrc --quiet \
      --command "DROP DATABASE IF EXISTS \"${DRILL_DB}\" WITH (FORCE)" \
      || log "could not drop ${DRILL_DB} -- drop it by hand"
  fi
  rm -rf "${WORKDIR}"
  exit "${status}"
}
trap cleanup EXIT

if ! command -v aws >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends awscli ca-certificates python3 >/dev/null
fi

mkdir -p "${WORKDIR}"

# --- Fetch the newest backup ----------------------------------------------
# The top-level manifest copies exist so that finding the latest backup is one
# listing rather than a walk of every dump prefix.
LATEST="$(aws "${s3_args[@]}" s3 ls "s3://${BACKUP_BUCKET}/${BACKUP_PREFIX}/" \
  | awk '$NF ~ /\.manifest\.json$/ {print $NF}' | sort | tail -1)"
if [[ -z "${LATEST}" ]]; then
  log "no backups found under s3://${BACKUP_BUCKET}/${BACKUP_PREFIX}/"
  exit 1
fi
STAMP="${LATEST%.manifest.json}"
log "drilling ${STAMP}"

aws "${s3_args[@]}" s3 cp "s3://${BACKUP_BUCKET}/${BACKUP_PREFIX}/${LATEST}" "${WORKDIR}/manifest.json" --only-show-errors
aws "${s3_args[@]}" s3 cp "s3://${BACKUP_BUCKET}/${BACKUP_PREFIX}/${STAMP}/dump" "${WORKDIR}/dump" \
  --recursive --only-show-errors

# The manifest's sha256 is over the dump's contents, so this catches a partial
# or corrupted download as well as a corrupted upload.
EXPECTED_SHA="$(grep -o '"sha256"[^,}]*' "${WORKDIR}/manifest.json" | sed 's/.*: *"//;s/"//')"
ACTUAL_SHA="$(dump_digest "${WORKDIR}/dump")"
if [[ "${EXPECTED_SHA}" != "${ACTUAL_SHA}" ]]; then
  log "CHECKSUM MISMATCH: manifest says ${EXPECTED_SHA}, dump hashes to ${ACTUAL_SHA}"
  exit 1
fi
log "checksum matches"

# --- Restore ---------------------------------------------------------------
if [[ "${DRILL_TARGET}" == local ]]; then
  # The restored database lands on the same disk as the dump just downloaded,
  # with WAL on top. Checked now, with the size the manifest recorded, rather
  # than found an hour into the restore as ENOSPC: a drill failing for want of
  # disk says nothing about the backup it was drilling.
  need="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["database_bytes"] + 5 * 2**30)' \
    "${WORKDIR}/manifest.json")"
  free="$(df --output=avail -B1 "${WORKDIR}" | tail -1 | tr -d ' ')"
  if (( free < need )); then
    log "NOT ENOUGH DISK: the restore needs about $(( need / 2**30 )) GiB and ${WORKDIR} has $(( free / 2**30 )) GiB free. Raise restore_ephemeral_storage_gib if it is below Fargate's 200; past that, run this drill by hand with DRILL_TARGET=server against a scratch RDS instance restored from a snapshot (RUNBOOK.md, 6)"
    exit 1
  fi
  log "starting the drill's own server"
  mkdir -p "${LOCAL_PGDATA}" "${LOCAL_SOCKET}"
  if [[ "$(id -u)" == 0 ]]; then chown -R postgres:postgres "${LOCAL_DIR}"; fi
  as_postgres initdb --pgdata="${LOCAL_PGDATA}" --username=postgres --auth=trust \
    --encoding=UTF8 >/dev/null
  # Durability is worth nothing to a database dropped in an hour, and the
  # restore is the whole of the drill's run time. No parallel query: its
  # workers share memory through /dev/shm, which a container may give only
  # 64 MB, and a verification query that tripped on it would fail the drill
  # for a reason that has nothing to do with the backup.
  as_postgres pg_ctl -D "${LOCAL_PGDATA}" -w -l "${LOCAL_DIR}/postgres.log" start -o \
    "-c listen_addresses='' -c unix_socket_directories='${LOCAL_SOCKET}' \
     -c fsync=off -c synchronous_commit=off -c full_page_writes=off \
     -c maintenance_work_mem=256MB -c max_wal_size=4GB \
     -c max_parallel_workers_per_gather=0" >/dev/null
fi
psql "${ADMIN_URL}" --no-psqlrc --quiet --set ON_ERROR_STOP=1 \
  --command "CREATE DATABASE \"${DRILL_DB}\""

# --exit-on-error, because a drill that ignores errors verifies nothing.
pg_restore --dbname="${DRILL_URL}" --jobs="${PGRESTORE_JOBS}" \
  --no-owner --no-privileges --exit-on-error "${WORKDIR}/dump"

# --- Verify (PLAN.md, "Verifying a restore") ----------------------------------------
psql "${DRILL_URL}" --no-psqlrc --quiet --set ON_ERROR_STOP=1 <<'SQL'
\set ON_ERROR_STOP on

-- Referential sanity. Each of these is a class of restore failure that a row
-- count cannot see.
DO $$
DECLARE
    bad bigint;
BEGIN
    SELECT count(*) INTO bad FROM jobs j
     WHERE NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.letterdist_id)
        OR NOT EXISTS (SELECT 1 FROM input_data d WHERE d.id = j.layout_id);
    IF bad > 0 THEN RAISE EXCEPTION 'jobs with missing pinned input data: %', bad; END IF;

    SELECT count(*) INTO bad FROM input_data
     WHERE role IN ('letterdist', 'layout') AND content IS NULL;
    IF bad > 0 THEN RAISE EXCEPTION 'input_data rows missing content: %', bad; END IF;

    -- The denormalized counters are what a restore is most likely to have
    -- silently wrong, and what the scheduler dispatches on.
    SELECT count(*) INTO bad
      FROM tasks t
      JOIN LATERAL (
        SELECT count(*) FILTER (WHERE c.state = 'completed') AS accepted,
               count(*) FILTER (WHERE c.state = 'claimed')   AS active
          FROM task_claims c WHERE c.task_id = t.id
      ) actual ON true
     WHERE t.accepted_count <> actual.accepted OR t.active_claim_count <> actual.active;
    IF bad > 0 THEN RAISE EXCEPTION 'tasks whose claim counters disagree with their claims: %', bad; END IF;
END
$$;
SQL

# Row counts against the manifest. Compared as text so a missing table and a
# zero-row table are distinguishable.
ACTUAL_COUNTS="$(psql "${DRILL_URL}" --no-psqlrc --tuples-only --no-align --quiet \
  --set ON_ERROR_STOP=1 --command "
  SELECT COALESCE(jsonb_object_agg(relname, cnt), '{}'::jsonb)::text
  FROM (
    SELECT c.relname,
           (xpath('/row/c/text()',
                  query_to_xml(format('SELECT count(*) AS c FROM %I.%I', n.nspname, c.relname),
                               false, true, '')))[1]::text::bigint AS cnt
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind = 'r' AND n.nspname = 'public'
  ) counts
")"

# The backups table is the one row count that legitimately differs: the dump
# was taken before its own row was inserted.
python3 - "${WORKDIR}/manifest.json" "${ACTUAL_COUNTS}" <<'PY'
import json, sys
manifest = json.load(open(sys.argv[1]))
actual = json.loads(sys.argv[2])
expected = manifest["table_row_counts"]
problems = []
for table, want in expected.items():
    if table == "backups":
        continue
    got = actual.get(table)
    if got is None:
        problems.append(f"{table}: missing from the restore")
    elif got != want:
        problems.append(f"{table}: expected {want}, restored {got}")
for table in set(actual) - set(expected):
    problems.append(f"{table}: present in the restore but not in the manifest")
if problems:
    print("ROW COUNT MISMATCH:", *problems, sep="\n  ", file=sys.stderr)
    sys.exit(1)
print(f"verified {len(expected)} tables against the manifest")
PY

log "drill of ${STAMP} passed"
if [[ "${BACKUP_METRICS}" == true ]] && command -v aws >/dev/null 2>&1; then
  aws cloudwatch put-metric-data --namespace "${METRIC_NAMESPACE}" \
    --metric-name DrillSuccess --value 1 --unit Count || log "metric publish failed"
fi
