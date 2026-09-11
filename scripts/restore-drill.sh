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
# The drill restores into a second database on the same instance rather than
# provisioning one, which keeps it a shell script rather than an orchestration.
# The cost is transient storage: the instance needs headroom for a second copy
# of the corpus, which `max_allocated_storage` (5x allocated) provides.

set -Eeuo pipefail

: "${DATABASE_URL:?DATABASE_URL is required}"
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

# The admin URL is the same server, different database. Postgres cannot drop a
# database from a connection to it, so CREATE/DROP go through this one. The
# query string is carried across: `?sslmode=require` belongs on every one of
# these connections, not just the original.
BASE_URL="${DATABASE_URL%%\?*}"
QUERY=""
if [[ "${DATABASE_URL}" == *\?* ]]; then
  QUERY="?${DATABASE_URL#*\?}"
fi
ADMIN_URL="${BASE_URL%/*}/postgres${QUERY}"
DRILL_URL="${BASE_URL%/*}/${DRILL_DB}${QUERY}"

cleanup() {
  local status=$?
  log "dropping ${DRILL_DB}"
  psql "${ADMIN_URL}" --no-psqlrc --quiet \
    --command "DROP DATABASE IF EXISTS \"${DRILL_DB}\" WITH (FORCE)" \
    || log "could not drop ${DRILL_DB} -- drop it by hand"
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
if command -v aws >/dev/null 2>&1; then
  aws cloudwatch put-metric-data --namespace "${METRIC_NAMESPACE}" \
    --metric-name DrillSuccess --value 1 --unit Count || log "metric publish failed"
fi
