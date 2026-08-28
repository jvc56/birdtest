# Bringing the birdtest Client into MAGPIE

Implementation specification. Every design decision here is settled.

## Status

Implemented on MAGPIE's `birdtest-contribute` branch and verified end to end
against a local birdtest instance: `magpie contribute` claims tasks, plays real
games, and submits results the server records and credits.

| Piece | State |
|---|---|
| `src/compat/chttp` (libcurl via dlopen, WinHTTP, wasm stub) | Done. Also now backs `get_gcg.c`, replacing its three `curl`-binary calls. |
| Vendored cJSON (`src/compat/cjson`, platform-conditional so it lives in `compat`) + `src/util/json` wrapper | Done |
| `src/util/http_client` (retry policy) | Done |
| `src/ent/client_state` (`contribute.txt`) | Done |
| `contribute` command and task loop | Done |
| `games` / `game_pairs` executors | Done, verified end to end |
| `opening_rack_analysis` executor | Written; not verified end to end (needs a job whose lexicon MAGPIE has, and a full English rack enumeration is millions of tasks) |
| `leave_generation` executor | **Not implemented.** Needs artifact download and reading the rack-equity table out of `RackList`; currently reports that clearly and stops. |
| Async GUI status surface (section 12) | Not implemented |
| Windows WinHTTP backend | Written, not compiled or run on Windows |

Three things learned while building it are folded in below: a task that fails
does not count toward `maxtasks`, so the loop needs a consecutive-failure guard
or an unrunnable job spins forever; `arg_token_t` is private to `config.c`, so
the settings-file path is read there and passed into `impl_contribute` rather
than looked up inside it; and the worker UUID is minted by the **server**, not
the client -- see "The worker UUID" below.

## Goal

A contributor should need **only MAGPIE**. No Python, no Docker, no separate
worker script. Today's client is a Python program that shells out to MAGPIE and
parses its stdout; this specifies what MAGPIE gains so the client becomes a
MAGPIE command instead.

Immediate target:

```
magpie> contribute
```

with everything it needs — server, credentials, limits — in a `contribute.txt`
beside it, so no API key ever reaches a command line or `settings.txt`.

Eventual target: a MAGPIE GUI button calling the same code path, which is why
the command runs asynchronously and exposes machine-readable status rather than
assuming a terminal.

### The second reason to do this

The current client's fragility is almost entirely about **crossing the process
boundary**. It invokes `autoplay`, `gen` and `leavegen` as subprocesses and
reconstructs results by parsing formatted stdout. Every integration bug found so
far has lived there: invented flags, output formats that turned out to be
aggregate-only, a rack-equity CSV written under a name the client did not
predict, and MAGPIE resolving its board layout from `./data` before parsing
`-path`. Running in-process deletes that entire class of problem.

---

## Ground rules

### Platform-specific code lives only in `src/compat/`

**No file outside `src/compat/` may contain `#ifdef _WIN32`, `#ifdef __APPLE__`,
`#ifdef __wasm__`, or any other platform test.** Everything else in MAGPIE is
platform-neutral and calls into compat through a neutral API.

This invariant **currently holds exactly**: grepping the tree for `_WIN32`,
`__APPLE__` and `__wasm__` outside `src/compat/` returns nothing. `cpthread.h`
wraps pthreads, `csched.h` stubs `sched_yield` for wasm, `endian_conv.h`
branches on `_WIN32`, `ctime.h` wraps clocks. The client work is the largest
new source of platform behaviour MAGPIE has taken on, and must not be what
breaks it. Concretely, one new piece of platform behaviour is needed, and it
goes in `src/compat/`:

| Need | Compat file | Neutral API it exposes |
|---|---|---|
| HTTP + TLS | `src/compat/chttp.h` / `chttp.c` | `chttp_request()` |

The vendored cJSON parser also lives in `src/compat/` (`cjson.h` / `cjson.c`),
not because MAGPIE's own code branches on platform, but because the vendored
source itself has a `#ifdef _WIN32`-shaped block -- the same reason it counts
as platform-specific code under this rule. `src/util/json` wraps it in an
`ErrorStack`-aware API and is the only file that includes `cjson.h`, so
everything above that wrapper stays platform-neutral.

