#!/usr/bin/env bash
#
# dev-restore.sh's SCRUB rule, checked without Docker: the script is run
# against a stub COMPOSE that only records what it is asked to do.
#
#   ./scripts/dev-restore-check.sh
#
# Unset, empty and 1 scrub -- the copy the run restores into; 0 does not;
# anything else is refused before a single compose call. `SCRUB=yes` used to skip the scrub, restoring a
# production dump's real addresses and password hashes (thirty-second audit).

set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

mkdir -p "${WORK}/dump"
: > "${WORK}/dump/db.dump"
cat > "${WORK}/compose" <<'STUB'
#!/usr/bin/env bash
echo "$*" >> "${STUBLOG}"
cat > /dev/null
STUB
chmod +x "${WORK}/compose"

failed=0
check() {
  local label="$1" want="$2"; shift 2
  local log="${WORK}/log.${RANDOM}${RANDOM}"
  : > "${log}"
  local status=0
  env "$@" STUBLOG="${log}" COMPOSE="${WORK}/compose" \
    bash "${HERE}/dev-restore.sh" "${WORK}/dump" < /dev/null > /dev/null 2>&1 || status=$?
  local scrubs calls got
  scrubs=$(grep -c "dev_copy=1" "${log}" || true)
  calls=$(wc -l < "${log}")
  if (( status == 2 && calls == 0 )); then
    got=refused
  elif (( status == 0 && scrubs == 1 )); then
    # The copy this run created, not the stack's database, which would have
    # swapped an unscrubbed dump in.
    local copy
    copy=$(grep -o "CREATE DATABASE birdtest_restore_[0-9_]*" "${log}" | awk '{print $3}')
    if [[ -n "${copy}" ]] && grep -q -- "-d ${copy} .*dev_copy=1" "${log}"; then
      got=scrubbed
    else
      got="scrubbed, but not the copy (${copy:-none created})"
    fi
  elif (( status == 0 && scrubs == 0 )); then
    got=kept
  else
    got="exit ${status}, ${calls} calls, ${scrubs} scrubs"
  fi
  if [[ "${got}" == "${want}" ]]; then
    echo "ok   ${label}: ${got}"
  else
    echo "FAIL ${label}: ${got}, wanted ${want}" >&2
    failed=1
  fi
}

check "SCRUB unset" scrubbed -u SCRUB
check "SCRUB=''" scrubbed SCRUB=
check "SCRUB=1" scrubbed SCRUB=1
check "SCRUB=0" kept SCRUB=0
for value in yes true TRUE no off 2 00 " 0" "1 "; do
  check "SCRUB='${value}'" refused "SCRUB=${value}"
done

if (( failed )); then
  echo "dev-restore check FAILED" >&2
  exit 1
fi
echo "dev-restore check passed"
