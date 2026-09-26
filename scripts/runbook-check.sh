#!/usr/bin/env bash
#
# Every ```bash block in RUNBOOK.md parses, checked without running anything.
#
#   ./scripts/runbook-check.sh [file]
#
# An operator pastes these blocks during an incident; one that does not parse
# leaves the shell at a continuation prompt that swallows what is pasted next.
# An apostrophe in a `${VAR:?message}` did exactly that to the restore's
# backup-settings command (thirty-second audit).
#
# Every fence is parsed, not pattern-matched: each opener (``` or ~~~, of any
# length, at any indent) must be labelled exactly `bash` or `sql`, and anything
# else -- `Bash`, `sh`, `bash title="x"`, an unlabelled fence, a fence inside a
# quote -- is refused rather than skipped, since a block that is not read is a
# block that is not checked. A bash block is read with its fence's indent
# removed and must parse; a fence never closed is refused. A placeholder must
# parse: `X=''  # what goes here`. A block that calls `aws` must turn its pager
# off.

set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNBOOK="${1:-${HERE}/../RUNBOOK.md}"
NAME="$(basename "${RUNBOOK}")"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

awk -v dir="${WORK}" -v name="${NAME}" '
  function refuse(msg) { printf "%s:%d: %s\n", name, NR, msg > "/dev/stderr"; bad = 1 }
  # The length of the run of character c at the start of s.
  function run(s, c,   n) { n = 0; while (substr(s, n + 1, 1) == c) n++; return n }
  !inside && /^[ \t>]*(```|~~~)/ {
    prefix = $0; sub(/(```|~~~).*$/, "", prefix)
    rest = substr($0, length(prefix) + 1)
    fence = substr(rest, 1, 1); flen = run(rest, fence)
    info = substr(rest, flen + 1); sub(/^[ \t]+/, "", info); sub(/[ \t]+$/, "", info)
    inside = 1; indent = length(prefix); start = NR + 1; kind = info; file = ""
    if (prefix ~ />/) {
      refuse("a fence inside a quote; runbook blocks are not quoted")
    } else if (info == "bash") {
      n++; file = sprintf("%s/block%03d-line%d.sh", dir, n, start); printf "" > file
    } else if (info != "sql") {
      refuse("a fence labelled \"" info "\": label a shell block exactly bash (or sql), so this check reads it")
    }
    next
  }
  inside {
    t = $0; sub(/^[ \t]*/, "", t)
    m = run(t, fence)
    if (m >= flen && substr(t, m + 1) ~ /^[ \t]*$/) { inside = 0; if (file != "") close(file); next }
    if (file != "") {
      line = $0; k = 0
      while (k < indent && substr(line, 1, 1) ~ /[ \t]/) { line = substr(line, 2); k++ }
      print line > file
    }
  }
  END {
    if (inside) { printf "%s: the %s block starting at line %d is never closed\n", name, kind, start > "/dev/stderr"; bad = 1 }
    exit bad
  }
' "${RUNBOOK}"

blocks=0
failed=0
for block in "${WORK}"/block*.sh; do
  [ -e "${block}" ] || continue
  blocks=$((blocks + 1))
  if ! bash -n "${block}" 2> "${WORK}/err"; then
    failed=$((failed + 1))
    start="${block##*-line}"
    echo "${NAME}: the bash block starting at line ${start%.sh} does not parse:" >&2
    sed 's/^/  /' "${WORK}/err" >&2
  fi
  # AWS CLI v2 pages long output through `less`, which reads the rest of a
  # pasted block as keystrokes: a restore's `wait` never ran (pass 13). Every
  # block that calls it turns the pager off itself, since a block may be the
  # first one pasted.
  if grep -v '^[[:space:]]*#' "${block}" | grep -qE '(^|[;&|( ]|\$\()aws ' \
      && ! grep -q '^export AWS_PAGER=""' "${block}"; then
    failed=$((failed + 1))
    start="${block##*-line}"
    echo "${NAME}: the bash block starting at line ${start%.sh} calls aws without export AWS_PAGER=\"\" first" >&2
  fi
done

if [ "${blocks}" -eq 0 ]; then
  echo "no bash blocks found in ${RUNBOOK}" >&2
  exit 1
fi
if [ "${failed}" -ne 0 ]; then
  exit 1
fi
echo "ok: ${blocks} bash blocks in ${NAME} parse"