Two things this design **does not need**, despite an earlier draft of this
spec calling for them:

- **No `src/compat/crandom.h`.** The worker UUID is minted by the server, not
  generated locally -- see "The worker UUID" below. A client never needs a
  secure random source at all.
- **No `src/compat/csleep.h`.** `src/compat/ctime.h` already has a portable
  blocking sleep, `ctime_nap(double seconds)`, used elsewhere in MAGPIE (e.g.
  the pre-endgame solver's poll loop). The task loop's poll/backoff waits call
  that directly instead of introducing a second sleep abstraction.

Everything above them — request construction, retry policy, JSON, the task loop
— is ordinary portable C in `src/util/`, `src/ent/` and `src/impl/`.

### The WASM build compiles everything

`Makefile-wasm` compiles every `.c` under `src`'s subdirectories, so every new
file must compile under Emscripten. Networking is meaningless there, so the compat layer defines
`MAGPIE_NO_NETWORK` for wasm and `chttp_request()` becomes a stub that pushes an
error onto the stack. Nothing above compat needs to know.

---

## What the client does today

| Responsibility | Current implementation |
|---|---|
| Persistent worker identity | UUID assigned by the server on the first claimed task, stored on disk |
| Authentication | `Authorization: Bearer <api-key>`, or `X-Worker-UUID` when anonymous |
| Claim a task | `POST /api/worker/task` -> task request, or 204 when idle |
| Version gate | Refuse the task if MAGPIE is older than the job's `min_magpie_version` |
| Execute | Dispatch on `job_type` to one of four handlers |
| Heartbeat | Background thread, `POST /api/worker/heartbeat` every 30s while working |
| Submit | `POST /api/worker/result` with the claim token |
| Artifact fetch | `GET /api/worker/artifact?key=...` for a previous generation's KLV |
| Backoff | Sleep on 204 and on 429 (`Retry-After`) |

---

## What MAGPIE already has

| Capability | Status |
|---|---|
| HTTP requests | **Partial.** `src/impl/get_gcg.c` shells out to the `curl` *binary* via `get_process_output()` (a `popen` wrapper). No headers, no POST bodies, no status codes. |
| JSON | **No.** `get_gcg.c` scrapes with `strstr(response, "\"gcg\":\"")`. |
| Threads | **Yes.** `src/compat/cpthread.h`, `src/ent/thread_control.h`. |
| Async command execution | **Yes.** `EXEC_MODE_ASYNC`, `src/compat/async_command_control.h`. |
| Command registration | **Yes.** `cmd()` / `arg()` in `config.c`. |
| File path resolution | **Yes.** `data_filepaths.c`, colon-separated `-path` search list, writes to the first entry. |
| Structured errors | **Yes.** `ErrorStack` + `error_stack_push()`. |
| Per-player settings under `-gp` | **Yes**, as of [#655](https://github.com/jvc56/MAGPIE/pull/655). |

---

## Design overview

```
contribute
  |- client_state       read contribute.txt; generate the UUID on first run
  |- birdtest_api       typed wrappers over the five worker endpoints
  |    |- http_client   portable request/retry logic
  |    |    `- chttp    COMPAT: libcurl (POSIX) / WinHTTP (Windows) / stub (wasm)
  |    `- json          portable wrapper over vendored cJSON
  |- heartbeat thread   runs for the lifetime of a claim
  `- task dispatch      one executor per job_type, calling existing impls directly
```

---

## 1. HTTP: `src/compat/chttp.{h,c}` + `src/util/http_client.{h,c}`

### The compat layer

One neutral entry point. Everything platform-specific is behind it.

```c
typedef enum { CHTTP_GET, CHTTP_POST } chttp_method_t;

typedef struct ChttpRequest {
  chttp_method_t method;
  const char *url;
  const char *const *headers;   // "Name: value" strings
  int num_headers;
  const char *body;             // NULL for GET
  size_t body_length;
  int timeout_seconds;
} ChttpRequest;

typedef struct ChttpResponse {
  long status_code;
  char *body;                   // caller frees; NUL-terminated
  size_t body_length;           // body may be binary (KLV artifacts)
  int retry_after_seconds;      // from the header; -1 when absent
} ChttpResponse;

void chttp_request(const ChttpRequest *request, ChttpResponse *response,
                   ErrorStack *error_stack);
void chttp_response_destroy(ChttpResponse *response);
```

Three implementations behind one `#ifdef` ladder in `chttp.c`:

| Platform | Backend | Notes |
|---|---|---|
| Linux, BSD, macOS | **libcurl**, `dlopen`ed at first use | Linux ships no OS HTTP API; libcurl is what the platform provides, and macOS ships it too (`/usr/lib/libcurl.4.dylib`), so one implementation covers both. |
| Windows | **WinHTTP** (`winhttp.dll`) | Ships with the OS, no redistributable, Schannel trust store already configured. |
| wasm | Stub | Pushes a new `ERROR_STATUS_HTTP_UNAVAILABLE`. |

**`dlopen` rather than link-time binding.** Only these symbols are needed:
`curl_easy_init`, `curl_easy_setopt`, `curl_easy_perform`, `curl_easy_getinfo`,
`curl_easy_cleanup`, `curl_slist_append`, `curl_slist_free_all`. Resolving them
at first use means a machine without libcurl still runs every offline MAGPIE
command, and `contribute` fails with "libcurl not found; install libcurl4" —
rather than MAGPIE refusing to start at all. Try `libcurl.so.4`, then
`libcurl.so`, then `libcurl.4.dylib`.

Requirements that hold on every backend:

- **TLS certificate verification is on and cannot be disabled.** No flag, no
  environment variable.
- Follow redirects, bounded at 5.
- `timeout_seconds` covers the whole exchange.
- The response body is length-delimited, not NUL-delimited: artifacts are binary.
- No global process state at exit; `contribute` may run many requests.

### The portable layer

`src/util/http_client.c` holds everything that is not platform-specific:
building the header list, the `Authorization` / `X-Worker-UUID` choice,
JSON content type, and the retry policy.

**Retry policy**, applied uniformly:

| Response | Action |
|---|---|
| 2xx | Return it. |
| 204 | Return it; the caller decides (for `/task` it means "no work"). |
| 429 | Sleep `Retry-After` (default 1s) and retry, up to 5 times. |
| 5xx, or a transport error | Exponential backoff 1s, 2s, 4s, 8s, 16s; then fail. |
| 4xx other than 429 | Return it; the caller decides. Never retried. |

---

## 2. JSON: vendored cJSON + `src/util/json.{h,c}`

Vendor [cJSON](https://github.com/DaveGamble/cJSON) (MIT, one `.c` + one `.h`)
into `src/util/cjson.{c,h}` **verbatim and unmodified**, so it can be updated by
replacing the files. It is portable C89 and compiles under Emscripten unchanged,
so it does not belong in `src/compat/`.

Wrap it in `src/util/json.{h,c}` so the rest of MAGPIE sees `ErrorStack` rather
than cJSON's conventions, and so a future swap touches one file:

```c
JsonValue *json_parse(const char *text, ErrorStack *error_stack);
void json_destroy(JsonValue *value);

const JsonValue *json_object_get(const JsonValue *object, const char *key);
bool         json_is_null(const JsonValue *value);
int          json_array_length(const JsonValue *array);
const JsonValue *json_array_get(const JsonValue *array, int index);

// Each pushes onto the error stack if absent or the wrong type.
const char *json_get_string(const JsonValue *object, const char *key, ErrorStack *es);
int64_t     json_get_int   (const JsonValue *object, const char *key, ErrorStack *es);
uint64_t    json_get_uint64(const JsonValue *object, const char *key, ErrorStack *es);
double      json_get_double(const JsonValue *object, const char *key, ErrorStack *es);
bool        json_get_bool  (const JsonValue *object, const char *key, bool fallback);

// Serialization builds on the existing StringBuilder.
void json_write_object_start(StringBuilder *sb);
void json_write_int(StringBuilder *sb, const char *key, int64_t value);
void json_write_double(StringBuilder *sb, const char *key, double value);
void json_write_string(StringBuilder *sb, const char *key, const char *value);
void json_write_object_end(StringBuilder *sb);
```

Two details that will otherwise bite:

- **`seed` is a `uint64`** and must not round-trip through a `double`. cJSON
  stores numbers as `double`, which loses precision above 2^53. Read `seed` from
  its raw string form (`valuestring` is not populated for numbers, so keep the
  token text) or, simplest, have birdtest send it as a JSON string. **Decision:
  the server already sends `seed` as a decimal string**, so MAGPIE reads it
  with `strtoull`; see the contract below.
- **Equity values are floats and must be locale-independent.** Write with
  `"%.6f"` under the C locale, never `%g`, and never rely on the process locale.

---

## 3. Contribution settings: `src/ent/client_state.{h,c}`

**Nothing about contributing is passed on the command line.** All of it lives in
a single settings file, for two reasons: an API key on a command line ends up in
shell history and in `ps` output, and contribution settings have no business
mixed into `settings.txt` alongside board layouts and simulation parameters.

### The file

`contribute.txt` in the current working directory, one setting per line as
`key value`. Blank lines and lines beginning with `#` are ignored.

```
# birdtest contribution settings
server    https://birdtest.example
apikey    bt_9f2c...
threads   7
maxtasks  0
idlewait  5
uuid      6f3d7198-178a-47c8-9ccc-6aa6995a5a9c
```

| Key | Required | Default | Meaning |
|---|---|---|---|
| `server` | **yes** | — | birdtest base URL |
| `apikey` | no | absent | Attributes work to an account. Without it the worker is anonymous, identified by `uuid`. |
| `threads` | no | cores − 1 | Threads given to MAGPIE while working |
| `maxtasks` | no | `0` | Tasks to complete before stopping; `0` runs until stopped |
| `idlewait` | no | `5` | Seconds to wait after the server reports no work |
| `uuid` | no | assigned by the server | The anonymous worker identity |

An unknown key is an error rather than a silent ignore — a typo'd `apikey`
should not quietly downgrade someone to anonymous.

### Reading and writing

```c
typedef struct ClientState {
  char *server_url;
  char *api_key;      // NULL when contributing anonymously
  char *worker_uuid;  // NULL until the server assigns one
  int threads;
  int max_tasks;
  int idle_wait_seconds;
} ClientState;

ClientState *client_state_load(const char *path, ErrorStack *error_stack);
void client_state_destroy(ClientState *state);

// Records a UUID the server assigned during this run: updates the in-memory
// state and appends it to the settings file.
void client_state_set_worker_uuid(ClientState *state, const char *uuid);
```

The file is **user-authored and MAGPIE does not rewrite it**, with exactly one
exception: once the server assigns a `uuid` (see below), MAGPIE **appends a
single line**. Appending rather than rewriting means comments, ordering and
formatting the contributor put there survive untouched.

If `server` is missing, `contribute` fails with a message naming the file and
the missing key — not a usage string, since the fix is editing a file.

### The worker UUID

**The server mints it, not the client.** A worker with no `apikey` and no
`uuid` yet sends no identity at all on its first request; the server responds
with a UUID (via the JSON body of the first successful `/api/worker/task`
claim, once there is actually a task to hand out — see section 5), and the
client persists it and sends it as `X-Worker-UUID` on every request after that,
for the rest of this run and every run to come.

This is a deliberate reversal from letting the client generate its own UUID: a
client-generated identity trusts a value the server never gets to validate.
Having the server mint it costs one extra round trip for a brand-new anonymous
worker's first task and nothing after that, and means MAGPIE's `contribute`
code needs no cryptographically secure random source at all.

Because the file is resolved relative to the working directory, a contributor
who runs MAGPIE from a different directory has no `contribute.txt` there and
`contribute` stops with a clear error — which is a better failure than silently
becoming a new anonymous worker and losing their contribution history.

### The API key needs no special file handling

`contribute.txt` holds a bearer credential when `apikey` is set, but nothing
about that requires MAGPIE-side file permission handling: it is a plain text
file the contributor already created and controls the permissions of, on their
own machine, the same as `settings.txt` or any CGP file MAGPIE reads. MAGPIE
does not `chmod` it, check who else can read it, or otherwise treat it as
special. The key still never appears on the command line, in `settings.txt`, or
in status output, logs, or error messages -- the file is the only place it
lives.

## 4. The `contribute` command

Registered in `config.c` alongside the others, taking **no settings arguments** —
only an optional path to the settings file (section 3), which defaults to
`contribute.txt`:

```c
cmd(ARG_TOKEN_CONTRIBUTE, "contribute", 0, 1, contribute, contribute, false);
```

```
magpie> contribute                      # reads ./contribute.txt
magpie> contribute /path/to/other.txt   # a path is not a secret
```

Implemented as `impl_contribute(Config *config, ErrorStack *error_stack)` in
`src/impl/contribute.c`, following the other `impl_*` entry points.

**Threads.** Default to `num_cores - 1`, minimum 1, overridable by the `threads`
key. Contributing should leave the machine usable — this will eventually be a
background activity someone opts into on their daily driver, and a machine that
becomes unresponsive is a machine whose owner turns contributing off. Note this
is deliberately independent of the global `-threads` setting: contributing
should not silently inherit whatever a user last set for simulation.

**The loop:**

1. Load `ClientState` from the settings file. `uuid` may be absent -- the
   server assigns one, see step 2.
2. `POST /api/worker/task`, identifying with the API key if set, the stored
   `uuid` if set, or no identity header at all if neither is set yet.
   - 204: sleep `idlewait`, repeat.
   - 200: if the response carries a `worker_uuid` and this worker had none
     locally, adopt it -- update `ClientState` and the request identity used
     from here on, and append it to the settings file. Continue.
3. **Version gate.** The response carries `min_magpie_version`. If it is present
   and this build is older, **stop the loop and report an error**: "this job
   requires MAGPIE >= 1.4.0; you are running 1.2.0. Update MAGPIE to continue
   contributing." Do not submit, do not retry — the claim lapses via the
   heartbeat timeout and another worker picks it up. A GUI shows one actionable
   message rather than a scrolling error.
   The same applies to a `job_type` this build does not recognise: it means the
   server is newer than this MAGPIE, so exit the same way.
4. Start the heartbeat thread.
5. Dispatch on `job_type` (section 5).
6. `POST /api/worker/result`.
7. Stop the heartbeat. If `maxtasks` is reached, stop; otherwise repeat.

**Stopping.** Cooperative. A stop request during a task lets the task finish and
submit; a stop request while idle returns immediately. A hard interrupt simply
abandons the claim, which the server's heartbeat timeout reclaims.

**Errors during execution** are reported and the claim is abandoned without
submitting; the loop continues. An error *claiming* or *submitting* is handled by
the retry policy in section 1, and only stops the loop if it exhausts retries.

Because it is a normal command it inherits `-mode async`, so a GUI can start it,
poll status, and stop it with the existing machinery.

---

## 5. Per-job-type executors

Each builds the in-memory configuration the equivalent command line would, calls
the implementation function directly, and reads results out of the result
structs. No subprocess, no stdout, no parsing.

**Player configuration** arrives as a JSON object per player and maps onto the
per-player settings, where `N` is 1 or 2:

| JSON field | Setting | Notes |
|---|---|---|
| `recorder_type` | `-rN` | `best` for all birdtest jobs |
| `sort_strategy` | `-sN` | `equity` or `score`; null for simming players |
| `leaves` | `-kN` | null means the lexicon default |
| `max_iterations` | `-iN` | null for a static player |
| `plies` | `-plN` | |
| `top_plays` | `-npN` | |
| `stopping_pct` | `-scN` | |
| `use_inference` | `-siN` | |
| `time_limit_secs` | `-tlN` | |

A player with `max_iterations` null is static; the simulation settings are all
null together and must be omitted rather than passed as zero.

- **Opening rack analysis** — load the CGP, apply the single player config, run
  move generation (or simulation when `max_iterations` is set), and read the
  ranked moves out of `MoveList` / `SimResults`, including per-ply
  `bingo_percentage` and `average_score`.
- **Games / game pairs** — set seed, batch size, both player configs, and `-gp`
  for pairs. Read counts and score moments out of the `GameData` the autoplay
  recorder already maintains: `total_games`, `p0_wins`, `p0_losses`, `p0_ties`,
  and the score `Stat` means and standard deviations. For pairs, read the
  divergent `GameData` as well.
- **Leave generation** — fetch the previous generation's KLV via
  `GET /api/worker/artifact`, write the forced-rack subset to a scratch file,
  run `leavegen`, and read the rack-equity table directly out of `RackList`
  rather than via `-writerackequitycsv` and a CSV re-read.

---

## 6. Result granularity: nothing new is needed

An earlier draft called for a per-game autoplay recorder. That was wrong, and the
question is now settled in both directions:

- **`games` jobs.** SPRT consumes wins, losses and draws, which is exactly what
  autoplay already reports. Nothing downstream ever needed individual games.
- **`game_pairs` jobs.** In `-gp` mode MAGPIE reports a second `GameData` over
  the **divergent** pairs — those whose two games did not play identically. A
  pair that played identically is a guaranteed tie carrying no information, so
  excluding those is the variance reduction pairing exists to provide. That is
  precisely the signal SPRT wants.

Verified against `main` at `e4eda01`, 20 pairs, seed 50, NWL23:

| Players differ by | Divergent games | Player 1 W-L-D | Score means |
|---|---|---|---|
| `-s1 equity -s2 score` | 40 / 40 | 25-14-1 | 429.5 / 403.4 |
| `-l1 NWL23 -l2 CSW21` | 40 / 40 | 14-26-0 | 412.3 / 466.5 |
| `-k1 NWL23 -k2 CSW21` | 26 / 40 | 20-20-0 | 426.9 / 420.0 |
| nothing (same config) | 0 / 40 | 20-20-0 | identical |
| `-r1 best -r2 all` | 0 / 40 | 20-20-0 | identical |

The last row is correct rather than a bug: move *record* type governs what is
recorded, not which move is played, and static play forces `MOVE_RECORD_BEST`.

**birdtest has already been changed to match** — `game_records` is gone,
replaced by `game_results` storing the two aggregates, with pairs SPRT and Glicko
computed from the divergent counts. So there is no schema work waiting on this,
and **no new MAGPIE recorder is required.**

---

## 7. Wordmap auto-provisioning

Clients are **required** to use wordmaps: games run dramatically faster with one,
and a contributor without one is donating far less compute than they think. There
is no `-wmp false` path for `contribute`.

Wordmaps are also never transmitted — they are roughly ten times the size of
everything else MAGPIE ships — so the client builds what it needs from the
`.kwg` it already has. The full `kwg -> txt -> wmp` chain measures **~1.3 seconds**
per lexicon (0.17s + 1.1s, NWL23, 4 threads).

Before running any task, if `<lexicon>.wmp` is absent:

1. If `<lexicon>.txt` is absent, `convert dawg2text <lexicon>`.
2. `convert text2wordmap <lexicon> -threads <n>`.

Both write into `./data`, which is **assumed writable**. If it is not, that is a
clear error and `contribute` stops — there is no fallback location.

Generate to a temporary name and `rename()` into place, so two MAGPIE processes
contributing from the same directory cannot race.

---

## 8. Heartbeat thread

`POST /api/worker/heartbeat` with `{"claim_token": "..."}` every 30 seconds for
the lifetime of a claim, using `cpthread` and a stop flag. Failures are logged
and ignored: the server treats a missed heartbeat as a lapsed claim and
reassigns the task, which is the designed behaviour.

The heartbeat must start *before* task execution, because wordmap generation and
a large batch both happen inside it.

---

## 9. Version negotiation replaces self-update

The Python client re-execs itself from a newer script the server offers. MAGPIE
cannot responsibly do that: it is a compiled binary, and an auto-updating
executable is a much larger security proposition.

Instead the server states a minimum and the client reports clearly when it falls
short (section 4, step 3). `GET /api/worker/client-version` changes meaning from
"script version and download URL" to "minimum MAGPIE version", and
`min_magpie_version` per job is the per-job form of the same thing.

---

## 10. The worker API contract

Five endpoints. Authentication on all of them is either
`Authorization: Bearer <api-key>` **or** `X-Worker-UUID: <uuid>`, never both.

### `POST /api/worker/task`

Empty body. `204` when there is no work -- no body, so a request that arrived
with no identity is not assigned a UUID here; it tries again with no identity
next time, and gets one for keeps once a task is actually available. `200`:

```json
{
  "claim_token": "6f3d7198-178a-47c8-9ccc-6aa6995a5a9c",
  "job_id": "4c7b64ad-8e5e-4db7-aeb0-afc44ee1ebf5",
  "min_magpie_version": "1.4.0",
  "worker_uuid": "6f3d7198-178a-47c8-9ccc-6aa6995a5a9c",
  "task_request": { "job_type": "games", "...": "..." }
}
```

`min_magpie_version` may be `null`. `worker_uuid` is present only when the
request carried no identity at all and the server just minted one for it; the
client persists this and sends it as `X-Worker-UUID` from then on. `task_request`
is internally tagged by `job_type`, one of four shapes:

```json
{ "job_type": "opening_rack_analysis",
  "lexicon": "NWL23", "variant": "classic",
  "position": "15/15/... AABCELT/ 0/0 0",
  "previous_play": null,
  "player": { "name": "static", "recorder_type": "best", "sort_strategy": "equity",
              "leaves": null, "max_iterations": null, "plies": null,
              "top_plays": null, "stopping_pct": null, "use_inference": null,
              "time_limit_secs": null } }

{ "job_type": "games",
  "lexicon": "NWL23", "variant": "classic",
  "seed": "1", "num_games": 10, "game_pairs": false,
  "player1": { ... }, "player2": { ... } }

{ "job_type": "game_pairs", "...": "as games, with game_pairs true",
  "num_games": 10 }

{ "job_type": "leave_generation",
  "lexicon": "NWL23", "variant": "classic",
  "generation": 2,
  "forced_racks": ["AA", "AB"],
  "previous_artifact_key": "leaves/<job>/generation-1.klv2",
  "num_games": 10000 }
```

`seed` is a **decimal string**, because it is a `uint64` and JSON numbers are
doubles. For `game_pairs`, `num_games` counts *pairs*; MAGPIE plays two games per
pair.

### `POST /api/worker/heartbeat`

`{"claim_token": "..."}` -> `204`.

### `POST /api/worker/result`

```json
{ "claim_token": "...", "result": { } }
```

Returns `200` with `{"accepted": true}`, or `{"accepted": false}` when the claim
had already lapsed — which is **not an error**, just work that was reassigned.
Returns `400` when the result does not satisfy its shape.

Result shapes by job type:

```json
{ "moves": [ { "move": "8D BEAD", "score": 24, "equity": 31.5,
               "plies": [ { "ply": 0, "bingo_percentage": 0.0,
                            "average_score": 24.0 } ] } ] }

{ "all_games": { "games": 20, "wins": 11, "losses": 9, "ties": 0,
                 "p1_score_mean": 429.5, "p1_score_sd": 60.8,
                 "p2_score_mean": 403.4, "p2_score_sd": 55.9 } }

{ "all_games": { "...": "as above" },
  "divergent_games": { "...": "same shape, the divergent subset" } }

{ "racks": [ { "rack": "AA", "count": 30, "mean": 1.5 } ] }
```

Server-side validation, so the client must satisfy it:

- `wins + losses + ties == games`, all non-negative.
- `games_pairs`: `games` is even and non-zero, `divergent_games` is required, its
  own counts are consistent, and `divergent_games.games` is even and `<= games`.
- `moves` and `racks` must be non-empty.

### `GET /api/worker/artifact?key=<key>`

Returns `application/octet-stream`. Only keys the server itself minted resolve;
anything else is `404`. Used for the previous generation's KLV.

### `GET /api/worker/client-version`

`{"version": "...", "download_url": "..."}` today; becomes the minimum MAGPIE
version (section 9).

### Rate limiting

Worker endpoints are limited to roughly one request per second per identity with
a small burst. A `429` carries `Retry-After` in seconds. A task costs at least
two requests, so this is reached under normal operation and must be handled as
backoff, not as an error.

---

## 11. Security

- TLS certificate verification on by default, with no way to disable it.
- The API key is never accepted on the command line, never written to
  `settings.txt`, and never appears in status output, logs or errors. It lives
  only in `contribute.txt`, an ordinary file the contributor already controls
  the permissions of on their own machine.
- The worker UUID is minted by the server, never trusted from the client, so a
  client cannot pick or collide an identity on its own.
- **Every field of a task request is untrusted input.** It becomes file paths
  (`forced_racks`, `previous_artifact_key`) and numeric parameters. Validate
  lexicon and variant against known values before they reach `data_filepaths`,
  and reject artifact keys containing `..` or a leading `/`.
- Bound everything the server can ask for — batch sizes, rack-subset sizes,
  iteration counts. A compromised or buggy server must not be able to make a
  contributor's machine allocate without limit.

---

## 12. GUI integration surface

In async mode `contribute` must expose:

- **State**: idle / claiming / working / submitting / stopped / error.
- **Progress**: current job type, and games completed within the current task.
- **Totals**: tasks completed this session, and the identity being credited.
- **Last error**, in a form suitable for display.

Emit these through the existing `-hr false` machine-readable convention so the
GUI parses one format.

---

## Changes on the birdtest side

The HTTP API does not change, so these are small:

- **Retire `worker/worker.py`** and the `worker` Docker profile. Contributor
  instructions become "install MAGPIE, run `contribute`".
- **`GET /api/worker/client-version`** changes meaning to a minimum MAGPIE
  version.
- ~~Send `seed` as a string rather than a JSON number.~~ **Done.**
- **Keep `worker/fake_worker.py`.** It speaks the worker API and submits
  synthetic results with no MAGPIE in the loop, which is what most server tests
  want — and it reaches paths a real client cannot reach deliberately
  (`--mode malformed`, `--mode stale`, `--mode abandon`).

### The client stops being birdtest's code

Organisational as much as technical. The HTTP API becomes a **cross-repo
integration boundary** between two independently released programs:

- A client bug is a MAGPIE bug, fixed on MAGPIE's cadence, reaching contributors
  only when they update.
- A server change can break every deployed client. `min_magpie_version` is a
  floor, not a ceiling, so it does not stop an old server confusing a new client.
  The worker API should be treated as frozen and extended only additively.
- Nothing currently pins the contract; it exists implicitly in `worker.py` and
  `routes/worker.rs` agreeing. Committed request/response fixtures that both
  repos test against are the cheap version of fixing that.

### Testing

birdtest's worker component and its coverage gate disappear. What replaces them:

1. **The fake worker** for server-side tests — scheduling, claim lifecycle,
   SPRT, Glicko, redundancy, reclamation, and the adversarial paths.
2. **A real MAGPIE client in CI**, building MAGPIE with `contribute` and running
   it against a seeded stack for one task of each type. MAGPIE compiles in a
   container in about 22 seconds, so this is affordable per-PR.
3. **Contract fixtures** shared with MAGPIE.

---

## Phasing

1. **Compat foundations** — `chttp` (both backends plus the wasm stub).
   Convert `get_gcg.c`'s three `curl`-via-`popen` call sites
   to `chttp_request()` as the first consumer: it exercises the layer against a
   real endpoint before any birdtest code exists, and removes MAGPIE's dependency
   on the `curl` binary.
2. **Portable foundations** — vendored cJSON, the `json` wrapper,
   `http_client`, `client_state`.
3. **`contribute`, `games` only** — claim, heartbeat, execute, submit. Proves
   the whole loop end to end against a local birdtest stack.
4. **Remaining job types** — opening rack analysis, game pairs, leave
   generation, plus wordmap auto-provisioning.
5. **Async status surface** — the GUI-facing state machine.
6. **Retire the Python contributor client**, keeping the fake worker.

Phases 1-3 are done, phase 4 is partial (opening rack written, leave generation
not), and phases 5-6 remain.
