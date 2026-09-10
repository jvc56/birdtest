#!/usr/bin/env bash
#
# Nightly logical backup of the birdtest database. Runs as a one-shot Fargate
# task on the official Postgres image (see infra/backup.tf, which passes this
# file as the container command), and can be run by hand against any database:
#
#   DATABASE_URL=postgres://... BACKUP_BUCKET=my-bucket ./scripts/backup.sh
#
# What it produces, per PLAN.md, "Layout and manifest":
#
#   s3://$BACKUP_BUCKET/pg/<stamp>/dump/...   pg_dump -Fd output
#   s3://$BACKUP_BUCKET/pg/<stamp>/manifest.json
#   s3://$BACKUP_BUCKET/pg/<stamp>.manifest.json
#
# It is deliberately loud on failure: a `backups` row with ok=false, a non-zero
# exit for the EventBridge failure rule, and no success metric so the staleness
# alarm fires too.

set -Eeuo pipefail

: "${DATABASE_URL:?DATABASE_URL is required}"
: "${BACKUP_BUCKET:?BACKUP_BUCKET is required}"
BACKUP_PREFIX="${BACKUP_PREFIX:-pg}"
PGDUMP_JOBS="${PGDUMP_JOBS:-4}"
BACKEND_IMAGE="${BACKEND_IMAGE:-unknown}"
METRIC_NAMESPACE="${METRIC_NAMESPACE:-birdtest/backup}"
WORKDIR="${WORKDIR:-/tmp/birdtest-backup}"

# Colons are legal in S3 keys and a nuisance in every shell that touches them.
STAMP="$(date -u +%Y-%m-%dT%H-%M-%SZ)"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
START_EPOCH="$(date -u +%s)"
DEST="s3://${BACKUP_BUCKET}/${BACKUP_PREFIX}/${STAMP}"
DUMP_DIR="${WORKDIR}/dump"

# `aws s3 cp` needs these when the bucket's default encryption is SSE-KMS and
# the caller has no kms:Decrypt: the upload must name the key explicitly.
sse_args=()
if [[ -n "${BACKUP_KMS_KEY_ARN:-}" ]]; then
  sse_args=(--sse aws:kms --sse-kms-key-id "${BACKUP_KMS_KEY_ARN}")
fi

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

# psql that fails the script rather than returning a partial answer.
psql_val() {
  psql "${DATABASE_URL}" --no-psqlrc --tuples-only --no-align \
       --quiet --set ON_ERROR_STOP=1 --command "$1"
}

# Whatever went wrong, record it where an admin will see it. Best-effort: if
# the database is what failed, the INSERT fails too and the exit code and the
# absent success metric are what remain.
failed=1
finish() {
  local status=$?
  if (( status != 0 )) && (( failed )); then
    log "FAILED with status ${status}"
    psql "${DATABASE_URL}" --no-psqlrc --quiet --command "
      INSERT INTO backups (kind, s3_key, started_at, finished_at, row_counts, ok)
      VALUES ('pg_dump', '${BACKUP_PREFIX}/${STAMP}', '${STARTED_AT}', now(), '{}'::jsonb, false)
    " || log "could not record the failure in the backups table"
  fi
  rm -rf "${WORKDIR}"
  exit "${status}"
}
trap finish EXIT

# --- Tools -----------------------------------------------------------------
# The Postgres image has psql and pg_dump and nothing else. Installing the AWS
# CLI at start beats maintaining an image of our own: this script's whole
# dependency set is then "the official postgres image", which is one fewer
# thing to rebuild when the engine version moves.
if ! command -v aws >/dev/null 2>&1; then
  log "installing the AWS CLI"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends awscli ca-certificates >/dev/null
fi

mkdir -p "${WORKDIR}"

# --- Dump ------------------------------------------------------------------
# Directory format, because -j is what makes a dump of the results tables
# finish and because it lets a restore pull one table out without streaming the
# whole archive. --no-owner/--no-privileges so the dump restores as whatever
# role the restoring side happens to use, which for a scratch restore or a
# local reproduction is never the production one.
log "dumping to ${DUMP_DIR}"
pg_dump "${DATABASE_URL}" \
  --format=directory \
  --jobs="${PGDUMP_JOBS}" \
  --compress=6 \
  --no-owner \
  --no-privileges \
  --file="${DUMP_DIR}"

# A dump that cannot be listed cannot be restored. This is cheap and catches
# truncation and corruption before the upload rather than during a recovery.
log "verifying the dump is readable"
pg_restore --list "${DUMP_DIR}" >/dev/null

DUMP_BYTES="$(du -sb "${DUMP_DIR}" | cut -f1)"
DUMP_SHA="$(dump_digest "${DUMP_DIR}")"

