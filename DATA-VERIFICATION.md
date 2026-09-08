# Input Data: Identity, Import, and Client Capability

birdtest names its input data by string — `"NWL23"`, `"winpct"`, `"english"` —
and a name is not an identity. This document replaces those names with rows in
an `input_data` table, each pinning one file by SHA-256; makes every job and
player config reference those rows; and defines what happens when a contributor
does not have the bytes a job requires — which is a normal, expected condition,
not a failure.

The source of truth is the **versioned data tarball** that
[MAGPIE-DATA](https://github.com/jvc56/MAGPIE-DATA) publishes and that MAGPIE's
`download_data.sh` installs. An admin imports one by date; birdtest shows what
is new; the admin confirms; the rows become the vocabulary that jobs are built
from.

---

## 1. The problem

birdtest hands a task to an arbitrary machine and folds what comes back into
permanent aggregates. A task request names its inputs but does not pin them:

| Config field | Names | Actually a file | Size (NWL23/english) |
|---|---|---|---|
| `lexicon` (per player) | `"NWL23"` | `lexica/NWL23.kwg` | 4,719,596 B |
| `leaves` (per player) | `"NWL23"` or null | `lexica/NWL23.klv2` | 3,667,340 B |
| `win_pct_model` (per player) | `"winpct"` | `strategy/winpct.csv` | 836,150 B |
| `letter_distribution` | `"english"` | `letterdistributions/english.csv` | 489 B |
| (board layout, never named) | — | `layouts/standard15.txt` | 244 B |
| `use_wordmap` | true/false | `lexica/NWL23.wmp` — built locally from the `.kwg` | ~104 MB |

Two contributors can both honestly report running `NWL23` with `winpct` and be
running different bytes. This is not hypothetical. MAGPIE's `download_data.sh`
installs `data-20251004.tgz`, whose `english.csv` is 489 bytes; the live file on
MAGPIE-DATA's `main` is a different 273-byte file that dropped two full-width
display columns:

```
installed by download_data.sh (data-20251004.tgz)   489 B  sha256 e698ce0b…
data/letterdistributions/english.csv on main        273 B  sha256 84d4b2c4…
```

Both are called `english`. Nothing in the protocol can tell them apart.

The failure is silent. A wrong `.kwg` does not crash, it plays a slightly
different game. A stale `winpct.csv` does not error, it makes different move
choices under simulation. Results are well-formed, pass every shape check in
`submit_result`, and land in `game_results`, `leave_rack_progress` and
`player_config_ratings` alongside everyone else's.

**Blast radius, worst first:**

- **`leave_generation`** — generation *N* becomes the KLV generation *N+1*
  plays with, so bad data propagates into every later generation and into
  whatever ships as a leaves file. It is folded in by a running
  `occurrence_count +=` upsert
  ([leave_gen.rs:107-125](backend/src/jobs/leave_gen.rs#L107-L125)), so there
  is no per-claim detail to subtract back out afterwards.
- **`games` / `game_pairs`** — SPRT is a decision procedure over an aggregate.
  A minority of workers on a different lexicon biases the win rate, and SPRT
  reaches a confident, wrong conclusion. Nothing about the output looks
  anomalous.
- **`opening_rack`** — most recoverable (per-rack rows can be deleted and
  recomputed), but easiest to corrupt: a different `.kwg` changes which plays
  exist at all.

**And a second problem, which is not corruption at all.** A contributor has the
lexica they downloaded, not every lexicon that exists. A job on `CSW24` is
simply not work that every machine can do. Today the protocol has no way to say
so: the scheduler hands out whatever is next, and a worker that cannot do it has
no vocabulary for declining. Any design that only detects *wrong* data and stops
would treat "I don't have that lexicon" as an error, when it is an ordinary fact
about a volunteer machine.

## 2. The approach

Four pieces, in dependency order:

1. **An `input_data` table** — one row per distinct file: relative path, SHA-256,
   the tarball date it came from, and, for the two file types the server itself
   reads, the bytes (§5, §5.1).
2. **Admin import** — pick a `YYYYMMDD`, birdtest fetches that versioned
   tarball, computes what is new, shows it, and inserts on confirmation (§6).
3. **Configs reference rows, not names**, and each file sits where MAGPIE
   actually scopes it: lexicon and leaves on the player, letter distribution and
   board layout on the job (§7). Every such field becomes a foreign key into
   `input_data`, so a job pins exact bytes at creation and dispatch has nothing
   to infer.
4. **Capability negotiation.** A client that lacks a task's data declines the
   claim, records that it cannot do that job, and sends that set with its next
   claim. The scheduler routes around it. When a worker can do nothing that is
   available, the server tells it to shut down and say why (§9).

**Four decisions shape the implementation more than the rest**, and each is
argued where it belongs rather than collected in a footnote: the server reads
letter distributions out of the pinned row rather than off its own disk, so
`DATA_PATH` and the `data/` directory go away entirely (§5.1); tarball import
runs as a background task rather than a long request (§6); the claim body is
required rather than optional (§8.1); and generation 1 of a leave job gets a
server-built zeroed KLV like any other generation (§3). birdtest runs as a
single instance, which several of these assume.

The result: wrong data cannot be contributed, missing data is an ordinary
routing decision, and a contributor who is simply out of date is told so once,
clearly, instead of being silently useless or noisily broken.

---

## 3. Which files a task needs, and where they come from

Every row here was checked against MAGPIE's source rather than inferred from
names, and three of them do not work the way the field names suggest. Under this
design these rules are applied **once, at job creation**, to choose an
`input_data` row — not at dispatch, and never by the client.

| Role | Where it comes from | File | MAGPIE's rule |
|---|---|---|---|
| `kwg` | **each player**, independently (`-l1` / `-l2`) | `lexica/<name>.kwg` | per-player lexicons are first-class; `PlayerSpec.lexicon` only fell back to a shared one because birdtest sent one (MAGPIE `src/impl/config.c:6186-6192`) |
| `klv` | each player (`-k1` / `-k2`) | `lexica/<name>.klv2` | `get_default_klv_name(lex) = lex` — the old default was a duplicate of the lexicon name |
| `winpct` | each player, **only if it simulates** | `strategy/<name>.csv` | `DEFAULT_WIN_PCT "winpct"` (`src/def/config_defs.h:16`); loaded lazily by `config_load_win_pcts` |
| `letterdist` | **the job** — one per job, shared by both players | `letterdistributions/<name>.csv` | stated by the job, never inferred |
| `layout` | **the job** — `standard15` unless stated | `layouts/standard15.txt` | `board_layout_get_default_name()` = `"standard" BOARD_DIM`, `DEFAULT_BOARD_DIM = 15` |

**`variant` is not the layout.** MAGPIE has two separate settings: `-var` (game
variant: `classic` | `wordsmog`) and `-bdn` (board layout: `standard15` |
`standard21`). birdtest sends only `variant`, and it is a rules setting with no
file behind it. So `variant` stays a plain `TEXT` column — it is the one field in
this area that must *not* become a foreign key. The board layout is a real file
that no config names today, which is why §7.2 gives the job a column for it:
`standard15` is a default, and a default is not a pin.

**A lexicon belongs to a player, not to a job.** `-l1` and `-l2` are independent
settings, and the two reasons a `games` job exists pull in opposite directions:
collecting data means both players run the same config, while comparing
strategies means they differ — and what differs may well be the lexicon. Putting
the lexicon on the job forced a shared value and made the per-player field an
"override", which is backwards. §7 moves it where it belongs, and the
consequence is that a job's expected data is the **union over its players**, not
a single lexicon's files.

**`leave_generation` needs neither leaves nor a win% model.** Generation 1
starts from a **zeroed KLV**, not from a lexicon's shipped leaves, and the bot
plays statically, so no `winpct.csv` is ever loaded. Its data requirement is
therefore just three files: the `.kwg`, the letter distribution, and the layout.
Two things follow. Every generation, generation 1 included, gets its KLV from
`GET /api/worker/artifact` ([worker.rs:55-79](backend/src/routes/worker.rs#L55-L79))
— server-supplied bytes, nothing to verify — so **no generation ever references
a `klv` row**.

Generation 1 is not a special case on the client. The server builds its zeroed
KLV when the job starts and serves it through the same artifact endpoint as
every later generation's; the client fetches a KLV by key and plays, with no
branch for "first generation" and no path that could load `<lexicon>.klv2`
instead.

Concretely: `run_transition` already writes `leaves/<job>/generation-<N>.klv2`
after folding generation *N*, and generation *N+1* reads it
([leave_gen.rs:194](backend/src/jobs/leave_gen.rs#L194)). Generation 1 therefore
needs `leaves/<job>/generation-0.klv2` — a KLV over the job's leave universe
with every value `0.0`. `initialize_job_state` builds and stores it at job
creation, **outside** the creating transaction, since it is a multi-megabyte
build and an object-store write; the transaction commits the job, and the
artifact is written before the job is marked runnable. There is no virtual
generation 0 anywhere else: no `leave_rack_progress` rows, no tasks, no
`leave_generation_artifacts` semantics beyond the one row recording the key.

This changes the current wire documentation: `LeaveRequest.previous_artifact_key`
is commented as "NULL for generation 1, where the worker falls back to the
lexicon's default leaves"
([handler.rs](backend/src/jobs/handler.rs)), and MAGPIE-CLIENT.md repeats it.
Both the comment and the nullability go: the field is always populated, because
there is always a server-built KLV to point at. The fallback described there
produces different generation-1 output from a zeroed start and nothing would
flag the difference, which is exactly why the branch is being removed rather
than fixed.

**Other exemptions:**

- `.wmp` — derived locally, never digested, absent from the tarball (§11.3).

**Byte-exact files only.** Byte identity is stricter than semantic identity, and
the gap is real: birdtest's own KLV builder
([klv.rs:9-32](backend/src/jobs/klv.rs#L9-L32)) deliberately emits a plain trie
where MAGPIE's `kwg_maker` emits a minimized DAWG — both correct, bytes differ.
That gap does not bite here, because every `input_data` row comes from a tarball
distributed byte-exact. It *would* bite the moment someone adds a row for a
locally-generated file. Don't.

---

## 4. How production data is actually distributed

**MAGPIE's `download_data.sh`** pins a version as a constant in the script:

```bash
DATA_VERSION="20251004"
BASE_URL="https://github.com/jvc56/MAGPIE-DATA/raw/main/versioned-tarballs"
```

It probes for `data-20251004.tgz.aa`, walks the chunk suffixes `aa`, `ab`, `ac`,
… while they exist, concatenates them, and pipes the result through
`tar -xzf - -C "$SCRIPT_DIR"`, extracting `data/` into the MAGPIE checkout root
— which is where `data_paths` looks by default. Today that is three 40 MB
chunks, ~94 MB total. A separate `testdata-<version>.tgz` is fetched the same
way and is irrelevant here: it is for MAGPIE's own tests.

Two consequences:

1. **The data version is a MAGPIE release-time constant.** Everyone running a
   given MAGPIE build has the same `DATA_VERSION` unless they went out of their
   way. This is what makes the steady state in §9 small: most workers converge
   on the same answer about what they can do.
2. **`download_data.sh` verifies nothing.** No checksum, no signature; a
   truncated chunk or a corrupted extraction is silent. Per-file digests from
   birdtest are, incidentally, the first integrity check anything in this
   pipeline performs.

**How the tarball is built.** `data/update_versioned_data.sh` copies
`data/versioned/<version>/` with `cp -RL` — dereferencing symlinks — into a
directory renamed `data`, tars it, and splits at 40 MB. A CI workflow
(`validate-versioned-tarballs.yml`) re-extracts the committed tarball, re-runs
the same `cp -RL`, and `diff -r`s the two, failing the PR if they differ.

**Most of `data/versioned/20251004/` is symlinks — and mostly *directory*
symlinks:**

```
120000 blob  data/versioned/20251004/layouts     -> ../../layouts
040000 tree  data/versioned/20251004/letterdistributions
120000 blob  data/versioned/20251004/lexica      -> ../../lexica
120000 blob  data/versioned/20251004/strategy    -> ../../strategy
```

Only `letterdistributions` is a real directory, and even it is a mix: four real
files (`dutch`, `english`, `english_super`, `french` — the ones that still carry
display columns) and three symlinks to the live copies.

**The consequence that matters:** a version *name* is a label, not a freeze.
Change `data/lexica/NWL23.kwg` and `versioned/20251004/lexica/NWL23.kwg` changes
with it; CI will make you regenerate the tarball, but the name stays `20251004`.
**The only thing that pins content is the built tarball** — which is what prod
installs, and what §6 imports. §5's dedupe rule is what makes this survivable:
if `20251004` is ever re-cut with different bytes, importing it again produces
*new rows*, visibly, rather than silently redefining what `20251004` meant.

---

## 5. The `input_data` table

One row per distinct file. This is the vocabulary everything else is built from.

```sql
-- Every input data file birdtest knows about, identified by content.
--
-- A row is a (path, sha256) pair: the same path with different bytes is a
-- different row, which is the entire point. `tarball_date` records the
-- versioned tarball a row was FIRST seen in -- provenance, not membership. A
-- file unchanged between 20251004 and 20260101 stays one row labelled
-- 20251004, because it is the same bytes and a job pinning it is pinning those
-- bytes regardless of which tarball the contributor installed.
CREATE TABLE input_data (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Path relative to the data root, including basename: 'lexica/NWL23.kwg'.
    -- This is what the tarball contains and what a contributor has on disk.
    path         TEXT NOT NULL,
    -- Derived from `path` at import and stored because dispatch and the client
    -- protocol address files by (role, name), not by path: MAGPIE resolves a
    -- name through its own data_paths search list (§10).
    role         TEXT NOT NULL CHECK (role IN ('kwg','klv','winpct','letterdist','layout')),
    name         TEXT NOT NULL,          -- 'NWL23', 'winpct', 'english', 'standard15'
    sha256       TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    bytes        BIGINT NOT NULL CHECK (bytes >= 0),
    -- YYYYMMDD name of the versioned tarball this content was first imported
    -- from. Text, not DATE: it is the artifact's name, and it appears verbatim
    -- in the message a contributor is told to act on.
    tarball_date TEXT NOT NULL CHECK (tarball_date ~ '^\d{8}$'),
    -- The file's bytes, for the roles the SERVER itself reads (§5.1). birdtest
    -- enumerates rack universes and builds KLVs from the letter distribution,
    -- so those bytes must be the pinned ones and not whatever is on the
    -- server's disk -- there is no server disk copy any more. Lexica stay out:
    -- a 15 MB .kwg in a row is a different proposition and nothing server-side
    -- reads one. 244-496 bytes per row, one row per distinct file.
    content      BYTEA
                 CHECK ((role IN ('letterdist','layout')) = (content IS NOT NULL)),
    imported_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    imported_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    UNIQUE (path, sha256)
);

CREATE INDEX input_data_role_name_idx ON input_data (role, name);
```

**Why `tarball_date` is first-seen rather than a set.** The alternative is a
join table recording every tarball each row appears in. That answers "which
versions contain this file", which nothing here asks: a job pins bytes, and a
contributor either has those bytes or does not, regardless of how their tarball
was labelled. First-seen is one column and answers the question that *is* asked
— "where did this come from, and what do I tell someone to download". If
membership is ever needed, it is an additive table, not a change to this
one.

**`content` is not optional for the roles that have it.** The `CHECK` is an
equivalence, not a nullable convenience: a `letterdist` or `layout` row without
bytes cannot exist, and a `kwg`, `klv` or `winpct` row with bytes cannot either.
Server-side code therefore never has a "fall back to the filesystem" branch — if
`content` is missing the row is malformed and the job fails loudly at creation,
which is the whole point of §5.1.

**Consequence worth stating plainly:** if the same file appears in two imported
tarballs, its `tarball_date` names the older one. A message built from it should
say "this file comes from data-20251004 or later", not "you must install
20251004".

**A job pins exactly one row per role.** There is no "any of these acceptable
versions" — the complexity is real and the payoff is narrow, since the dedupe
above already means an unchanged file across two tarballs is one row, not two.

**Rows are deletable when nothing references them.** No soft `retired_at`, no
tombstones. The foreign keys from `jobs`, `player_configs` and
`job_leave_config` have no `ON DELETE` clause, so Postgres defaults to
`NO ACTION` and a referenced row cannot be deleted — the constraint *is* the
safety mechanism, and a foreign-key violation on `DELETE` is the correct,
self-explaining outcome. The admin endpoint translates it into "this row is used
by 3 jobs" rather than surfacing the raw error.

### 5.1 Why the row carries bytes: the server reads data files too

**What the rest of this design assumes.**

Everywhere else, this document treats an `input_data` row as *the* identity of a
file. A job pins one row per role; the row carries a digest; a worker that
cannot produce that digest from its own copy declines the task (§9). The whole
verification story rests on one premise: **the bytes a job depends on are named
by a row in the database, and every participant is checked against that row.**

**Where the premise was false.**

The server is a participant, and it is not checked.

birdtest does not only dispatch work — it computes things itself, and to do that
it reads letter-distribution CSVs off its own filesystem, from `DATA_PATH`
(`data_path` in [config.rs:82](backend/src/config.rs#L82), defaulting to
`../data`). Three places do this today, all through
`LetterDistribution::load` ([racks.rs:34](backend/src/jobs/racks.rs#L34)):

- **Opening-rack job creation.** `total_racks`
  ([opening_rack.rs:133](backend/src/jobs/opening_rack.rs#L133)) counts the rack
  space with a dynamic-programming table over the distribution, and the count is
  stored on the job. `expand` ([opening_rack.rs:122](backend/src/jobs/opening_rack.rs#L122))
  turns a stored rack range back into actual racks when a task is claimed.
- **Leave generation.** `seed_generation`
  ([leave_gen.rs:280](backend/src/jobs/leave_gen.rs#L280)) enumerates the leaves
  for a generation from the distribution.
- **KLV building.** [klv.rs](backend/src/jobs/klv.rs) bakes MAGPIE's machine-letter
  numbering — index 0, 1, 2… assigned in *file order*, per the comment at
  [racks.rs:19-26](backend/src/jobs/racks.rs#L19-L26) — into the KWG node bytes
  of the KLV it produces.

None of these reads goes anywhere near `input_data`. They open a path built from
an environment variable and trust whatever is there. So there are two copies of
`english.csv` in the system with no relationship between them:

| | who reads it | how it is identified | verified against the job? |
|---|---|---|---|
| the pinned row | the worker, via MAGPIE | `input_data.id` + digest | yes (§9) |
| `DATA_PATH/letterdistributions/english.csv` | the server, at job creation and claim | a filesystem path | **no** |

**A concrete failure.**

Suppose a distribution gains a tile — a new letter appended to the file, or a
count changed from 2 to 3. MAGPIE-DATA ships it; the admin imports the new
tarball, so `input_data` has a new `letterdist` row; a new job pins that row.
But the operator deploying birdtest forgot to update the container's `../data`,
so the server still has last month's CSV.

Now:

1. The server counts the rack space from **the old** distribution and writes
   `total_racks` onto the job — a number for a universe that no longer exists.
2. A worker claims a task. The server expands the task's rack range from **the
   old** distribution and sends those racks.
3. The worker checks its own `english.csv` against the pinned row's digest. It
   matches — the worker has **the new** file, exactly as pinned. Verification
   passes.
4. The worker plays the racks it was given, using the new distribution's bag.

Every check in the system is green. The racks are syntactically valid, the games
complete, the results are well-formed and get aggregated. What has actually
happened is that the sampling frame was enumerated over one alphabet while every
game was played with another: some racks that exist in the new distribution are
never sampled, the per-rack weights are wrong, and if the letter *order* changed
rather than the counts, the KLV's machine-letter numbering disagrees with the
worker's — leave values silently attach to the wrong leaves.

**Nothing reports any of this.** There is no error, no decline, no gap row. The
job runs to completion and produces a confidently wrong answer. This is worse
than every failure mode §9 was built to catch, because those all announce
themselves.

**To be clear about what does *not* happen.**

The server does not send these files to anyone. `LetterDistribution` is consumed
server-side and only its *products* — a rack list, a `total_racks` count, a
built KLV — cross the wire. Workers obtain their own data through
`download_data.sh` or a MAGPIE-DATA clone, and MAGPIE reads it locally (§9.1).
So this is not a distribution problem and the fix does not change the protocol;
it is purely about the server disagreeing with the row it pinned.

Note also that only `letterdist` is read server-side today. `layout` is covered
too because it is the other tiny file and the next plausible candidate —
nothing currently loads one — and lexica are read only by workers.

**The fix: `content` on the row.**

That is what the `content` column above is for: populated at import for the
`letterdist` and `layout` roles only. These files are 244–496 bytes, and the
dedupe above means one row per distinct file rather than one per tarball, so the
table grows by a few hundred bytes per data generation.

`LetterDistribution::load` ([racks.rs:34](backend/src/jobs/racks.rs#L34)) grows
a sibling that takes bytes instead of a path, and every server-side read —
`total_racks` and `expand` in
[opening_rack.rs](backend/src/jobs/opening_rack.rs), `seed_generation` in
[leave_gen.rs](backend/src/jobs/leave_gen.rs), and KLV building in
[klv.rs](backend/src/jobs/klv.rs) — resolves through the row the job pins rather
than through `DATA_PATH`. `data_path` then drops out of those call chains
entirely; the parameter is threaded through `registry.rs` and `handler.rs` today
purely to reach these reads.

The result is that there is exactly one copy of the truth and drift is not
representable: the bytes the server computes over and the digest the worker is
checked against come from the same row. `DATA_PATH` stops being load-bearing for
anything a job pins.

Lexica stay out. A 15 MB `.kwg` in a table row is a different proposition, and
nothing server-side reads one — they are only ever read by workers, locally.

**What this deletes.** `DATA_PATH` and `cfg.data_path` go, along with the
`data_path` parameter threaded through
[registry.rs](backend/src/jobs/registry.rs),
[handler.rs](backend/src/jobs/handler.rs) and the job handlers purely to reach
these reads; so do `COPY data /app/data` and `ENV DATA_PATH` in
[docker/Dockerfile](docker/Dockerfile) and the `DATA_PATH` line in
`docker-compose.yml`. The `data/` directory itself goes: `testdist.csv` becomes
compiled-in fixture bytes in the test tree, from which tests insert an
`input_data` row, and `english.csv` is deleted because the one test that reads
it already puts a MAGPIE checkout on its search path. That also retires the
"non-MAGPIE-DATA distributions need birdtest's own data dir" workaround at
[klv.rs:404](backend/src/jobs/klv.rs#L404) — the `testdist` round-trip writes its
fixture into the temp directory it already creates.

The alternative that was rejected — hashing the server's local file at job
creation and refusing on a mismatch — is not sufficient on its own: it checks
only at creation, so a deployment that swaps `DATA_PATH` underneath an existing
job puts the drift back, and `expand` re-reads the file at claim time with no
check at all. It is worth keeping only as a health check for any role that is
ever added to the server-read set without being added to `content`.

---

## 6. Importing a tarball

Admin-triggered, two-phase, and never on the dispatch path. Dispatch reads local
tables only; GitHub can be down for a week without a worker noticing.

**Phase 1 — fetch and diff, in the background.** The archive is ~94 MB, so this
is not a request that waits. The endpoint resolves the ref, inserts a
`running` row, spawns a tokio task, and returns the import id immediately; the
admin UI polls `GET /api/admin/input-data/imports/<id>`. birdtest runs as a
single instance, so a spawned task needs no lease — and, for the same reason,
startup marks any row still `running` as `failed`, since nothing else can be
working on it. That assumption is load-bearing only here: if birdtest is ever
replicated, the import is the first thing that breaks, and it would need a lease
and a reaper keyed to the instance that owns the row.

```
POST /api/admin/input-data/imports   { "tarball_date": "20260101",
                                       "ref": "main" | "<commit-sha>" }
-> 202 { "id": "…", "state": "running" }
```

1. Resolve `ref` → a commit SHA, so `main` is pinned at import time and the
   record names a commit, never a branch.
2. Fetch `versioned-tarballs/data-<date>.tgz` at that commit from
   `raw.githubusercontent.com`, mirroring `download_data.sh`'s chunking exactly:
   try `.aa` first, walk `aa → ab → … → az → ba …` while chunks exist, fall back
   to the unchunked name, cap the walk (the script stops at 26) and cap total
   bytes. A `404` on the first probe is the ordinary "no such version" answer
   and should say so, not surface as a transport error.
3. Stream the concatenated chunks through SHA-256 (recording the tarball's own
   digest), then gunzip, then tar. This is the one place birdtest parses an
   untrusted container format, so every entry is checked against an explicit
   allowlist before it is trusted enough to hash: **regular files only**; the
   path must be relative, carry no `..` segment, and match the expected
   `data/<dir>/<basename>` shape. Never construct a filesystem path from an
   archive name — the import hashes bytes and has no reason to form one.
   Anything failing aborts the whole import rather than skipping the entry,
   because a malformed archive is not a partially trustworthy one. Map
   `data/<dir>/<basename>` → (path, role, name) by the inverse of §3's table;
   ignore unrecognised directories.
4. Compare each (path, sha256) against `input_data`. Stage the result, keeping
   the bytes of every `letterdist` and `layout` entry so confirmation does not
   have to download again (§5's `content`).
5. Mark the row `staged`, or `failed` with the reason in `error`.

**Phase 2 — confirm.**

```
GET  /api/admin/input-data/imports/<id>    -> the staged diff
POST /api/admin/input-data/imports/<id>/confirm
```

The admin sees three groups: **new** rows (path+sha256 not present), **known**
rows (already present — the majority, and the reason the diff exists), and
**path collisions** — a path already known under a *different* sha256. That last
group is the one that deserves a second look, because it is either a legitimate
data update or the §4 case of a tarball being re-cut under a name that was
already used. Confirmation inserts only the new rows, all in one transaction,
with `tarball_date` set to this import's date.

```sql
-- Staged imports. Phase 1 writes; phase 2 reads and commits. Rows here are
-- proposals, not data -- nothing dispatch or job creation reads.
CREATE TABLE input_data_imports (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tarball_date   TEXT NOT NULL CHECK (tarball_date ~ '^\d{8}$'),
    commit_sha     TEXT NOT NULL,
    -- NULL until the download completes: the row exists from the moment the
    -- background task is spawned.
    tarball_sha256 TEXT,
    state          TEXT NOT NULL DEFAULT 'running'
                   CHECK (state IN ('running', 'staged', 'confirmed',
                                    'cancelled', 'failed')),
    -- What the poller renders while state = 'running'.
    progress_bytes   BIGINT NOT NULL DEFAULT 0,
    progress_entries INT    NOT NULL DEFAULT 0,
    -- Why it failed, shown verbatim to the admin.
    error          TEXT,
    requested_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    requested_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at   TIMESTAMPTZ
);

CREATE TABLE input_data_import_rows (
    import_id  UUID NOT NULL REFERENCES input_data_imports(id) ON DELETE CASCADE,
    path       TEXT NOT NULL,
    role       TEXT NOT NULL,
    name       TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    bytes      BIGINT NOT NULL,
    -- 'new' | 'known' | 'collision' (same path, different sha256 already known)
    disposition TEXT NOT NULL,
    -- Carried from phase 1 for letterdist/layout entries so confirmation
    -- inserts input_data.content without re-downloading 94 MB.
    content     BYTEA,
    PRIMARY KEY (import_id, path, sha256)
);
```

**Limits, all enforced during the walk:**

| Limit | Value |
|---|---|
| Compressed bytes downloaded | 512 MiB |
| Chunks walked (`aa`, `ab`, …) | 64 |
| Total uncompressed bytes | 1 GiB |
| Uncompressed : compressed ratio | 20× |
| Single entry | 128 MiB |
| Entry count | 5,000 |
| Whole task | 30 min, 30 s connect, 120 s idle read |

Every one of them aborts the import rather than skipping the entry. The ratio is
checked continuously, not at the end, since that is the zip-bomb case a total cap
alone lets through. The archive URL is built from `MAGPIE_DATA_REPO` and never
from user input, so the residual exposure is a compromised upstream; that is the
threat model these checks are written against.

Staging rather than recomputing on confirm means the ~94 MB download happens
once, and the admin confirms exactly what they were shown. Unconfirmed imports
are garbage-collected after 24 hours; `tarball_sha256` is kept because it is a
single value identifying a whole install, which makes "did this version change
under its own name?" one comparison.

Both phases are audit-logged (`audit::log`, `"input_data.import_staged"` /
`"input_data.import_confirmed"`), consistent with every other admin mutation.

**New dependencies:** the backend has no HTTP client today (`hex` and `sha2` are
already there; `aws-sdk-*` brings hyper but not a usable general client). Add
`reqwest = { version = "0.12", default-features = false, features = ["stream", "rustls-tls"] }`,
plus `flate2` and `tar`. Config gains `MAGPIE_DATA_REPO` (default
`jvc56/MAGPIE-DATA`) and an optional `GITHUB_TOKEN` — optional in development,
set in production, because unauthenticated ref resolution is 60 calls per hour
per IP. A `403` from GitHub is rendered with the `X-RateLimit-Remaining` and
`X-RateLimit-Reset` headers it carries, so the failure names its own remedy.

Config **loses** `DATA_PATH`. Nothing on the server reads a data file off disk
any more (§5.1), so `data_path` disappears from `Config`, from `AppState`, and
from the `registry.rs` / `handler.rs` / `opening_rack.rs` / `leave_gen.rs` call
chains that thread it today; `COPY data /app/data` and `ENV DATA_PATH` come out
of [docker/Dockerfile](docker/Dockerfile) and `docker-compose.yml` with it.

---

## 7. Config schema changes in `0001_initial.sql`

The site is not live, so all of this edits the initial migration rather than
stacking a second one on it. `input_data` must be created before `jobs`,
`player_configs`, and the job config tables — so it belongs above the `job_type`
enum at [0001_initial.sql:64](backend/migrations/0001_initial.sql#L64).

Three moves, and they are entangled: files move onto players where MAGPIE puts
them, settings shared by every job type move up onto `jobs`, and what is left in
the per-type tables is only what is genuinely per-type.

### 7.1 `player_configs` — the lexicon lives here

Three `TEXT` columns become foreign keys:

```sql
    -- was: lexicon TEXT (per-player override; NULL = the job's lexicon)
    kwg_id     UUID NOT NULL REFERENCES input_data(id),
    -- was: leaves TEXT (NULL = lexicon default)
    klv_id     UUID NOT NULL REFERENCES input_data(id),
    -- was: win_pct_model TEXT (NULL = 'winpct')
    -- NULL is still meaningful, but it now means "this player never loads one",
    -- which is true of any static player. Validated against the sim columns.
    winpct_id  UUID REFERENCES input_data(id),
```

`kwg_id` becomes **`NOT NULL`**, which is the whole point of the move: there is
no job lexicon left to fall back to, so every player names its own. Two players
in a `games` job may name the same row (collecting data with one strategy) or
different rows (comparing two), and nothing constrains which — including the
degenerate case where both `player_config_id`s are the *same config row*, which
must stay legal and therefore gets no `CHECK (player1 <> player2)`.

`klv_id` becomes `NOT NULL` too: "NULL = the lexicon default" was exactly the
implicit name-based resolution this design removes, and with two independent
lexicons in play there is no longer even a single lexicon to take a default
from.

`winpct_id` stays nullable, but with a new meaning. It is not "use the default"
— it is "this player never loads a win% model", which is true of every static
player, since MAGPIE only reads one through `config_load_win_pcts`. Job creation
validates the pairing: a player with simulation parameters (`max_iterations`,
`num_plies`, …) must have a `winpct_id`; a player with none must not. This is
worth the extra rule because `expected_data` is built from what a task actually
loads, and a contributor missing `winpct.csv` should not be locked out of jobs
that would never have opened it.

**A consequence to accept deliberately:** a player config now pins bytes, so it
cannot be reused across a data update — a new `winpct.csv` means a new config
row. That is the correct outcome rather than an inconvenience:
`player_config_ratings` are only comparable among players that ran on identical
data, and nothing in the schema says so today. The cost is that a data update
means cloning configs, which the admin UI should make one click.

Cloning needs two more things. `name` is `UNIQUE`, so every
clone needs a new one, and the convention is fixed before the first clone
exists: `simmer-NWL23-4ply@20260101` — base name, `@`, the `tarball_date` of the
data it pins — generated by the clone endpoint rather than typed. And the clone
starts with no rating history, because `player_config_ratings` is keyed by
config, so the lineage has to be visible or it reads as a bug:

```sql
    -- The config this one was cloned from, for a data update. Ratings do NOT
    -- carry over -- they are only comparable on identical data -- so the UI
    -- must show where a config with no history came from.
    cloned_from_id UUID REFERENCES player_configs(id),
```

The config page reads "cloned from simmer-NWL23-4ply@20250101 on 2026-01-01 —
ratings restart on new data", with a link.

### 7.2 `jobs` — what every job type shares

`lexicon`, `variant` and `letter_distribution` are currently duplicated across
all four job config tables, which is three chances to disagree and no way for a
query to ask "what letter distribution is this job on" without knowing its type
first. The lexicon leaves for `player_configs` (§7.1); the other two move **up**
to `jobs`, along with the board layout, which nothing named before:

```sql
    -- Settings every job type has, regardless of what it does.
    variant       TEXT NOT NULL,                                -- 'classic' | 'wordsmog'; a rules setting, not a file
    letterdist_id UUID NOT NULL REFERENCES input_data(id),      -- one per job, shared by both players
    layout_id     UUID NOT NULL REFERENCES input_data(id),      -- 'standard15' unless a job says otherwise
```

These sit next to `min_magpie_version` in `CREATE TABLE jobs`. A job row is
already inserted before its config row in the same transaction
([admin.rs:333-380](backend/src/routes/admin.rs#L333-L380)), so this is a
straightforward move of three values from the second insert to the first.

Putting the letter distribution here rather than on the player is not arbitrary:
MAGPIE takes one `-ld` for the whole game, and two players cannot draw from
different bags. The same is true of the board.

### 7.3 What is left in the per-type tables

```sql
-- job_opening_rack_config: player_config_id, racks_per_batch, rack_size, total_racks
-- job_game_config:         player1_config_id, player2_config_id, games_per_batch,
--                          min_games, max_games, sprt_*, elo_*, capture_positions
-- job_game_pair_config:    as job_game_config, with pairs_per_batch / min_pairs / max_pairs
-- job_leave_config:        kwg_id, num_iterations, generation_count, target_rack_count,
--                          racks_per_task, max_leave_size, use_wordmap
```

`job_leave_config` is the one place a lexicon still sits on a job, because leave
generation has one bot and no `player_configs` row to hold it. It needs **no
`klv_id` and no `winpct_id`** (§3): generations start from a zeroed KLV, and the
bot plays statically. Its complete data requirement is `kwg_id` plus the job's
`letterdist_id` and `layout_id` — three files.

### 7.4 Validation at job creation

The schema cannot express these, so `create_job` must:

- **Role match.** `kwg_id` names a row with `role = 'kwg'`, `letterdist_id` a
  `letterdist` row, and so on. A composite foreign key on `(id, role)` with a
  redundant `role` column on each referencing table would enforce it in the
  database, and is worth doing if this validation ever feels too load-bearing to
  leave in application code.
- **Cross-player compatibility.** With independent lexicons the check is no
  longer trivially satisfied: both players' lexicons must be compatible with
  each other and each with its own leaves (`lexicons_and_leaves_compat`,
  [config.c:6130](file:///home/josh/MAGPIE/src/impl/config.c)), and both must be
  compatible with the job's single letter distribution (`ld_types_compat`,
  [letter_distribution.h:689](file:///home/josh/MAGPIE/src/ent/letter_distribution.h)).
  MAGPIE already has these rules; birdtest should not be able to build a job
  MAGPIE would refuse to load.

  **These rules are ported to Rust rather than approximated.** They are
  name-prefix rules over lexicon and distribution names, small enough to
  transcribe and stable enough to stay transcribed. The risk of a second copy is
  drift, so the port is pinned by a test asserting a table of known-good and
  known-bad combinations — every pair MAGPIE accepts and a representative set it
  rejects — which is what turns a future divergence into a failing test rather
  than a job that builds here and refuses to load there. The one thing a port
  must not do is guess: if a combination is not covered by the transcribed
  rules, reject it and let the table grow.
- **Sim/winpct pairing**, as described in §7.1.

### 7.5 What dispatch does now

Almost nothing. The digests come from a join over the job's `letterdist_id` and
`layout_id` plus its players' `kwg_id` / `klv_id` / `winpct_id`, **deduplicated**
— two players on the same lexicon contribute one `kwg` entry, not two. The
`expected_data` builder is a query, not an inference engine, which removes the
piece the previous design most needed unit tests for.

`TaskRequest` still carries names, because that is what MAGPIE's command-line
surface takes.

**What the server reads for itself comes from the same rows.** Rack enumeration
(`total_racks`, `expand`), leave seeding, and KLV building take the letter
distribution's bytes from the job's pinned `letterdist_id` — `content`, not a
path (§5.1). `LetterDistribution::load` keeps its parsing and gains a
bytes-taking constructor; the path-taking one goes away with `DATA_PATH`.

---

## 8. Wire protocol

Three changes to the worker API. MAGPIE-CLIENT.md §10 declares the contract
frozen-and-extended-only-additively; two of these obey that and one does not,
and neither fact matters much yet. birdtest is not deployed, there are no
contributors and no third-party clients, and the only client is MAGPIE's own
`contribute` command on the `birdtest-contribute` branch, changed in step with
the server. Every field here is free to change until the first release
and expensive afterwards, which is the argument for getting the shapes right
now rather than filing them as follow-ups.

### 8.1 The task claim carries what the worker cannot do

`POST /api/worker/task` had an empty body. It now **requires** one, carrying the
worker's version and the set of jobs it has already found it cannot run:

```json
{ "magpie_version": "1.4.0",
  "unsupported_jobs": ["4c7b64ad-8e5e-4db7-aeb0-afc44ee1ebf5"] }
```

`magpie_version` is covered in §10; `unsupported_jobs` is every job this worker
has found it cannot run, for any reason.

**The body is required, not optional.** Both fields are load-bearing —
the version drives the `min_magpie_version` filter, and without it the server
would have to assume a version for every claim, which is a wrong answer dressed
as a safe one. `claim_task` ([worker.rs:97](backend/src/routes/worker.rs#L97))
takes no body today, so this is a plain `Json<ClaimBody>` extractor and a
matching change to `contribute`; a bodyless claim must be rejected with an error
that names the fix rather than a bare `422`, because that error is what a stale
MAGPIE build will show a contributor after launch. `Option<Json<…>>` becomes the
right shape only once such a build can plausibly exist, which is after the first
release, not before it.

`unsupported_jobs` is attacker-controlled input flowing into a query, so it is
capped at **200** entries and silently truncated past that — far above any
honest client, since the list is bounded by the jobs a worker has actually seen
— bound as a `bigint[]` (`WHERE id <> ALL($1)`) rather than interpolated, with
non-positive and malformed ids dropped before binding.

### 8.2 The assignment carries the digests

`expected_data`, next to `min_magpie_version`, absent when the job pins nothing:

```json
"expected_data": {
  "algorithm": "sha256",
  "files": [
    { "role": "kwg",        "name": "NWL23",      "path": "lexica/NWL23.kwg",
      "sha256": "3e74af981fdd974e107283f686da0fe4b7ec84ad0d825d444330c338c33b91ba",
      "bytes": 4719596, "tarball_date": "20251004" },
    { "role": "klv",        "name": "NWL23",      "path": "lexica/NWL23.klv2",
      "sha256": "37dea945c29c3773eb4cd5a4117f3d3256c8b548cd3bf3b5ce8a219cb5e0a3fa",
      "bytes": 3667340, "tarball_date": "20251004" },
    { "role": "winpct",     "name": "winpct",     "path": "strategy/winpct.csv",
      "sha256": "51b651f149760a2b3385e8c2ef979032299a1e35ccb23170b4f492126e363a81",
      "bytes": 836150,  "tarball_date": "20251004" },
    { "role": "letterdist", "name": "english",    "path": "letterdistributions/english.csv",
      "sha256": "e698ce0b93e025daccd3390107a914d74581c4603de38df65131768b1c6f9102",
      "bytes": 489,     "tarball_date": "20251004" },
    { "role": "layout",     "name": "standard15", "path": "layouts/standard15.txt",
      "sha256": "c1a81c35a7730f64abcca23c8c56cc872e892a68f95289a4189b373c8cc5a01f",
      "bytes": 244,     "tarball_date": "20251004" }
  ]
}
```

(Digests are the real ones from `data-20251004.tgz`.) `role` and `name` are what
the client resolves through `data_filepaths`; `path` and `tarball_date` exist for
the message it prints. The list is the deduplicated union over the job and its
players (§7.5): a `games` job whose two players run different lexicons carries
two `kwg` and two `klv` entries; one whose players share a config carries one of
each. A `leave_generation` task carries exactly three — `kwg`, `letterdist`,
`layout` (§3). A client that does not recognise `algorithm` runs
unverified and warns once — refusing work because the server named a newer hash
would turn an algorithm change into a fleet-wide outage, and `min_magpie_version`
is the correct lever for that.

### 8.3 The lexicon leaves the request

Moving the lexicon onto players (§7.1) changes the request shapes, and this is
the one part of this design that is **not** additive:

- `GameRequest` loses its top-level `lexicon`; `player1.lexicon` and
  `player2.lexicon` become required rather than nullable overrides.
- `OpeningRackRequest` loses its top-level `lexicon`; `player.lexicon` becomes
  required.
- `LeaveRequest` **keeps** its top-level `lexicon`, because leave generation has
  no player spec (§7.3).

MAGPIE-CLIENT.md §10 asks for the worker API to be extended only additively, and
this violates it. With nothing deployed it is not a break at all: the client is
mid-development (MAGPIE-CLIENT.md phase 4 is partial, leave generation
unwritten), birdtest is not live, and the only reader is `contribute` on the
`birdtest-contribute` branch. So the field simply goes, in one coordinated
change across the two repositories — no version gate, no deprecation window, no
shim to delete later. `min_magpie_version` still moves to the release
that reads the new shape, so that once there *are* releases, an older one is
refused rather than silently reading `lexicon: null` and configuring a player
with no lexicon.

That window closes at launch, and it closes for every field in §8, not just this
one. Anything here known to be wrong gets fixed before the first release.

### 8.4 Declining a claim

```
POST /api/worker/decline
{ "claim_token": "…",
  "reason": "missing_data",
  "missing": [ { "role": "kwg", "name": "CSW24", "expected": "…", "actual": null } ] }
-> 204
```

`reason` is `missing_data`, `magpie_version`, or `unknown_job_type` (§10.2);
`missing` is present only for the first.

`actual: null` means the file was not found at all; a hex string means it was
found with different content. The server derives task and job from the token,
releases the claim immediately rather than waiting out the heartbeat timeout,
and records the gap (§9.3).

### 8.5 The shutdown directive

When a worker can do nothing that exists, `POST /api/worker/task` answers `200`
with a body that has no `claim_token`:

```json
{ "shutdown": {
    "reason": "data_out_of_date",
    "message": "Every active job needs input data you do not have.",
    "required_tarball_dates": ["20260101"],
    "required_magpie_version": null,
    "download_url": null } }
```

`reason` is `data_out_of_date`, `magpie_too_old`, or `both` (§10.3).

It is only sent to a worker whose own `unsupported_jobs` list ruled out
everything, which — now that the claim body is required (§8.1) — is every
worker that can claim at all.

`204` keeps its existing meaning: **there is no work right now**, sleep and ask
again. The two are genuinely different and must not be conflated — one is a
quiet server, the other is a worker that will never be useful again until its
data changes.

---

## 9. Capability negotiation

### 9.1 Client side

1. On receiving an assignment, resolve each `expected_data` entry with
   `data_filepaths_get_readable_filename()` (MAGPIE `src/ent/data_filepaths.h:31`)
   and the matching `data_filepath_t` — `kwg` → `DATA_FILEPATH_TYPE_KWG`, `klv`
   → `DATA_FILEPATH_TYPE_KLV`, `winpct` → `DATA_FILEPATH_TYPE_WIN_PCT`,
   `letterdist` → `DATA_FILEPATH_TYPE_LD`, `layout` → `DATA_FILEPATH_TYPE_LAYOUT`.
   Using the same resolver the executor uses is the point: it checks the file
   that will actually load, across the whole `data_paths` search list. A
   contributor with both a `download_data.sh` install and a MAGPIE-DATA clone on
   `data_paths` has two `english.csv` files, and only the resolver knows which
   wins. **Every message prints the resolved absolute path**, because "your
   english.csv does not match" is unactionable when there are two of them and
   the message does not say which.
2. Hash each with SHA-256 and compare. Cache in `ClientState` by
   **(resolved path, size, mtime, inode, ctime)**, so a file is hashed once per
   process rather than once per task. The inode and ctime are not decoration:
   `(path, size, mtime)` alone collides when a file is replaced with different
   bytes of the same size inside one mtime tick, which is exactly what archive
   extraction does, and a stale cache entry is the one way a bad file passes
   verification.
3. **Any missing or mismatched file:** `POST /api/worker/decline` with the
   details, add the `job_id` to an in-memory unsupported set, do not start the
   heartbeat, do not run the task, and go straight back to claiming. Print one
   line per newly-discovered gap — not once per claim, or a client with a
   missing lexicon becomes a log firehose. "Newly-discovered" is keyed by
   (resolved path, expected digest): the first occurrence logs at warn, repeats
   are silent until the key changes, and the shutdown summary in step 6 covers
   the after-the-fact view.
4. **All files match:** proceed exactly as today — heartbeat, execute, submit.
5. Send the unsupported set with every subsequent claim.
6. **On a `shutdown` directive:** print the accumulated gaps — which the client
   knows in full detail, file by file — followed by the server's message and the
   `download_data.sh` remedy, then exit cleanly through the `ErrorStack` the
   other `impl_*` entry points use, so a GUI driving `contribute` in
   `-mode async` gets one terminal state rather than a scrolling failure.

```
Cannot contribute to any available job.

Missing or outdated input data:
  lexica/CSW24.kwg          not found in any data path (./data)
  strategy/winpct.csv       has sha256 4f2a…, jobs require 51b651f1…

These come from MAGPIE-DATA data-20260101 or later. Run ./download_data.sh
from your MAGPIE directory to update, then start contribute again.
```

**The unsupported set is in memory only.** It is never written to
`contribute.txt`, and the assumption behind that is explicit: a contributor who
stops and restarts `contribute` has, in the case that matters, just updated
their data — that is what the shutdown message told them to do. A client that
remembered its limitations across restarts would refuse work it can now do, and
the only cure would be a config file the user has to know to edit. Forgetting
costs one wasted claim per job on the next run.

### 9.2 Scheduler side

`candidate_jobs` ([scheduler.rs:26](backend/src/scheduler.rs#L26)) takes the
unsupported set *and the worker's MAGPIE version* (§10.1) and excludes every job
either rules out — and it must do so **before** computing `MIN(priority)`, not
after. Both filters feed a single `eligible_jobs` CTE and the priority is
computed over its output, so the ordering is structural rather than
remembered. Filtering afterwards would compute the top priority
tier from jobs the worker cannot do, then hand back nothing, and a worker locked
out of tier 0 would never see doable work in tier 1. The worker's tier must be
the top tier *among jobs it can actually run*.

The four answers are one decision, not four checks: compute them in one function
returning `ClaimOutcome { Task, Idle, Unsupported, Shutdown }` and map to HTTP
once at the edge, so the compiler enforces that every branch is considered —
which scattered early returns cannot. Shutdown detection then falls out
precisely:

- Candidate jobs remain after filtering, and one has an available task → assign
  it.
- Candidate jobs remain, none has an available task right now → `204`.
- There are active jobs, but the unsupported set excludes **all** of them →
  `shutdown`.
- There are no active jobs at all → `204`. A quiet server is not the worker's
  fault, and telling a contributor to update their data because nothing is
  running would be actively wrong.

### 9.3 Declines are the observability

Declining releases the claim the same way reclamation does, and the bookkeeping
must match `reclaim_expired` exactly: set the claim state, decrement
`tasks.active_claim_count`, and recompute the task's state so it becomes
available again. Rather than writing that twice, both paths call one
`release_claim(tx, claim_id, terminal_state)`, differing only in the state they
pass — which is the only thing that should differ. Add `'declined'` to the
`claim_state` enum rather than reusing `'abandoned'` — the two mean different
things, and only one of them is diagnostic.

That enum addition has a sharp edge worth catching in review: the unique indexes
`task_claims_user_unique_idx` and `task_claims_anon_unique_idx` are partial on
`WHERE state != 'abandoned'`. They must become
`WHERE state NOT IN ('abandoned', 'declined')`, or a worker that declines a task
is permanently barred from claiming it again after fixing its data.

```sql
CREATE TABLE worker_data_gaps (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id       UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    claim_id     UUID NOT NULL REFERENCES task_claims(id) ON DELETE CASCADE,
    role         TEXT NOT NULL,
    name         TEXT NOT NULL,
    expected     TEXT NOT NULL,
    actual       TEXT,                    -- NULL = file absent
    reported_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX worker_data_gaps_job_idx ON worker_data_gaps (job_id, role, name);
```

This is worth more than it looks. A job pinned to data nobody has does not
announce itself — it just quietly gets no work done, and the server-side symptom
is an absence: claims issued, no results. `worker_data_gaps` turns that absence
into a statement: *"job X: 14 workers, all missing `lexica/CSW24.kwg`"*. It also
answers "what is actually installed out there", which is what tells an admin
whether the fleet has picked up a new tarball yet, and therefore whether a job
pinned to it will find anyone to run it.

**Scheduling uses the client's list, not this table.** The server records gaps
for humans; it does not use them to route. If it did, a contributor who updated
their data would stay blocked by a record of a problem they had already fixed,
and the only cure would be a server-side reset nobody would remember to run. The
client sending its own set each time makes the state self-correcting.

---

## 10. MAGPIE version negotiation

Client capability has two axes, and they behave identically: a worker either has
the data a job needs or it does not, and it either has a new enough MAGPIE or it
does not. §9 routes around the first. This section applies the same shape to the
second, replacing the current arrangement where the server dispatches work and
the client discovers afterwards that it cannot run it.

**What exists today.** `jobs.min_magpie_version` is a nullable semver `TEXT`; the
server sends it with the assignment; the client compares and, if it falls short,
stops the loop entirely (MAGPIE-CLIENT.md §4 step 3). `GET /api/worker/client-version`
advertises a global floor from `MIN_MAGPIE_VERSION` (`0.0.0` today). So a client
below a job's floor burns a claim per attempt, and one job it cannot run ends its
whole session even when every other job is within reach.

### 10.1 The client states its version; the server filters

`POST /api/worker/task` carries it alongside the unsupported set:

```json
{ "magpie_version": "1.4.0",
  "unsupported_jobs": ["4c7b64ad-8e5e-4db7-aeb0-afc44ee1ebf5"] }
```

The scheduler excludes any job whose minimum exceeds it, in the same pass and
with the same ordering requirement as §9.2 — **before** `MIN(priority)`, so a
worker locked out of the top tier still sees work below it.

`min_magpie_version` becomes three integer columns rather than one `TEXT`:

```sql
    -- Minimum MAGPIE for this job, as sortable parts. Semver in TEXT compares
    -- lexically, where '1.10.0' < '1.9.0' -- a bug that appears only once a
    -- minor version reaches double digits, i.e. long after it is written.
    min_magpie_major INT NOT NULL DEFAULT 0 CHECK (min_magpie_major >= 0),
    min_magpie_minor INT NOT NULL DEFAULT 0 CHECK (min_magpie_minor >= 0),
    min_magpie_patch INT NOT NULL DEFAULT 0 CHECK (min_magpie_patch >= 0),
```

Postgres compares row constructors element-wise, so the filter reads directly and
needs no function:

```sql
WHERE (j.min_magpie_major, j.min_magpie_minor, j.min_magpie_patch)
      <= ($1, $2, $3)
```

**The floor is no longer optional.** Every job now pins input data — at minimum a
letter distribution and a layout (§7.2) — and a client too old to understand
`expected_data` will contribute unverified rather than decline. So
`min_magpie_version` stops being nullable: it defaults from config at job
creation and an admin may raise it per job. "No floor" is not a state worth
being able to express once every job depends on the client honouring a protocol.

**The floor is `0.0.1`** — the migration default for
`min_magpie_{major,minor,patch}` and the value of `MIN_MAGPIE_VERSION`, replacing
today's `0.0.0` ([config.rs:88](backend/src/config.rs#L88)). It is a placeholder
for the MAGPIE release that implements the check, to be raised to that release's
real number before launch. Because a stale config value silently floors every
new job too low, the effective value is shown on the job creation form,
pre-filled and editable, with its age beside it — a visible default
rather than a hidden one.

**An unparseable version** is treated as `0.0.0`, which under the floor above
means the client is offered nothing and told to update. An *absent* version is
no longer a case: the claim body is required (§8.1), so a claim without one is
rejected outright.

The assignment still carries `min_magpie_version` as a formatted string. The
server filtering is the mechanism; the client's own comparison stays as a
cross-check, because a client that somehow receives work above its version should
refuse it rather than run it.

### 10.2 Version mismatch is a decline, not an exit

If the client does receive a job above its version — a server bug, a race with a
job whose floor was just raised — it declines exactly as it would for missing
data, with `reason: "magpie_version"`, and adds the job to its in-memory
unsupported set. The set is not "jobs whose data I lack"; it is **jobs I cannot
do**, whatever the cause.

That generalisation resolves an existing rough edge. MAGPIE-CLIENT.md §4 step 3
says an unrecognised `job_type` means "the server is newer than this MAGPIE, so
exit". Under this design it is one more reason to decline: a client that cannot
do `leave_generation` because it predates that executor can still play `games`
all day. Exit is reserved for the case where *nothing* is doable, which §9.2
already detects.

### 10.3 Shutdown says which remedy

The directive from §8.5 carries a reason, because "update MAGPIE" and "update
your data" are different actions:

```json
{ "shutdown": {
    "reason": "magpie_too_old",
    "message": "Every active job requires MAGPIE 1.6.0 or newer; you are running 1.4.0.",
    "required_magpie_version": "1.6.0",
    "download_url": "https://github.com/jvc56/MAGPIE",
    "required_tarball_dates": [] } }
```

`reason` is `magpie_too_old`, `data_out_of_date`, or `both`. When both apply,
say so but **lead with the MAGPIE version**, because updating MAGPIE is the
remedy that fixes both: a release bumps `DATA_VERSION`, and the contributor runs
`download_data.sh` as part of updating. Telling someone to fix their data first
sends them on a trip they would have made anyway.

The global floor short-circuits all of this: a client below `MIN_MAGPIE_VERSION`
gets `magpie_too_old` on its first claim without any job being consulted.

### 10.4 Recording what the field is running

`task_claims` gains one column:

```sql
    magpie_version TEXT,   -- as reported at claim time; NULL for clients predating §10.1
```

This is not bookkeeping for its own sake. §12 records that keeping birdtest's
pinned data rows in step with MAGPIE's `DATA_VERSION` is a human decision, and
this column is what informs it: "how many distinct workers claimed anything in
the last week, and what were they running" is one query, and it is the difference
between raising a job's floor on evidence and raising it on hope. Together with
`worker_data_gaps` (§9.3) it covers both axes — who is behind on code, and who is
behind on data.

---

## 11. Client details that are easy to get wrong

### 11.1 The check runs before the heartbeat

Between the version gate (MAGPIE-CLIENT.md §4 step 3) and starting the heartbeat
thread (step 4). Hashing ~9 MB takes single-digit milliseconds, and a decline
should not look like a worker that started and died.

### 11.2 SHA-256 in MAGPIE

`src/util/` has no hash implementation. Vendor SHA-256 as `src/compat/sha256/`
alongside the existing `src/compat/cjson` precedent, and wrap it in
`src/util/hash.{h,c}`:

```c
// SHA-256 of the file at `path`, lowercase hex.
char *sha256_hash_file(const char *path, ErrorStack *error_stack);
```

Streaming, 64 KB at a time — a `.kwg` is up to 15 MB and there is no reason to
map it.

### 11.3 The wordmap hole

`.wmp` gets no row — it is ~104 MB, absent from the tarball, and built locally
from the `.kwg` — but a client that updates its `.kwg` and keeps a wordmap built
from the *old* one passes every check while running exactly the corruption this
design exists to prevent.

So MAGPIE-CLIENT.md §7's provisioning gains a condition. Alongside
`<lexicon>.wmp`, write `<lexicon>.wmp.src` containing the verified `.kwg`
digest. Rebuild when the `.wmp` is absent, **or** `.wmp.src` is absent, **or**
its contents differ from the `.kwg` digest just verified. Cost is the ~1.3 s
`kwg → txt → wmp` chain, paid once per lexicon update. Write the sidecar with
the generate-to-temp-then-`rename()` discipline §7 already specifies, and only
*after* the `.wmp` is renamed into place, so an interrupted build re-runs rather
than being trusted.

This matters more under tarball distribution than it would otherwise: a
contributor upgrading MAGPIE re-runs `download_data.sh`, which overwrites the
`.kwg` in place and leaves any `.wmp` built from the previous one beside it.

A `.wmp` with no sidecar at all — which is every wordmap a contributor already
has — is treated as stale and rebuilt once. That is correct and costs one
rebuild.

A client running an unpinned job has no digest to compare and keeps today's
behaviour: use the wordmap if present.

---

## 12. Operational consequences

**A job can now be created that nobody can run.** Pinning a job to a
just-imported tarball while every released MAGPIE still installs the previous
one means every worker declines it. That is no longer silent: the workers keep
contributing to other jobs, `worker_data_gaps` fills with a single repeated
answer, and the admin UI can say which file and how many workers. The remedy is
an admin decision — wait for the MAGPIE release that bumps `DATA_VERSION`, or
pin the job to the older rows.

Visible is not the same as noticed, though, so the job list carries a **stalled**
badge: at least one decline and zero submissions in the last 24 hours, with no
active claims. A stricter variant catches a bad pin the same day it is made — a
job older than an hour with zero submissions ever and at least one decline. Both
are computed from rows already written, shown where an admin already looks. There
is deliberately no alert: notifying on the stalled transition is the only thing
that works when nobody is looking, and it needs a channel birdtest does not have.
Revisit if a job ever stalls unnoticed.

**The MAGPIE floor is what makes any of this binding.** A client that ignores
`expected_data` never declines and contributes unverified — capability
negotiation cannot route around a client that does not speak it. Two things stop
that. The claim body is required, so a client that does not send a version
cannot claim at all (§8.1); and `min_magpie_version` is non-nullable with a
real floor (§10.1), so a client that sends one too low is offered nothing. With
every job pinning data, a job without a floor would be a job whose verification
is optional.

**Adoption order follows cost.** Pin a `games` job first: it is the cheapest
place to discover that a resolution rule or a message is wrong, since a declined
claim costs one round trip. Then `leave_generation`, whose bad data propagates
into later generations and cannot be subtracted back out — and which, once the
mechanism is trusted, should never run unpinned.

**Keeping the pinned rows and `DATA_VERSION` in step is a human job.** birdtest
does not read `download_data.sh`, does not warn when an import is newer than what
the released client installs, and will not grow a mechanism for it now — the
coupling is real but it moves at the speed of MAGPIE releases, which is slow
enough for a person to handle. What makes that workable is evidence rather than
automation: `task_claims.magpie_version` (§10.4) says what the fleet is running
and `worker_data_gaps` (§9.3) says what it is missing, so the decision to raise a
job's floor is made by looking. **This belongs in birdtest's README**, in the
admin runbook: import a tarball only when a MAGPIE release installs it, and check
both tables before pinning a job to it.

**The CI end-to-end test guards the two constants.** It compiles MAGPIE, runs
that MAGPIE's `download_data.sh`, and runs one task from a pinned job against a
seeded stack. If birdtest's pinned rows name content that MAGPIE does not
install, the client declines, the task never completes, and the CI job fails the
pull request — so the mismatch surfaces as a red build on a branch rather than
as a dead job in production. It proves the pin agrees with the MAGPIE in CI, not
with the MAGPIE contributors are running; `worker_data_gaps` is what covers the
difference.

---

## 13. What this deliberately does not do

- **It does not constrain a hostile contributor.** A digest the client computes
  is a digest the client can fabricate, and a client can decline work it is
  perfectly capable of. This targets the real and current threat — an honest
  contributor with stale, missing, or off-channel files. Constraining a hostile
  one needs output-side checks: redundant claims compared for equality (games
  are deterministic, which the scheduler already assumes,
  [mod.rs:88-91](backend/src/jobs/mod.rs#L88-L91)) or occasional canary tasks
  whose answers the server already knows. Complements, not alternatives.
- **It does not secure the distribution channel.** `download_data.sh` fetches
  over HTTPS with no signature and no checksum. This design detects, for pinned
  jobs, that what landed is not what birdtest expects. A signed manifest shipped
  with the tarball is the real answer and belongs in MAGPIE-DATA.
- **It does not verify the binary.** `min_magpie_version` stays the floor. A
  hash of the executable is close to useless — it differs by platform and
  compiler for builds that are semantically identical.
- **It does not serve data.** The server names the file a contributor is missing
  and the tarball to get it from; it does not hand over bytes. Import already
  downloads the tarball, so serving it through `GET /api/worker/artifact` is a
  small step — but it is a distribution feature with a storage cost and a
  redistribution question, and `download_data.sh` already exists.
- **It does not accept out-of-tarball files.** Every `input_data` row comes from
  an imported MAGPIE-DATA tarball. An admin endpoint that uploads arbitrary
  bytes as a row was considered and deferred: it is the right answer the first
  time a real hand-built lexicon or layout needs pinning, and it is a door
  through which unverifiable bytes enter the vocabulary, so it is not being
  built for a 108-byte test fixture (§5.1).
- **It does not defend against a client that spins.** A client that declined a
  job for missing data and then failed to record that fact would re-claim the
  same task immediately, looping as fast as the network allows. Tracking the
  unsupported set is the entire point of the decline path, so a client that
  omits it is broken in a way that would not survive its first run; no
  mitigation is designed for it. The per-worker rate limit at
  [worker.rs:98](backend/src/routes/worker.rs#L98) bounds the damage
  incidentally, and `worker_data_gaps` records declines per identity, so the
  behaviour would be visible without anything being built for it.

---

## 14. Work breakdown

**birdtest — schema and import**

1. `0001_initial.sql`: `input_data` **including `content BYTEA` and its
   role/NOT-NULL equivalence check** (§5, §5.1), `input_data_imports` with
   `running`/`failed` states, progress columns, `error`, and a nullable
   `tarball_sha256` (§6), `input_data_import_rows` with `content`,
   `worker_data_gaps`; `kwg_id` / `klv_id` / `winpct_id` / `cloned_from_id` on
   `player_configs` (§7.1); `variant` / `letterdist_id` / `layout_id` moved up
   to `jobs`; the per-type tables reduced to what is per-type, with
   `job_leave_config` keeping `kwg_id` and gaining nothing else (§7);
   `'declined'` on `claim_state` **and** the two partial unique indexes widened
   to `WHERE state NOT IN ('abandoned','declined')` (§9.3);
   `min_magpie_{major,minor,patch}` replacing `min_magpie_version` on `jobs`,
   non-nullable, defaulting `(0,0,1)` (§10.1); `magpie_version` on
   `task_claims` (§10.4). `input_data` goes at the top of the file with a
   comment saying why — `jobs` and `player_configs` reference it (§7).
2. `backend/src/inputdata.rs` — chunked tarball fetch mirroring
   `download_data.sh`, streaming gunzip/untar, per-entry SHA-256, path →
   (role, name) mapping, the §6 allowlist and caps, retention of
   `letterdist`/`layout` bytes, diff against `input_data`. Runs as a spawned
   background task writing progress and terminal state onto the import row;
   startup fails any row left `running` (§6). Add `reqwest`, `flate2`,
   `tar`; add `MAGPIE_DATA_REPO` / `GITHUB_TOKEN` to
   [config.rs](backend/src/config.rs).
3. `POST /api/admin/input-data/imports` (returns `202` and an id), `GET .../<id>`
   (polled for progress, staged diff, or failure), `POST .../<id>/confirm`,
   plus listing `input_data` for the job form.
4. Job and player config creation: FK fields, role validation, the Rust port of
   MAGPIE's `lexicons_and_leaves_compat` / `ld_types_compat` rules with its
   known-good/known-bad table (§7.4), the sim/`winpct_id` pairing rule, and the
   "clone this config onto newer data" path — which generates the
   `base@tarball_date` name and sets `cloned_from_id` (§7.1, §7.4).
5. **Remove `DATA_PATH`.** `LetterDistribution` gains a bytes-taking
   constructor and loses the path-taking one; `total_racks`, `expand`,
   `seed_generation` and KLV building take the job's pinned `letterdist`
   content; `data_path` comes out of `Config`, `AppState`,
   [registry.rs](backend/src/jobs/registry.rs),
   [handler.rs](backend/src/jobs/handler.rs) and the handlers; `COPY data
   /app/data` and `ENV DATA_PATH` come out of the Dockerfile and compose file
   (§5.1, §7.5). `data/letterdistributions/testdist.csv` moves into the test
   tree as compiled-in fixture bytes and `english.csv` is deleted — the English
   round-trip test already has `MAGPIE_DATA_PATH` on its search path, and the
   `testdist` round-trip writes its fixture into the temp dir it already
   creates, which retires the "non-MAGPIE-DATA distributions need birdtest's own
   data dir" workaround at [klv.rs:404](backend/src/jobs/klv.rs#L404).
6. **Generation-0 KLV.** `initialize_job_state` builds a zeroed KLV over the
   job's leave universe and stores it at `leaves/<job>/generation-0.klv2`,
   outside the creating transaction, with its `leave_generation_artifacts` row;
   `previous_artifact_key` becomes non-nullable and generation 1 stops being a
   special case ([leave_gen.rs:194](backend/src/jobs/leave_gen.rs#L194), §3).

**birdtest — dispatch and scheduling**

7. `expected_data` builder — a deduplicated join over the job's and players'
   FKs (§7.5) — plus the request-shape change in §8.3, and `ClaimOutcome` /
   `TaskAssignment` wiring
   ([worker.rs:83-93](backend/src/routes/worker.rs#L83-L93)).
8. `unsupported_jobs` and `magpie_version` on a **required** `Json<ClaimBody>`,
   the list capped at 200 and bound as `bigint[]`; `candidate_jobs` filtering on
   both inside an `eligible_jobs` CTE, before the priority computation (§8.1,
   §9.2, §10.1).
9. `POST /api/worker/decline`: release the claim through the shared
   `release_claim` that `reclaim_expired` also calls, write `worker_data_gaps`
   (§9.3).
10. Shutdown directive, its four-way decision (§9.2) and its three reasons
   (§10.3); the global-floor short-circuit.
11. `DELETE /api/admin/input-data/<id>`, relying on the foreign keys to refuse a
   referenced row (§5).

**birdtest — admin UI**

12. Import wizard (date → progress poll → staged diff → confirm), `input_data`
    browser with delete, job and player forms that pick rows rather than type
    names, a per-job data-gap report, a fleet view over
    `task_claims.magpie_version` (§10.4), the config clone action showing
    lineage (§7.1), a **stalled** badge on the job list (§12), and the
    effective `min_magpie_version` shown on the job creation form (§10.1).

**MAGPIE**

13. Vendored SHA-256 + `hash.{h,c}`.
14. `expected_data` parsing, the check, the `ClientState` digest cache, the
    in-memory unsupported set, decline, and the shutdown path (§9.1).
15. Send `magpie_version` on every claim; decline rather than exit on a version
    or `job_type` it cannot handle (§10.2).
16. Wordmap `.wmp.src` invalidation, treating a missing sidecar as stale
    (§11.3).

**Shared**

17. Contract fixtures for an assignment carrying `expected_data`, a claim
    carrying `unsupported_jobs` and `magpie_version`, a decline, and each
    shutdown reason — committed to both repos as `contract-fixtures/`, the cheap
    version of pinning a cross-repo contract that MAGPIE-CLIENT.md already asks
    for.

All MAGPIE-side work (13–16) lives on the `birdtest-contribute` branch, and
§8.3's request-shape change lands in both repositories together.

Order: 1–7 first (they prove dispatch end to end against `fake_worker.py`), then
8–11 with the fake worker driving the negotiation, then 13–15, then 12, 16, 17.

## 15. Tests

- **Import**, against a fixture tarball built the same way (`cp -RL` +
  `tar -czf` + `split`): chunk reassembly walks `aa`→`ab`→`ac`, the unchunked
  fallback works, a symlink or `..` entry is rejected, role mapping is correct,
  and a missing version gives a clean "no such tarball" rather than a transport
  error. Each cap aborts the whole import rather than skipping the entry, and an
  archive exceeding the 20× ratio aborts mid-walk rather than at the end.
- **Import lifecycle**: the endpoint returns `202` with an id before the
  download finishes; the row moves `running` → `staged` with progress advancing;
  a fetch failure lands `failed` with a message; a row left `running` is failed
  at startup.
- **`content` round-trip**: import stores the `letterdist` and `layout` bytes,
  confirmation copies them into `input_data`, and the digest of the stored bytes
  equals the row's `sha256`. A `kwg` row with `content`, and a `letterdist` row
  without, are both rejected by the check constraint.
- **Server-side reads use the row, not a path**: two `letterdist` rows with the
  same name and different bytes produce different `total_racks` for otherwise
  identical jobs — the §5.1 failure, and the only test that would have caught
  it.
- **Diff semantics**: importing the same tarball twice stages zero new rows;
  importing a tarball where one file changed stages exactly one row and marks it
  a collision; confirming inserts only new rows and sets `tarball_date` to the
  import's date.
- **Real import of `data-20251004` matches a local extraction**, gated behind a
  network-tests flag — the strongest available check that mapping and hashing
  agree with what a contributor's disk holds.
- **Config validation**: a `kwg_id` pointing at a `letterdist` row is rejected;
  an incompatible lexicon/leaves pair is rejected; two players whose lexicons are
  mutually incompatible, or incompatible with the job's letter distribution, are
  rejected; a simming player without `winpct_id`, and a static player with one,
  are both rejected.
- **The compatibility port**: a table of known-good and known-bad
  (lexicon, leaves, letter distribution) combinations, asserted against the Rust
  port of MAGPIE's rules (§7.4). This is the test that catches drift between the
  two implementations, so the table is the artifact worth maintaining — an
  uncovered combination must be rejected, not guessed.
- **Generation-0 KLV**: creating a leave job writes
  `leaves/<job>/generation-0.klv2`, it parses, the sum of its values is exactly
  zero, generation 1's request carries that key, and no generation carries a
  null `previous_artifact_key`.
- **Config cloning**: a clone gets the `base@tarball_date` name, sets
  `cloned_from_id`, and starts with no `player_config_ratings` rows.
- **`expected_data` composition**: two players on different lexicons produce two
  `kwg` and two `klv` entries; the same player config on both sides produces one
  of each; a static player contributes no `winpct` entry; a `leave_generation`
  job produces exactly `kwg`, `letterdist`, `layout` and never a `klv` or
  `winpct`.
- **Scheduler**: a worker whose `unsupported_jobs` covers the whole top priority
  tier is offered work from the next tier, not shut down — the §9.2 ordering
  bug, which is the one worth a dedicated test.
- **Scheduler**: all active jobs unsupported → shutdown; no active jobs → `204`;
  an empty unsupported list → today's behaviour exactly.
- **Decline bookkeeping**: `active_claim_count` decrements, the task returns to
  `available`, and the same worker can claim that task again after declining —
  the partial-index trap from §9.3.
- **Version filtering**: a worker on `1.9.0` is offered a job requiring
  `1.10.0`'s predecessor and *not* one requiring `1.10.0` — the lexical-compare
  trap from §10.1, which no other test would catch; a worker below the global
  floor gets `magpie_too_old` without any job being consulted.
- **Claim body required**: a claim with no body and no `Content-Type` is
  rejected with an error naming the fix, not a bare `422`; an
  `unsupported_jobs` list longer than 200 is truncated rather than rejected, and
  a list containing junk ids is cleaned before binding (§8.1).
- **Shutdown reasons**: data-only, version-only, and both, with `both` leading on
  the version.
- **`fake_worker.py`**: modes that decline a claim (each reason), accumulate an
  unsupported set across claims, and claim with an old or absent version, so the
  whole negotiation is exercised with no MAGPIE in the loop.
- **MAGPIE**: `sha256_hash_file` against known vectors; a mismatch declines
  rather than exits; a shutdown directive exits cleanly with the accumulated
  gaps printed; the digest cache invalidates when a file is replaced with
  same-size bytes inside one mtime tick — the collision the inode and ctime
  exist to close (§9.1) — and a gap logs once rather than once per claim; a
  `.wmp` with no `.wmp.src` is rebuilt; the unsupported set does *not* survive a
  restart.
