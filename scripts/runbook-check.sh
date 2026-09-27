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
# of code. What it cannot see: an indented (unfenced) code block, `<pre>` or
# an HTML comment (and a fence inside one throws off how the fences after it
# pair, so a later block can go unread), a shell block labelled `text`, code
# after a ` #` inside quotes (read as a comment by the ASCII rule), a block
# that turns the pager back on, and a `!` inside double quotes, which an
# interactive shell expands as history and `bash -n` does not. A `\ #`
# anywhere -- in quotes, a word, a heredoc or a comment -- is refused as a
# continuation it is not (fails safe). A fence indented four columns or more
# is read as one, though a renderer outside a list shows it as indented code.
# And bash 3.2 (macOS's /bin/bash) does not warn of a heredoc never
# terminated; CI's bash does.

set -Eeuo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "$#" -eq 0 ]; then
  set -- "${HERE}/../RUNBOOK.md"
fi
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

check() {
  local file="$1" name blocks=0 failed=0 block start first bad
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
      lead = $0; sub(/[^ \t].*$/, "", lead)
      # A line of an indented block less indented than its fence -- the
      # closing fence too -- ends the list item it sits in, and a renderer
      # closes the block there: read on, this check swallowed what came next,
      # and taken as a closer, a stray fence opened an unlabelled block over
      # the next one (pass 15).
      if (indent > 0 && t != "" && length(lead) < indent) {
        refuse("a line less indented than its block'"'"'s fence (line " start - 1 "); indent the block'"'"'s every line, its closing fence too")
        inside = 0; if (file != "") close(file); next
      }
      # A closer indented past its opener is refused below: a renderer
      # measures its indent from the page or the list item, not the opener,
      # and may read it as content.
      if (m >= flen && substr(t, m + 1) ~ /^[ \t]*$/ && length(lead) <= indent) {
        inside = 0; if (file != "") close(file); next
      }
      # A fence at least as long as the opener that does not close the block
      # -- one indented four past it, or labelled -- is shown as text inside
      # it, and in bash is a run of backquotes. (A shorter one is how a block
      # quotes a fence, and is left alone.)
      if (m >= flen) {
        refuse("a fence inside a block that does not close it (line " start - 1 "): indent a closer as its opener, or open the block with a longer fence")
        inside = 0; if (file != "") close(file); next
      }
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
    # `bash -n` passes two shapes a paste never finishes: a heredoc whose
    # terminator never matches (an indented `EOF` -- the twenty-fourth audit's)
    # only warns, and a backslash ending the last line says nothing. Any
    # output from it, or that backslash, fails the block.
    bad=''
    # An odd run of backslashes ends the line in a continuation; an even
    # run is a backslash, escaped.
    if awk 'NF { last = $0 } END { n = 0; while (n < length(last) && substr(last, length(last) - n, 1) == "\\") n++; exit !(n % 2) }' "${block}"; then
      echo "its last line ends in a backslash, which waits for another line" > "${WORK}/err"
      bad=yes
    elif awk '
        # A continuation into a blank or comment line: bash ends the command
        # there, and the line after runs on its own -- an option dropped.
        cont && /^[ \t]*(#|$)/ { print "line " NR ": a continuation into a blank or comment line"; bad = 1 }
        { n = 0; while (n < length($0) && substr($0, length($0) - n, 1) == "\\") n++; cont = n % 2 }
        END { exit !bad }
      ' "${block}" > "${WORK}/err"; then
      bad=yes
    elif LC_ALL=C grep -nE '(^|[^\\])(\\\\)*\\[[:blank:]]+(#.*)?$' "${block}" > "${WORK}/err"; then
      # A backslash then blanks -- and perhaps a comment -- escapes a blank,
      # not the newline: the line after it ran as a command of its own.
      LC_ALL=C sed 's/^\([0-9]*\):.*/line \1: a backslash followed by blanks, not a continuation/' \
        "${WORK}/err" > "${WORK}/err2" && mv "${WORK}/err2" "${WORK}/err"
      bad=yes
    elif LC_ALL=C awk '
        # Code, not comments, must be printable ASCII: a no-break space after
        # a backslash, a CR or a smart quote reads as text to the eye and not
        # to bash. (A comment may say what it likes.)
        { code = $0; sub(/(^|[ \t])#.*$/, "", code) }
        code ~ /[^ -~\t]/ { print "line " NR ": a byte in code that is not printable ASCII"; bad = 1 }
        END { exit !bad }
      ' "${block}" > "${WORK}/err"; then
      bad=yes
    elif ! bash -n "${block}" 2> "${WORK}/err" || [ -s "${WORK}/err" ]; then
      bad=yes
    fi
    if [ -n "${bad}" ]; then
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
