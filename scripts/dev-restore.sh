#!/usr/bin/env bash
#
# Restore a local snapshot taken by dev-dump.sh, or a production dump.
#
#   ./scripts/dev-restore.sh .dev-backups/20260907-120000
#   ./scripts/dev-restore.sh path/to/pg/2026-09-07T03-00-00Z/dump   # a production dump
#
# Every restore is scrubbed on the way in unless SCRUB=0 is set -- a snapshot
# of your own stack too, whose addresses, passwords, API keys, anonymous
# identities and backup history it then resets (scripts/scrub.sql). A production dump needs it: it carries real email addresses and
# password hashes, and the whole point of restoring it locally is the shape of
# the data, not those (PLAN.md, "Local development"). Set SCRUB=0 to restore
# your own snapshot as it was; any value but 0 or 1 is refused.

set -Eeuo pipefail

SRC="${1:?usage: dev-restore.sh <snapshot-dir-or-dump>}"
COMPOSE="${COMPOSE:-docker compose}"
SCRUB="${SCRUB:-1}"
# Only 0 turns the scrub off, and only 0 or 1 is taken: `SCRUB=true` or `yes`
# used to mean "off" too, restoring a production dump's real addresses and
# password hashes onto a laptop with no word. Refused before anything changes.
if [[ "${SCRUB}" != 0 && "${SCRUB}" != 1 ]]; then
  echo "SCRUB must be 0 (keep the data as it is) or 1 (scrub it, the default), not '${SCRUB}'" >&2
  exit 2
fi
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Nothing may hold a connection while the schema is replaced.
echo "stopping the backend"
${COMPOSE} stop backend >/dev/null 2>&1 || true

echo "dropping and recreating the public schema"
${COMPOSE} exec -T postgres psql -U birdtest -d birdtest -q \
  -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;'

if [[ -f "${SRC}/db.dump" ]]; then
  echo "restoring ${SRC}/db.dump"
  ${COMPOSE} exec -T postgres pg_restore -U birdtest -d birdtest --no-owner --no-privileges \
    --exit-on-error < "${SRC}/db.dump"
elif [[ -d "${SRC}" && -f "${SRC}/toc.dat" ]]; then
  # A directory-format dump, as scripts/backup.sh produces. Copied in rather
  # than piped: pg_restore -Fd needs a real directory to read.
  echo "restoring the directory-format dump at ${SRC}"
  ${COMPOSE} exec -T postgres rm -rf /tmp/restore
  ${COMPOSE} cp "${SRC}" "$(${COMPOSE} ps -q postgres)":/tmp/restore >/dev/null 2>&1 \
    || docker cp "${SRC}" "$(${COMPOSE} ps -q postgres)":/tmp/restore
  ${COMPOSE} exec -T postgres pg_restore -U birdtest -d birdtest --no-owner --no-privileges \
    --exit-on-error -j4 /tmp/restore
  ${COMPOSE} exec -T postgres rm -rf /tmp/restore
else
  echo "no db.dump and no directory-format dump at ${SRC}" >&2
  exit 1
fi

if [[ -d "${SRC}/artifacts" ]]; then
  echo "restoring the artifact bucket"
  ${COMPOSE} run --rm --no-deps -T -v "$(cd "${SRC}/artifacts" && pwd):/in" \
    --entrypoint /bin/sh minio-init -c '
      mc alias set local http://minio:9000 birdtest birdtestbirdtest >/dev/null
      mc mb --ignore-existing local/birdtest-artifacts >/dev/null
      mc mirror --overwrite /in local/birdtest-artifacts
    '
fi

if [[ "${SCRUB}" != 0 ]]; then
  echo "scrubbing"
  ${COMPOSE} exec -T postgres psql -U birdtest -d birdtest -q -v ON_ERROR_STOP=1 \
    < "${SCRIPT_DIR}/scrub.sql"
fi

echo "starting the backend"
${COMPOSE} start backend >/dev/null 2>&1 || ${COMPOSE} up -d backend

echo "restored from ${SRC}"
