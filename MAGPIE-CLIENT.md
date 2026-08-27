# Bringing the birdtest Client into MAGPIE

## Goal

A contributor should need **only MAGPIE**. No Python, no Docker, no separate
worker script. Today's client is a Python program that shells out to MAGPIE and
parses its stdout; this document specifies what MAGPIE needs to gain so that
the client becomes a MAGPIE command instead.

The immediate target is a command:

```
magpie> contribute https://birdtest.example -apikey bt_...
```

The eventual target is a MAGPIE GUI button that calls the same code path, which
is why the design below runs asynchronously and exposes machine-readable status
rather than assuming a terminal.

### A second reason to do this

The current client's fragility is almost entirely about **crossing the process
boundary**. It invokes `autoplay`, `gen` and `leavegen` as subprocesses and
reconstructs results by parsing human- or UCGI-formatted stdout. That is where
every integration bug found so far has lived, and one of them is not fixable
from outside: `autoplay` only ever *reports* aggregate statistics
(`autoplay games <n> <p0_wins> <p0_losses> ...`), while birdtest's schema stores
one record per game. Internally MAGPIE has exactly what birdtest wants —
`game_data_add_game()` receives the full `Game` and turn count for every single
game before aggregating them away. Running in-process makes the mismatch
disappear rather than requiring a workaround on either side.

---

## What the client does today

| Responsibility | Current implementation |
|---|---|
| Persistent worker identity | UUID generated on first run, stored in `~/.birdtest/worker_uuid` |
| Authentication | `Authorization: Bearer <api-key>`, or `X-Worker-UUID` when anonymous |
| Claim a task | `POST /api/worker/task` → JSON task request, or 204 when idle |
| Version gate | Skip the task if MAGPIE is older than the job's `min_magpie_version` |
| Execute | Dispatch on `job_type` to one of four handlers |
| Heartbeat | Background thread, `POST /api/worker/heartbeat` every 30s while working |
| Submit | `POST /api/worker/result` with the claim token and a JSON result |
| Artifact fetch | `GET /api/worker/artifact?key=...` for a previous generation's KLV |
| Backoff | Sleep on 204 and on 429 (`Retry-After`) |
| Self-update | Re-exec a newer copy of the script fetched from the server |

Four job types, each mapping to work MAGPIE already knows how to do:

| Job type | MAGPIE equivalent | Result birdtest stores |
|---|---|---|
| `opening_rack_analysis` | `gen` or `sim` on a CGP position | Ranked move list with per-ply sim stats |
| `games` | `autoplay games` from a seed | One record per game |
| `game_pairs` | `autoplay` with `-gp true` | Two records per pair, one per ordering |
| `leave_generation` | `leavegen` over a forced-rack subset | Per-rack occurrence counts and mean equity |

---

## What MAGPIE already has, and what is missing

| Capability | Status |
|---|---|
| HTTP requests | **Partial.** `src/impl/get_gcg.c` shells out to the `curl` *binary* through `get_process_output()` (a `popen` wrapper). No headers, no POST bodies, no status codes. |
| JSON | **No.** `get_gcg.c` scrapes with `strstr(response, "\"gcg\":\"")`. There is no parser and no serializer. |
| Threads | **Yes.** `src/compat/cpthread.h`, `src/ent/thread_control.h`. |
| Async command execution | **Yes.** `EXEC_MODE_ASYNC`, `src/compat/async_command_control.h`. |
| Command registration | **Yes.** `cmd()` / `arg()` in `config.c`. |
| Config persistence | **Partial.** `settings.txt` via `-savesettings`. No per-user state directory. |
| File path resolution | **Yes.** `data_filepaths.c`, colon-separated `-path` search list. |
| Structured error reporting | **Yes.** `ErrorStack` + `error_stack_push()`. |
| Per-game result capture | **Internally yes, externally no.** See above. |

---

## Design overview

Add one long-running command, `contribute`, that owns a claim → execute →
submit loop. Task execution calls MAGPIE's existing implementation functions
directly; nothing is serialized to stdout and re-parsed.

