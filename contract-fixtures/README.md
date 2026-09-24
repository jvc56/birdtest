# Worker API contract fixtures

The worker API is a cross-repo boundary: birdtest serves it, MAGPIE's
`contribute` command speaks it, and the two are released independently. Nothing
else pins the contract — it exists implicitly in
[`routes/worker.rs`](../backend/src/routes/worker.rs) and MAGPIE's
`src/impl/contribute.c` agreeing.

These files are the cheap version of fixing that: one committed example of each
message either side has to produce or read. Copy them into both repositories
and have each side's tests parse them. A field renamed on one side and not the
other then fails a test rather than a contributor's run.

| File | Direction | What it pins |
|---|---|---|
| `claim-request.json` | client → server | The required claim body: version and unsupported set |
| `assignment-games.json` | server → client | A games assignment with `expected_data`, two lexicons |
| `assignment-opening-rack.json` | server → client | A rack batch for a simming player: the one job type whose request carries `racks` and a single `player` |
| `assignment-leave-generation.json` | server → client | Generation 1, reading the server-built zeroed KLV |
| `decline-missing-data.json` | client → server | A decline naming a missing file and a mismatched one |
| `shutdown-data-out-of-date.json` | server → client | Every job unreachable because the data is stale |
| `shutdown-magpie-too-old.json` | server → client | Every job unreachable because the build is old |
| `shutdown-both.json` | server → client | Both, leading with the MAGPIE version |
| `assignment-game-pairs.json` | server → client | A `game_pairs` assignment (`game_pairs: true`) whose players ask for a wordmap |
| `anon-uuid-assignment.json` | server → client | The first assignment of a worker with no identity: a games task carrying the minted `worker_uuid`, and a job that pins no derived file (`derived: []`) |
| `expected-data.json` | server → client | The `expected_data` digest list of an assignment, input files and a derived wordmap |
| `heartbeat.json` | client → server | A heartbeat |
| `result-games.json` | client → server | A games result with `capture_positions` on: the tally and every captured position |
| `result-game-pairs.json` | client → server | A pairs result: the tally, the pentanomial and the divergent subset |
| `result-opening-rack.json` | client → server | A simulating player's rack analyses, with win%, blended utility and per-ply statistics |
| `result-leave-generation.json` | client → server | Every rack a leave-generation task saw, on MAGPIE's two-letter test distribution |

The digests here are the real ones from `data-20251004.tgz`, so a fixture that
stops matching what an import produces is itself a signal.

## Capturing

The first nine were written by hand. The rest were **captured from a real
exchange**: `scripts/capture_contract.py` is a recording proxy that sits
between `magpie contribute` and the backend, forwards everything unchanged, and
writes the first body of each message type here. Nothing is normalised --
tokens, ids and timings are the ones that crossed the wire, so a recapture
changes them -- and the JSON is pretty-printed with two-space indentation.

Recapture with a real stack and a real MAGPIE (a `portable_release` build and a
`download_data.sh` install):

    MAGPIE_ROOT=../MAGPIE scripts/e2e_magpie_native.sh --cases capture --capture-out contract-fixtures

or, against a compose stack already up, `python3 scripts/e2e_magpie.py --magpie
../MAGPIE/bin/magpie --magpie-root ../MAGPIE --cases capture --capture-out
contract-fixtures --github-fixture-port 8481`, with the backend's
`GITHUB_API_URL`/`GITHUB_RAW_URL` pointing at that port (the leave-generation
result runs on the small test distribution that script serves). The `capture`
case creates one job of each type, runs a contributor through the proxy, and
fails unless every fixture above was captured. Then copy the directory into
MAGPIE's `test/birdtest_contract/` in the same change.

`backend/src/routes/worker.rs` (`mod contract_fixtures`) decodes each
client → server fixture into the type that handles it and runs a result through
its job type's validation, as a submission would be; server → client fixtures
are compared with what the wire types serialize by field structure. MAGPIE's
`test/contribute_test.c` checks the other half: every key its client reads is
in the assignments, and its result serializers produce every key in the
results.
