#!/usr/bin/env bash
#
# Restore a local snapshot taken by dev-dump.sh, or a production dump.
#
#   ./scripts/dev-restore.sh .dev-backups/20260907-120000
#   ./scripts/dev-restore.sh path/to/pg/2026-09-07T03-00-00Z/dump   # a production dump
#
# Every restore is scrubbed on the way in unless SCRUB=0 is set -- a snapshot
# of your own stack too, whose addresses, passwords, API keys, anonymous
# identities and backup history it then resets (scripts/scrub.sql). A
# production dump needs it: it carries real email addresses and password
# hashes, and the whole point of restoring it locally is the shape of the data,
# not those (PLAN.md, "Local development"). Set SCRUB=0 to restore your own
# snapshot as it was; any value but 0 or 1 is refused.
#
# The dump is restored, and scrubbed, into a database of its own
# (birdtest_restore), which replaces the stack's only once both have
# succeeded. Until then the stack's database is untouched, and anything that
# stops the restore -- a dump cut short in its data, a signal, the scrub
# failing -- drops the copy and leaves the stack as it was. Restored in place,
# as it was, a dump that failed part-way had already emptied the database, and
# one stopped by a signal went on restoring inside the container and was never
# scrubbed (the audit's pass 23). A copy left by a process killed outright is
# dropped by the next run.

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
COPY=birdtest_restore

if [[ -f "${SRC}/db.dump" ]]; then
  KIND=custom
elif [[ -d "${SRC}" && -f "${SRC}/toc.dat" ]]; then
  KIND=directory
else
  echo "no db.dump and no directory-format dump at ${SRC}; nothing was changed" >&2
  exit 1
fi

sql() { ${COMPOSE} exec -T postgres psql -U birdtest -d "$1" -q -v ON_ERROR_STOP=1 "${@:2}"; }

discard() {
  sql postgres -c "DROP DATABASE IF EXISTS ${COPY} WITH (FORCE)" >/dev/null 2>&1 || true
  ${COMPOSE} exec -T postgres rm -rf /tmp/restore >/dev/null 2>&1 || true
}
failed() {
  echo "the restore did not finish; the copy is dropped and the stack's database is as it was" >&2
  discard
}
trap failed ERR
trap 'failed; exit 130' INT TERM HUP

discard
sql postgres -c "CREATE DATABASE ${COPY}"

if [[ "${KIND}" == custom ]]; then
  echo "restoring ${SRC}/db.dump into a copy"
  ${COMPOSE} exec -T postgres pg_restore -U birdtest -d "${COPY}" --no-owner --no-privileges \
    --exit-on-error < "${SRC}/db.dump"
else
  # A directory-format dump, as scripts/backup.sh produces. Copied in rather
  # than piped: pg_restore -Fd needs a real directory to read.
  echo "restoring the directory-format dump at ${SRC} into a copy"
  ${COMPOSE} cp "${SRC}" "$(${COMPOSE} ps -q postgres)":/tmp/restore >/dev/null 2>&1 \
    || docker cp "${SRC}" "$(${COMPOSE} ps -q postgres)":/tmp/restore
  ${COMPOSE} exec -T postgres pg_restore -U birdtest -d "${COPY}" --no-owner --no-privileges \
    --exit-on-error -j4 /tmp/restore
  ${COMPOSE} exec -T postgres rm -rf /tmp/restore
fi

if [[ "${SCRUB}" != 0 ]]; then
  echo "scrubbing the copy"
  sql "${COPY}" -v dev_copy=1 < "${SCRIPT_DIR}/scrub.sql"
fi

# The copy is whole and scrubbed: it replaces the stack's database. Nothing may
# hold a connection to either while they are renamed.
echo "stopping the backend"
${COMPOSE} stop backend >/dev/null 2>&1 || true
echo "swapping the copy in"
sql postgres \
  -c "DROP DATABASE IF EXISTS birdtest_replaced WITH (FORCE)" \
  -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'birdtest' AND pid <> pg_backend_pid()" \
  -c "ALTER DATABASE birdtest RENAME TO birdtest_replaced" \
  -c "ALTER DATABASE ${COPY} RENAME TO birdtest" \
  -c "DROP DATABASE birdtest_replaced WITH (FORCE)" >/dev/null
trap - ERR INT TERM HUP

if [[ -d "${SRC}/artifacts" ]]; then
  echo "restoring the artifact bucket"
  ${COMPOSE} run --rm --no-deps -T -v "$(cd "${SRC}/artifacts" && pwd):/in" \
    --entrypoint /bin/sh minio-init -c '
      mc alias set local http://minio:9000 birdtest birdtestbirdtest >/dev/null
      mc mb --ignore-existing local/birdtest-artifacts >/dev/null
      mc mirror --overwrite --remove /in local/birdtest-artifacts
    ' || echo "the artifact bucket was not restored (the database was): run this again once MinIO is up" >&2
fi

echo "starting the backend"
${COMPOSE} start backend >/dev/null 2>&1 || ${COMPOSE} up -d backend

echo "restored from ${SRC}"