```
contribute
  ├── client_identity      load/create UUID + API key from the client state file
  ├── birdtest_api         typed wrappers over the five worker endpoints
  │     ├── http_client    POST/GET with headers, status codes, timeouts
  │     └── json           parse + serialize
  ├── heartbeat thread     runs for the lifetime of a claim
  └── task dispatch        one executor per job_type, each calling existing impls
        ├── opening rack → config_execute_gen / config_execute_sim
        ├── games        → autoplay with a per-game recorder
        ├── game pairs   → autoplay -gp with a per-game recorder
        └── leave gen    → leavegen over the forced-rack subset
```

---

## Required changes

### 1. HTTP client — `src/util/http_client.{c,h}` (new)

The worker API needs POST with a JSON body, request headers, response status
codes, and `Retry-After`. Shelling out to `curl` cannot supply these without
building shell command strings out of server-controlled values, which is both
fragile and an injection risk.

```c
typedef struct HttpResponse {
  long status_code;
  char *body;          // NUL-terminated, caller frees
  int retry_after_seconds;  // parsed from the header; -1 when absent
} HttpResponse;

HttpResponse *http_get(const char *url, const char *const *headers,
                       int num_headers, int timeout_seconds,
                       ErrorStack *error_stack);
HttpResponse *http_post_json(const char *url, const char *body,
                             const char *const *headers, int num_headers,
                             int timeout_seconds, ErrorStack *error_stack);
void http_response_destroy(HttpResponse *response);
```

**Implementation: OS-native backends, no new installable dependency.**

In practice this is two backends rather than three, because libcurl already
ships with macOS (`/usr/lib/libcurl.4.dylib`) and is present on essentially
every Linux desktop:

| Platform | Backend | Why |
|---|---|---|
| Linux / BSD / macOS | **libcurl**, against the system copy | Already present; no static build, no vendored TLS, no CA bundle to keep fresh - certificate trust stays the OS's problem |
| Windows | **WinHTTP** (`winhttp.dll`) | Ships with the OS, no redistributable, and the Schannel trust store is already configured |

The point is that a contributor installs MAGPIE and nothing else on every
platform, while MAGPIE never owns a TLS implementation or a certificate bundle.

Structure it as one header with two implementations selected by the existing
platform conventions in `src/compat/`:

```
src/util/http_client.h          // the interface above
src/util/http_client_curl.c     // #if !defined(_WIN32)
src/util/http_client_winhttp.c  // #if defined(_WIN32)
```

Two details worth deciding up front:

- **Prefer `dlopen`ing libcurl over link-time binding.** A minimal Linux install
  (a container, a headless server) can genuinely lack `libcurl.so.4`. Resolving
  it at runtime turns that from "MAGPIE will not start" into "MAGPIE runs, and
  `contribute` reports that libcurl is missing and names the package" - which
  also keeps every non-networking command working on a machine without it.
- **`Makefile-wasm` globs all of `src`**, so both files must compile under
  Emscripten. Guard them with `MAGPIE_NO_NETWORK`, which the WASM build defines,
  leaving stubs that push an error onto the stack.

Also replace the three `curl`-via-`popen` call sites in `get_gcg.c` with this
client, so there is one HTTP path rather than two.

### 2. JSON — `src/util/json.{c,h}` (new)

Needed for both directions: parsing task requests (nested objects, arrays of
player configs) and serializing results (arrays of per-game records, arrays of
rack occurrences with floating-point means).

Vendor a small permissively-licensed parser rather than writing one — cJSON
(MIT, one `.c` + one `.h`) is the usual choice and matches MAGPIE's existing
habit of vendoring (`src/compat/linenoise.c`). Wrap it in a thin MAGPIE-flavoured
API so the rest of the codebase sees `ErrorStack` rather than the vendor's error
conventions:

```c
JsonValue *json_parse(const char *text, ErrorStack *error_stack);
const JsonValue *json_object_get(const JsonValue *obj, const char *key);
bool json_get_bool(const JsonValue *v, const char *key, bool fallback);
int64_t json_get_int(const JsonValue *v, const char *key, ErrorStack *es);
double json_get_double(const JsonValue *v, const char *key, ErrorStack *es);
const char *json_get_string(const JsonValue *v, const char *key, ErrorStack *es);
// Serialization builds on the existing StringBuilder.
```

Two parsing details that bite: `seed` is a `uint64` and must not round-trip
through a `double`, and equity values are signed floats that need locale-
independent formatting (`%.6f` with the C locale, not `%g`).

### 3. Client identity and state - `src/ent/client_state.{c,h}` (new)

