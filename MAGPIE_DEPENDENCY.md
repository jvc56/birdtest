# Making MAGPIE a server-side dependency of birdtest

Status: **proposal, nothing implemented.** Written 2026-09-14 against birdtest
`audit/birdtest-2026-09-14-pass2` (`d7083a9`) and MAGPIE `birdtest-contribute`
(`c2daa027`). Builds on [RIT_WMP_PROVENANCE.md](RIT_WMP_PROVENANCE.md), which
explains the provenance problem and how data reaches workers; read that first if
the terms are new.

## Summary

Today birdtest's server contains no MAGPIE. Workers run MAGPIE, but the server
only dispatches work, checks file digests, and builds leave-generation KLVs with
its own Rust port of MAGPIE's code (`backend/src/jobs/klv.rs`).

This proposes adding a pinned MAGPIE binary to the backend and using it for two
things:

1. **Wordmap and rack-info-table provenance.** The server builds the `.wmp` and
   `.rit` a job needs from the `.kwg` and `.klv2` files the job pins, hashes the
   results, and sends those hashes to workers alongside the existing
   `expected_data` digests. A worker builds the files itself from the inputs it
   has already verified, and uses them only if its hashes match. This closes
   every gap RIT_WMP_PROVENANCE.md describes and lets birdtest accept `use_rit`
   again.
2. **Leave-generation KLVs.** Once the server runs MAGPIE anyway, MAGPIE builds
   the generation-0 KLV and every generation's KLV, replacing `klv.rs`. A change
   to how MAGPIE derives leave values then happens in one place instead of being
   ported by hand.

The design depends on MAGPIE's builders producing byte-identical files for the
same inputs. **We tested that: they do, across thread counts, on one machine and
one build.** We also found that **a MAGPIE change can alter a builder's output
without changing the file format version**, so every published hash has to be
tied to the builder that produced it. That constraint shapes most of the design
below.

## What exists today

| Piece | Where | How it works |
|---|---|---|
| Input digests | `expected_data` on every claim | The server lists every `.kwg`, `.klv2`, letter distribution, layout and win% file a task loads, with SHA-256. MAGPIE hashes its copies and declines on a mismatch |
| Wordmaps | Built by each worker (`config_contribute_ensure_wordmap`) | Built locally from the `.kwg`; a `.wmp.src` sidecar records the `.kwg` digest, and the wordmap is rebuilt when that changes. The `.wmp` itself is never checked |
| Rack info tables | Nowhere | Refused: `use_rit = true` is rejected at player-config creation, and contribute keeps tables off |
| Leave-generation KLVs | `backend/src/jobs/klv.rs` (868 lines), called from `leave_gen.rs` | A Rust translation of MAGPIE's `csv2klv` and `rack_list_write_to_klv`. It builds a plain trie where MAGPIE builds a minimized DAWG, so its bytes differ from MAGPIE's for the same leave values |
| Server data | `input_data` | Import hashes every file in the MAGPIE-DATA tarball but keeps bytes only for letter distributions and layouts. **The server has no copy of any `.kwg` or `.klv2`** |
| Backend image | `docker/Dockerfile` | The Rust binary only. The file's header says "Nothing in this repo builds or ships a MAGPIE binary" |

## What we measured

To know whether server-built hashes can be checked against worker-built files,
we built CSW24's wordmap and rack info table twice from the same inputs, once
single-threaded and once with 8 threads, in separate scratch data directories.
The build was MAGPIE `c2daa027`, `BUILD=no_pgo_release` (`-O3 -flto
-march=native`), GCC 10.5, on an Intel i7-10750H (x86_64 Linux).

| File | 1 thread | 8 threads | Same bytes? | Size | Peak memory (8 threads) |
|---|---|---|---|---|---|
| `CSW24.wmp` (`convert dawg2wordmap`) | 2.4 s | 1.7 s | **Yes** | 179 MB | 710 MB |
| `CSW24.rit` (`convert klvwmp2rit`) | 169 s | 59 s | **Yes** | 1.9 GB | 2.4 GB |

Two comparisons with files already on disk:

- **The rack info table matched** one built eight days earlier by an earlier
  MAGPIE commit.
- **The wordmap did not match** one built in December 2025. Both files carry
  format version 3 in their header, yet 72,852,152 bytes differ and the sizes
  differ by 60 bytes. A change to MAGPIE's wordmap builder altered its output
  without touching the format version.

