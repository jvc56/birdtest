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
rm -rf "${PARTIAL}"
mkdir -p "${PARTIAL}"

echo "dumping the database"
${COMPOSE} exec -T postgres pg_dump -U birdtest -Fc birdtest > "${PARTIAL}/db.dump"

# The artifact bucket. MinIO ships mc in its own image, so this needs no host
# tooling either.
echo "mirroring the artifact bucket"
mkdir -p "${PARTIAL}/artifacts"
${COMPOSE} run --rm --no-deps -T -v "$(cd "${PARTIAL}" && pwd)/artifacts:/out" \
  --entrypoint /bin/sh minio-init -c '
    mc alias set local http://minio:9000 birdtest birdtestbirdtest >/dev/null
    mc mirror --overwrite local/birdtest-artifacts /out
  '

rm -rf "${OUT}"
mv "${PARTIAL}" "${OUT}"
echo "snapshot ${NAME} written to ${OUT}"
