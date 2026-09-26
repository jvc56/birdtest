#!/usr/bin/env bash
#
# Every ```bash block in RUNBOOK.md (or each file named) parses, checked
# without running anything.
#
#   ./scripts/runbook-check.sh [file...]
#
# An operator pastes these blocks during an incident; one that does not parse
# leaves the shell at a continuation prompt that swallows what is pasted next.
# An apostrophe in a `${VAR:?message}` did exactly that to the restore's
# backup-settings command (thirty-second audit).
#
# Every fenced block is parsed, not pattern-matched: each opener (``` or ~~~,
# of any length, at any indent) must be labelled exactly `bash`, `sql` or
# `text`, and anything else -- `Bash`, `sh`, `bash title="x"`, an unlabelled
# fence, a fence after a quote mark, a list marker or any other text on its
# line -- is refused
# rather than skipped, since a block that is not read is a block that is not
# checked. A bash block is read with its fence's indent removed and must parse;
# a fence never closed is refused. A placeholder must parse: `X=''  # what goes
# here`. A block that mentions `aws` must turn the pager off in its first line
# of code. What it cannot see: an indented (unfenced) code block or `<pre>`, a
# shell block labelled `text`, and a block that turns the pager back on.

set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "$#" -eq 0 ]; then
  set -- "${HERE}/../RUNBOOK.md"
fi
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

check() {
  local file="$1" name blocks=0 failed=0 block start first
  name="$(basename "${file}")"
  rm -f "${WORK}"/block*.sh
  if ! awk -v dir="${WORK}" -v name="${name}" '
    function refuse(msg) { printf "%s:%d: %s\n", name, NR, msg > "/dev/stderr"; bad = 1 }
    # The length of the run of character c at the start of s.
    function run(s, c,   n) { n = 0; while (substr(s, n + 1, 1) == c) n++; return n }
    # A label as it would print, a CR or tab made visible.
    function shown(s) { gsub(/\r/, "\\r", s); gsub(/\t/, "\\t", s); return s }
    # A fence after anything but blanks -- a list marker, a quote, a nested
    # list, prose -- is refused rather than guessed at: each opened a block
    # a renderer shows and this check did not read (passes 13 and 14).
    !inside && /(```|~~~)/ && !/^[ \t]*(```|~~~)/ {
      refuse("a fence run after other text on its line; start a block on a line of its own")
      next
    }
    !inside && /^[ \t]*(```|~~~)/ {
      prefix = $0; sub(/(```|~~~).*$/, "", prefix)
      rest = substr($0, length(prefix) + 1)
      fence = substr(rest, 1, 1); flen = run(rest, fence)
      info = substr(rest, flen + 1); sub(/^[ \t]+/, "", info); sub(/[ \t]+$/, "", info)
      inside = 1; indent = length(prefix); start = NR + 1; kind = info; file = ""
      if (info == "bash") {
        n++; file = sprintf("%s/block%03d-line%d.sh", dir, n, start); printf "" > file
      } else if (info != "sql" && info != "text") {
        refuse("a fence labelled \"" shown(info) "\": label a shell block exactly bash (others sql or text), so this check reads it")
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
      if (inside) { printf "%s: the %s block starting at line %d is never closed\n", name, shown(kind), start > "/dev/stderr"; bad = 1 }
      exit bad
    }
  ' "${file}"; then
    return 1
  fi

  for block in "${WORK}"/block*.sh; do
    [ -e "${block}" ] || continue
    blocks=$((blocks + 1))
    start="${block##*-line}"
    start="${start%.sh}"
    if ! bash -n "${block}" 2> "${WORK}/err"; then
      failed=$((failed + 1))
      echo "${name}: the bash block starting at line ${start} does not parse:" >&2
      sed 's/^/  /' "${WORK}/err" >&2
    fi
    # AWS CLI v2 pages long output through `less`, which reads the rest of a
    # pasted block as keystrokes: a restore's `wait` never ran (pass 13). A
    # block that so much as mentions `aws` outside a comment line turns the
    # pager off in its first line of code, since a block may be the first one
    # pasted and an export after a call is too late. (A mention in an `echo`
    # costs a harmless export.) No pipes: one closed early under pipefail
    # passed a large block.
    if awk '!/^[ \t]*#/ && /(^|[^[:alnum:]_.])aws([ \t`)}"'"'"']|\\?$)/ { found = 1; exit } END { exit !found }' "${block}"; then
      first="$(awk '!/^[ \t]*(#|$)/ { sub(/^[ \t]+/, ""); sub(/[ \t]+#.*$/, ""); sub(/[ \t]+$/, ""); print; exit }' "${block}")"
      if [ "${first}" != 'export AWS_PAGER=""' ] && [ "${first}" != "export AWS_PAGER=''" ]; then
        failed=$((failed + 1))
        echo "${name}: the bash block starting at line ${start} mentions aws; its first line of code must be export AWS_PAGER=\"\"" >&2
      fi
    fi
  done

  if [ "${blocks}" -eq 0 ]; then
    echo "no bash blocks found in ${file}" >&2
    return 1
  fi
  if [ "${failed}" -ne 0 ]; then
    return 1
  fi
  echo "ok: ${blocks} bash blocks in ${name} parse"
}

status=0
for file in "$@"; do
  check "${file}" || status=1
done
exit "${status}"