Three single-value files in the current working directory, matching how
`settings.txt` is already resolved:

```
birdtest_worker_uuid    generated on first run, persisted
birdtest_api_key        optional; mode 0600
birdtest_server         server URL
```

```c
typedef struct ClientState {
  char *worker_uuid;
  char *api_key;
  char *server_url;
} ClientState;

ClientState *client_state_load(ErrorStack *error_stack);   // creates the UUID if absent
void client_state_save(const ClientState *state, ErrorStack *error_stack);
void client_state_destroy(ClientState *state);
```

MAGPIE has no UUID generator today; a v4 UUID from `/dev/urandom` (and
`BCryptGenRandom` on Windows) is a dozen lines and belongs in `src/util/`.

**The API key must never be printed** by `-savesettings`, status output, or the
error stack.

### 4. The `contribute` command

Registered alongside the others in `config.c`:

```c
cmd(ARG_TOKEN_CONTRIBUTE, "contribute", 0, 1, contribute, contribute, false);

arg(ARG_TOKEN_CONTRIBUTE_SERVER,   "server",     1, 1);
arg(ARG_TOKEN_CONTRIBUTE_API_KEY,  "apikey",     1, 1);
arg(ARG_TOKEN_CONTRIBUTE_MAX_TASKS,"maxtasks",   1, 1);  // 0 = run forever
arg(ARG_TOKEN_CONTRIBUTE_IDLE_SECS,"idlewait",   1, 1);
```

Implementation in `src/impl/contribute.c`, following the shape of the other
`impl_*` entry points: `impl_contribute(Config *config, ErrorStack *error_stack)`.

The loop:

1. Resolve identity from `ClientState`, overridden by any `-server` / `-apikey`.
2. `POST /api/worker/task`. On 204, sleep `idlewait` and repeat. On 429, honour
   `Retry-After`.
3. Compare the job's `min_magpie_version` against MAGPIE's own version. On a
   mismatch, report it through the error stack and **stop**, rather than looping
   on tasks it cannot run — a GUI needs to surface "your MAGPIE is too old"
   once, not every second.
4. Start the heartbeat thread.
5. Dispatch on `job_type`.
6. `POST /api/worker/result`.
7. Stop the heartbeat, repeat.

Because it is registered as a normal command it inherits `-mode async`, so a GUI
can start it, poll status, and halt it with the existing machinery rather than
new plumbing.

### 5. Per-job-type executors

Each executor builds the same in-memory configuration the equivalent
command-line invocation would, calls the implementation function directly, and
reads results out of the result structs. No subprocess, no stdout, no parsing.

- **Opening rack analysis** — load the CGP from the request, apply the player
  config (`-r1`, `-s1`, sim parameters), call the existing move generation or
  simulation path, and read the ranked moves out of `MoveList` / `SimResults`,
  including per-ply `bingo_percentage` and `average_score`.
- **Games / game pairs** — set the seed, batch size, both player configs and
  `-gp`, then run autoplay with the new per-game recorder (§6).
- **Leave generation** — fetch the previous generation's KLV via
  `GET /api/worker/artifact`, write the forced-rack subset to a scratch file,
  run `leavegen`, and read the rack-equity table directly out of `RackList`
  rather than via `-writerackequitycsv` and a CSV re-read.

### 6. Result granularity - what MAGPIE actually needs to report

An earlier draft of this document called for a per-game autoplay recorder. That
was overstated. Testing against the binary shows the requirement is narrower and
splits by job type.

**`games` jobs need nothing new.** SPRT consumes wins, losses and draws, and
that is exactly what `autoplay` already reports:

```
autoplay games <total> <p0_wins> <p0_losses> <p0_ties> <p0_firsts> <p0_mean> <p0_sd> <p1_mean> <p1_sd> ...
```

birdtest's `game_records` table stores one row per game, but nothing downstream
reads the individual rows for a `games` job - SPRT and the dashboard percentages
both work off the counts. The schema is finer-grained than any consumer needs.

**`game_pairs` jobs need something pair-aware, but coarser than per-game.** What
SPRT wants for a pair is a single outcome - did player 1 take the pair, split
it, or lose it - not two score lines. And the pooled per-game aggregate cannot
supply it: two pairs recorded as two wins and two losses could be two splits, or
one pair won and one lost, and those are different distributions.