**What this establishes:** for one build on one machine, output is a function of
the inputs alone, independent of thread count and scheduling. **What it does
not:** whether a different compiler, optimisation flags, CPU architecture
(`-march=native` on another machine, ARM) or operating system produces the same
bytes. Both builders do floating-point work (leave values), so this needs a
test before anything depends on it. See [Open questions](#open-questions).

## Part 1: server-built hashes for wordmaps and rack info tables

### The flow

1. **Import keeps lexica and leaves.** The tarball import already downloads and
   hashes every `.kwg` and `.klv2`. It starts storing their bytes in the object
   store, keyed by SHA-256, so the server has the exact inputs a job pins.
2. **A job that needs a derived file triggers a build.** When a player config or
   leave job asks for a wordmap, the server needs a `.wmp` for that `.kwg`. When
   a player config asks for a rack info table, it needs a `.rit` for that
   `(.kwg, .klv2)` pair.
3. **A builder task runs MAGPIE.** It fetches the inputs, writes them into a
   scratch data directory with the job's letter distribution, runs
   `convert dawg2wordmap` or `convert klvwmp2rit`, hashes the output, records the
   hash, and deletes the file.
4. **Dispatch waits for the hash.** A job whose derived files are not built yet
   is not dispatched, the same way a leave-generation job waits for its universe
   to be seeded.
5. **The claim carries the hashes.** `expected_data` gains a `derived` list:

   ```json
   "derived": [
     { "role": "wmp", "name": "CSW24", "sha256": "1830…", "bytes": 178929167,
       "builder": "wmp-4", "from": { "kwg": "3e74…" } },
     { "role": "rit", "name": "CSW24.CSW_quackle_leaves", "sha256": "6da8…",
       "bytes": 1885416048, "builder": "rit-2",
       "from": { "kwg": "3e74…", "klv": "37de…" } }
   ]
   ```

6. **The worker builds and checks.** After verifying its inputs, MAGPIE looks
   for each derived file. If a cached digest (the existing stat-keyed digest
   cache) says a file on disk already has the listed hash, it uses it. Otherwise
   it builds the file from the verified inputs, hashes it, and uses it only if
   the hash matches. On a mismatch it declines with a new reason,
   `derived_mismatch`, carrying both hashes, so the fleet's non-determinism shows
   up in the admin view instead of being silently worked around.

### Why hash the output, when RIT_WMP_PROVENANCE.md hashed inputs

RIT_WMP_PROVENANCE.md recommended recording input digests in each file's header.
That shows a file was built *from* the right inputs, but it trusts the builder.
Our wordmap measurement is exactly the case it misses: same inputs, same format
version, different bytes. Hashing the output checks what the worker will actually
load, and with the server building its own reference copy there is a known-good
answer to compare against. The two approaches can coexist: header digests remain
a cheap guard for CLI users, who have no server.

### Tie every hash to its builder

Because a MAGPIE change can alter a builder's output, a hash means something only
together with the builder that produced it.

- **Builder versions separate from `MAGPIE_VERSION`.** MAGPIE gains
  `WMP_BUILDER_VERSION` and `RIT_BUILDER_VERSION`, bumped whenever a change alters
  that builder's output, even with an unchanged file format. The server records
  the builder version with each hash; the claim states it; a worker whose builder
  differs declines rather than building something that cannot match.
- **A test that forces the bump.** A MAGPIE test builds the wordmap and rack info
  table for a small test lexicon (`CSW21_ab` or similar) and compares them with
  pinned hashes. A builder change that alters output fails the test until someone
  bumps the version and updates the hash. Without this, "bump when output
  changes" depends on noticing.
- **The server rebuilds on a bump.** Deploying a server with a newer MAGPIE
  records new hashes under the new builder version; old ones stay for workers
  still on the old builder until the floor moves past them.

### The server runs a binary, not a library

MAGPIE can be built as `libmagpie.so`, but its API (`src/impl/cmd_api.h`) is the
same string-command interface as the CLI, so linking it gains nothing over a
subprocess. Running it in-process would put MAGPIE's memory use (1.9 GB for a
rack info table) and any crash inside the web server. A subprocess of a pinned
binary keeps failures contained and needs no FFI.

### Where builds run

