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
# snapshot as it was; any value but 0 or 1 is refused, and so is SCRUB=0 for a
# directory-format dump, which only production's backup.sh makes.
#
# The dump is restored, and scrubbed, into a database of this run's own
# (birdtest_restore_<run>), which replaces the stack's in one transaction
# once both have succeeded: both renames happen or neither does, so there is
# always a `birdtest`. Until then the stack's database is untouched, and
# anything that stops the restore -- a dump cut short in its data, a signal,
# the scrub failing -- drops the copy and leaves the stack as it was. A signal
# during the swap waits for it to finish in the container (a signal stops this
# script, not the psql inside the container) and then says which it was.
# Restored in place, as it was until the audit's pass 23, a dump that failed
# part-way had already emptied the database; swapped by two renames, as it
# was until pass 24, a failure between them lost it. Each run's names are its
# own, so two at once cannot swap in each other's copy; the second may fail.
# A copy left by a process killed outright (unscrubbed, if it was killed
# before the scrub) is dropped by the next run.

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

if [[ "${SRC%/}" == *.partial ]]; then
  echo "${SRC} is a snapshot dev-dump.sh did not finish; nothing was changed" >&2
  exit 1
fi
if [[ -f "${SRC}/db.dump" ]]; then
  KIND=custom
elif [[ -d "${SRC}" && -f "${SRC}/toc.dat" ]]; then
  KIND=directory
  if [[ "${SCRUB}" == 0 ]]; then
    echo "a directory-format dump is production's (scripts/backup.sh), and is always scrubbed: SCRUB=0 is refused; nothing was changed" >&2
    exit 2
  fi
else
  echo "no db.dump and no directory-format dump at ${SRC}; nothing was changed" >&2
  exit 1
fi

RUN="$(date +%s)_$$"
COPY="birdtest_restore_${RUN}"
OLD="birdtest_replaced_${RUN}"
DUMPDIR="/tmp/restore_${RUN}"
MIRROR="birdtest-restore-mirror-${RUN}"
# Held by the swap's transaction, and taken by a stopped run before it looks
# at what the swap did.
LOCK=4711001

sql() { ${COMPOSE} exec -T postgres psql -U birdtest -d "$1" -q -v ON_ERROR_STOP=1 "${@:2}"; }

phase=copy
stopped=0
restart() {
  if (( stopped )); then
    echo "starting the backend" >&2
    # A stop the daemon is still carrying out would end after the start,
    # leaving it stopped: `stop` waits for it first.
    ${COMPOSE} stop backend >/dev/null 2>&1 || true
    ${COMPOSE} start backend >/dev/null 2>&1 || echo "the backend did not start: docker compose start backend" >&2
  fi
}
stop() {
  # Ignored, not reset: a second Ctrl-C while this waits for the swap killed
  # it with the backend still stopped and nothing said.
  trap - ERR
  trap '' INT TERM HUP
  if [[ "${phase}" == swap ]]; then
    # The swap may still be running in the container, or have committed:
    # wait for it, drop the copy if it is still there, and read which.
    local by
    if ! by=$(sql postgres -At -c "SELECT pg_advisory_lock(${LOCK})" \
      -c "DROP DATABASE IF EXISTS ${COPY} WITH (FORCE)" \
      -c "SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = 'birdtest'" \
      2>/dev/null | tail -n 1); then
      echo "stopped during the swap, and whether it finished could not be read: the database named birdtest has the comment 'dev-restore ${RUN}' if it did" >&2
      restart
      return
    fi
    if [[ "${by}" == "dev-restore ${RUN}" ]]; then
      echo "stopped, but the swap had finished: the stack runs the restored copy (${SRC}); the artifact bucket was not restored" >&2
      sql postgres -c "DROP DATABASE IF EXISTS ${OLD} WITH (FORCE)" >/dev/null 2>&1 || true
      restart
      return
    fi
  else
    sql postgres -c "DROP DATABASE IF EXISTS ${COPY} WITH (FORCE)" >/dev/null 2>&1 || true
  fi
  ${COMPOSE} exec -T postgres rm -rf "${DUMPDIR}" >/dev/null 2>&1 || true
  echo "the restore did not finish; the copy is dropped and the stack's database is as it was" >&2
  restart
}
trap 'stop; exit 1' ERR
trap 'stop; exit 130' INT TERM HUP

# What earlier runs left: their copies (killed outright, or one running now,
# which then fails harmlessly) and a replaced database whose drop was cut
# short -- only while `birdtest` exists, so never the last copy there is.
# The stack's database must be there to be replaced. A swap by an earlier
# version of this script could leave it renamed away.
if [[ "$(sql postgres -At -c "SELECT count(*) FROM pg_database WHERE datname = 'birdtest'" || true)" == 0 ]]; then
  echo "there is no database named birdtest; nothing was changed. Replaced ones:" >&2
  sql postgres -At -c "SELECT datname FROM pg_database WHERE datname LIKE 'birdtest\_replaced%'" >&2 || true
  echo "rename the one to keep back (ALTER DATABASE ... RENAME TO birdtest), or create an empty one, and run this again" >&2
  trap - ERR INT TERM HUP
  exit 1
