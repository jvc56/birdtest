# birdtest

Crowdsourced crossword game research, modelled after Fishnet. birdtest runs
[MAGPIE](https://github.com/jvc56/MAGPIE), a crossword board game engine, on
volunteers' computers to play test matches between versions, tune its settings
and study openings. Admins define jobs; contributors run MAGPIE itself — `magpie
contribute` claims tasks, executes them locally, and submits results. The site
aggregates everything onto a live dashboard.

[PLAN.md](PLAN.md) is the design document — architecture, schema, API surface
and rationale all live there, including the
[Worker Client](PLAN.md#worker-client-1) specification for the `contribute`
command contributors run, how [input data is pinned by content and negotiated
with workers](PLAN.md#input-data-and-capability-negotiation), and what is
[backed up and why](PLAN.md#backups-and-restore). This file is how to run it,
[TESTING.md](TESTING.md) is what is guaranteed and how it is checked, and
[RUNBOOK.md](RUNBOOK.md) is the recovery procedure itself.

The command blocks here and in RUNBOOK.md are bash: in zsh, run `bash` first.
Stock zsh treats a `#` as a word, so a commented line fails or passes its
comment on as arguments, and an apostrophe in a comment opens a quote that
swallows the rest of the paste.

## Layout

| Path | What it is |
|---|---|
| `backend/` | Axum + SQLx server. Owns scheduling, validation, the match test, ratings and aggregation. |
| `frontend/` | SvelteKit SPA (dark mode only), built statically and served by Nginx in production. |
| `worker/` | `fake_worker.py`, a synthetic client used by the **end-to-end suite only** — see [TESTING.md](TESTING.md). Local development uses real MAGPIE; the contributor client is MAGPIE itself, see [Worker Client](PLAN.md#worker-client-1). |
| `infra/` | Terraform: VPC, ALB, ECS Fargate, RDS Postgres, S3, SES, SSM, backups. |
| `scripts/` | `dev.py` (the local development command), `seed.py` (empty database → work flowing), `e2e_magpie.py` and `e2e_magpie_native.sh` (tier 6, real MAGPIE), `prod-sql.sh` and `prod-shell.sh` (SQL and a shell inside the production VPC), `restore-job.sh` (a selective restore), plus backup, restore-drill, local snapshot and check scripts. |
| `docker/` | The Dockerfile for the backend (with its pinned MAGPIE), the derived-file builder and the fake worker. |
| `e2e/` | Tier 5: the Playwright journeys and `run.sh`, which runs them on a stack of their own. |
| `fixtures/` | Tier 5's stand-in for GitHub: MAGPIE-DATA tarballs and the Nginx config that serves them. |
| `contract-fixtures/` | One example of each worker API message, parsed by both birdtest's and MAGPIE's tests. |

Tile distributions and every other input file are no longer carried in the
repo: they are imported from a MAGPIE-DATA tarball into the `input_data` table
and pinned by SHA-256 — see [Input Data](PLAN.md#input-data-and-capability-negotiation).

## Running locally

One command brings up the stack, seeds it, starts real MAGPIE contributors and
opens the site:

```bash
./scripts/dev.py --pairs-job
```

That is the whole setup. It waits for the backend, imports the MAGPIE-DATA
tarball your own checkout installed, creates the jobs its job flags ask for
(here one game-pairs job, and its two player configs), launches four `magpie
contribute` workers, and opens **http://localhost:5173** signed in as the
seeded admin.

**It starts with no job.** Each job flag adds one, created and activated -- or
reused, when an active job of its name is already running -- and they stack:

| Flag | The job |
|---|---|
| `--leavegen-job` | "dev leave generation": six generations, targets 100, 200, 500, 1,000, 1,000 and 1,000 occurrences per rack, played by `static-equity-no-rit` |
| `--opening-rack-job` | "dev opening racks": every rack, 20 to a task, every play ranked by the 2-ply simmer `sim-2ply-rack`; each rack is analysed until 80% of its analyses agree on the best move, 2 to 5 analyses |
| `--games-job` | "dev games (positions saved)": `static-equity` against `static-score`, saving every position played |
| `--pairs-job` | "dev game pairs (first divergences saved)": the same two players in pairs, saving the positions where each pair first diverges |
| `--sim-games-job` | "dev sim games (positions saved)": `sim-2ply` against `sim-1ply` (10 plays considered, an iteration budget of 200), two games to a task, saving every position played |
| `--sim-pairs-job` | "dev sim game pairs (first divergences saved)": the same two simmers in pairs, one to a task, saving the positions where each pair first diverges |

Each runs on `--lexicon` (CSW24 by default) and the english distribution.
Append `_ab` to a flag (`--leavegen-job_ab`, or `--leavegen-job-ab`) to run
that job on the two-letter test data below instead: "dev leave generation
(english_ab)" and the others, played by `ab-` player configs. There are only
eight racks, so the leave-generation and opening-rack jobs finish in minutes
(the opening-rack one two racks to a task).

New jobs share the allocation the active ones leave free, so four on a fresh
database get 25% each. The games and pairs jobs stop at 100,000 games or pairs
if their test has not decided first, and neither test is acted on before
50,000 games or pairs. Their players use a
wordmap and a rack info table; the workers share one ~1.9 GB copy of the
table (MAGPIE maps it by default rather than reading it into each), and
`--no-rit` seeds players without one.

The workers run in the background and write to
`.dev-workers/worker-NN/contribute.log`. With `--worker-windows` each runs in
its own terminal window instead (gnome-terminal, konsole, xfce4-terminal,
kitty, alacritty or xterm, whichever is found first; ignored with no display):
Ctrl-C in a window stops that worker and Enter starts it again under the same
identity, and a second Ctrl-C, or closing the window, leaves it stopped.
Either way, Ctrl-C in dev.py stops every worker.

**A small data set to import.** While it runs, `dev.py` also serves MAGPIE's
two-letter test data, which the `_ab` job flags are seeded on — the `english_ab` distribution and the `CSW21_ab`
lexicon, eight possible racks — as MAGPIE-DATA version `20000101` on branch
`two-letter`, which the **Input data** page imports like any other. It stands in
for GitHub on Docker's bridge address (port 8482) and passes every other
request through, so real imports still work; and it copies those three files
into the MAGPIE data directory the workers share. A leave-generation job on it
finishes a generation in seconds, where English has 3,199,724 racks to cover;
[JOURNEYS.md](JOURNEYS.md) walks through one.

**Contributors are always real MAGPIE.** There is no fake-worker mode here.
`worker/fake_worker.py` belongs to the end-to-end suite, where a browser
journey needs contributions to arrive on cue at predictable values without a C
toolchain in the loop — that is a property of an assertion harness, not of a
place you develop. Watching synthetic numbers move a dashboard tells you
nothing about what your change did.

**A job whose players ask for a wordmap, a rack info table or a word info
table waits for the builder.** The server publishes the hash of a copy it built itself, and nothing
in the compose stack builds one on its own — production runs the builder as a
scheduled task ([infra/derived.tf](infra/derived.tf)). `scripts/dev.py` runs
it for you, after seeding and then whenever something is queued while it
runs. Without dev.py, run it once after creating such a job, and it builds what is queued — up to eight files a run —
and exits. A build that failed waits 5 minutes (then 15) before it is tried
again, so a run straight after a failure builds nothing; `/admin/derived-data`
shows the error, and the Retry button once a build has failed three times:

```bash
docker compose run --rm derived-builder
```

`/admin/derived-data` shows what is waiting. The end-to-end script does this
itself for the jobs it creates that need it.

So this needs two things Docker cannot provide, and fails naming both when
either is missing:

| | Default | Override |
|---|---|---|
| A built MAGPIE binary | `~/MAGPIE/bin/magpie` | `--magpie`, or `$MAGPIE_BIN` |
| A real MAGPIE-DATA install | the `data/` of that binary's checkout | `--magpie-data`, or `$MAGPIE_DATA_PATH` |

### Choosing how it runs

Everything worth varying is a flag; `./scripts/dev.py --help` is the full list.

```bash
./scripts/dev.py --workers 6                  # six contributors instead of four
./scripts/dev.py --workers 1 --threads 12     # one contributor, more threads each
./scripts/dev.py --games-job --leavegen-job    # start with a games job and a small leave job
./scripts/dev.py --no-browser                 # SSH sessions and CI (prints the sign-in link)
./scripts/dev.py --login-as alice             # open the site signed in as another account
./scripts/dev.py --hot-reload                 # add the Vite dev server on :5174
./scripts/dev.py --no-up --no-seed            # attach contributors to a stack already running
./scripts/dev.py --rebuild --reset-db         # after a schema change: new images, fresh database
./scripts/dev.py --fresh                      # as a new deployment: no accounts, data or jobs
```

After a schema change the backend refuses a database made by the migration's
older edit ("migration 1 was previously applied but has been modified": the one
migration changes in place until release), and `dev.py` says so. `--reset-db`
drops the schema so the backend rebuilds it; the database's data goes, the
MinIO bucket stays, and `scripts/dev-dump.sh` snapshots both first if you want
them. Add `--rebuild` when the images predate the change.

The fresh database is seeded with:

- the `dev` admin and the input data;
- the jobs the job flags ask for, and none without one;
- two contributor accounts, `dev-contributor-1` and `-2`, each with a new API
  key that workers 3 and 4 run under and keep in their `contribute.txt` for
  later runs. Workers 1 and 2 contribute anonymously.

| Flag | Default | What it changes |
|---|---|---|
| `-w`, `--workers` | 4 | How many `magpie contribute` processes run. Workers 1 and 2 are anonymous; 3 and 4 run as `dev-contributor-1` and `-2` under the API keys `--reset-db` makes (anonymous until a reset has made them) |
| `--threads` | 2 | Threads inside each contributor |
| `--max-tasks` | 0 | Tasks each contributor runs before exiting; 0 runs until stopped |
| `--idle-wait` | 5 | Seconds a contributor waits when there is no work |
| `--build-threads` | `$MAGPIE_THREADS`, or every core | Threads the server's derived-file builder (wordmaps, rack info tables, word info tables) gives MAGPIE |
| `--api-key` | anonymous | Contribute under an account instead of anonymously |
| `--leavegen-job`, `--opening-rack-job`, `--games-job`, `--pairs-job`, `--sim-games-job`, `--sim-pairs-job` | none | The jobs to start with, which stack: see above. Each also comes as `<flag>-ab`, the same job on the two-letter `english_ab` data |
| `--lexicon`, `--variant` | CSW24, classic | The lexicon of every job not on the `-ab` data, and of its players; the variant of every job (left unset, `seed.py`'s default, classic) |
| `--tarball-date` | your `DATA_VERSION` | Which MAGPIE-DATA version to import |
| `--min-magpie-version` | your build's version | The version floor, on the server and on the job |
| `--web-port`, `--backend-port` | 5173, 8080 | Host ports |
| `--worker-windows` | off | Run each worker in its own terminal window, to stop and restart by hand |
| `--workdir` | `.dev-workers` | Where per-worker directories live |
| `--reset-workers` | off | Delete them first, so each starts as a brand-new anonymous worker (the keyed workers lose their keys until the next `--reset-db`) |
| `--rebuild` | off | Rebuild images before starting |
| `--reset-db` | off | Drop the database's schema before starting, so the backend rebuilds it (after a schema change), and seed the fresh database with the admin, the input data, the jobs the job flags ask for and the two contributor accounts: see below |
| `--fresh` | off | Start as a new deployment does: `--reset-db` and `--reset-workers` without the seed, so no accounts, data imports or jobs; opens signed out and prints how to become the first admin |
| `--keep-up` | off | Leave the stack running on exit instead of stopping it |
| `--no-seed` | off | Skip seeding: no admin, data import or job (refused beside a job flag) |
| `--no-up` | off | Assume the stack is already running |
| `--login-as` | the seeded admin (`--username`) | Open the site signed in as this account |
| `--no-login` | off | Open the site signed out |

Each contributor gets its own directory under `--workdir`, holding its
`contribute.txt`, a `contribute.log`, and a symlink to your data directory
(MAGPIE loads its board from `./data` before anything else). Each needs a
`contribute.txt` of its own because MAGPIE writes the identity it is issued
into that file — sharing one would collapse every worker onto a single
identity. The file is rewritten from the flags on every run, keeping only
that identity, and only while the database still has it: after `--reset-db`
or a restore a worker is issued a new one instead of being refused. Watch one
with `tail -f .dev-workers/worker-01/contribute.log`; when one exits, `dev.py`
prints the last line of its log (MAGPIE exits 0 on errors too).

The site opens signed in, as the seeded admin unless `--login-as` names
another account: through `/api/dev/login`, a sign-in without a password that
the local stack enables (`DEV_LOGIN` in `docker-compose.yml`) and the backend
refuses anywhere cookies are secure. A stack built before it needs
`--rebuild` once.

Ctrl-C stops the contributors and then the stack (`docker compose down`,
without `-v`: the database and MinIO volumes are kept, so the next run starts
where this one stopped). `--keep-up` leaves the stack running so the site stays
browsable, and a stack attached to with `--no-up` is always left running.

### Seeding on its own

`scripts/seed.py` is what `dev.py` calls, and it runs standalone against any
birdtest:

```bash
./scripts/seed.py --api http://localhost:8080 --magpie-root ../MAGPIE
```

It drives the **real HTTP API** rather than writing SQL, so seeding is itself a
smoke test of registration, confirmation, import, validation and job creation.
Two things have no endpoint and are done directly: promoting a user to admin
(`is_admin` is settable through no endpoint, by design) and reading the emailed
confirmation code out of the backend's log, which is the only place the
plaintext exists — `email_confirmations` stores a hash. Re-running is safe.

The tarball date defaults to the `DATA_VERSION` in your MAGPIE checkout's
`download_data.sh`, so the digests the server pins are the bytes your workers
actually have. If those diverge, every worker declines every task.

### Doing it by hand

`docker compose up` still brings up just the stack — database, object storage,
backend and frontend. Docker is the only host dependency besides a MAGPIE
build, which the backend mounts and refuses to start without (below):

```bash
docker compose up --build
```

Then open **http://localhost:5173**. Nginx serves the SPA and proxies `/api` to
the backend, exactly as the ALB does in production, so the app runs on a single
origin locally too. The API is also exposed directly on :8080 for poking at
with `curl`. Nothing dispatches until a job is active, which is what `seed.py`
is for.

Migrations run inside the backend process before it binds, and the artifact
bucket is created by a one-shot `minio-init` container, so there is nothing to
sequence by hand.

If a port is taken, copy `.env.example` to `.env` and override it — no need to
edit the compose file:

```bash
WEB_PORT=5174 POSTGRES_PORT=5433 MINIO_PORT=9002 docker compose up --build
```

The first registered user is deliberately *not* an admin, so promotion is a
manual step if you are not using `seed.py`:

```bash
# MAIL_BACKEND=console puts the confirmation link in the backend's log:
docker compose logs -f backend
docker compose exec postgres \
  psql -U birdtest -d birdtest -c "UPDATE users SET is_admin = true WHERE username = 'you';"
```

**The backend runs your MAGPIE checkout.** It builds every wordmap, rack info
table and leave-generation KLV with it, reads the builder versions out of the
binary at startup, and refuses to start without one. Build it first (`make
magpie BUILD=portable_release`) and set `MAGPIE_ROOT` if the checkout is not at
`../MAGPIE`; the same binary runs the contributors, which is what keeps the
server's builder and the fleet's identical.

**The version floor stops an old MAGPIE from contributing.**
`MIN_MAGPIE_VERSION` defaults to `0.1.1`, the version `birdtest-contribute`
reports. A checkout that reports something
lower has every task declined with "update MAGPIE" until you update it or lower
the floor — on the server *and* on the job, which records its own floor at
creation:

```bash
MIN_MAGPIE_VERSION=0.0.0 docker compose up -d
```

`dev.py` reads the version out of your checkout and sets both for you.

Leave-generation jobs build a zeroed KLV at creation, and each generation writes
one progress row per full 7-tile rack — 3,199,724 rows for a real English bag —
when its first claim finds the universe missing (seeded off the claim path, so
that claim is answered at once). Worth knowing before you create one by hand.
Opening-rack jobs only *count* their rack space (3,199,724 racks for English)
and address it by range, so they are cheap to create. As racks are analysed,
each gets a progress row (`opening_rack_progress`), whatever the job's
consensus settings, so those settings can be changed later from the job's
admin page.

### Contributing with MAGPIE

A contributor needs only MAGPIE — no Python, no Docker, nothing else to
install. `./bin/magpie contribute`, run from the directory holding MAGPIE's
`data/`, contributes to https://birdtest.org anonymously on every core but one.
Settings go in a `contribute.txt` in that directory (MAGPIE reads both from its
working directory); the file is optional, and any setting it leaves out takes
its default: `server` https://birdtest.org, `apikey` none (anonymous),
`threads` every core but one, `maxtasks` 0 (no limit), `idlewait` 5 seconds.
MAGPIE prints the settings it is using at start, marking the defaulted ones.
A local stack is not the default server, so name it:

```text
server   http://localhost:5173
threads  7
maxtasks 0
```

then run `./bin/magpie contribute` there. Given an anonymous identity, MAGPIE
appends a `uuid` line to `contribute.txt`, creating the file if there is none;
it never writes a defaulted setting into it. A second process in the same
directory needs a file of its own — a copy of the one above, without the
`uuid` line MAGPIE appends on a first run, named on the command line (a named
file must exist, unlike the default `contribute.txt`; an empty one will do)
(`./bin/magpie contribute second.txt`): MAGPIE appends the identity it is issued
to that file. (A directory of its own does not work unless it also holds
MAGPIE's `data/`, or a link to it: MAGPIE loads its default board from
`./data` before it reads anything else.) Settings never
go on the command line, so an API
key stays out of shell history and `ps` output. A job's players decide whether
a wordmap (`.wmp`) and a rack info table (`.rit`) are used — both are on by
default, since they make game play much faster — and a word info table
(`.wit`), which is off by default, as it is in MAGPIE. MAGPIE builds each from
files it already has the first time a job asks (a wordmap or a word info table
in a couple of seconds, a rack info table in one to three minutes and about
2.4 GB of memory, 1.9 GB on disk and in memory after), checks it against the
hash the server published, and never transmits any of them. Workers sharing a data directory build each file once
between them -- the others wait, saying so -- and map the table
(`-ritmmap`, on unless `magpie contribute ... -ritmmap false`), so they share
one copy of it in memory; see [MAGPIE on the server](#magpie-on-the-server). See
[Worker Client](PLAN.md#worker-client-1) for the full protocol.

### Running the tests

```bash
(cd backend && cargo test --lib --bins)  # unit and contract tests; no services
(cd backend && TEST_DATABASE_URL=postgres://birdtest:birdtest@localhost:5432/birdtest \
  TEST_S3_ENDPOINT=http://localhost:9000 \
  cargo nextest run)                      # plus the integration and API tests in backend/tests/
(cd frontend && npm run check && npm test)
MAGPIE_ROOT=../MAGPIE e2e/run.sh          # the Playwright journeys, on a stack of their own
MAGPIE_ROOT=../MAGPIE scripts/e2e_magpie_native.sh   # real `magpie contribute` tasks (tier 6)
```

The integration tests clone a template database per test on the server
`TEST_DATABASE_URL` points at (any database there will do; they never write to
it), and the object-store tests make a bucket each on the MinIO at
`TEST_S3_ENDPOINT`, so the compose Postgres and MinIO are enough. They fail
rather than skip without them. `cargo test` works too; `cargo nextest run` adds
a ten-minute ceiling per test. [TESTING.md](TESTING.md) has what each tier
needs, including the opt-in tests that run a real MAGPIE.

### Frontend hot reload

```bash
docker compose --profile dev up
```

That adds a Vite dev server with HMR on **http://localhost:5174**, source
bind-mounted from `frontend/`, alongside the production-style Nginx build on
5173. `node_modules` lives in a named volume so the container's install never
collides with a host one.

### After a schema change

There is a single migration until release — schema changes edit
`backend/migrations/0001_initial.sql` in place rather than adding a numbered
one. sqlx checksums applied migrations, so an edited `0001` will not apply over
a database that already has the old version, and the backend will refuse to
start. Reset it:

```bash
docker compose exec postgres \
  psql -U birdtest -d birdtest -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;'
docker compose restart backend
```

Production is the same: `scripts/deploy.sh` refuses a commit that changed an
applied migration, and `scripts/deploy.sh --reset-db` empties the production
database as part of the deploy (below, "Operator scripts").

After release, a schema change is a new numbered migration, never an edit, and
it is **additive**: new tables, new nullable or defaulted columns, new
indexes. A drop or a rename waits for a later release, once no image that
reads the old shape can run. A new value in an enum type (`job_type`,
`job_status`, `task_state`, `claim_state`) is not additive in this sense: the
backend reads those into closed Rust enums, and one row the previous image
cannot read fails every query that reads it, the claim's included. Ship the
reading of a new value in one release and write it in a later one. The previous image then runs on the newer
schema. The backend starts against a database with migrations it does not
know (it refuses only an applied migration whose file changed), so rolling
back is an image change (RUNBOOK, "Rolling back a deploy").

### Without Docker

The backend and frontend still run directly on the host if you would rather:
`cargo run` in `backend/` and `npm run dev` in `frontend/`. The backend reads a
`backend/.env` if there is one: copy `backend/.env.example` (not the root
`.env.example`, which is compose's) to it, which sets everything a local run
needs — `DATABASE_URL`, `SESSION_SIGNING_KEY`, `MAGPIE_BIN` (a built MAGPIE
checkout's `bin/magpie`), `S3_ENDPOINT` and the MinIO credentials — and
adjust `MAGPIE_BIN` if your checkout is not at `../MAGPIE`. You need a Postgres
and MinIO for it to point at — `docker compose up -d postgres minio
minio-init` gives you both without the rest of the stack.

`scripts/dev.py` and `scripts/seed.py` need only `requests`.

## MAGPIE on the server

The backend runs MAGPIE itself: a pinned binary in its image in production,
your checkout (mounted) locally. It uses it for two things, and only on the
path where a worker's results are mixed into everyone else's — a wrong answer
there passes every plausibility check and lands in a job's totals.

1. **Wordmaps, rack info tables and word info tables are checked, not
   trusted.** For every wordmap (`.wmp`), rack info table (`.rit`) and word
   info table (`.wit`) a job needs, the server builds its own copy from the
   exact `.kwg` and `.klv2` bytes the job pins, keeps the
   SHA-256, and throws the file away. Workers build their own copy from inputs
   they have already verified and use it only if the hashes match.
2. **Leave-generation KLVs are MAGPIE's.** The zeroed generation-0 KLV and
   every generation's KLV are built by MAGPIE, so a change to how leave values
   are derived happens in one place. (Until 2026-09 the server built them with
   `backend/src/jobs/klv.rs`, a hand port of MAGPIE's code whose bytes differed
   from MAGPIE's.)

[Wordmap and rack info table provenance](PLAN.md#wordmap-and-rack-info-table-provenance)
in PLAN.md is the specification; this section is what it rests on and what it
costs.

### Why hash the output

A derived file records nothing reliable about how it was built, and MAGPIE's
CLI finds both by lexicon name alone. Recording the input digests — the
wordmap's `.wmp.src` sidecar did that for the `.kwg` — shows a file was built
*from* the right inputs and still trusts the builder, which is not a
theoretical gap: a CSW24 wordmap built in December 2025 and one built in
September 2026 differ in 72,852,152 bytes (and 60 bytes of size) with the same
inputs and the same format version 3. A builder change altered the output
without touching the format. Hashing the output checks what the worker will
actually load, and the server's own copy is a known-good answer.

That depends on MAGPIE's builders being deterministic, and they are, as far as
has been measured (MAGPIE `c2daa027`, GCC 10.5, Intel i7-10750H, x86-64
Linux):

| File | 1 thread | 8 threads | Same bytes? | Size | Peak memory |
|---|---|---|---|---|---|
| `CSW24.wmp` (`convert dawg2wordmap`) | 2.4 s | 1.7 s | **yes** | 179 MB | 710 MB |
| `CSW24.rit` (`convert klvwmp2rit`) | 169 s | 59 s | **yes** | 1.9 GB | 2.4 GB |
| `CSW24.wit` (`convert kwg2wit`, MAGPIE `birdtest-contribute`, 2026-10) | — | 2.8 s at 2 threads | **yes** (`NWL23`, 1 and 4 threads) | 122 MB | 310 MB |

| Comparison | `CSW21_ab.wmp` | `NWL23.wmp` | `CSW21_ab.rit` |
|---|---|---|---|
| `-march=native` vs `-march=nehalem` | identical | identical | identical |
| `CSW21_ab.wit`, `-march=native` vs `-march=nehalem` | identical | — | — |
| `dawg2wordmap` vs `dawg2text` + `text2wordmap` | — | identical | — |

A rack info table built by an earlier MAGPIE commit eight days before also
matched. **Not measured:** another compiler (Ubuntu 20.04's clang 10 cannot
build the tree — it rejects `-march=x86-64-v2`, which is why the portable
target is spelled `nehalem`) and another architecture (no ARM machine was to
hand). Nothing waits on either: see "The build target is reported, not
compared" below.

### What happens, end to end

1. **Import keeps lexica and leaves.** The tarball import stores every `.kwg`
   and `.klv2` in the object store, keyed by SHA-256, so the server holds the
   exact inputs a job pins.
2. **A job that needs a derived file queues a build** in `derived_data`: a
   wordmap for a `.kwg` when a player (or a leave job) uses one, a rack info
   table for a `(.kwg, .klv2)` pair when a player uses one, and a word info
   table for a `.kwg` when a player (or a leave job) uses one. Files are queued when the job
   is created, when it is activated, and by the first claim that finds one with
   no row under the running binary's builder — which is what rebuilds
   everything after a deploy that bumps a builder version. Not at player-config
   creation: a config has no letter distribution until a job gives it one.
3. **A builder task runs MAGPIE**: it writes the inputs and the job's letter
   distribution into a scratch data directory, runs `convert dawg2wordmap`,
   `convert klvwmp2rit` or `convert kwg2wit`, records the hash, and deletes the directory (in a
   `Drop`, deliberately blocking, so an early return cannot leak 1.9 GB).
4. **Dispatch waits for the hash**, the same way a leave-generation job waits
   for its rack universe. `/admin/derived-data` shows what is waiting, what
   failed and why, and retries a failure.
5. **The claim carries the hashes** in `expected_data.derived`, beside the
   input digests in `expected_data.files`:

   ```text
   "derived": [
     { "role": "wmp", "name": "CSW24", "sha256": "1830…",
       "builder": "wmp-1", "build_target": "nehalem" },
     { "role": "rit", "name": "CSW24.CSW_quackle_leaves", "sha256": "6da8…",
       "builder": "rit-1", "build_target": "nehalem" }
   ]
   ```

6. **The worker builds and checks.** After verifying its inputs it hashes what
   is on its disk (through its stat-keyed digest cache, so once per file, not
   per task), builds the file if that does not match, hashes it again, and uses
   it only if the bytes agree. Otherwise it declines with `derived_mismatch`,
   carrying both hashes, so a disagreement between builders shows up in the
   admin view rather than being worked around silently.

### Decisions worth knowing

- **Every hash is tied to its builder.** MAGPIE's `src/def/builder_defs.h`
  carries `WMP_BUILDER_VERSION`, `RIT_BUILDER_VERSION`, `WIT_BUILDER_VERSION`
  and `KLV_BUILDER_VERSION`, separate from `MAGPIE_VERSION` and bumped whenever a
  change alters that builder's output, even with an unchanged file format.
  `test/builder_hash_test.c` pins the output for `CSW21_ab` and fails until a
  change that alters it bumps the version *and* the hash; CI runs it on every
  birdtest pull request. A deploy with a newer builder records new hashes under
  the new version, and old ones stay for workers still on the old builder until
  the version floor passes them.
- **The server asks the binary, not its configuration.** `magpie builders`
  prints the four builder versions, `magpie_version` and the build target as
  JSON; the backend reads it
  at startup and refuses to start if it cannot. A configured value would drift
  the first time an image changed without the variable.
- **A worker declines on a hash, not a builder version.** Declining as soon as
  the versions differ would save one build, and would be wrong whenever the two
  builders in fact agree. The wasted build happens once: the job is remembered
  as unsupported.
- **The build target is reported, not compared**, for the same reason: a
  worker whose target differs builds and compares. Refusing on the field would
  lock out every contributor who builds from source for nothing measured; if
  targets ever do differ, the answer is a `derived_mismatch` in the admin view.
- **A table is named for its pair**, `CSW24.CSW_quackle_leaves`, not `CSW24`,
  and the claim names it (`rit_name`): two jobs on one lexicon with different
  leaves need different tables, and a table found by lexicon name would rank
  moves on leaves the job never pinned. That needed `klvwmp2rit` to take the
  KLV's and the wordmap's names separately — `convert klvwmp2rit <output> <ld>
  <klv_name> <wmp_name>`, both optional, so the CLI's `convert klvwmp2rit
  CSW24` is unchanged. Wordmaps and word info tables depend only on the `.kwg`
  and keep its name.
- **The server runs a binary, not a library.** `libmagpie.so` exists, but its
  API is the CLI's command strings, and in-process MAGPIE would put a 2.4 GB
  build and any crash inside the web server.
- **Builds are scheduled, not triggered.** The web task could call
  `ecs:RunTask` the moment a job is created; that would start a minutes-long
  build seconds sooner, at the price of an AWS control-plane call on the
  job-creation path, permission to run tasks and pass roles, and a retry story
  of its own. The builder task runs every five minutes.
- **The server keeps no derived file.** A copy would cost 1.9 GB per table for
  inspections nobody has needed, and the bytes are reproducible from inputs
  the object store holds.
- **No input digests in file headers.** They would still help CLI users, who
  have no server, but it is a format change to both `.wmp` and `.rit` —
  invalidating every one in existence — for a check weaker than the one the
  fleet now has. The `.wmp.src` sidecar is gone: the contribute path was its
  only reader, and every claim pins a wordmap's hash.
- **Leave generation keeps tables off**: its KLV changes every generation, and
  a 1.9 GB table per generation on every worker would cost far more than it
  saves.

### Where builds run

| Build | Where | Why |
|---|---|---|
| Wordmap (about 2 s, 710 MB peak), word info table (about 3 s, 310 MB peak, 122 MB file) and rack info table (1 to 3 min, 2.4 GB peak, 1.9 GB file) | the `birdtest-derived-builder` scheduled task ([infra/derived.tf](infra/derived.tf)): 4 vCPU, 8 GB, 30 GB of ephemeral storage, every five minutes, up to eight files a run under a lease | neither fits the web task's 1 vCPU and 2 GB |
| A generation's KLV (`convert rackequity2klv`) | the web task, off the claim path | the same derivation the Rust port did in about 13 s for English; the transition already runs on its own task |
| The generation-0 KLV (`createdata klv`) | the web task, at job creation | built from the letter distribution alone |

Locally, `docker compose run --rm derived-builder` is the scheduled task, and
`scripts/dev.py` runs it for you (see [Running locally](#running-locally)).

### Leave-generation KLVs

`leave_rack_progress` holds, per full rack, an occurrence count and an equity
sum. `convert rackequity2klv <name> <ld>` reads them from
`lexica/<name>.csv` (one `rack,count,equity_sum` row per full rack) and writes
the KLV through MAGPIE's own `rack_list_write_to_klv`, through a `RackList`
setter that takes a rack's count and mean directly. Every rack is marked unset
first, and a file that leaves any unset is refused: a rack observed zero times
and a rack the file forgot are different things, and valuing the second at
zero would be a real-looking leave value nobody measured. The server streams a
generation's rows (about 3.2 million for English) to that CSV and reads the KLV
back, holding neither in memory. The zeroed generation-0 KLV is `magpie
createdata klv <name> <ld>`, which already built exactly that; the `convert
zero2klv` once proposed for it was not added.

The server writes the job's pinned letter distribution into each scratch
directory, so nothing server-side reads a data file off its own disk.
`rebuild-artifacts` compares a rebuilt KLV's hash with the stored one, and
`leave_generation_artifacts.builder` records which builder wrote each: a
rebuild by a different builder reports that rather than "differs", or the
first upgrade after a restore drill would read as data loss.

Before `klv.rs` was deleted the two were run against each other over the
whole 149-rack test distribution: `rackequity2klv` and the Rust port agreed on
every one of the 431 leave values.

### The pinned binary

- `docker/Dockerfile` has a stage that checks out MAGPIE's
  `birdtest-contribute` at a pinned commit (`MAGPIE_COMMIT`, recorded as the
  image label `org.birdtest.magpie-commit`) and builds it; the backend and `derived-builder` targets both carry
  it. Push MAGPIE first — the image fetches the commit.
- It is built `BUILD=portable_release`, which targets `-march=nehalem` rather
  than the `-march=native` every other optimised MAGPIE build uses: a binary
  built on a CI runner could otherwise use instructions a Fargate CPU lacks.
  `nehalem` names the same instruction set as `x86-64-v2`, which only GCC 11
  and Clang 12 know. `PORTABLE_MARCH` overrides it on a non-x86 host, and the
  object directory is keyed by it, so changing it recompiles. Released
  contributor binaries use the same target.
- The binary is about 1 MB, and the builder task needs no MAGPIE-DATA install:
  it fetches only the files a build uses.
- birdtest is AGPL-3.0 and MAGPIE GPL-3.0; shipping the GPL binary in an AGPL
  service image is compatible.
- `MAGPIE_VERSION` (0.1.1 at the time of writing) moves with every change that
  can alter what a task computes or submits, and birdtest's version floor moves
  with it.

### What it costs a contributor

- **Time, once per file and builder version:** about 2 s to build a wordmap and
  0.8 s to hash it, about 3 s to build a word info table; one to three minutes and about 2.4 GB of memory to build a
  rack info table, and about 9 s to hash it. The heartbeat is already running,
  so a long build does not lose the claim.
- **Memory:** a process that plays with a rack info table holds it — about
  1.9 GB for CSW24 — so several `magpie contribute` processes on one machine
  hold one copy each.
- **Disk, still unbounded:** 179 MB per wordmap, 122 MB per word info table and
  1.9 GB per `(.kwg, .klv2)` pair with a rack info table, and nothing evicts them. Player configs use both by
  default, so a contributor on many lexicons accumulates tables; one who runs
  out of disk gets a failed build and a declined task, not a wrong result. The
  fix is a size cap with least-recently-used eviction in `contribute.txt`,
  which is a change to MAGPIE's `contribute.c` and to no protocol.

### What is not covered

- **The CLI.** `magpie` outside `contribute` still finds a table by lexicon
  name and checks nothing about it. A CLI user is analysing their own positions on their
  own data.
- **Data updates.** A tarball that changes a `.kwg` makes a new `input_data`
  row, so a job pinning the old one keeps its old derived files, correctly.
  Nothing prompts a rebuild — an admin creates a new job against the new row —
  and old `derived_data` rows (a row and a hash each) are never collected.
- **A full-size table end to end.** Tier 6's `M-10` runs a `use_rit` job
  through a real server and a real `magpie contribute` on MAGPIE's two-letter
  test data (`CSW21_ab`, `english_ab`), so its table is tiny; nothing tests a
  real 1.9 GB one. `M-11` forces a `derived_mismatch` by altering the recorded
  hash, standing in for a server whose builder differs. `M-13` does both for
  a word info table, on the same two-letter data.

### Alternatives considered

| Alternative | Why not |
|---|---|
| Input digests in file headers only | Trusts the builder, which is the failure measured above |
| The server distributes the files | Removes the determinism question, at 1.9 GB per table and 179 MB per wordmap to every worker |
| Link `libmagpie` | The same command-string API as the CLI, with MAGPIE's memory and crashes inside the web server |
| Keep `klv.rs` and add MAGPIE only for derived files | Two implementations of leave derivation that must agree by hand, beside a MAGPIE that can do it |

### Where it lives

- **birdtest:** `backend/src/magpie.rs` (the binary and its scratch
  directories), `backend/src/derived.rs` (what a job needs, the queue, the
  build), `backend/src/bin/build-derived.rs` (the builder task),
  `backend/src/jobs/leave_gen.rs` (KLVs), the `derived_data` table,
  `input_data.object_key`, `leave_generation_artifacts.builder`,
  `/admin/derived-data`, `docker/Dockerfile`, `infra/derived.tf`, and the
  contract fixtures for `expected_data.derived`, `rit_name` and
  `derived_mismatch`.
- **MAGPIE (`birdtest-contribute`):** `src/def/builder_defs.h` and `magpie
  builders`, `test/builder_hash_test.c`, contribute's derived-file check and
  build (`config_contribute_ensure_wordmap`,
  `config_contribute_ensure_rack_info_table`,
  `config_contribute_ensure_word_info_table`), `convert rackequity2klv`,
  `klvwmp2rit`'s separate input names, and `BUILD=portable_release`.

## Deploying

A first deployment, in order (each step is described below):

1. Tools: Terraform 1.9, the AWS CLI with the Session Manager plugin, `jq`,
   `openssl`, and `python3` for RUNBOOK.md's procedures.
2. Build and push the three images (below, "The three images"), and push
   MAGPIE's `birdtest-contribute` first — the backend image fetches the commit
   `docker/Dockerfile` pins.
3. Request an ACM certificate for the site's hostname in the stack's region,
   add its validation CNAME, and wait for it: the first apply creates the HTTPS
   listener, which refuses a certificate still pending validation and leaves
   the apply half done. First the request, which prints the certificate's ARN
   and, once ACM has made it (a few seconds), the CNAME to add:
   ```bash
   export AWS_PAGER=""   # no pager: one would swallow the rest of a paste
   REGION=us-east-1   # the stack's region, as prod.tfvars will say (step 4)
   SITE_HOSTNAME=''   # the site's hostname, e.g. birdtest.example.org
   # Pasted again, this requests a second certificate: to see the first one's
   # CNAME, run describe-certificate with its ARN instead.
   if [ -z "$SITE_HOSTNAME" ]; then
     echo "set SITE_HOSTNAME first" >&2
   elif ARN=$(aws acm request-certificate --region "$REGION" --domain-name "$SITE_HOSTNAME" \
       --validation-method DNS --query CertificateArn --output text); then
     echo "ARN=$ARN   # for the next block, and acm_certificate_arn in prod.tfvars"
     for _ in $(seq 60); do   # up to five minutes
       CNAME=$(aws acm describe-certificate --region "$REGION" --certificate-arn "$ARN" \
         --query 'Certificate.DomainValidationOptions[0].ResourceRecord.[Name,Value]' \
         --output text) && [ -n "$CNAME" ] && [ "$CNAME" != None ] && break
       CNAME=''
       sleep 5
     done
     if [ -n "$CNAME" ]; then echo "add this CNAME: $CNAME"
     else echo "no CNAME yet: ask describe-certificate for it by hand" >&2; fi
   fi
   ```
   Then, with the CNAME in DNS, wait for it. DNS validation can take half an
   hour, and the CLI's own wait gives up after about four minutes (older CLIs
   waited forty), so it is asked again, up to ten times:
   ```bash
   export AWS_PAGER=""   # no pager: one would swallow the rest of a paste
   if [ -z "${ARN:-}" ] || [ -z "${REGION:-}" ]; then
     echo "no ARN or REGION: run the block above first (or set both from it)" >&2
   else
     validated=''
     for round in $(seq 10); do
       if out=$(aws acm wait certificate-validated --region "$REGION" --certificate-arn "$ARN" 2>&1); then
         validated=yes
         break
       fi
       echo "$out" >&2
       # Only a wait that ran out on a certificate still pending is worth
       # another: one that failed validation or timed out, one this region
       # does not have, denied access, expired credentials or no network fail
       # the same way each time.
       case "$out" in *"Max attempts exceeded"*PENDING_VALIDATION*) ;; *) break ;; esac
       [ "$round" -lt 10 ] && echo "not validated yet; waiting again" >&2
     done
     if [ -n "$validated" ]; then echo "acm_certificate_arn = \"$ARN\""   # for prod.tfvars, step 4
     else echo "not validated: see the error above (check the CNAME, REGION and credentials); a failed or timed-out certificate needs a new request" >&2; fi
   fi
   ```
4. Write `infra/prod.tfvars` with the eight variables that have no default --
   `backend_image`, `derived_builder_image`, `frontend_image`, `alert_email`,
   `acm_certificate_arn`, `ses_domain`, `mail_from_address`, `public_url` --
   and `region` if it is not us-east-1, with `dr_region` (default us-west-2)
   if the stack is in us-west-2: the two must differ. Then
   `terraform -chdir=infra init` and
   `terraform -chdir=infra apply -var-file=prod.tfvars -var desired_count=0 -var scheduled_tasks_enabled=false`
   -- the scheduled builder and backup would fail until step 6 creates the two
   SSM parameters. Then pin the zones the stack chose:
   `AZS=$(terraform -chdir=infra output -json azs) && ! grep -q '^azs' infra/prod.tfvars && printf '\nazs = %s\n' "$AZS" >> infra/prod.tfvars`. Left to
   the default, the pair is recomputed on every plan, and a change to what the
   region reports would plan to replace the subnets the database sits in.
5. Add the SES DNS records straight away (the `ses_dkim_records`,
   `ses_mail_from_records` outputs, and `ses_dmarc_record`'s unless the
   domain has a DMARC record already -- a second one voids both; SES looks for
   the first two for about 72 hours) and request SES production access, which
   can take a day.
6. Confirm the SNS subscription mail, set the database password and the two
   SSM parameters (below), then
   `terraform -chdir=infra apply -var-file=prod.tfvars` (one task, and the
   scheduled tasks on). This apply creates the `-down` alarms before the
   task is healthy, so expect an ALARM mail for each and an OK a few minutes
   later.
7. Point DNS at the load balancer, run the alert-path checks
   (`scripts/check-alerts.sh`), and make the first admin
   (`scripts/confirm-user.sh --admin <name>` once they have registered). Until production access is granted SES sends only to verified
   identities, so the first admin's confirmation mail arrives only if their
   address is in `ses_domain` or verified on its own (below, "SES starts in the
   sandbox").
8. Import the input data. A new stack has none, and no job or player config
   can be made without it: signed in as the admin, open **Admin → Input
   data**, enter the MAGPIE-DATA version contributors' MAGPIE downloads --
   the `DATA_VERSION` in `download_data.sh` at the commit `docker/Dockerfile`
   pins, `20260925` today -- and the branch or tag (`main`), then **Fetch and
   diff** and confirm. A version other than the one contributors install pins
   digests their files do not have, and every worker declines every task.
   The import's GitHub calls are 60 an hour per address without
   `github_token_parameter_arn`; set it first (below, after the two SSM
   parameters) if more than a few imports are expected. Then make player configs and jobs. A job whose players need a
   wordmap, a rack info table or a word info table is not dispatched until
   the derived-data builder (every five minutes) has built them:
   `/admin/derived-data` shows the queue.

`infra/` is a complete Terraform description of the AWS side. Keep the stack's
variables in `infra/prod.tfvars` (not committed: it names the account's
certificate and addresses) and pass `-var-file=prod.tfvars` to every `apply`,
`plan` and `import` — RUNBOOK.md's recovery steps assume it, and a command run
without it evaluates the configuration in the default region, prompting for
eight variables. Two SSM parameters must be created out of band right after
the first `terraform apply` — Terraform only names them, and never reads or
writes them, so their values stay out of its state — so make the first apply
with `-var desired_count=0 -var scheduled_tasks_enabled=false`, create them as
below, and apply again with the service at one task and the schedules on:
started before they exist, the service's tasks cannot start, the
derived-data builder fails every five minutes, and a 03:00 backup fails
without starting, which only the 36-hour staleness alarm reports.

**Keep Terraform's state off the machine that applies.** The repository
chooses no backend, so unless you add one the state is local --
`infra/terraform.tfstate`, ignored by git. RUNBOOK.md's recovery steps, the
ops scripts and the operator scripts below read it (`terraform output`), so
keep it somewhere that survives that machine and the stack's region: an S3
backend in another region, versioned, with S3's lock file, before the first
apply. The backend is a file of your own, `infra/backend.tf` (ignored by
git), naming a bucket you create (versioning on, public access blocked) --
production's is `birdtest-tfstate-<account>` in us-east-2:

```bash
cat > infra/backend.tf <<EOF
terraform {
  backend "s3" {
    bucket       = "$STATE_BUCKET"
    key          = "birdtest/terraform.tfstate"
    region       = "$STATE_REGION"
    use_lockfile = true
  }
}
EOF
terraform -chdir=infra init
```

Every script works with whatever backend `terraform -chdir=infra` is
initialized with, local state included. Keep `infra/prod.tfvars` (and RUNBOOK
§5's `infra/dr.tfvars`, when there is one) with the state: it records no input
variables, and every later apply and the region-loss rebuild read them. The
operator scripts keep `prod.tfvars` beside the state, at
`s3://$STATE_BUCKET/birdtest/prod.tfvars`, fetching it before and uploading
it after each change. Neither holds a secret. (A stack applied before the thirty-second
audit's pass 17 managed the two SSM parameters as resources, and every refresh
wrote their decrypted values into the state: its next apply drops them from
the state without deleting them (an apply does; a `destroy` run first would
delete them), but older copies of the state —
`infra/terraform.tfstate.backup` among them — and a versioned backend's
history still hold them — so after that apply, rotate both, as below and in
RUNBOOK.md, "Rotating the database password".)

The database master password is set by hand, not managed by RDS (RDS rotation
would break the fixed `DATABASE_URL`). Terraform creates the instance with a
placeholder; replace it, then write the URL:

```bash
export AWS_PAGER=""   # no pager: one would swallow the rest of a paste
DB_INSTANCE=birdtest   # the RDS identifier Terraform created
# The stack's region on every command: with the CLI's default elsewhere,
# put-parameter quietly creates the parameters in the wrong region, the real
# ones are never created, and the service cannot start on the second apply.
# Assigned first: `export X=$(...)` hides a failed command. Both names: the
# CLI's version 1 reads only AWS_DEFAULT_REGION.
REGION=$(terraform -chdir=infra output -raw region)
export AWS_REGION=$REGION AWS_DEFAULT_REGION=$REGION
DB_PASSWORD=$(openssl rand -hex 24)   # hex: nothing to percent-encode in a URL
aws rds modify-db-instance --db-instance-identifier "$DB_INSTANCE" \
  --master-user-password "$DB_PASSWORD" --apply-immediately
aws rds wait db-instance-available --db-instance-identifier "$DB_INSTANCE"
ENDPOINT=$(aws rds describe-db-instances --db-instance-identifier "$DB_INSTANCE" \
  --query 'DBInstances[0].Endpoint.Address' --output text)

aws ssm put-parameter --name /birdtest/DATABASE_URL --type SecureString --overwrite \
  --value "postgres://birdtest:$DB_PASSWORD@$ENDPOINT:5432/birdtest"
aws ssm put-parameter --name /birdtest/SESSION_SIGNING_KEY --type SecureString --overwrite \
  --value "$(openssl rand -hex 32)"
```

To rotate the password later, run `scripts/rotate-db-password.sh` (RUNBOOK.md,
"Rotating the database password", says what it does and how by hand).

The GitHub token for input-data imports is optional and goes in a third
parameter, made the same way; a fine-grained token with read-only access to
public repositories is enough. Keep the default `aws/ssm` key (no `--key-id`):
the tasks' execution role has no `kms:Decrypt`, so under a key of your own
every task fails to start. `github_token_parameter_arn` takes the parameter's
ARN, not its name, in the stack's region (the plan refuses anything else; KL-62):
add it to `infra/prod.tfvars` and apply. `scripts/set-github-token.sh` does
all of that, the token read without echoing it; by hand:

```bash
export AWS_PAGER=""
# In the shell above: AWS_REGION set to the stack's region.
read -rsp 'GitHub token: ' GITHUB_TOKEN; echo   # not echoed, not in the history
aws ssm put-parameter --name /birdtest/GITHUB_TOKEN --type SecureString --overwrite \
  --value "$GITHUB_TOKEN"
aws ssm get-parameter --name /birdtest/GITHUB_TOKEN --query Parameter.ARN --output text
# arn:aws:ssm:<region>:<account>:parameter/birdtest/GITHUB_TOKEN, for prod.tfvars
```

`acm_certificate_arn` has no default either. The site is HTTPS-only — port 80
redirects — because the backend sets `Secure` cookies, which a browser will not
keep over plain HTTP. The API is not redirected but refused (`426`), so a
worker set to `http://` fails at once rather than sending its credential in
the clear on every request. `public_url`, `ses_domain` and `mail_from_address` have
none: they are what every confirmation and reset mail links to and is sent
from, and a placeholder left in is refused. `min_magpie_version` defaults to
`0.1.1`, the `birdtest-contribute` version the backend image pins; raise it
whenever a MAGPIE release changes results, since it is the only way to keep a
build that computes something wrong off the fleet. `derived_builder_image`
has no default — it is the backend image built with `--target derived-builder`,
and it must carry the same MAGPIE as `backend_image`, since the builder version
recorded beside every hash comes from the binary that produced it.

The three images are built from this repository and pushed, at one tag per
release, to ECR in any region (Terraform creates no repository; the task
execution role's managed policy covers the pull, and a repository in another
account must also admit this one in its own policy) or to a public registry. A
private registry elsewhere — a private GHCR or Docker Hub repository — needs
`repositoryCredentials`, which the task definitions do not set: every task
would fail with `CannotPullContainerError`.

```bash
docker build --pull --platform linux/amd64 -f docker/Dockerfile --target backend         -t $REGISTRY/birdtest-backend:$TAG .
docker build --pull --platform linux/amd64 -f docker/Dockerfile --target derived-builder -t $REGISTRY/birdtest-derived-builder:$TAG .
docker build --pull --platform linux/amd64 frontend -t $REGISTRY/birdtest-frontend:$TAG
docker push ...   # all three, then apply with backend_image, derived_builder_image, frontend_image
```

Build for `linux/amd64`, which is what the Fargate task definitions run, even on
an arm64 machine: the backend's MAGPIE build targets `-march=nehalem`, and an
arm64 frontend image fails on Fargate with "exec format error". Docker Desktop
emulates amd64 as it is; on an arm64 Linux host, register the emulator first
(`docker run --privileged --rm tonistiigi/binfmt --install amd64`), and expect
the emulated release builds to be slow.

The backend image fetches MAGPIE at `docker/Dockerfile`'s `MAGPIE_COMMIT`
from GitHub, so that commit must be pushed to `birdtest-contribute` first.

A release whose task fails to start three times is rolled back by ECS (the
service's deployment circuit breaker) to the last task definition that ran
steadily, and `apply` does not wait to see it: Terraform's state still names
the new one, and the next apply deploys it again. RUNBOOK.md, "Rolling back a
deploy", says how to tell and what to do. The rollback mails the alerts
topic (`birdtest-deploy-failed`, with ECS's reason), so it is not noticed only
by looking.

**Check that the alarms reach you** after the first apply (once the SNS
subscription is confirmed), and after any change to the alerts topic: nothing
else will say an alert was dropped. `scripts/check-alerts.sh` runs the checks
below and says which mails to look for; by hand, with `REGION` set as above:

```bash
export AWS_PAGER=""   # no pager: one would swallow the rest of a paste
# A function, not a variable holding the command: zsh does not split one. Not
# named `tf`: that is a common alias for terraform, and in bash an alias is
# expanded in a function definition -- `tf() {...}` then redefined
# `terraform` as a function calling itself.
tfout() { terraform -chdir=infra output -raw "$@"; }
SUFFIX=""   # the stack's name_suffix: "-dr" for RUNBOOK §5's copy
# To OK first: a fresh stack's staleness alarm is already in ALARM (no backup
# has run), and setting the state it is in sends nothing.
aws cloudwatch set-alarm-state --region "$REGION" --alarm-name "birdtest$SUFFIX-backup-stale" \
  --state-value OK --state-reason "testing the alert path"
aws cloudwatch set-alarm-state --region "$REGION" --alarm-name "birdtest$SUFFIX-backup-stale" \
  --state-value ALARM --state-reason "testing the alert path"     # a mail arrives
aws rds describe-event-subscriptions --region "$REGION" --subscription-name "birdtest$SUFFIX-db-storage" \
  --query 'EventSubscriptionsList[0].Status' --output text      # "active"

# A backup run that fails: its failure mail arrives. The task's entry point is
# `bash -c`, so the override is the whole script, one string.
aws ecs run-task --region "$REGION" --cluster "$(tfout cluster_name)" \
  --task-definition "$(tfout backup_task_definition)" --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[$(terraform -chdir=infra output -json service_subnet_ids | jq -r 'join(",")')],securityGroups=[$(tfout service_security_group_id)],assignPublicIp=ENABLED}" \
  --overrides '{"containerOverrides":[{"name":"backup","command":["exit 1"]}]}'
```

Then `AWS/Events` `TriggeredRules` for `birdtest$SUFFIX-backup-failed` is 1 and
its `FailedInvocations` 0. (RUNBOOK §5 runs the same checks with `SUFFIX=-dr`.)

A deploy the circuit breaker rolls back mails the same topic, through the
`birdtest$SUFFIX-deploy-failed` rule. ECS's deployment events cannot be sent by
hand (`aws.ecs` is AWS's own source), so check instead that the rule's pattern
matches a failed deployment of the live service; delivery is the backup
mail's, through the same topic and policy:

```bash
export AWS_PAGER=""
SERVICE=$(aws ecs describe-services --region "$REGION" --cluster "$(tfout cluster_name)" \
  --services "birdtest$SUFFIX" --query 'services[0].serviceArn' --output text)
aws events test-event-pattern --region "$REGION" \
  --event-pattern "$(aws events describe-rule --region "$REGION" --name "birdtest$SUFFIX-deploy-failed" \
     --query EventPattern --output text)" \
  --event "$(jq -nc --arg s "$SERVICE" --arg r "$REGION" '{id: "1", account: "123456789012",
     source: "aws.ecs", time: "2026-01-01T00:00:00Z", region: $r, resources: [$s],
     "detail-type": "ECS Deployment State Change", detail: {eventName: "SERVICE_DEPLOYMENT_FAILED"}}')"
# "Result": true
```

**SES starts in the sandbox.** A new account's SES sends only to verified
addresses, so until [production access](https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html)
is granted every registration and password reset to anyone else fails -- and
answers the caller as if it had not, so the `-mail-failed` alarm fires instead.
Request it, and add the `ses_dkim_records` output's CNAME records (each
`<token>._domainkey.<domain>` to `<token>.dkim.amazonses.com`), the
`ses_mail_from_records` output's MX and TXT records and, if the domain has no
DMARC record, the `ses_dmarc_record` output's TXT record, before opening
registration.

**Raise `mail_max_per_second`** in `prod.tfvars` to the account's maximum send
rate once production access is granted (the SES console shows it; 14 is usual):
the backend spaces its sends to it, and at the default of 1 a burst of
registrations waits in line.

**Mail has three alarms** (`infra/ses.tf`), to the same topic as the rest:
`-mail-failed` on any account mail that failed to send (the backend's log says
why, with SES's own code; search it for `mail_failed`), and `-ses-bounce-rate`
and `-ses-complaint-rate` at 4% and 0.08%, below the 5% and 0.1% at which SES
reviews an account (it may pause one at 10% and 0.5%, which would stop every
confirmation and reset). `-mail-failed` sends no OK: it clears itself after
five minutes with no failed send, which is not mail working again. Addresses
that hard-bounced or complained are suppressed account-wide, so a made-up
address bounces once. Registration mails any address it is given, so the bounce
rate is one a visitor can push (KL-91): on the bounce alarm, look for a burst
of new unconfirmed accounts (`users` rows with no `email_confirmed_at`).

**The database is reachable only from inside the VPC** — no public address, no
bastion, and its security group admits only the service's. SQL runs through
`scripts/prod-sql.sh`, which starts the ops task (`infra/ops.tf`: the postgres
image, `DATABASE_URL` from SSM, the service's network) with psql reading the
SQL and prints what psql printed; `scripts/prod-shell.sh` opens an interactive
shell in the same task through ECS Exec, for RUNBOOK.md's longer procedures
(the task outlives the session, which ECS ends after twenty idle minutes:
`scripts/prod-shell.sh --attach <task>` returns to it). The first admin is made that way,
after registering and confirming the account through the site — there is no
endpoint for it, by design:

```bash
scripts/prod-sql.sh "UPDATE users SET is_admin = true WHERE lower(username) = lower('alice') RETURNING username"
```

While SES is in the sandbox the confirmation mail cannot arrive, so
`scripts/confirm-user.sh --admin alice` confirms the address by hand (as the
site's confirm-email route does, with an audit row saying so) and makes the
account an admin in one step.

`alert_email` has no default: `terraform apply` refuses to run without
somewhere to send alarms (every one the stack raises, backup failures among
them), because an unmonitored backup is the failure mode the whole design
exists to avoid. SNS emails a subscription confirmation
that has to be accepted once.

The backend image carries a pinned MAGPIE, built from a commit the image
records. The server runs it to build the reference copy of every wordmap, rack
info table, word info table and leave-generation KLV, publishes the SHA-256 for workers to
reproduce, and throws the file away — see
[MAGPIE on the server](#magpie-on-the-server). It still carries no data
directory: every conversion runs against a throwaway directory written from the
bytes a job pins, so nothing server-side reads a data file off its own disk.

Wordmap, rack info table and word info table builds run in a separate scheduled task
(`birdtest-derived-builder`), because a table peaks at about 2.4 GB of memory
and writes a 1.9 GB file — neither of which fits the web task's 1 vCPU and
2 GB. A job whose derived files are not built yet is not dispatched; the admin
page at `/admin/derived-data` is where that wait is visible.

### Operator scripts

The procedures that repeat run as scripts on an operator's machine, with
their AWS login, instead of pasted blocks. Each reads `~/.birdtest-env`
(`BIRDTEST_ENV`): `AWS_PROFILE`, `SITE` (the domain), `STATE_BUCKET` and
`STATE_REGION` (the Terraform state bucket, which also keeps `prod.tfvars`),
and for deploys `REGISTRY` (ECR) -- `scripts/onboard-deployer.sh` writes it.
They take the stack's region and names from Terraform's outputs, print the
workspace and refuse any but `default` unless `BIRDTEST_WORKSPACE` names it,
say to run `aws sso login` when the login has expired, and ask on the
terminal before changing anything. Every apply goes through one plan gate:
the plan is shown and applied only once you say so, and one that destroys,
replaces or forgets the database, a bucket, the network or a KMS key is
refused outright (`BIRDTEST_ALLOW_DESTRUCTIVE_PLAN=yes` and a typed word let
one through). Releases, rollbacks, settings and resets are logged in
`~/birdtest-releases.log` (`BIRDTEST_RELEASE_LOG`). They need `aws` (with the
Session Manager plugin for ECS Exec), `terraform`, `jq`, `git`, `gh`,
`docker`, `python3`, `openssl` and `curl`, each script checking for its own.

| Script | Does |
| --- | --- |
| `deploy.sh [--reset-db] [--set KEY=VALUE]` | Deploys the checked-out commit of main: a clean tree, CI green for it (`gh`), the MAGPIE pin pushed to `birdtest-contribute` (`MAGPIE_DIR`, default `~/MAGPIE`); builds and pushes the images ECR lacks (`linux/amd64`, `CARGO_BUILD_JOBS=2`, `MAKE_JOBS=3`), retags `prod.tfvars`, plans, applies, uploads, waits for both target groups. Refuses a commit that changed a migration the live release applied (until launch `0001_initial.sql`, edited in place) unless `--reset-db`. |
| `reset-prod-db.sh [--no-start]` | Empties the production database (the hostname typed): the service stopped, the schema dropped and made again, the service started so the backend applies `0001`. Then: register, `confirm-user.sh --admin`, import the input data. `deploy.sh --reset-db` does the same once its plan is approved and before the new task starts. |
| `rollback.sh [--to TAG] [--reset-db]` | RUNBOOK "Rolling back a deploy": the previous release from the log, or the one a circuit breaker went back to. |
| `set-setting.sh KEY=VALUE...` | Changes `prod.tfvars` values (e.g. `mail_max_per_second=14`) and applies them. |
| `check-alerts.sh` | The alert-path checks above. |
| `confirm-user.sh [--admin] NAME` | Confirms an address by hand while SES is in the sandbox, as the confirm-email route does; `--admin` promotes too. |
| `rotate-db-password.sh [--signing-key] [--resume]` | RUNBOOK "Rotating the database password", never leaving the password only in a shell. |
| `set-github-token.sh` | The optional GitHub token, then `github_token_parameter_arn`. |
| `onboard-deployer.sh` | A new deployer's machine: settings, `infra/backend.tf`, `init`, `prod.tfvars`, and a plan that must show no changes. |
| `assess-damage.sh` | RUNBOOK §0. Reads only. |
| `restore-artifact.sh JOB GENERATION` | RUNBOOK §3: an older version of a KLV, chosen from a list. |
| `prod-psql.sh [-v NAME=VALUE] FILE.sql...` | SQL files in the ops task, with psql variables and their own transactions (`scripts/ops-sql/`: RUNBOOK §2.0, §2.3, §2.6, §4); `--keep`, `--task` and `--follow` for work that spans steps or outlasts the terminal. |
| `pitr-restore.sh STAGE` | RUNBOOK §1's mechanical stages, one at a time. |

`scripts/ops-scripts-check.sh` runs them against stand-ins for `aws`,
`terraform`, `gh` and `docker` (CI's scripts job), and with `PG_EXEC` their
SQL against a real Postgres (nightly).

## Backups

Two mechanisms, covering different failures — see
[Backups and Restore](PLAN.md#backups-and-restore) for which answers which:

- **RDS point-in-time recovery**, 30 days. The fast path for instance failure
  or a bad migration.
- **A nightly `pg_dump`** to an encrypted, versioned, Object-Locked and
  cross-region-replicated bucket, run by a scheduled Fargate task
  (`scripts/backup.sh`). This is the one that can be restored selectively,
  read on a laptop, or carried out of the account.

Health is on `/admin/backups`, and two alarms cover the rest: a task that
exits non-zero, and no successful backup in 36 hours. A restore drill runs
monthly, restoring the newest dump into a throwaway database and verifying it
(`scripts/restore-drill.sh`) — the only check that catches a dump that has been
silently producing unusable output.

Recovering from anything is [RUNBOOK.md](RUNBOOK.md).

### Locally

```bash
./scripts/dev-dump.sh before-experiment                # database + artifact bucket
SCRUB=0 ./scripts/dev-restore.sh .dev-backups/before-experiment
```

`dev-restore.sh` scrubs every restore unless `SCRUB=0` is set (and refuses any
value but 0 or 1) — your own snapshot too, whose addresses, passwords (your
admin's included), API keys, worker identities and backup history it would
reset, hence the `SCRUB=0` above. It also takes a production dump directory, which must be
scrubbed on the way in (`scripts/scrub.sql`: emails become `@example.invalid`,
every password becomes `birdtest-local`, credentials and tokens are truncated,
every anonymous worker's UUID -- its whole credential -- is replaced, and ban
reasons are blanked). Restoring production data locally without that is a
disclosure risk, not a shortcut. A restore goes into a copy, is scrubbed
there, and replaces the stack's database in one transaction only when both
have succeeded, so one that fails or is stopped before then leaves the stack as
it was (a stop during the swap waits for it and says which way it went); one
that fails or is stopped after it — in the artifact mirror — leaves the
database restored and the bucket not, says so, and exits 1. A process
killed outright (`kill -9`) leaves its copy, unscrubbed if the scrub had not
run, in the dev Postgres until the next restore drops it. `dev-dump.sh` refuses
a name that exists unless `FORCE=1`, and leaves nothing behind when it fails.
Snapshots taken before the audit's pass 24 hold no artifacts on Linux (the
mirror could not write them, and said nothing): a restore of one leaves the
bucket as it is.

The stack's ports are published on loopback only: with the repo's fixed
passwords and signing key, a stack holding a restored dump was open to anyone
on the same network. `BIND_HOST=0.0.0.0` opens them when that is wanted.

After any schema change, prove a dump still round-trips:

```bash
docker compose up -d postgres backend
./scripts/restore-roundtrip.sh
```

On an empty database (a fresh schema) it first seeds a row in each of seven
core tables; any other it round-trips as it is, which proves only as much as
its own rows do, and it wants the stack idle while it runs. It dumps, restores
into a fresh database, and checks row counts, referential integrity, the denormalized task
counters, and that `BYTEA` and `DOUBLE PRECISION` columns survived intact.
