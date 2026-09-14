# Rack info tables and wordmaps: knowing what they were built from

Status: **proposal, nothing implemented.** Written 2026-09-14 against MAGPIE
`birdtest-contribute` at `fa6dfcf9` and birdtest `audit/birdtest-2026-09-14-pass2`.

## Summary

The first two sections explain birdtest and how its data flows from MAGPIE-DATA
to workers, for readers new to the project. The rest describes the problem and
the options.

A rack info table (`.rit`) and a wordmap (`.wmp`) are both built locally from
other data files: a `.wmp` from a `.kwg`, and a `.rit` from a `.klv2` and a
`.wmp`. Neither file records what it was built from. MAGPIE finds both **by
lexicon name alone** and uses them as-is. If the inputs have changed, or the
player loaded different leaves, moves are generated or ranked on data that
doesn't match what the player actually loaded, and nothing reports it.

For wordmaps, contribute already catches this with a sidecar digest. For rack
info tables nothing does, so today they are turned off throughout contribute,
and birdtest refuses `use_rit`. The table speeds up move generation, so that
setting costs speed on every job, not just the ones where it would be unsafe.

This document proposes recording the SHA-256 of each file's **inputs** in its
header and checking those digests at load time, reusing the hashing MAGPIE
already has. Only the small input files get hashed, never the large derived
files. With that check in place, both files become safe to use on every job,
and birdtest can accept `use_rit` again.

## birdtest in brief

birdtest is a crowdsourced analysis platform for word games, modelled on
Fishnet, the system that crowdsources chess analysis for Lichess. It has three
parts:

- **The server** (Rust, Postgres). Admins define long-running **jobs**, and the
  server splits each one into small **tasks** as volunteers ask for work. It
  then combines the returned results: win rates and SPRT decisions for games,
  per-rack tables for opening racks, and a new set of leave values for each
  round of leave generation.
- **Workers.** Volunteers run MAGPIE, the word-game engine, in `contribute`
  mode. A worker claims a task over HTTP, runs it, and posts back a JSON
  result.
- **The data.** A worker runs every task against lexical data files on its own
  disk, so birdtest has to make sure every worker uses the same bytes.

There are four job types:

| Job type | What a task does | What the server builds from results |
|---|---|---|
| `games` | Play a batch of games between two player configs | Win/loss/draw totals, SPRT, ratings |
| `game_pairs` | The same, as matched pairs (same seed, seats swapped) | Pair outcomes, SPRT, ratings |
| `opening_rack` | Rank the plays for a range of opening racks | One row per rack |
| `leave_generation` | Play games with the current leave values, forcing chosen racks | The next generation's leave values |

**Why exact data matters.** Workers are volunteer machines that birdtest
doesn't control. A worker with slightly different data doesn't crash. It plays
slightly different games and returns results that look normal and pass every
check. Mixed in with everyone else's, those results can push SPRT to a
confident wrong conclusion. In leave generation they carry into every later
generation, because each generation plays with the leave values the previous
one produced. So birdtest pins every input file by SHA-256, and a worker whose
files don't match declines the task rather than running it.

### Terms

| Term | File | What it is |
|---|---|---|
| Lexicon | `lexica/<name>.kwg` | The word list, stored as a compressed word graph (KWG) |
| Leaves | `lexica/<name>.klv2` | Leave values (KLV): how much the tiles kept after a move are worth |
| Win % model | `strategy/<name>.csv` | Used only by players that simulate |
| Letter distribution | `letterdistributions/<name>.csv` | The alphabet, tile counts and scores |
| Board layout | `layouts/<name>.txt` | Premium squares |
| Wordmap (WMP) | `lexica/<lexicon>.wmp` | A faster lookup structure built locally from the `.kwg`; never downloaded |
| Rack info table (RIT) | `lexica/<lexicon>.rit` | Precomputed facts about every full rack, built locally from a `.klv2` and a `.wmp`; never downloaded |
| MAGPIE-DATA | GitHub repo `jvc56/MAGPIE-DATA` | Publishes dated tarballs of the first five file types |

## How lexical data gets into birdtest and out to workers

A data file passes through five stages. The wordmap and the rack info table
enter only at the last one, which is why they fall outside every check.

### 1. MAGPIE-DATA publishes a tarball

MAGPIE-DATA publishes versioned tarballs such as `data-20251004.tgz`, split into
40 MB chunks (`.aa`, `.ab`, …; about 94 MB in total). The tarball contains
`.kwg`, `.klv2`, letter distribution, layout and win % files, but no `.wmp` or
`.rit`.

