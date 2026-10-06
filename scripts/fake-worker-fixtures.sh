#!/usr/bin/env bash
# Re-emits the fake worker's captured submissions in backend/src/jobs/testdata/
# (fake_worker_*.json) from worker/fake_worker.py and the contract fixtures --
# see the README there. The fixtures pin what the fake sends, so the server's
# tests fail when the two drift apart; that only works while they are what the
# fake sends *now*, which is what --check (run by CI) holds them to.
#
#   scripts/fake-worker-fixtures.sh           rewrite them in place
#   scripts/fake-worker-fixtures.sh --check   emit to a scratch directory and
#                                             fail naming each that differs
#
# Needs only python3: --emit-fixture contacts no server.
set -euo pipefail
cd "$(dirname "$0")/.."

T=backend/src/jobs/testdata
F=contract-fixtures
out=$T
check=0
if [[ "${1:-}" == --check ]]; then
  check=1
  out=$(mktemp -d)
  trap 'rm -rf "$out"' EXIT
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
fi

emit() {
  local name=$1
  shift
  python3 worker/fake_worker.py "$@" >"$out/$name"
}
# A pairs request is the games one under the other tag, which is all the fake
# reads to tell them apart.
PAIRS=(--override 'job_type="game_pairs"')

emit fake_worker_games.json --emit-fixture $F/assignment-games.json
# One game, captured, its player 1 (a simmer) inferring: simulated positions
# with inferences on player 1's turns, static ones on player 2's.
emit fake_worker_games_captured.json --emit-fixture $F/assignment-games.json \
  --override capture_positions=true --override num_games=1 --override player1.use_inference=true
emit fake_worker_game_pairs.json --emit-fixture $F/assignment-games.json "${PAIRS[@]}"
# The pairs assignment as captured: two static players keeping first divergences.
emit fake_worker_game_pairs_divergence.json --emit-fixture $F/assignment-game-pairs.json
emit fake_worker_opening_rack.json --emit-fixture $F/assignment-opening-rack.json
emit fake_worker_leave_generation.json --emit-fixture $F/assignment-leave-generation.json

emit fake_worker_malformed_games.json --mode malformed --emit-fixture $F/assignment-games.json
emit fake_worker_malformed_game_pairs.json --mode malformed --emit-fixture $F/assignment-games.json "${PAIRS[@]}"
emit fake_worker_malformed_opening_rack.json --mode malformed --emit-fixture $F/assignment-opening-rack.json
emit fake_worker_malformed_leave_generation.json --mode malformed --emit-fixture $F/assignment-leave-generation.json
emit fake_worker_stale.json --mode stale --emit-fixture $F/assignment-games.json
emit fake_worker_abandon.json --mode abandon --emit-fixture $F/assignment-games.json

if ((check)); then
  stale=0
  for emitted in "$out"/*.json; do
    committed=$T/$(basename "$emitted")
    if ! cmp -s "$emitted" "$committed"; then
      echo "stale: $committed is not what worker/fake_worker.py emits now" >&2
      stale=1
    fi
  done
  if ((stale)); then
    echo "regenerate with scripts/fake-worker-fixtures.sh, and commit the result" >&2
    exit 1
  fi
  echo "every fake-worker fixture is what the fake emits"
fi
