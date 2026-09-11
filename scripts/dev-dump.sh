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

mkdir -p "${OUT}"

echo "dumping the database to ${OUT}/db.dump"
${COMPOSE} exec -T postgres pg_dump -U birdtest -Fc birdtest > "${OUT}/db.dump"

# The artifact bucket. MinIO ships mc in its own image, so this needs no host
# tooling either.
echo "mirroring the artifact bucket to ${OUT}/artifacts"
mkdir -p "${OUT}/artifacts"
${COMPOSE} run --rm --no-deps -T -v "$(cd "${OUT}" && pwd)/artifacts:/out" \
  --entrypoint /bin/sh minio-init -c '
    mc alias set local http://minio:9000 birdtest birdtestbirdtest >/dev/null
    mc mirror --overwrite local/birdtest-artifacts /out
  '

echo "snapshot ${NAME} written to ${OUT}"