Contributors install it with MAGPIE's `download_data.sh`, which fetches the
chunks for a data version fixed in the script, joins them and extracts them
into `./data`. **The script verifies nothing**: no checksum, no signature. It
also overwrites files in place and leaves any locally built `.wmp` or `.rit`
beside them.

### 2. An admin imports the tarball into birdtest

Importing is an admin action, done once per data release. It happens in two
phases (`backend/src/inputdata.rs`; PLAN.md, *Importing a tarball*).

**Phase 1: fetch, hash and compare** (runs in the background; the admin UI
polls it):

1. Resolve the git ref (for example `main`) to a commit SHA, so the import
   records a fixed commit, not a branch.
2. Download the chunks from that commit, walking the chunk names the same way
   `download_data.sh` does, with caps on bytes, chunks and time.
3. Stream the joined bytes through SHA-256 to get the tarball's own digest,
   then gunzip and untar. Every entry must be a regular file with a relative
   path of the form `data/<dir>/<basename>`; anything else aborts the whole
   import. Limits on total size, per-entry size, entry count and compression
   ratio guard against malformed or malicious archives.
4. Classify each entry by directory and extension. Anything else in the
   tarball is ignored:

   | Entry | Role | Name |
   |---|---|---|
   | `data/lexica/*.kwg` | `kwg` | basename without extension |
   | `data/lexica/*.klv2` | `klv` | 〃 |
   | `data/letterdistributions/*.csv` | `letterdist` | 〃 |
   | `data/layouts/*.txt` | `layout` | 〃 |
   | `data/strategy/*.csv` | `winpct` | 〃 |

5. SHA-256 every classified file. Keep the bytes only for `letterdist` and
   `layout`, so that anything the server reads itself comes from exactly the
   row a job pins. For example, it counts the opening-rack space and numbers
   the letters in the leave files it builds from the letter distribution.
   Lexica, leaves and win % files are recorded by digest only.
6. Compare each `(path, sha256)` with the existing `input_data` table and
   stage the result.

**Phase 2: confirm.** The admin sees three groups: new files, files already
known, and path collisions (a known path with different bytes, which is either
a real data update or a tarball re-cut under an existing name). Confirming
inserts only the new rows, each labelled with the tarball date it was first
seen in.

**The import doesn't change the files.** It doesn't convert, rebuild or
normalise anything. Its output is a table of `(role, name, path, sha256,
bytes, tarball_date)` rows, one per distinct file. The same path with
different bytes becomes a separate row.

### 3. Admins build player configs and jobs from those rows

- A **player config** pins a `kwg` row and a `klv` row, plus a `winpct` row if
  the player simulates. It also holds engine settings such as `use_wordmap`
  and `use_rit`.
- A **job** pins one `letterdist` row and one `layout` row, and names its
  player configs. A leave-generation job has one bot and no player configs:
  it pins a `kwg` row itself and **no leaves**, because it starts from a KLV of
  all zeros that the server builds, not from shipped leaves.
- At creation, birdtest checks that each player's leaves suit its lexicon.
  This check (`backend/src/compat.rs`, ported from MAGPIE) compares
  **alphabets, not names**. It deliberately accepts NWL23 words with CSW21
  leaves (`compat.rs:200`), and `CSW24` words with `CSW_quackle_leaves`.
  **A player's leaves are often not its lexicon's own leaves**, and that is a
  supported configuration.

### 4. Dispatch tells the worker exactly which bytes to use

When a worker claims a task, the response includes:

- `expected_data`: every file the task will load, listing role, name, path,
  SHA-256 and tarball date. It is built directly from the rows the job and
  its players pin.
- `task_request`: the settings, referring to files **by name** (`"lexicon":
  "NWL23"`, `"leaves": "CSW21"`), along with `use_wordmap` and `use_rit` for
  each player.

A leave-generation task instead carries `previous_artifact_key`, pointing to
the KLV the server built from the previous generation's results.

### 5. The worker checks its files, then loads data by name

In `contribute` mode MAGPIE:

1. **Hashes every file in `expected_data`** (`expected_data_matches` in
   `contribute.c`). A digest cache avoids re-hashing unchanged files. On any
   mismatch the worker declines the task with the file, the expected digest
   and the actual one. The server releases the claim, and the worker stops
   asking for that job.
