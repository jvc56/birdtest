#!/usr/bin/env bash
#
# Every ```bash block in RUNBOOK.md parses, checked without running anything.
#
#   ./scripts/runbook-check.sh
#
# An operator pastes these blocks during an incident; one that does not parse
# leaves the shell at a continuation prompt that swallows what is pasted next.
# An apostrophe in a `${VAR:?message}` did exactly that to the restore's
# backup-settings command (thirty-second audit).

set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNBOOK="${1:-${HERE}/../RUNBOOK.md}"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

awk -v dir="${WORK}" '
  /^```bash[[:space:]]*$/ { n++; file = sprintf("%s/block%03d-line%d.sh", dir, n, NR + 1); inside = 1; next }
  /^```/ && inside { inside = 0; close(file); next }
  inside { print > file }
' "${RUNBOOK}"

blocks=0
failed=0
for block in "${WORK}"/block*.sh; do
  [ -e "${block}" ] || continue
  blocks=$((blocks + 1))
  if ! bash -n "${block}" 2> "${WORK}/err"; then
    failed=$((failed + 1))
    start="${block##*-line}"
    echo "RUNBOOK.md: the bash block starting at line ${start%.sh} does not parse:" >&2
    sed 's/^/  /' "${WORK}/err" >&2
  fi
done

if [ "${blocks}" -eq 0 ]; then
  echo "no bash blocks found in ${RUNBOOK}" >&2
  exit 1
fi
if [ "${failed}" -ne 0 ]; then
  exit 1
fi
echo "ok: ${blocks} bash blocks in RUNBOOK.md parse"
