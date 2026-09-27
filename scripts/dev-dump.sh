#!/usr/bin/env bash
#
# Snapshot the local docker compose stack — database and object store — so an
# experiment that wrecks it is cheap to undo. Restore with dev-restore.sh.
#
#   ./scripts/dev-dump.sh [name]
#
# This is a development convenience, not the backup path: production backups
# are scripts/backup.sh, running as a scheduled task. See PLAN.md, "Local development".

set -Eeuo pipefail

NAME="${1:-$(date -u +%Y%m%d-%H%M%S)}"
OUT="${DEV_BACKUP_DIR:-.dev-backups}/${NAME}"
COMPOSE="${COMPOSE:-docker compose}"

# Written aside and moved into place only when complete: dumping over an
# existing snapshot while postgres was down left it empty, and one cut off
# part-way left a partial dump that emptied the database when restored (the
# audit's pass 23). An existing name is refused unless FORCE=1.
if [[ -e "${OUT}" && "${FORCE:-0}" != 1 ]]; then
  echo "${OUT} exists; choose another name, or FORCE=1 to replace it" >&2
  exit 1
fi
PARTIAL="${OUT}.partial"
MIRROR="birdtest-dump-mirror-$$"
rm -rf "${PARTIAL}"
mkdir -p "${PARTIAL}"
# A run that fails or is stopped leaves nothing behind: its partial copy used
# to stay, beside the snapshots, under a name dev-restore.sh would take. The
# mirror's container is removed too: stopping the compose CLI leaves it running.
discard() {
  docker rm -f "${MIRROR}" >/dev/null 2>&1 || true
  rm -rf "${PARTIAL}"
}
trap discard ERR
trap 'discard; exit 130' INT TERM HUP

echo "dumping the database"
${COMPOSE} exec -T postgres pg_dump -U birdtest -Fc birdtest > "${PARTIAL}/db.dump"

# The artifact bucket. MinIO ships mc in its own image, so this needs no host
# tooling either. As the host's user: the image's own (uid 65532) could not
# write into the directory, and mc wrote nothing and exited 0 -- every
# snapshot's bucket was empty on Linux, and restoring one emptied the bucket
# (the audit's pass 24). So the objects are counted as well.
echo "mirroring the artifact bucket"
mkdir -p "${PARTIAL}/artifacts"
${COMPOSE} run --rm --no-deps -T --name "${MIRROR}" --user "$(id -u):$(id -g)" \
  -e MC_CONFIG_DIR=/tmp/.mc -e MC_HOST_local=http://birdtest:birdtestbirdtest@minio:9000 \
  -v "$(cd "${PARTIAL}" && pwd)/artifacts:/out" --entrypoint /bin/sh minio-init -c '
    set -e
    mc mirror --overwrite local/birdtest-artifacts /out
    want=$(mc ls -r local/birdtest-artifacts | wc -l)
    got=$(find /out -type f | wc -l)
    [ "$want" = "$got" ] || { echo "mirrored $got of $want objects" >&2; exit 1; }
  '

# Moved aside before the new one goes in, and removed after: a stop while an
# old snapshot was being removed took the new one with it.
trap - ERR INT TERM HUP
[[ -e "${OUT}" ]] && mv "${OUT}" "${OUT}.old"
mv "${PARTIAL}" "${OUT}"
rm -rf "${OUT}.old" 2>/dev/null \
  || echo "the replaced snapshot is left at ${OUT}.old (files an older mirror wrote as another user: remove them with sudo)" >&2
echo "snapshot ${NAME} written to ${OUT}"