2. **For a leave-generation task, downloads the previous generation's KLV** and
   writes it to `lexica/<lexicon>_birdtest_previous.klv2`, overwriting the
   last one.
3. **Builds a wordmap if the task asks for one** and the `.wmp` is missing or
   stale (`kwg → txt → wmp`, about a second). Stale means the `.wmp.src`
   sidecar doesn't match the `.kwg` digest. This is the only check on a
   locally built file.
4. **Loads the lexical data by name.** Here the wordmap and rack info table
   are picked by the player's **lexicon name**, whatever leaves the player
   uses.

### Where the derived files fall outside the checks

| File | Pinned by the job? | Checked on the worker? | How MAGPIE finds it |
|---|---|---|---|
| `.kwg`, `.klv2`, letter distribution, layout, win % | Yes, SHA-256 | Yes, before the task runs | Name from the task |
| Previous-generation KLV | Named by artifact key | Downloaded fresh for each task | Fixed name, overwritten |
| `.wmp` | No (built locally, ~100–180 MB) | Contribute only, via the `.kwg` sidecar | **Lexicon name** |
| `.rit` | No (built locally, ~1.9 GB) | **No** | **Lexicon name** |

Everything birdtest checks is one of the files it imported. The wordmap and
rack info table never pass through the import, so no job pins them. MAGPIE
builds them on the contributor's machine from other files and picks them by
lexicon name alone. The rest of this document is about closing that gap.

## The two derived files

| File | Built by | Built from | Holds | Size (CSW24) |
|---|---|---|---|---|
| `.kwg` | shipped in MAGPIE-DATA | — | the word graph | 6.0 MB |
| `.klv2` | shipped, or produced by leave generation | — | leave values | 3.7 MB |
| `.wmp` | `convert text2wordmap` (via `dawg2text` from the `.kwg`) | `.kwg` | words by rack, for fast lookup | 179 MB |
| `.rit` | `convert klvwmp2rit` | `.klv2` **and** `.wmp` | per full rack: every subset's leave value, best leave per size, bingo words, playthrough unions, best exchange | 1.9 GB |

Key facts, with where they come from in MAGPIE:

- **Both are found by lexicon name.** `config_load_lexicon_dependent_data` sets
  the `.rit` name to the player's lexicon name (`config.c:6538`), and does the
  same for the `.wmp`. The player's leaves name plays no part.
- **The `.rit` stands in for the KLV on full racks.** In `move_gen.c`, when a
  player has a full rack and a table, `rack_info_table_entry_unpack_leaves`
  fills the leave map from the table rather than from the loaded KLV
  (`move_gen.c:3322-3362`).
- **The `.rit` also holds word data.** Bingo words, playthrough unions and
  existence bitvectors come from the `.wmp` it was built with. A stale table is
  wrong about words as well as leaves.
- **Neither header says where the file came from.** The `.rit` header is a
  version, rack size, minimum played size for playthrough coverage, and bucket
  and entry counts (`rack_info_table.h:386-392`). The `.wmp` header is a
  version and sizes.
- **`klvwmp2rit` assumes one name for everything.** It loads the KLV and the WMP
  under the same `input_name` (`convert.c:386-390`).
- **Neither is transmitted or pinned.** birdtest's `expected_data` pins the
  `.kwg`, `.klv2` and other inputs by SHA-256. It can't sensibly pin the `.wmp`
  or `.rit`: they are generated on the contributor's machine and are far too
  large to ship.

## The problem

### How a wrong table or wordmap gets used

1. **Leaves that don't match the table.** A player config pins NWL23 words and
   CSW21 leaves, a pairing birdtest accepts on purpose (see stage 3 above).
   With RIT on, MAGPIE loads `NWL23.rit`, which was built from `NWL23.klv2`.
   Every full-rack position is then ranked on NWL23's leaves, not the CSW21
   leaves the job pinned and the worker just verified. The same happens with
   `CSW24` and `CSW_quackle_leaves`, `CSW_old_leaves` or
   `CSW_macondo_superleaves_v2`, all of which are in MAGPIE-DATA.
2. **Leave generation.** Each generation plays with a KLV fetched for that
   generation (`<lexicon>_birdtest_previous`). A table built from the shipped
   leaves would replace exactly the values being generated. Errors would carry
   into every later generation.
3. **A stale local file with the right name.** `download_data.sh` overwrites
   `CSW24.klv2` or `CSW24.kwg` in place and leaves the old `.rit` or `.wmp` next
   to it. The names still match; the contents no longer do.