The backend's ECS task has 1 vCPU and 2 GB of memory (`infra/variables.tf`).

- **A wordmap build** takes about 2 seconds but peaks at 710 MB, a third of the
  web task's memory, alongside everything else the server is doing.
- **A rack info table build** peaks at 2.4 GB, writes a 1.9 GB file, and takes
  59 seconds on 8 threads, about 170 on one. It does not fit the web task at all.

So both run in a separate, on-demand ECS task, as the backup job already does
(`infra/backup.tf`, with its own ephemeral storage). The web task records a build
request; the builder task runs, writes the hash, and exits. It needs at least
4 GB of memory and several GB of ephemeral storage, and more vCPUs shorten a
table build roughly in proportion.

### Naming derived files

MAGPIE finds a rack info table by lexicon name alone
(`config_load_lexicon_dependent_data`), but a table belongs to a
`(.kwg, .klv2)` pair. Two jobs on CSW24 with different leaves need different
tables. MAGPIE's contribute path would load tables by an explicit name the claim
supplies, such as `CSW24.CSW_quackle_leaves`, instead of by lexicon name.
Wordmaps depend only on the `.kwg`, so their names can stay as they are.

### What this replaces

- **The `.wmp.src` sidecar.** An output hash is a strictly stronger check than
  the recorded `.kwg` digest.
- **The `use_rit` refusal**, in both repositories, and the blanket rack-info-table
  switch-off in contribute, for games and opening-rack jobs.

Leave generation keeps tables off: its KLV changes every generation, and building
a 1.9 GB table per generation on every worker costs far more than the table saves.

### Costs on the worker

- Hashing a built file: about 0.8 s for a wordmap and 8.8 s for a rack info
  table (measured earlier with `sha256sum`). The digest cache makes this a
  one-time cost per file, not per task.
- Building: 2 s for a wordmap, one to three minutes for a rack info table,
  once per `(.kwg, .klv2)` pair and builder version. Contribute already builds
  wordmaps on demand.

## Part 2: leave-generation KLVs built by MAGPIE

### Why

`klv.rs` is a hand translation of MAGPIE's `csv2klv`, `rack_list_write_to_klv`
and `generate_leaves`. If MAGPIE changes how leave values are derived, `klv.rs`
has to change the same way, and nothing makes it. Its KLVs also differ in bytes
from MAGPIE's (a plain trie against a minimized DAWG). They load to the same
values, but that difference is a standing source of confusion.

### What MAGPIE needs

MAGPIE already has the pieces:

- `klv_create_empty(ld, name)` builds a KLV's structure from the letter
  distribution alone, with every leave worth 0. That is the generation-0 KLV.
- `rack_list_write_to_klv` derives every leave's value from full-rack means,
  weighted by draw combinations. It is what `klv.rs` ports.

It lacks a way to feed in the server's aggregated results. `leave_rack_progress`
holds, per full rack, an occurrence count and an equity sum; MAGPIE's `RackList`
is built up one game at a time. Two new conversions would cover it:

| Command | Input | Output |
|---|---|---|
| `convert zero2klv <letter_distribution>` | The distribution | A KLV with every leave 0 (generation 0) |
| `convert rackequity2klv <letter_distribution>` | CSV of `rack,count,equity_sum`, one row per full rack | The generation's KLV, via `rack_list_write_to_klv` |

`rackequity2klv` needs a `RackList` setter that takes a rack's count and mean
directly, where `rack_list_add_rack` today takes one game's equity at a time.

### How the server uses them

- **Letter distribution.** The server writes the job's pinned distribution bytes
  (`input_data.content`) into the scratch data directory for each run, so MAGPIE
  reads exactly the row the job pins. That keeps PLAN.md's rule that nothing
  server-side reads a distribution off its own disk.
- **Transition.** `generation_klv` streams the generation's roughly 3.2 million
  rows (English) to a CSV in the scratch directory, runs `rackequity2klv`, and
  reads the KLV back. The derivation takes about 13 seconds in Rust today. It
  needs a measurement in MAGPIE; if it is similar it can stay a subprocess of the
  web task, and otherwise it moves to the builder task.