fi
sql postgres <<'SQL'
SELECT format('DROP DATABASE %I WITH (FORCE)', datname) FROM pg_database
 WHERE datname LIKE 'birdtest\_restore%'
    OR (datname LIKE 'birdtest\_replaced%'
        AND EXISTS (SELECT 1 FROM pg_database WHERE datname = 'birdtest'))
\gexec
SQL
${COMPOSE} exec -T postgres sh -c 'rm -rf /tmp/restore /tmp/restore_*' >/dev/null 2>&1 || true
sql postgres -c "CREATE DATABASE ${COPY}"

if [[ "${KIND}" == custom ]]; then
  echo "restoring ${SRC}/db.dump into a copy"
  ${COMPOSE} exec -T postgres pg_restore -U birdtest -d "${COPY}" --no-owner --no-privileges \
    --exit-on-error < "${SRC}/db.dump"
else
  # A directory-format dump, as scripts/backup.sh produces. Copied in rather
  # than piped: pg_restore -Fd needs a real directory to read.
  echo "restoring the directory-format dump at ${SRC} into a copy"
  ${COMPOSE} cp "${SRC}" "postgres:${DUMPDIR}" >/dev/null 2>&1 \
    || docker cp "${SRC}" "$(${COMPOSE} ps -q postgres)":"${DUMPDIR}"
  ${COMPOSE} exec -T postgres pg_restore -U birdtest -d "${COPY}" --no-owner --no-privileges \
    --exit-on-error -j4 "${DUMPDIR}"
  ${COMPOSE} exec -T postgres rm -rf "${DUMPDIR}"
fi

if [[ "${SCRUB}" != 0 ]]; then
  echo "scrubbing the copy"
  sql "${COPY}" -v dev_copy=1 < "${SCRIPT_DIR}/scrub.sql"
fi

# The copy is whole and scrubbed: it replaces the stack's database, in one
# transaction. Nothing may hold a connection to either while they are renamed.
if [[ -n "$(${COMPOSE} ps -q --status running backend 2>/dev/null || true)" ]]; then
  echo "stopping the backend"
  stopped=1
  ${COMPOSE} stop backend >/dev/null 2>&1
fi
echo "swapping the copy in"
phase=swap
sql postgres >/dev/null <<SQL
BEGIN;
SELECT pg_advisory_xact_lock(${LOCK});
SELECT pg_terminate_backend(pid) FROM pg_stat_activity
 WHERE datname IN ('birdtest', '${COPY}') AND pid <> pg_backend_pid();
ALTER DATABASE birdtest RENAME TO ${OLD};
ALTER DATABASE ${COPY} RENAME TO birdtest;
COMMENT ON DATABASE birdtest IS 'dev-restore ${RUN}';
COMMIT;
SQL
phase=done
# From here the database is restored whatever happens: a stop says so and
# brings the backend back (it used to leave it stopped, and say nothing).
after() {
  trap '' INT TERM HUP
  docker rm -f "${MIRROR}" >/dev/null 2>&1 || true
  echo "stopped after the swap: the database is restored from ${SRC}, the artifact bucket perhaps not; run the restore again" >&2
  restart
}
trap - ERR
trap 'after; exit 1' INT TERM HUP
sql postgres -c "DROP DATABASE ${OLD} WITH (FORCE)" >/dev/null \
  || echo "the replaced database ${OLD} was not dropped; the next restore drops it" >&2

status=0
if [[ -d "${SRC}/artifacts" && -z "$(find "${SRC}/artifacts" -type f -print -quit)" ]]; then
  # Mirrored with --remove, an empty directory empties the bucket: and before
  # the audit's pass 24 every snapshot's was empty on Linux (dev-dump.sh).
  echo "the snapshot holds no artifact objects: the bucket is left as it is" >&2
elif [[ -d "${SRC}/artifacts" ]]; then
  # Worked out before the mirror starts, so that a signal is not held while
  # they are.
  user="$(id -u):$(id -g)"
  artifacts="$(cd "${SRC}/artifacts" && pwd)"
  echo "restoring the artifact bucket"
  # `exec`, so that mc, not a shell that ignores it, gets a signal. The
  # container is removed whatever the exit: stopped any way the trap above
  # does not see -- its own `docker stop` -- it went on mirroring, with
  # --remove, after this had said it had stopped (pass 25).
  mirrored=0
  ${COMPOSE} run --rm --no-deps -T --name "${MIRROR}" --user "${user}" \
    -e MC_CONFIG_DIR=/tmp/.mc -e MC_HOST_local=http://birdtest:birdtestbirdtest@minio:9000 \
    -v "${artifacts}:/in:ro" --entrypoint /bin/sh minio-init -c '
      set -e
      mc mb --ignore-existing local/birdtest-artifacts >/dev/null
      exec mc mirror --overwrite --remove /in local/birdtest-artifacts
    ' || mirrored=$?
  docker rm -f "${MIRROR}" >/dev/null 2>&1 || true
  case "${mirrored}" in
    0) ;;
    129 | 130 | 143)
      echo "the artifact mirror was stopped: the database is restored, the bucket only partly; run the restore again" >&2
      status=1 ;;
    *)
      echo "the artifact bucket was not restored (the database was): see mc's message above, and run the restore again" >&2
      status=1 ;;
  esac
fi

restart
trap - INT TERM HUP
if (( status )); then
  echo "restored the database from ${SRC}, but not the artifact bucket" >&2
else
  echo "restored from ${SRC}"
fi
exit "${status}"