4. **A stale wordmap.** The same as case 3, but a `.wmp` from an older `.kwg`
   produces a different set of moves.

In every case the output looks normal. For birdtest, a contribution computed
this way passes every plausibility check and ends up in the results.

### What's in place today

| Guard | Where | Covers | Gap |
|---|---|---|---|
| RIT off for the `leavegen` command | `config.c:6349-6354`, `disable_rit = exec_parg_token == ARG_TOKEN_LEAVE_GEN` (`config.c:9656`) | Case 2, CLI only | Contribute's leave-gen executor passes `disable_rit=false` (`config.c:7337`) |
| RIT always off in contribute | `config_contribute_load_lexicon_and_variant` (`config.c:7306`, commit `2a68705a`) | Cases 1–3 in contribute | Turns off the speedup for every job, not just the unsafe ones |
| birdtest refuses `use_rit` | `validate_player_config_body` (`backend/src/routes/admin.rs:683-696`) | Same | Same |
| Wordmap `.src` sidecar | `config_contribute_ensure_wordmap` (`config.c:7143-7261`) | Case 4 in contribute, for tasks that ask for a wordmap | CLI never checks; the sidecar is a separate file that can go missing or get copied without its `.wmp` |
| Nothing | — | Cases 1–4 on the CLI | Everything |

## What we can reuse

MAGPIE already has the pieces:

- **SHA-256.** `sha256_hash_file` and `sha256_hash_bytes` (`src/util/hash.h`).
- **Checking pinned inputs before a task runs.** `expected_data_matches`
  (`contribute.c`) hashes each file a task will load and compares it with the
  job's pinned digest. On a mismatch it declines the task and names the file.
- **A digest cache.** `hash_with_cache` and `contribute_digest_cache_key`
  (`contribute.c:222-290`). The cache key is path, size, mtime, inode and ctime
  (to the nanosecond), so an unchanged file is hashed only once per worker
  process.
- **Recording a build input.** The wordmap sidecar `<name>.wmp.src` holds the
  digest of the `.kwg` the wordmap was built from. `wordmap_is_current` compares
  it with the `.kwg` on disk and rebuilds on a mismatch. The sidecar is written
  only after the `.wmp` is in place, so an interrupted build never leaves a
  sidecar vouching for a wordmap that isn't there.

The sidecar is already this proposal on a small scale: record the digest of the
input and compare it at use time. The options below extend that to the rack info
table, and move the check to where the files are loaded so the CLI is covered
too.

### What hashing costs

`sha256sum` on this machine, CSW24, file already in the page cache. MAGPIE's
built-in SHA-256 may be slower; these figures show the scale, not exact numbers.

| File | Size | Hash time |
|---|---|---|
| `.klv2` | 3.7 MB | 0.03 s |
| `.kwg` | 6.0 MB | 0.06 s |
| `.wmp` | 179 MB | 0.82 s |
| `.rit` | 1.9 GB | 8.80 s |

**Takeaway:** hash the inputs (`.kwg`, `.klv2`), never the outputs. Contribute
already hashes the pinned `.kwg` and `.klv2` for `expected_data`, so the digests
this check needs are usually already in the cache.

## Options

### Option A — a sidecar for the rack info table too

`convert klvwmp2rit` writes `<name>.rit.src` holding the `.klv2` digest and the
`.kwg` digest (the one the `.wmp` was built from). Before contribute loads a
table, it compares those digests with the player's loaded KLV and KWG.

- **For:** copies an existing, tested pattern; no file format change.
- **Against:** the sidecar is a separate file, so it can be deleted, left behind
  or copied without its data file. Like the wordmap check, it lives in
  contribute, so the CLI stays unprotected unless the check moves into the
  loader. There would be two sidecar formats to maintain.

### Option B — record input digests in the file headers (recommended)

Add input digests to both headers and bump each format version:

- **`.wmp` header:** SHA-256 of the `.kwg` it was built from.
- **`.rit` header:** SHA-256 of the `.klv2` it was built from, plus the `.kwg`
  digest copied from the header of the `.wmp` it was built from. Word data
  therefore traces back to the `.kwg` without hashing the 179 MB wordmap.

When `config_load_lexicon_dependent_data` loads a player's data:

1. Get the digest of the `.kwg` and `.klv2` that player loaded. Use a digest the
   caller already has (contribute's `expected_data` pass); otherwise hash the
   file, through a stat-keyed cache like `hash_with_cache`, moved out of
   `contribute.c` so the CLI can use it too.
2. Read the `.wmp` header. Use the wordmap only if its recorded `.kwg` digest
   matches.
3. Read the `.rit` header. Use the table only if both its `.klv2` digest and its
   `.kwg` digest match. Reading the header is cheap even when the table is
   memory-mapped.
4. Handle a mismatch as described under [If a check fails](#if-a-check-fails).

- **For:** the proof of origin is inside the file, so it can't be separated or
  lost. One check covers the CLI and contribute. It catches cases 1–4. Leave
  generation's fetched KLV fails the check on its own, although an explicit
  disable stays as a clear statement of intent. It hashes nothing larger than
  6 MB. Once every wordmap carries the digest, the `.wmp.src` sidecar can go.
- **Against:** a format change. Existing `.wmp` and `.rit` files fail the
  version check and must be rebuilt once (about a second for a wordmap; a few
  seconds of 8 threads for a table, going by the timing in `6c527962`). A
  `magpie_version` floor is needed so birdtest can tell builds with the check
  from builds without it.

### Option C — name the table after its leaves

Build `CSW_quackle_leaves.rit` rather than `CSW24.rit`, and look the table up by
the player's leaves name.

- **For:** tiny change; fixes case 1.
- **Against:** names aren't contents. Cases 2–4 remain: leave generation reuses
  one leaves name across generations, and `download_data.sh` changes contents
  without changing names. It also doesn't cover the word data a table holds.
  Worth doing only alongside A or B.

### Option D — birdtest pins the derived files

Add `.wmp` and `.rit` digests to `expected_data`.

- **Against:** the files are generated locally, so the server would need their
  exact expected bytes. That holds only if both builders are byte-for-byte
  deterministic across platforms and thread counts, which hasn't been checked.
  Hashing a 1.9 GB table at claim time also costs about 9 s, and it still
  leaves the CLI unprotected. Not recommended.

### Option E — keep things as they are

RIT stays off throughout contribute and `use_rit` stays refused. It is safe and
needs no work, but gives up the table's speedup (about 2x per leave lookup and
about 3% end to end on a 200k-game autoplay, per `6c527962`), and the CLI stays
unprotected.

## If a check fails

| Situation | Suggested behaviour | Why |
|---|---|---|
| CLI, `-rit`/`-wmp` given explicitly | Error naming the file and both digests | The user asked for that file; using something else silently would be wrong |
| CLI, turned on from saved settings | Warn once, run without the file | Settings from an earlier session shouldn't break an unrelated command |
| Contribute, wordmap | Rebuild, as the sidecar check does now | About a second, once per `.kwg` |
| Contribute, rack info table | Run without it and log once; optionally rebuild when the data directory is writable | A rebuild writes about 2 GB; running without the table is always correct, just slower |
| Leave generation (CLI and contribute) | Don't load a table at all | The KLV changes every generation. Keep `disable_rit` and pass it explicitly from `config_contribute_leave_gen` too |

## Changes in birdtest after the MAGPIE work

1. Stop refusing `use_rit` in `validate_player_config_body`, update
   `admin_api::a_player_config_cannot_ask_for_a_rack_info_table` to match, and
   bring the option back in the player-config form.
2. Require the MAGPIE version that includes the check (for example `0.4.0`) for
   any job with a player that sets `use_rit`: either a per-job
   `min_magpie_version`, or raise the default floor once the build is released.
3. Update PLAN.md (the executor table row for `use_rit`, and the admin
   semantics) and record the reversal of audit finding M2.

`use_rit` is already on the wire, so there is no contract change.

## Open questions

1. **Are `text2wordmap` and `klvwmp2rit` byte-for-byte deterministic?** If so,
   option D becomes possible later as an extra check. It isn't needed for B.
2. **Should a table record the digest of its `.wmp` as well as the `.kwg`?**
   Recording only the `.kwg` assumes a given `.kwg` and `.wmp` format version
   always build the same wordmap. If a wordmap builder change can alter results
   without bumping the `.wmp` version, the table should record the `.wmp`
   digest itself, taken from the bytes as it builds.
3. **Players with different lexicons.** Each player's check uses that player's
   own `.kwg` and `.klv2`. Nothing is shared between players, so this should
   fall out naturally, but a test should cover it.
4. **Should a rack info table rebuild automatically in contribute?** It is
   large on disk and in memory; contributors may prefer to opt in.
