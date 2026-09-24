# Unit-test data for `jobs`

Read by `include_bytes!` / `include_str!`, so unit tests never touch the
filesystem at run time.

## Letter distributions

- `english.csv` — MAGPIE's `data/letterdistributions/english.csv`, copied
  verbatim. Its sha256 is the one `contract-fixtures/` pins for
  `data-20251004.tgz`, and `racks::tests` checks that it still is.
- `catalan.csv` — MAGPIE's `data/letterdistributions/catalan.csv`, copied
  verbatim: the shipped distribution with multi-character letters (`L·L`,
  `NY`, `QU`), which parses but has no rack space here.
- `testdist.csv` — a small hand-written distribution with a blank and tile
  counts of 1 to 3, small enough to enumerate exhaustively.

## Captured fake-worker submissions (`fake_worker_*.json`)

Output of `worker/fake_worker.py --emit-fixture`, never written or edited by
hand: the point is to pin what the fake worker actually sends, so the server's
tests (`plausibility::fixture_tests`) fail when the two drift apart. The emit
mode builds the submission through the same function a running worker uses, for
worker 0 under the default `--seed`, against a claim response from
`contract-fixtures/`, and prints it without contacting a server.

Regenerate every one, from the repository root:

```sh
T=backend/src/jobs/testdata; F=contract-fixtures; W="python3 worker/fake_worker.py"
PAIRS="--override job_type=\"game_pairs\" --override game_pairs=true"

$W --emit-fixture $F/assignment-games.json > $T/fake_worker_games.json
$W --emit-fixture $F/assignment-games.json \
   --override capture_positions=true --override num_games=1 > $T/fake_worker_games_captured.json
$W --emit-fixture $F/assignment-games.json $PAIRS > $T/fake_worker_game_pairs.json
$W --emit-fixture $F/assignment-opening-rack.json > $T/fake_worker_opening_rack.json
$W --emit-fixture $F/assignment-leave-generation.json > $T/fake_worker_leave_generation.json

$W --mode malformed --emit-fixture $F/assignment-games.json > $T/fake_worker_malformed_games.json
$W --mode malformed --emit-fixture $F/assignment-games.json $PAIRS > $T/fake_worker_malformed_game_pairs.json
$W --mode malformed --emit-fixture $F/assignment-opening-rack.json > $T/fake_worker_malformed_opening_rack.json
$W --mode malformed --emit-fixture $F/assignment-leave-generation.json > $T/fake_worker_malformed_leave_generation.json
$W --mode stale --emit-fixture $F/assignment-games.json > $T/fake_worker_stale.json
$W --mode abandon --emit-fixture $F/assignment-games.json > $T/fake_worker_abandon.json
```

There is no `game_pairs` assignment fixture; the pairs request is the games one
with its `job_type` and `game_pairs` overridden, which is all that differs.

What each mode prints (see `emit_fixture` in the script):

| Mode | Output |
|---|---|
| `normal` | the `result` a worker posts |
| `malformed` | `{variant: result}` for every corruption in `CORRUPTIONS`, not the one a run draws at random |
| `stale` | the whole body, `{"claim_token", "result"}`, under a token the assignment did not issue |
| `abandon` | `null`: the mode never submits |

Regenerating changes the numbers only if `fake_worker.py`'s random draws
change. A test that then fails on a *value* (a rack, a count) is expected to be
updated alongside; one that fails on *validation* is a fake-worker bug.