- **Generation 0.** `seed_zero_generation` runs `zero2klv`.
- **Rebuilding artifacts.** `rebuild-artifacts` compares a rebuilt KLV's hash
  with the stored one. Once a MAGPIE upgrade can legitimately change those bytes,
  a mismatch no longer proves corruption. `leave_generation_artifacts` gains the
  builder version that produced each KLV, and a rebuild with a different builder
  reports "built by a different builder" rather than "differs".

`klv.rs` is then deleted. Its format tests become a MAGPIE-side test, or stay in
birdtest as a check that the server's MAGPIE loads what it writes.

## Building MAGPIE into the backend image

- **Pin a commit.** The Dockerfile gains a build stage that checks out MAGPIE at a
  pinned commit (a build argument recorded in the image) and builds it. CI's
  `magpie-contract` and nightly jobs already check out and build MAGPIE.
- **Portable flags.** Every optimised MAGPIE build uses `-march=native`. A binary
  built on a CI runner can use instructions a Fargate CPU lacks, and — more
  importantly here — different instruction sets could change floating-point
  results. MAGPIE needs a build variant with a fixed target (for example
  `-march=x86-64-v2`), used by both the server image and released worker binaries
  if their outputs are to match.
- **Size.** The binary is about 1 MB. The builder task needs no MAGPIE-DATA
  install: it fetches only the files a build uses.
- **Licences.** birdtest is AGPL-3.0 and MAGPIE is GPL-3.0; shipping a GPL binary
  in an AGPL service image is compatible.
- **PLAN.md.** The sections that say the backend has no MAGPIE dependency
  (Worker Client; Artifacts: back up, or rebuild?) and the Dockerfile's header
  change.

## Changes by repository

**MAGPIE (`birdtest-contribute`)**

- `WMP_BUILDER_VERSION`, `RIT_BUILDER_VERSION`, and the pinned-hash builder test.
- Contribute: verify derived files against `expected_data.derived`, build on a
  miss, decline with `derived_mismatch`; load a rack info table by the claim's
  name; stop switching tables off except for leave generation.
- `convert zero2klv` and `convert rackequity2klv`, and a count-and-mean `RackList`
  setter.
- A portable optimised build variant.

**birdtest**

- Import: store `.kwg` and `.klv2` bytes in the object store.
- Schema: a `derived_data` table (role, builder version, `kwg_id`, `klv_id`,
  SHA-256, size, build status), unique per role, builder and inputs; a builder
  version on `leave_generation_artifacts`.
- A builder ECS task and a build queue; dispatch gated on built hashes.
- `expected_data.derived` on claims; the `derived_mismatch` decline reason;
  contract fixtures in both repositories.
- Accept `use_rit` again.
- `leave_gen.rs` runs MAGPIE for KLVs; `klv.rs` removed.
- Dockerfile, Terraform (builder task), PLAN.md, TESTING.md, README.

## Alternatives considered

| Alternative | Why not |
|---|---|
| **Header input digests only** (RIT_WMP_PROVENANCE.md, option B) | No server dependency, and still worth doing for CLI users, but it trusts the builder. The wordmap measurement shows a builder change altering output with the same inputs and format version |
| **Server distributes the files** | Removes the determinism question, but a rack info table is 1.9 GB and a wordmap 179 MB, per lexicon or pair, to every worker |
| **Link `libmagpie`** | Same command-string API as the CLI, with MAGPIE's memory and crashes inside the web server |
| **Keep `klv.rs`, add MAGPIE only for derived files** | Smaller change, but keeps two implementations of leave derivation that must agree by hand, now alongside a MAGPIE that could do the job |

## Open questions

1. **Is the output identical across compilers, flags and architectures?** Build
   the same wordmap and table with GCC and Clang, with `-march=native` and a
   fixed target, and on ARM (Graviton, Apple silicon). If not, the builder
   version has to include the build target, or both sides must use one pinned
   binary.
2. **Does the server keep derived files?** Hash-and-discard costs nothing to
   store. Keeping a copy would let an admin inspect a mismatch, at 1.9 GB per
   table.
3. **When is a rack info table built?** At player-config creation (an admin waits
   minutes before a job can dispatch) or at job activation. Either way the admin
   UI needs to show build status.
4. **How fast is `rackequity2klv`** against `klv.rs`'s 13 seconds, and does the
   transition stay in the web task?
5. **Worker disk.** A contributor running several jobs with rack info tables
   holds 1.9 GB per `(.kwg, .klv2)` pair. Contribute may need an eviction policy
   or a cap.