# --- Facts about what was dumped -------------------------------------------
# Exact counts, not reltuples: this is what a restore is verified against
# (PLAN.md, "Verifying a restore"), and an estimate would make the check meaningless.
log "counting rows"
ROW_COUNTS="$(psql_val "
  SELECT COALESCE(jsonb_object_agg(relname, cnt), '{}'::jsonb)::text
  FROM (
    SELECT c.relname,
           (xpath('/row/c/text()',
                  query_to_xml(format('SELECT count(*) AS c FROM %I.%I', n.nspname, c.relname),
                               false, true, '')))[1]::text::bigint AS cnt
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind = 'r' AND n.nspname = 'public'
  ) counts
")"

# What code can read this dump. Under a single migration edited in place, this
# is the only thing that makes an older dump restorable at all -- see
# PLAN.md, "Restoring across a schema change".
MIGRATIONS="$(psql_val "
  SELECT COALESCE(jsonb_agg(jsonb_build_object(
           'version', version,
           'description', description,
           'checksum', encode(checksum, 'hex')) ORDER BY version), '[]'::jsonb)::text
  FROM _sqlx_migrations
")"

PG_VERSION="$(psql_val 'SHOW server_version')"
DB_BYTES="$(psql_val 'SELECT pg_database_size(current_database())')"
ARTIFACT_KEYS="$(psql_val 'SELECT count(*) FROM leave_generation_artifacts')"
FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
DURATION=$(( $(date -u +%s) - START_EPOCH ))

cat > "${WORKDIR}/manifest.json" <<JSON
{
  "stamp": "${STAMP}",
  "started_at": "${STARTED_AT}",
  "finished_at": "${FINISHED_AT}",
  "duration_seconds": ${DURATION},
  "postgres_version": "${PG_VERSION}",
  "pg_dump_version": "$(pg_dump --version | awk '{print $NF}')",
  "backend_image": "${BACKEND_IMAGE}",
  "migration_checksums": ${MIGRATIONS},
  "database_bytes": ${DB_BYTES},
  "dump_bytes": ${DUMP_BYTES},
  "dump_format": "directory",
  "table_row_counts": ${ROW_COUNTS},
  "artifact_keys_referenced": ${ARTIFACT_KEYS},
  "sha256": "${DUMP_SHA}"
}
JSON

# --- Upload ----------------------------------------------------------------
# Dump first, manifest second, top-level manifest last: each is only written
# once what it describes is durable, so a manifest is never a promise about an
# upload that did not finish.
log "uploading to ${DEST}"
aws "${s3_args[@]}" s3 cp "${DUMP_DIR}" "${DEST}/dump" --recursive --only-show-errors "${sse_args[@]}"
aws "${s3_args[@]}" s3 cp "${WORKDIR}/manifest.json" "${DEST}/manifest.json" --only-show-errors "${sse_args[@]}"
aws "${s3_args[@]}" s3 cp "${WORKDIR}/manifest.json" \
  "s3://${BACKUP_BUCKET}/${BACKUP_PREFIX}/${STAMP}.manifest.json" \
  --only-show-errors "${sse_args[@]}"

# --- Record ----------------------------------------------------------------
# Bound through psql variables rather than interpolated into the SQL: the row
# counts are a JSON document, and :'name' quotes it correctly whatever it
# contains. The statement comes in on stdin because psql interpolates
# variables there and not in --command.
psql "${DATABASE_URL}" --no-psqlrc --quiet --set ON_ERROR_STOP=1 \
  --set stamp="${BACKUP_PREFIX}/${STAMP}" \
  --set started="${STARTED_AT}" \
  --set finished="${FINISHED_AT}" \
  --set bytes="${DUMP_BYTES}" \
  --set sha="${DUMP_SHA}" \
  --set counts="${ROW_COUNTS}" <<'SQL'
INSERT INTO backups (kind, s3_key, started_at, finished_at, dump_bytes, row_counts, sha256, ok)
VALUES ('pg_dump', :'stamp', :'started'::timestamptz, :'finished'::timestamptz,
        :'bytes'::bigint, :'counts'::jsonb, :'sha', true);
SQL

# The staleness alarm reads this metric and treats its absence as breaching, so
# emitting it is the last thing that happens and only on the success path.
if command -v aws >/dev/null 2>&1; then
  aws cloudwatch put-metric-data --namespace "${METRIC_NAMESPACE}" \
    --metric-name Success --value 1 --unit Count || log "metric publish failed"
  aws cloudwatch put-metric-data --namespace "${METRIC_NAMESPACE}" \
    --metric-name DurationSeconds --value "${DURATION}" --unit Seconds || true
  aws cloudwatch put-metric-data --namespace "${METRIC_NAMESPACE}" \
    --metric-name DumpBytes --value "${DUMP_BYTES}" --unit Bytes || true
fi

failed=0
log "backup ${STAMP} complete: ${DUMP_BYTES} bytes in ${DURATION}s, sha256 ${DUMP_SHA}"
