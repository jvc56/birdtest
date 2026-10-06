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

`scripts/fake-worker-fixtures.sh` regenerates every one, and is the one list of
how each is made: which assignment, which `--mode`, which `--override`s. CI
runs it with `--check`, which emits them to a scratch directory and fails on
any that differs from the committed file, so a change to the fake cannot leave
them behind. After changing `fake_worker.py` (or a contract fixture), from the
repository root:

```sh
scripts/fake-worker-fixtures.sh
```

The `game_pairs` fixtures come from two assignments: the games one under the
`game_pairs` tag (`--override job_type='"game_pairs"'`), whose players are a
simmer and a static player, and `contract-fixtures/assignment-game-pairs.json`
itself, the captured pairs request -- two static players keeping first
divergences -- for `fake_worker_game_pairs_divergence.json`. Each position is
shaped by the player on turn, as MAGPIE's are: simulated for a simmer, static
otherwise.

What each mode prints (see `emit_fixture` in the script):

| Mode | Output |
|---|---|
| `normal` | the `result` a worker posts |
| `malformed` | `{variant: result}` for every corruption in `CORRUPTIONS`, not the one a run draws at random |
| `stale` | the whole body, `{"claim_token", "result"}`, under a token the assignment did not issue |
| `abandon` | `null`: the mode never submits |

`fake_worker_games_captured.json` is also read by the frontend's
`cgp.test.ts` (`F-CGP-4`), which checks that every position in it is a board
the saved-positions page can draw, each following from the one before.

Regenerating changes the numbers only if `fake_worker.py`'s random draws
change. A test that then fails on a *value* (a rack, a count) is expected to be
updated alongside; one that fails on *validation* is a fake-worker bug.
