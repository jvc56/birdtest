#!/usr/bin/env bash
# Builds the tier-5 input-data fixture: MAGPIE-DATA tarballs small enough to
# commit, and the static tree the end-to-end stack's `fixtures` service serves
# in place of GitHub (docker-compose.e2e.yml). See TESTING.md, "The fixture
# tarball".
#
#   fixtures/build.sh
#
# The tarballs are cut the way MAGPIE-DATA cuts the real ones -- `cp -RL` the
# data directory, `tar -czf`, and `split` for a chunked version -- so an import
# walks chunks and extracts an archive for real rather than being handed bytes
# some other way. The one addition is what makes the output reproducible
# (fixed mtimes, owners and order), so re-running this does not dirty git.
#
# THE LEXICA HERE ARE STUBS. `lexica/NWL23.kwg` is not NWL23: it is named so
# because compat.rs rejects unknown lexicon names, and it is never read below
# tier 6. Any code that loads it is broken by construction. The distribution is
# `english_fixture` (english-prefixed, so it pairs with NWL23) and is the
# 13-tile bag the leave tests use, not English.
#
# Each version is a directory under versions/ whose files are mostly symlinks
# into the repository, which `cp -RL` dereferences:
#
#   20260101  the version the suite seeds from. One file.
#   20260201  one file re-cut under new bytes and one new one, so importing it
#             after 20260101 stages a diff with every disposition in it. Split
#             into chunks, the way MAGPIE-DATA ships a large version.
set -euo pipefail

cd "$(dirname "$0")"

# The repository and commit the e2e stack pretends to be. MAGPIE_DATA_REPO in
# docker-compose.e2e.yml must match REPO; SHA is what `main` resolves to.
REPO=birdtest/fixtures
SHA=e2e0000000000000000000000000000000000001

# version  chunk size in bytes (0: a single file)
VERSIONS=(
  "20260101 0"
  "20260201 400"
)

api="github/api/repos/$REPO/commits"
raw="github/raw/$REPO/$SHA/versioned-tarballs"

rm -rf github data-*.tgz data-*.tgz.??
mkdir -p "$api" "$raw"
# `GET /repos/<repo>/commits/<ref>` with `Accept: application/vnd.github.sha`
# answers the bare sha. Only `main` exists; any other ref is a 404, as it is
# on GitHub.
echo "$SHA" > "$api/main"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for entry in "${VERSIONS[@]}"; do
  read -r date chunk <<<"$entry"
  rm -rf "$work/data"
  cp -RL "versions/$date" "$work/data"
  (
    cd "$work"
    tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner --mode=u=rwX,go=rX \
      -czf "data-$date.tgz" data
  )
  if [ "$chunk" -gt 0 ]; then
    split -b "$chunk" "$work/data-$date.tgz" "data-$date.tgz."
    outputs=(data-"$date".tgz.??)
  else
    cp "$work/data-$date.tgz" .
    outputs=("data-$date.tgz")
  fi
  # The served copies are links back to these files, so the tree and the
  # committed tarballs cannot disagree.
  for file in "${outputs[@]}"; do
    ln -s "../../../../../../$file" "$raw/$file"
    echo "$file $(wc -c <"$file") bytes"
  done
done