MAGPIE already has machinery pointed at this problem. In `-gp` mode it tracks
whether the two games of a pair **diverged** (played different moves at any
point), and reports a second `GameData` covering divergent games only. Pairs
that played identically are guaranteed ties that carry no signal, so excluding
them is the same variance reduction that pentanomial pair scoring achieves by a
different route.

So there are two candidate designs, and this needs deciding before anything is
built:

| | What MAGPIE reports | What birdtest stores |
|---|---|---|
| **Adopt MAGPIE's model** | The existing all-games and divergent-games aggregates | Two aggregates per task; SPRT runs on the divergent counts |
| **Pair-outcome reporting** | A new recorder emitting one win/split/loss per pair | One row per pair |

The first needs no MAGPIE change at all. The second is a smaller change than a
per-game recorder and keeps birdtest's SPRT operating on units it already
understands.

**Per-game records are only required for the raw-data export** - the
`GET /api/jobs/:id/results/stream` download offered for offline analysis. That
is a product decision about what birdtest promises contributors and researchers,
not a statistical necessity. If that export is worth keeping at per-game
granularity, a `gamelog` recorder is the way to get it, and it would incidentally
give command-line MAGPIE per-game output it does not have today.

#### The pairs signal, confirmed

Testing initially showed `-gp` producing perfectly symmetric, uninformative
aggregates. That turned out to be a MAGPIE bug, not a property of `-gp`: static
play called `get_top_equity_move` unconditionally, so a player's configured
move sort type never affected which move it chose, both games of every pair
played identically, and every pair was a guaranteed tie. Fixed in
[jvc56/MAGPIE#655](https://github.com/jvc56/MAGPIE/pull/655) and verified here
against `main` at `e4eda01`, 20 pairs, seed 50, NWL23:

| Players differ by | Divergent games | Player 1 W-L-D | Score means |
|---|---|---|---|
| `-s1 equity -s2 score` | 40 / 40 | 25-14-1 | 429.5 / 403.4 |
| `-l1 NWL23 -l2 CSW21` | 40 / 40 | 14-26-0 | 412.3 / 466.5 |
| `-k1 NWL23 -k2 CSW21` | 26 / 40 | 20-20-0 | 426.9 / 420.0 |
| nothing (same config) | 0 / 40 | 20-20-0 | identical |
| `-r1 best -r2 all` | 0 / 40 | 20-20-0 | identical |

The last row is **correct, not a residual bug**: move *record* type governs what
gets recorded, not which move is played, and the fix explicitly forces
`MOVE_RECORD_BEST` for static play. An earlier version of this document wrongly
grouped `-r` with `-s`; only the sort-type half was ever broken. birdtest's own
schema already notes that autoplay should always use `best`, so the two are
consistent.

**This settles the design.** `-gp` reports exactly what pair-level SPRT needs -
the all-games aggregate plus the divergent-games aggregate, both already
accounting for both sides of the pair, with identical pairs excluded as the
noise-free ties they are. So:

- **No new MAGPIE recorder is needed.** Not per-game, not per-pair.
- **birdtest should store the two aggregates per task**, not one row per game.
  `game_records` is finer-grained than any consumer, and for `game_pairs` the
  per-game rows cannot reconstruct the pair outcomes anyway.
- **birdtest should stop deriving pair outcomes in SQL.** The current query
  pairs consecutive `game_index` rows and inverts the second game's winner on
  the assumption that the players swapped seats. With `-gp` the reported
  aggregates already account for both orderings, so that derivation is
  unnecessary and wrong.
- SPRT for a pairs job should run on the **divergent** counts, which is where
  the variance reduction lives; the all-games aggregate is still worth storing
  for the dashboard's raw win/loss/draw display.

### 7. Wordmap auto-provisioning

Wordmaps make game play dramatically faster, so a contributing client is
**required** to use one - there is no `-wmp false` path for `contribute`. They
are also cheap to build: the `kwg -> txt -> wmp` chain measures **~1.3 seconds**
per lexicon.

So `contribute` builds a missing wordmap on demand rather than failing with
`file 'NWL23' not found for data type wordmap`, writing it into `./data`
alongside every other MAGPIE artifact. The data directory is assumed writable;
if it is not, that is a clear error and `contribute` stops, rather than falling
back to a second location.

One safety requirement: two MAGPIE processes contributing from the same
directory must not race. Generate to a temporary name and `rename()` into place.

### 8. Heartbeat thread

`POST /api/worker/heartbeat` every 30s for the lifetime of a claim, using
`cpthread` and a stop flag. Failures are logged and ignored — the server treats
a missed heartbeat as a lapsed claim and reassigns the task, which is already
the designed behaviour.

### 9. Version negotiation replaces self-update

The Python client re-execs itself from a newer script the server offers. MAGPIE
cannot responsibly do that: it is a compiled binary, and an auto-updating
executable is a much larger security proposition than a script.

Instead, `contribute` should report its version on every claim and surface a
clear, actionable message when the server requires a newer one. `GET
/api/worker/client-version` becomes a *minimum MAGPIE version* endpoint rather
than a script download, and birdtest's `min_magpie_version` per job is already
the right mechanism.

### 10. Build and platform

- No new *installable* dependency: libcurl is resolved at runtime on POSIX and
  WinHTTP is linked on Windows. `MAGPIE_NO_NETWORK` guards both for the WASM
  build, which compiles everything under `src`.
- The vendored JSON parser joins the existing `src/compat` precedent.
- Windows: the client is the first part of MAGPIE that would need
  `BCryptGenRandom` (for the worker UUID) and the WinHTTP backend. Both belong
  in `src/compat/` and `src/util/` respectively.
- `-march=native` in the release profile bakes the build machine's CPU features
  into the binary. Anyone distributing prebuilt MAGPIE binaries to contributors
  needs a portable baseline (`x86-64-v2` or similar) instead.

### 11. Security

- TLS certificate verification on by default; no flag to disable it.
- The API key is a bearer credential: never logged, never in `settings.txt`,
  never in error messages, file mode `0600`.
- Treat every field of a task request as untrusted input. It becomes file
  paths (`forced_racks`, artifact keys) and numeric parameters. Validate
  lexicon and variant names against known values before they reach
  `data_filepaths`, and reject artifact keys containing `..` or absolute paths.
- Bound everything the server can ask for: batch sizes, rack-subset sizes,
  iteration counts. A compromised or buggy server should not be able to make a
  contributor's machine allocate without limit.

### 12. GUI integration surface

For the one-click button to work, `contribute` needs to expose, in async mode:

- **State**: idle / claiming / working / submitting / stopped / error.
- **Progress**: current job type, games completed within the current task.
- **Totals**: tasks completed this session, plus the identity being credited.
- **Last error**, if any, in a form suitable for display.

Emit these through the existing `-hr false` machine-readable convention so the
GUI parses one format, and make stop cooperative — a task in flight should be
allowed to finish and submit, or be abandoned cleanly so the server's heartbeat
timeout reclaims it promptly.

---

## Changes on the birdtest side

Small, because the HTTP API does not change:

- **Retire `worker/` entirely** — the Python client, its Dockerfile and the
  worker compose profile. The contributor instructions become "install MAGPIE,
  run `contribute`".
- **`GET /api/worker/client-version`** changes meaning from "script version and
  download URL" to "minimum MAGPIE version".
- **The result schema is too fine-grained** and, for pairs, wrong. `game_records`
  stores one row per game; nothing reads the individual rows, and the pair
  derivation built on them is incorrect. See section 6: both job types should
  store the aggregates `autoplay` reports, and `game_pairs` should run SPRT on
  the divergent-games counts. This is a birdtest change, not a MAGPIE one, and
  it does not depend on the client move - it is worth doing either way.
### The client stops being birdtest's code

This is the part with the widest blast radius, and it is organisational as much
as technical. Today the client is birdtest's: same repo, same PR, same CI, same
review. Afterwards it is MAGPIE's, and the HTTP API becomes a **cross-repo
integration boundary** between two independently released programs.

Consequences worth planning for rather than discovering:

- A client bug is a MAGPIE bug — filed there, fixed there, released on MAGPIE's
  cadence, and only reaching contributors when they update MAGPIE.
- A server change that alters the worker API can break every deployed client.
  `min_magpie_version` per job is a floor, not a ceiling, so it does not stop an
  *old server* from confusing a *new client*. Either the worker API gets an
  explicit version, or it gets treated as frozen and only extended additively.
- Nothing in either repo currently pins the contract. It exists implicitly in
  `worker.py` and `routes/worker.rs` agreeing. Once they are in different repos
  that agreement needs to be written down and checked — a committed set of
  request/response fixtures both sides test against is the cheap version.

### Testing has to be re-planned

The [Testing](PLAN.md) section assumes three components with coverage gates:
backend, frontend, and worker client. The worker component disappears from
birdtest entirely, and with it the pytest suite, its 100% gate, and the
golden-file corpus of captured MAGPIE output. Those tests do not transfer —
they exist to verify subprocess output parsing, which is precisely what this
change deletes.

What replaces them is a three-tier arrangement:

1. **A fake worker** — a small test-only HTTP client that speaks the worker API
   and submits synthetic results without running MAGPIE at all. This is what
   most server tests want: it makes scheduler, SPRT, Glicko, redundancy,
   reclamation and dashboard tests fast and deterministic, and it is the only
   practical way to test the *adversarial* paths — malformed submissions, stale
   claim tokens, and the chi-square anomaly detection, which needs a client that
   deliberately submits bad data. Worth building whether or not MAGPIE takes
   over the real client.
2. **A real MAGPIE client in CI** — one job that builds MAGPIE with `contribute`
   and runs it against a seeded stack for one task of each type. In-container
   MAGPIE compiles in about 22 seconds, so this is affordable per-PR, not just
   nightly.
3. **Contract fixtures** shared with MAGPIE, so a change to either side that
   breaks the other fails in both repos.

---

## Suggested phasing

Each phase is independently useful and independently reviewable.

1. **Foundations** — `http_client`, `json`, UUID generation, `ClientState`.
   Convert `get_gcg.c` to the new HTTP client as the first consumer, which
   tests it against a real endpoint before any birdtest code exists.
2. **Move birdtest's result schema onto the aggregates** (section 6) — store
   what `autoplay` reports rather than one row per game, and run pairs SPRT on
   the divergent counts. Needs no MAGPIE change and does not depend on the
   client move, so it can happen first and independently.
3. **`contribute`, single job type** — claim, heartbeat, execute and submit for
   `games` only. This proves the whole loop end to end.
4. **Remaining job types** — opening rack analysis, game pairs, leave
   generation, plus wordmap auto-provisioning.
5. **Async status surface** — the GUI-facing state machine and machine-readable
   status output.
6. **Retire the Python client.**

---

## Open questions

### 1. How the HTTP dependency reaches a contributor - **decided**

**OS-native backends.** libcurl on POSIX (system-provided on both Linux and
macOS, resolved at runtime), WinHTTP on Windows. MAGPIE stays a single download
on every platform, takes no static TLS dependency, and never ships a CA bundle.

Rejected: dynamic libcurl as a hard link-time dependency (re-adds an install
step), static libcurl + TLS (MAGPIE would inherit OpenSSL CVEs and a stale CA
bundle), and continuing to shell out to the `curl` binary (unreliable on
Windows, and builds shell strings out of server-controlled values).

### 2. Where per-user state lives - **decided**

Deliberately simple, matching `settings.txt`'s existing convention rather than
introducing platform state directories:

- **Generated wordmaps go in `./data`**, like every other MAGPIE artifact. The
  data directory is **assumed writable**; if it is not, `contribute` fails with
  a clear error rather than falling back somewhere else. One convention for
  where MAGPIE's files live, at the cost of not supporting a read-only
  system-wide install - worth revisiting only if that install shape appears.
- **Clients are required to use wordmaps.** There is no `-wmp false` path for
  `contribute`: a missing wordmap is generated, and a data directory that
  cannot be written is an error. Games run dramatically faster with one, and a
  contributor running without one is donating much less compute than they think.
- **Client state goes in three files in the current working directory**, each
  holding one value: the worker UUID, the API key, and the server URL. Same
  cwd-relative model as `settings.txt`, so MAGPIE gains no new concept.
- The API key file is still a credential: `0600` on POSIX, never echoed by
  `-savesettings`, never in status output or error messages.

A consequence worth stating: because state is cwd-relative, a contributor who
runs MAGPIE from a different directory becomes a **new anonymous worker** and
loses their contribution history. That is the same footgun `settings.txt`
already has. Authenticating with an API key avoids it, since attribution then
follows the account rather than the generated UUID - a good reason for the GUI
to steer people toward signing in.

### Remaining open questions

- **Thread budget.** `contribute` should probably default to leaving a core
  free rather than saturating a contributor's machine.
