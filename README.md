# birdtest

Crowdsourced word game analysis, modelled after Fishnet. Admins define jobs;
contributors run [MAGPIE](https://github.com/jvc56/MAGPIE) itself — `magpie
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

## Layout

| Path | What it is |
|---|---|
| `backend/` | Axum + SQLx server. Owns scheduling, validation, SPRT, ratings and aggregation. |
| `frontend/` | SvelteKit SPA (dark mode only), built statically and served by Nginx in production. |
| `worker/` | `fake_worker.py`, a synthetic client used by the **end-to-end suite only** — see [TESTING.md](TESTING.md). Local development uses real MAGPIE; the contributor client is MAGPIE itself, see [Worker Client](PLAN.md#worker-client-1). |
| `infra/` | Terraform: VPC, ALB, ECS Fargate, RDS Postgres, S3, SES, SSM, backups. |
| `scripts/` | `dev.py` (the local development command), `seed.py` (empty database → work flowing), plus backup, restore-drill and local snapshot scripts. |

Tile distributions and every other input file are no longer carried in the
repo: they are imported from a MAGPIE-DATA tarball into the `input_data` table
and pinned by SHA-256 — see [Input Data](PLAN.md#input-data-and-capability-negotiation).

## Running locally

One command brings up the stack, seeds it, starts real MAGPIE contributors and
opens the site:

```bash
./scripts/dev.py
```

That is the whole setup. It waits for the backend, imports the MAGPIE-DATA
tarball your own checkout installed, creates two player configs and an active
game-pairs job, launches two `magpie contribute` processes, and opens
**http://localhost:5173**.

**Contributors are always real MAGPIE.** There is no fake-worker mode here.
`worker/fake_worker.py` belongs to the end-to-end suite, where a browser
journey needs contributions to arrive on cue at predictable values without a C
toolchain in the loop — that is a property of an assertion harness, not of a
place you develop. Watching synthetic numbers move a dashboard tells you
nothing about what your change did.

**A job whose players ask for a wordmap or a rack info table waits for the
builder.** The server publishes the hash of a copy it built itself, and nothing
in the compose stack builds one on its own — production runs the builder as a
scheduled task ([infra/derived.tf](infra/derived.tf)). Run it once, after
creating such a job, and it drains the queue and exits:

```bash
docker compose run --rm derived-builder
```

`/admin/derived-data` shows what is waiting. The end-to-end script does this
itself for the jobs it creates that need it.

So this needs two things Docker cannot provide, and fails naming both when
either is missing:

| | Default | Override |
|---|---|---|
| A built MAGPIE binary | `../MAGPIE/bin/magpie` | `--magpie`, or `$MAGPIE_BIN` |
| A real MAGPIE-DATA install | `../MAGPIE/data` | `--magpie-data`, or `$MAGPIE_DATA_PATH` |

### Choosing how it runs

Everything worth varying is a flag; `./scripts/dev.py --help` is the full list.

```bash
./scripts/dev.py --workers 6                  # six contributors instead of two
./scripts/dev.py --workers 1 --threads 12     # one contributor, more threads each
./scripts/dev.py --job-type games             # seed a plain games job
./scripts/dev.py --no-browser                 # SSH sessions and CI
./scripts/dev.py --hot-reload                 # add the Vite dev server on :5174
./scripts/dev.py --no-up --no-seed            # attach contributors to a stack already running
```

| Flag | Default | What it changes |
|---|---|---|
| `-w`, `--workers` | 2 | How many `magpie contribute` processes run |
| `--threads` | 4 | Threads inside each contributor |
| `--max-tasks` | 0 | Tasks each contributor runs before exiting; 0 runs until stopped |
| `--idle-wait` | 5 | Seconds a contributor waits when there is no work |
| `--api-key` | anonymous | Contribute under an account instead of anonymously |
| `--job-type` | `game_pairs` | `game_pairs`, `games` or `opening_rack` |
| `--lexicon`, `--variant` | NWL23, classic | What the seeded job plays |
| `--tarball-date` | your `DATA_VERSION` | Which MAGPIE-DATA version to import |
| `--min-magpie-version` | your build's version | The version floor, on the server and on the job |
| `--web-port`, `--backend-port` | 5173, 8080 | Host ports |
| `--workdir` | `.dev-workers` | Where per-worker directories live |
| `--reset-workers` | off | Delete them first, so each starts as a brand-new anonymous worker |
| `--rebuild` | off | Rebuild images before starting |
| `--down` | off | Stop the stack on exit instead of leaving it up |
| `--no-seed` | off | Skip seeding (the stack already has an active job) |
| `--no-up` | off | Assume the stack is already running |

Each contributor gets its own directory under `--workdir`, holding its
`contribute.txt`, the `settings.txt` MAGPIE writes, a `contribute.log`, and a
symlink to your data directory. They need separate directories because
`magpie contribute` reads and writes both files in its working directory —
sharing one would race on them and collapse every worker onto a single
identity. Watch one with `tail -f .dev-workers/worker-01/contribute.log`.

Ctrl-C stops the contributors and leaves the stack up, so the site stays
browsable; `--down` tears it down instead.

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

Leave-generation jobs write one progress row per full 7-tile rack at creation
time — 3,199,724 rows for a real English bag, copied again for every later
generation — and build a zeroed KLV. Worth knowing before you create one by hand.
Opening-rack jobs only *count* their rack space (3,199,724 racks for English)
and address it by range, so they are cheap to create.

### Contributing with MAGPIE

A contributor needs only MAGPIE — no Python, no Docker, nothing else to
install. Put a `contribute.txt` beside it:

```
server   http://localhost:5173
threads  7
maxtasks 0
```

then run `magpie contribute`. Settings never go on the command line, so an API
key stays out of shell history and `ps` output. Wordmaps (`.wmp`) make game
play dramatically faster, so MAGPIE always wants one for a lexicon it's
contributing with; it derives the word list and the wordmap from the `.kwg` it
already has on first use, in about 1.3 seconds per lexicon, and never
transmits either. See [Worker Client](PLAN.md#worker-client-1) for the full
protocol.

### Running the tests

```bash
cd backend && cargo test --lib --bins     # unit and contract tests; no services
cd backend && TEST_DATABASE_URL=postgres://birdtest:birdtest@localhost:5432/birdtest \
  TEST_S3_ENDPOINT=http://localhost:9000 \
  cargo nextest run                       # plus the integration and API tests in backend/tests/
cd frontend && npm run check && npm test
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

### Without Docker

The backend and frontend still run directly on the host if you would rather:
`cargo run` in `backend/` (see `.env.example`) and `npm run dev` in `frontend/`.
You need a Postgres to point `DATABASE_URL` at — `docker compose up -d postgres
minio minio-init` gives you one without the rest of the stack.

`scripts/dev.py` and `scripts/seed.py` need only `requests`.

## Deploying

`infra/` is a complete Terraform description of the AWS side. Keep the stack's
variables in `infra/prod.tfvars` (not committed: it names the account's
certificate and addresses) and pass `-var-file=prod.tfvars` to every `apply`,
`plan` and `import` — RUNBOOK.md's recovery steps assume it, and a command run
without it evaluates the configuration in the default region, prompting for
eight variables. Two values must be set out of band right after the first
`terraform apply` — Terraform manages
the parameter *names* but never their values.

The database master password is set by hand, not managed by RDS (RDS rotation
would break the fixed `DATABASE_URL`). Terraform creates the instance with a
placeholder; replace it, then write the URL:

```bash
DB_INSTANCE=birdtest   # the RDS identifier Terraform created
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

To rotate the password later, run the same `modify-db-instance` and
`put-parameter` pair, then force a new ECS deployment so tasks re-read SSM
(RUNBOOK.md, "Rotating the database password").

`acm_certificate_arn` has no default either. The site is HTTPS-only — port 80
redirects — because the backend sets `Secure` cookies, which a browser will not
keep over plain HTTP. `public_url`, `ses_domain` and `mail_from_address` have
none: they are what every confirmation and reset mail links to and is sent
from, and a placeholder left in is refused. `min_magpie_version` defaults to
`0.1.1`, the `birdtest-contribute` version the backend image pins; raise it
whenever a MAGPIE release changes results, since it is the only way to keep a
build that computes something wrong off the fleet. `derived_builder_image`
has no default — it is the backend image built with `--target derived-builder`,
and it must carry the same MAGPIE as `backend_image`, since the builder version
recorded beside every hash comes from the binary that produced it.

The three images are built from this repository and pushed to a registry of
your choice (Terraform creates none), at one tag per release:

```bash
docker build -f docker/Dockerfile --target backend         -t $REGISTRY/birdtest-backend:$TAG .
docker build -f docker/Dockerfile --target derived-builder -t $REGISTRY/birdtest-derived-builder:$TAG .
docker build frontend -t $REGISTRY/birdtest-frontend:$TAG
docker push ...   # all three, then apply with backend_image, derived_builder_image, frontend_image
```

The backend image fetches MAGPIE at `docker/Dockerfile`'s `MAGPIE_COMMIT`
from GitHub, so that commit must be pushed to `birdtest-contribute` first.

**SES starts in the sandbox.** A new account's SES sends only to verified
addresses, so until [production access](https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html)
is granted every registration and password reset to anyone else fails. Request
it, and add the `ses_dkim_tokens` output as CNAME records, before opening
registration.

**The database is reachable only from inside the VPC** — no public address, no
bastion, and its security group admits only the service's. SQL runs through
`scripts/prod-sql.sh`, which starts the ops task (`infra/ops.tf`: the postgres
image, `DATABASE_URL` from SSM, the service's network) with psql reading the
SQL and prints what psql printed; `scripts/prod-shell.sh` opens an interactive
shell in the same task through ECS Exec, for RUNBOOK.md's longer procedures. The first admin is made that way,
after registering and confirming the account through the site — there is no
endpoint for it, by design:

```bash
scripts/prod-sql.sh "UPDATE users SET is_admin = true WHERE username = 'alice'"
```

`alert_email` has no default: `terraform apply` refuses to run without
somewhere to send backup failures, because an unmonitored backup is the failure
mode the whole design exists to avoid. SNS emails a subscription confirmation
that has to be accepted once.

The backend image carries a pinned MAGPIE, built from a commit the image
records. The server runs it to build the reference copy of every wordmap, rack
info table and leave-generation KLV, publishes the SHA-256 for workers to
reproduce, and throws the file away — see
[MAGPIE_DEPENDENCY.md](MAGPIE_DEPENDENCY.md). It still carries no data
directory: every conversion runs against a throwaway directory written from the
bytes a job pins, so nothing server-side reads a data file off its own disk.

Wordmap and rack info table builds run in a separate scheduled task
(`birdtest-derived-builder`), because a table peaks at about 2.4 GB of memory
and writes a 1.9 GB file — neither of which fits the web task's 1 vCPU and
2 GB. A job whose derived files are not built yet is not dispatched; the admin
page at `/admin/derived-data` is where that wait is visible.

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
./scripts/dev-dump.sh before-experiment      # database + artifact bucket
./scripts/dev-restore.sh .dev-backups/before-experiment
```

`dev-restore.sh` also takes a production dump directory, and scrubs it on the
way in (`scripts/scrub.sql`: emails become `@example.invalid`, every password
becomes `birdtest-local`, credentials and tokens are truncated). Restoring
production data locally without that is a disclosure risk, not a shortcut.

After any schema change, prove a dump still round-trips:

```bash
docker compose up -d postgres backend
./scripts/restore-roundtrip.sh
```

It seeds a row in every table a result touches, dumps, restores into a fresh
database, and checks row counts, referential integrity, the denormalized task
counters, and that `BYTEA` and `DOUBLE PRECISION` columns survived intact.
