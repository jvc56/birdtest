# Making MAGPIE a server-side dependency of birdtest

Status: **implemented**, 2026-09-15, in both repositories. Written 2026-09-14
against birdtest `audit/birdtest-2026-09-14-pass2` (`d7083a9`) and MAGPIE
`birdtest-contribute` (`c2daa027`).

This document is now the record of a design that exists rather than a proposal.
Where the build departed from the proposal, the section says so and why; the
[Open questions](#open-questions) at the end are answered. The behaviour itself
lives in PLAN.md's [Wordmap and rack info table
provenance](PLAN.md#wordmap-and-rack-info-table-provenance) and in the code,
which is where to look first — this is the argument, not the reference.

It supersedes RIT_WMP_PROVENANCE.md, which described the provenance problem and
four narrower options; that document is deleted, and its analysis is folded into
PLAN.md's section above. It reverses audit finding M2
([AUDIT_FINDINGS_1.md](AUDIT_FINDINGS_1.md)), which refused `use_rit` precisely
because there was nothing to check a table against.

**Scope.** This is about contributed results — data a worker computes and
birdtest mixes into everyone else's. MAGPIE's CLI still finds both derived files
by lexicon name and checks neither; that is out of scope and stays the user's
responsibility, because those results are nobody else's. What made the
contribute path different is that a wrong answer there passes every plausibility
check and lands in a job's totals.

## Summary

birdtest's server used to contain no MAGPIE. Workers ran MAGPIE, but the server
only dispatched work, checked file digests, and built leave-generation KLVs with
its own Rust port of MAGPIE's code (`backend/src/jobs/klv.rs`).

A pinned MAGPIE binary is now in the backend image, doing two things:

1. **Wordmap and rack-info-table provenance.** The server builds the `.wmp` and
   `.rit` a job needs from the `.kwg` and `.klv2` files the job pins, hashes the
   results, and sends those hashes to workers alongside the existing
   `expected_data` digests. A worker builds the files itself from the inputs it
   has already verified, and uses them only if its hashes match. This closes
   every gap the provenance problem had, and birdtest accepts `use_rit` again.
2. **Leave-generation KLVs.** Since the server runs MAGPIE anyway, MAGPIE builds
   the generation-0 KLV and every generation's KLV; `klv.rs` is gone. A change to
   how MAGPIE derives leave values now happens in one place instead of being
   ported by hand.

The design depends on MAGPIE's builders producing byte-identical files for the
same inputs. **We tested that: they do, across thread counts, compilers flags
and instruction-set targets** (see [What we measured](#what-we-measured)). We
also found that **a MAGPIE change can alter a builder's output without changing
the file format version**, so every published hash has to be tied to the builder
that produced it. That constraint shapes most of the design below.

## What existed before

| Piece | Where | How it worked |
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
the inputs alone, independent of thread count and scheduling.

### And then across builds

The open question that gated the whole design was whether that holds across
*builds*, since both builders do floating-point work and `-march` decides how a
compiler may vectorize it. Measured before anything was built on it, on the same
machine (GCC 10.5, x86_64):

| Comparison | `CSW21_ab.wmp` | `NWL23.wmp` | `CSW21_ab.rit` |
|---|---|---|---|
| `-march=native` vs `-march=nehalem` | **identical** | **identical** | **identical** |
| `dawg2wordmap` vs `dawg2text` + `text2wordmap` | — | **identical** | — |

So the instruction-set target does not change these builders' output, and the
two wordmap paths are interchangeable — which is why the worker now uses
`dawg2wordmap`, the same one the server does, and writes no intermediate `.txt`.

Two things this still does not establish: a different *compiler* (Ubuntu 20.04's
clang 10 cannot build the tree at all — it rejects `-march=x86-64-v2`, which is
why the portable target is spelled `nehalem`), and a different architecture. The
design does not depend on either. `build_target` travels with every hash and is
reported, but is deliberately **not** compared: a worker whose target differs
builds the file and checks the bytes, so if this ever stops holding the answer
is a `derived_mismatch` in the admin view rather than an assumption nobody
revisits. Refusing on the field alone would have locked out every contributor
who builds from source in exchange for nothing measurable.

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
       "builder": "wmp-1", "build_target": "nehalem" },
     { "role": "rit", "name": "CSW24.CSW_quackle_leaves", "sha256": "6da8…",
       "bytes": 1885416048, "builder": "rit-1", "build_target": "nehalem" }
   ]
   ```

   The proposal had each entry repeat the digests of the inputs it was built
   from. That went: they are already in `expected_data.files`, the worker has
   verified them before it reaches this, and a second copy is a second thing
   that can disagree with the first. `build_target` took the space instead,
   which is a fact about the entry rather than a restatement of another one.

6. **The worker builds and checks.** After verifying its inputs, MAGPIE looks
   for each derived file. If a cached digest (the existing stat-keyed digest
   cache) says a file on disk already has the listed hash, it uses it. Otherwise
   it builds the file from the verified inputs, hashes it, and uses it only if
   the hash matches. On a mismatch it declines with a new reason,
   `derived_mismatch`, carrying both hashes, so the fleet's non-determinism shows
   up in the admin view instead of being silently worked around.

### Why hash the output, and not the inputs

The obvious alternative, and the one the earlier provenance document
recommended, is to record the input digests in each derived file's header. That
shows a file was built *from* the right inputs, and still trusts the builder.
Our wordmap measurement is exactly the case it misses: same inputs, same format
version, different bytes. Hashing the output checks what the worker will
actually load, and with the server building its own reference copy there is a
known-good answer to compare against.

The two could coexist — header digests would be a cheap guard for CLI users, who
have no server — and that half was **not built**. It is a file-format change to
both `.wmp` and `.rit`, invalidating every one a contributor already has, for a
check strictly weaker than the one that now exists on the path that matters. The
wordmap's existing `.wmp.src` sidecar stays and still covers the CLI, so nothing
regressed; what a CLI user does not get is protection against a builder change,
which is the gap the server closes for the fleet. It is worth doing on its own
merits later, and is not part of this.

### Tie every hash to its builder

Because a MAGPIE change can alter a builder's output, a hash means something only
together with the builder that produced it.

- **Builder versions separate from `MAGPIE_VERSION`.** `src/def/builder_defs.h`
  carries `WMP_BUILDER_VERSION`, `RIT_BUILDER_VERSION` and `KLV_BUILDER_VERSION`,
  bumped whenever a change alters that builder's output, even with an unchanged
  file format. The server records the builder version with each hash and the
  claim states it.
- **The server asks the binary, not its configuration.** `magpie builders`
  prints the three versions and the build target as JSON; the server reads it at
  startup and refuses to bind if it cannot. A configured value would drift the
  first time someone deployed a new image without editing a variable, and the
  whole point of recording the builder is that a hash without one means nothing.
- **A test that forces the bump.** `test/builder_hash_test.c` builds the wordmap
  and rack info table for `CSW21_ab` and compares them with pinned hashes, and
  checks that the pinned hash and the version it belongs to moved together. A
  builder change that alters output fails it until someone bumps the version
  *and* updates the hash. CI runs it on every birdtest pull request, because it
  is birdtest's fleet that a silent change would take down.
- **The server rebuilds on a bump.** Deploying a server with a newer MAGPIE
  records new hashes under the new builder version; old ones stay for workers
  still on the old builder until the floor moves past them. Activating a job
  again is what re-queues its files, which is why activation requests them as
  well as creation.

**One departure.** The proposal had a worker decline immediately when its
builder version differed, to avoid a wasted three-minute build. That is not what
was built: a worker declines on a *hash* mismatch, and the builder version in
the claim is diagnostic. The reason is the same one that governs the build
target — an early decline is right exactly when the two builders really do
disagree, and wrong the rest of the time, and a contributor who builds from
source should not be turned away by a field. The wasted build happens once: the
job is remembered as unsupported, so the worker does not try it again.

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

So both run in a separate ECS task (`infra/derived.tf`), sized at 4 vCPU, 8 GB
and 30 GB of ephemeral storage. The web task records a build request in
`derived_data`; the builder task takes rows under a lease, builds, writes the
hash, and exits when the queue is empty.

**It is scheduled, not triggered.** The proposal said "on-demand", and the web
task could indeed call `ecs:RunTask` the moment an admin creates a job. That
would start a build seconds earlier, and would put an AWS control-plane call on
the job-creation path, give the web task permission to run tasks and pass roles,
and need its own retry story for a call that can be throttled. A build takes
minutes; a five-minute poll is a small fraction of that. The queue is what makes
the work reliable either way, and `/admin/derived-data` is what makes the wait
visible rather than mysterious.

### Naming derived files

MAGPIE finds a rack info table by lexicon name alone
(`config_load_lexicon_dependent_data`), but a table belongs to a
`(.kwg, .klv2)` pair. Two jobs on CSW24 with different leaves need different
tables. MAGPIE's contribute path now loads tables by an explicit name the claim
supplies — `CSW24.CSW_quackle_leaves` — instead of by lexicon name. Wordmaps
depend only on the `.kwg`, so their names stay as they are.

**This needed a change to `klvwmp2rit` that the proposal missed.** The
conversion loaded the KLV *and* the wordmap under the output's name, so a table
called `CSW24.CSW_quackle_leaves` would have looked for
`CSW24.CSW_quackle_leaves.klv2` and `CSW24.CSW_quackle_leaves.wmp`, neither of
which exists. Naming the file for the pair therefore forced the inputs to be
named separately: `convert klvwmp2rit <output> <ld> <klv_name> <wmp_name>`, with
both optional and defaulting to the output's name, so the CLI's `convert
klvwmp2rit CSW24` is unchanged. Without it the only options were copying a
179 MB wordmap under a second name on every worker, or keeping the lexicon-name
collision this whole section exists to remove.

### What this replaces

- **The `.wmp.src` sidecar**, wherever the claim pins a hash: an output hash is
  a strictly stronger check than the recorded `.kwg` digest. It is kept, not
  deleted, for the CLI and for a server that pins nothing.
- **The `use_rit` refusal**, in both repositories, and the blanket rack-info-table
  switch-off in contribute, for games and opening-rack jobs.
- **`dawg2text` + `text2wordmap`** on the worker, replaced by the
  `dawg2wordmap` the server runs. Measured identical, one step instead of two,
  and no intermediate `.txt`.

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

It lacked a way to feed in the server's aggregated results. `leave_rack_progress`
holds, per full rack, an occurrence count and an equity sum; MAGPIE's `RackList`
is built up one game at a time. One new conversion covers it:

| Command | Input | Output |
|---|---|---|
| `convert rackequity2klv <name> <ld>` | `lexica/<name>.csv`, one `rack,count,equity_sum` row per full rack | The generation's KLV, via `rack_list_write_to_klv` |

It needs a `RackList` setter that takes a rack's count and mean directly, where
`rack_list_add_rack` folds one game's equity at a time — plus a way to tell "a
rack observed zero times" from "a rack the file never mentioned". Those are not
the same thing: the first contributes a mean of zero at full draw weight to
every leave it contains, which is a real leave value and indistinguishable from
a measured one. So every rack is marked unset before the file is read, and a
file that leaves any of them that way is refused rather than having its gaps
valued at zero.

**`zero2klv` was not built.** The proposal asked for it; `magpie createdata klv
<name> <ld>` already is it, building exactly that file from the distribution
alone through the same `klv_create_empty` the proposal named. One spelling is
better than two to keep in step.

### How the server uses them

- **Letter distribution.** The server writes the job's pinned distribution bytes
  (`input_data.content`) into the scratch data directory for each run, so MAGPIE
  reads exactly the row the job pins. That keeps PLAN.md's rule that nothing
  server-side reads a distribution off its own disk.
- **Transition.** `generation_klv` streams the generation's roughly 3.2 million
  rows (English) to a CSV in the scratch directory, runs `rackequity2klv`, and
  reads the KLV back. Neither side holds a generation in memory. It stays a
  subprocess of the web task: the work is the same derivation the Rust port did
  in about 13 seconds, and the transition already runs on its own task outside
  the claim transaction, so nothing waits on it that was not waiting before.
- **Generation 0.** `seed_zero_generation` runs `createdata klv`.
- **Rebuilding artifacts.** `rebuild-artifacts` compares a rebuilt KLV's hash
  with the stored one. Now that a MAGPIE upgrade can legitimately change those
  bytes, a mismatch no longer proves corruption.
  `leave_generation_artifacts.builder` records which builder wrote each KLV, and
  a rebuild with a different one reports "built by a different builder" rather
  than "differs". Without that, the first upgrade after a restore drill reads as
  data loss.

`klv.rs` is deleted. **Before deleting it, the two were run against each other**
over the whole 149-rack test distribution — `FullRackLeaves` building a KLV,
`rackequity2klv` building one from the same rows, both dumped through
`klv2csv` — and they agreed on every one of the 431 leave values. That is the
evidence that the replacement is faithful, and it could only be gathered while
both existed.

## Building MAGPIE into the backend image

- **Pin a commit.** The Dockerfile gains a build stage that checks out MAGPIE at a
  pinned commit (a build argument recorded in the image) and builds it. CI's
  `magpie-contract` and nightly jobs already check out and build MAGPIE.
- **Portable flags.** Every optimised MAGPIE build used `-march=native`. A binary
  built on a CI runner can use instructions a Fargate CPU lacks, and different
  instruction sets could in principle change floating-point results.
  `BUILD=portable_release` fixes the target, and the server image uses it.

  The target is `-march=nehalem`, not `-march=x86-64-v2` as proposed: they name
  the same instruction set, but `x86-64-v2` only reached GCC 11 and Clang 12,
  and Ubuntu 20.04's clang 10 rejects it outright — which is not a target to
  build a reproducibility story on. `PORTABLE_MARCH` overrides it for a non-x86
  host, and the object directory is keyed by it so changing it recompiles rather
  than relinking.
- **Size.** The binary is about 1 MB. The builder task needs no MAGPIE-DATA
  install: it fetches only the files a build uses.
- **Licences.** birdtest is AGPL-3.0 and MAGPIE is GPL-3.0; shipping a GPL binary
  in an AGPL service image is compatible.
- **PLAN.md.** The sections that say the backend has no MAGPIE dependency
  (Worker Client; Artifacts: back up, or rebuild?) and the Dockerfile's header
  change.

## Changes by repository

**MAGPIE (`birdtest-contribute`)**

- `src/def/builder_defs.h`: `WMP_BUILDER_VERSION`, `RIT_BUILDER_VERSION`,
  `KLV_BUILDER_VERSION` and `MAGPIE_BUILD_TARGET`; the `builders` command that
  prints them; `test/builder_hash_test.c`, which pins both derived builders'
  output and their thread-independence.
- Contribute: verify derived files against `expected_data.derived`, build on a
  miss, decline with `derived_mismatch`; load a rack info table by the claim's
  `rit_name`; stop switching tables off except for leave generation; build
  wordmaps with `dawg2wordmap`.
- `convert rackequity2klv`, a count-and-mean `RackList` setter, and
  `rack_list_mark_all_racks_unset` so a missing rack is an error rather than a
  zero.
- `klvwmp2rit` takes the KLV's and the wordmap's names separately.
- `BUILD=portable_release`.
- `MAGPIE_VERSION` 0.5.0.

**birdtest**

- Import: store `.kwg` and `.klv2` bytes in the object store, keyed by digest.
- Migration `0002_magpie_dependency.sql`: `derived_data`;
  `input_data.object_key` and its staging counterpart;
  `leave_generation_artifacts.builder`.
- `magpie.rs` (the pinned binary and its scratch directories) and `derived.rs`
  (what a job needs, the queue, the build).
- `build-derived`, a second binary run as a scheduled ECS task
  (`infra/derived.tf`).
- Dispatch gated on built hashes; `expected_data.derived` and `rit_name` on
  claims; the `derived_mismatch` decline reason; contract fixtures in both
  repositories.
- `use_rit` accepted, stored, and offered in the player-config form;
  `/admin/derived-data` for the queue.
- `leave_gen.rs` runs MAGPIE for KLVs; `klv.rs` removed.
- Dockerfile (a pinned MAGPIE stage and a `derived-builder` target), CI,
  docker-compose, PLAN.md, TESTING.md, RUNBOOK.md, README.

## Alternatives considered

| Alternative | Why not |
|---|---|
| **Header input digests only** | No server dependency, and still worth doing for CLI users, but it trusts the builder. The wordmap measurement shows a builder change altering output with the same inputs and format version |
| **Server distributes the files** | Removes the determinism question, but a rack info table is 1.9 GB and a wordmap 179 MB, per lexicon or pair, to every worker |
| **Link `libmagpie`** | Same command-string API as the CLI, with MAGPIE's memory and crashes inside the web server |
| **Keep `klv.rs`, add MAGPIE only for derived files** | Smaller change, but keeps two implementations of leave derivation that must agree by hand, now alongside a MAGPIE that could do the job |

## Open questions, answered

1. **Is the output identical across compilers, flags and architectures?**
   **Across flags, yes** — `-march=native` and `-march=nehalem` produce
   byte-identical wordmaps and rack info tables under GCC 10.5 on x86-64, for
   both a two-letter test lexicon and NWL23. **Across compilers and
   architectures, still unmeasured**: clang 10 cannot build the tree at all, and
   no ARM machine was to hand. The design does not wait on it. The build target
   travels with every hash and is reported but never compared, so an
   architecture that turns out to differ produces a `derived_mismatch` in the
   admin view — visible, attributable, and not a silent wrong answer. Both the
   server image and released contributor binaries use `portable_release`
   regardless.

2. **Does the server keep derived files?** **No — hash and discard.** Keeping a
   copy would cost 1.9 GB per table for an inspection nobody has needed yet, and
   the bytes are reproducible from inputs the object store already holds. The
   scratch directory is removed in a `Drop`, deliberately blocking, so an early
   return cannot leak one.

3. **When is a rack info table built?** **Requested at job creation, and again
   at activation; the queue is drained by a scheduled task.** Creation, because
   an admin who creates a job usually activates it in the next breath and the
   wait then happens while they are still deciding. Activation as well, because
   a deployment with a newer MAGPIE needs the job's files rebuilt under the new
   builder and nothing else would ask. Not at player-config creation: a config
   is not yet attached to a job, so its letter distribution — which a derived
   file is built against — is not known. `/admin/derived-data` shows the queue,
   and the job-creation form says so where `use_rit` is ticked.

4. **How fast is `rackequity2klv`, and does the transition stay in the web
   task?** **It stays.** It is the same derivation the Rust port did in about
   13 seconds for English, and the transition already runs on its own task
   outside the claim transaction — nothing waits on it that was not waiting
   before. What moved to the builder task is the work that could not fit: a rack
   info table at 2.4 GB peak and a 1.9 GB output.

5. **Worker disk.** **Still open, and deliberately not solved here.** A
   contributor running several jobs with rack info tables holds 1.9 GB per
   `(.kwg, .klv2)` pair, and nothing evicts them. Two things make it less
   pressing than it looks: a table is only built for a job that asks for one,
   and `use_rit` is off by default; and a contributor who runs out of disk gets
   a failed build and a declined task rather than a wrong result. The right fix
   is a cap in contribute's settings file with least-recently-used eviction,
   which is a change to `contribute.c` and to no protocol.

## What is not covered

- **The CLI**, deliberately — see Scope above. `magpie` outside `contribute`
  still finds a rack info table by lexicon name and checks nothing about it, and
  the `.wmp.src` sidecar is read only on the contribute path. A CLI user is
  analysing their own position on their own data; getting it right is theirs.
  Header input digests would close it and are a file-format change to both
  files, invalidating every `.wmp` and `.rit` in existence for a check weaker
  than the one contributors now get — see [Why hash the output, and not the
  inputs](#why-hash-the-output-and-not-the-inputs).
- **Rebuilding on a data update.** Importing a tarball that changes a `.kwg`
  creates a new `input_data` row, so a job pinning the old one keeps its old
  derived file — correct, and by construction. What is not automatic is any
  prompt to rebuild; an admin creates a new job against the new row, and the old
  derived rows are never collected. Nothing reads them, and each is a row plus a
  hash.
- **An end-to-end rack info table.** No test builds a real 1.9 GB table through
  a real `magpie contribute` against a real server. `M-10` and `M-11` in
  TESTING.md's tier 6 are where that belongs, and both are unwritten.
