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

**Implementation: link libcurl.** It is the only realistic option that gives
TLS certificate verification without vendoring a TLS stack. This is the first
true external dependency MAGPIE would take, so it needs:

- `LDLIBS += -lcurl` and a `pkg-config --cflags libcurl` probe in the `Makefile`.
- A `MAGPIE_NO_NETWORK` compile guard. `Makefile-wasm` globs `src/**/*.c`, so
  the WASM build will try to compile this file; it must compile to stubs that
  push an error rather than failing to link.
- A note in `setup.sh` / the README that `libcurl4-openssl-dev` (or platform
  equivalent) is a build prerequisite.
- For distributing a self-contained binary to contributors, either static
  linking against libcurl + a TLS library, or accepting the shared dependency.
  **This is the single largest packaging decision in this document.**

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

### 3. Client identity and state — `src/ent/client_state.{c,h}` (new)

```c
typedef struct ClientState {
  char *worker_uuid;   // generated on first run, persisted
  char *api_key;       // optional
  char *server_url;
} ClientState;
```

Stored in a per-user directory (`$XDG_CONFIG_HOME/magpie/birdtest.txt`, falling
back to `$HOME/.magpie/`), **not** in the MAGPIE data directory — a contributor
may run MAGPIE from a read-only install, and the API key must not sit next to
shared lexica. Use the existing `settings.txt` key/value conventions rather than
inventing a second format.

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

### 6. A per-game autoplay recorder

The one genuinely new MAGPIE capability. `AUTOPLAY_RECORDER_TYPE_GAME`
aggregates into a `GameData`; birdtest needs the individual games.

Add a recorder — call it `gamelog` — that appends a row per game instead of
folding it in:

```c
typedef struct GameRecord {
  int score1;
  int score2;
  int winner;        // 1, 2, or 0 for a draw
  int num_turns;
  uint64_t seed;
  int game_index;    // ordering within a pair, 0 or 1
} GameRecord;
```

`game_data_add_game()` already computes `p0_game_score`, `p1_game_score` and
`args->number_of_turns`, so the recorder body is a mutex-guarded append. It
must be selectable via the existing options string (`autoplay games,gamelog`)
so it is useful from the command line too, and it needs a bound — a batch of
100k games should not accumulate 100k records in memory unbounded.

This also gives command-line MAGPIE users per-game output, which it does not
have today.

### 7. Wordmap auto-provisioning

Wordmaps make game play dramatically faster, so a contributing client always
wants one, but they are ~10x the size of everything else and are cheap to
build: the `kwg → txt → wmp` chain measures **~1.3 seconds** per lexicon.

`contribute` should build a missing wordmap on demand rather than failing with
`file 'NWL23' not found for data type wordmap`. Two prerequisites:

- A **writable data path**. Reuse the existing search-list semantics: MAGPIE
  writes to the first `-path` entry and reads from all of them, so a client
  writes generated artifacts into a user-owned directory while shared lexica
  stay read-only.
- Concurrency safety. Two MAGPIE processes contributing on one machine must not
  race; generate to a temporary name and `rename()` into place.

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

- `libcurl` linkage, `pkg-config` probe, `MAGPIE_NO_NETWORK` guard for WASM.
- The vendored JSON parser joins the existing `src/compat` precedent.
- Windows: the client is the first part of MAGPIE that would need
  `BCryptGenRandom` and `%APPDATA%` path handling. Both belong in `src/compat/`.
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
- **`GameResultsResponse` becomes achievable as specified.** The current schema —
  one record per game with `score1`, `score2`, `winner`, `num_turns` — is what
  the `gamelog` recorder produces, so the mismatch discovered against the
  subprocess client resolves without changing the schema.
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
2. **The `gamelog` recorder** — valuable to command-line MAGPIE on its own, and
   it unblocks the games and game-pairs executors.
3. **`contribute`, single job type** — claim, heartbeat, execute and submit for
   `games` only. This proves the whole loop end to end.
4. **Remaining job types** — opening rack analysis, game pairs, leave
   generation, plus wordmap auto-provisioning.
5. **Async status surface** — the GUI-facing state machine and machine-readable
   status output.
6. **Retire the Python client.**

---

## Open questions

### 1. How does the HTTP dependency get onto a contributor's machine?

MAGPIE links exactly one library today (`LDLIBS := -lm`), which is part of libc
and always present. That is why "download MAGPIE and run it" works. Adding
libcurl means adding a dependency on `libcurl.so.4`, which itself pulls in a TLS
library. Four ways to handle that, and the choice constrains the whole HTTP
layer, so it wants deciding first:

| Option | Contributor installs | Cost |
|---|---|---|
| **Dynamic libcurl** | MAGPIE **and** libcurl | `apt install libcurl4` on Linux; ships with macOS; DLLs to bundle on Windows. Re-introduces the setup step this project is trying to delete. |
| **Static libcurl + TLS** | MAGPIE only | Genuinely one file, but MAGPIE's build now has to produce or vendor static libcurl and OpenSSL. You inherit TLS security updates — an OpenSSL CVE means contributors run a vulnerable MAGPIE until you rebuild and they re-download. Full static against glibc is also awkward (NSS), so this usually means building against musl. |
| **Keep shelling out to `curl`** | MAGPIE **and** the `curl` binary | Zero build changes; it is what `get_gcg.c` does today. Status codes and headers are recoverable (`-w '%{http_code}'`, `-D -`), but every request becomes a shell string built partly from server-controlled values, and `curl` is not reliably present on Windows. |
| **Per-platform native HTTP** | MAGPIE only | WinHTTP on Windows, `NSURLSession`/CFNetwork on macOS, libcurl on Linux where it is effectively always installed. No new dependency anywhere, and TLS trust is the OS's problem rather than yours. Three small backends instead of one, but the client only makes five kinds of request. |

**A detail that catches people out:** a statically linked TLS stack does not know
where the system CA store lives, so certificate verification needs either a
probe of the usual paths (`/etc/ssl/certs/ca-certificates.crt` and friends) or an
embedded CA bundle, which then goes stale. The OS-native backends get this right
for free, which is the strongest argument for the last row.
### 5. Where does per-user state live?

MAGPIE has no concept of per-user state today. It has a shared, read-mostly
`data/` directory resolved through `-path`, and a `settings.txt` that is written
relative to the **current working directory** — so MAGPIE already behaves
differently depending on where you launch it.

The client needs to persist four things with quite different characteristics:

| What | Size | Sensitive | Losing it costs |
|---|---|---|---|
| Worker UUID | bytes | no | Contribution history: the contributor silently becomes a new anonymous worker |
| API key | bytes | **yes** | Re-generate from the account page |
| Server URL / preferences | bytes | no | Nothing |
| Generated wordmaps | ~122 MB per lexicon | no | ~1.3s per lexicon to rebuild |

Those do not belong in one place: the first three are tiny and precious, the
last is large and disposable. Every platform has a convention for exactly that
split:

| | Config (UUID, key, prefs) | Derived data (wordmaps) |
|---|---|---|
| Linux/BSD | `$XDG_CONFIG_HOME/magpie/`, default `~/.config/magpie/` | `$XDG_CACHE_HOME/magpie/`, default `~/.cache/magpie/` |
| macOS | `~/Library/Application Support/MAGPIE/` | `~/Library/Caches/MAGPIE/` |
| Windows | `%APPDATA%\MAGPIE\` | `%LOCALAPPDATA%\MAGPIE\` |

This matters beyond tidiness. If MAGPIE is installed system-wide, `data/` is
read-only, so generated wordmaps **must** go somewhere user-writable — which the
existing `-path` semantics already accommodate, since writes go to the first
entry of the search list. The client should prepend a user-writable directory
automatically rather than requiring the contributor to pass `-path`.

The open question is one of **scope**: is this a birdtest-client feature, or does
MAGPIE adopt a user-state directory generally and move `settings.txt` into it?
Doing it only for the client leaves MAGPIE with two conventions. Doing it
generally is the better end state but changes behaviour for existing users who
rely on a per-directory `settings.txt`, so it probably wants an override
(`MAGPIE_HOME`) and a portable mode for people running from a USB stick.

Also: the API key file needs `0600` on POSIX and equivalent ACLs on Windows, and
must never be written by `-savesettings`.

### Smaller open questions

- **Wordmap generation policy.** Build on demand when a task needs a missing
   lexicon, or refuse and tell the user to run `convert text2wordmap` first?
  On-demand is friendlier; refusing keeps `contribute` free of side effects on
  the data directory.
- **Should `contribute` be able to run without a wordmap at all?** A
  contributor on a small disk might prefer `-wmp false` and slower games to
  122 MB per lexicon.
- **Thread budget.** `contribute` should probably default to leaving a core
  free rather than saturating a contributor's machine.
