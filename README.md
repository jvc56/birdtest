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
and [RUNBOOK.md](RUNBOOK.md) is the recovery procedure itself.

## Layout

| Path | What it is |
|---|---|
| `backend/` | Axum + SQLx server. Owns scheduling, validation, SPRT, Glicko and aggregation. |
| `frontend/` | SvelteKit SPA (dark mode only), built statically and served by Nginx in production. |
| `worker/` | `fake_worker.py`, a test client that submits synthetic results with no MAGPIE in the loop — see [Testing without MAGPIE](#testing-without-magpie--the-fake-worker). The real contributor client is MAGPIE itself; see [Worker Client](PLAN.md#worker-client-1). |
| `infra/` | Terraform: VPC, ALB, ECS Fargate, RDS Postgres, S3, SES, SSM, backups. |
| `scripts/` | Backup, restore-drill and local snapshot scripts. See [Backups and Restore](PLAN.md#backups-and-restore) and [RUNBOOK.md](RUNBOOK.md). |

Tile distributions and every other input file are no longer carried in the
repo: they are imported from a MAGPIE-DATA tarball into the `input_data` table
and pinned by SHA-256 — see [Input Data](PLAN.md#input-data-and-capability-negotiation).

## Running locally

`docker compose up` is the whole setup. Database, object storage, backend and
frontend all run in containers, so Docker is the only thing the host needs —
no Rust, Node, Python or Postgres install.

```bash
docker compose up --build
```

Then open **http://localhost:5173**. Nginx serves the SPA and proxies `/api` to
the backend, exactly as the ALB does in production, so the app runs on a single
origin locally too. The API is also exposed directly on :8080 for poking at
with `curl`.

Migrations run inside the backend process before it binds, and the artifact
bucket is created by a one-shot `minio-init` container, so there is nothing to
sequence by hand.

If a port is taken, copy `.env.example` to `.env` and override it — no need to
edit the compose file:

```bash
WEB_PORT=5174 POSTGRES_PORT=5433 MINIO_PORT=9002 docker compose up --build
```

### An admin and a first job

The first registered user is deliberately *not* an admin, so promotion is a
manual step:

```bash
# 1. Register at http://localhost:5173/register. MAIL_BACKEND=console puts the
#    confirmation link in the backend's log:
docker compose logs -f backend

# 2. Promote yourself:
docker compose exec postgres \
  psql -U birdtest -d birdtest -c "UPDATE users SET is_admin = true WHERE username = 'you';"
```

Then create a player config at `/admin/player-configs/new`, create a job at
`/admin/jobs/new`, and activate it with an allocation. Nothing dispatches until
a job is active.

Opening-rack and leave-generation jobs enumerate their whole rack space at
creation time — for a real English bag that is millions of rows. Use lexicon
`TESTDIST` while poking at the UI; it is a deliberately tiny bag that exists
for exactly this.

### Testing without MAGPIE — the fake worker

Most server behaviour is best tested without a real engine in the loop.
Scheduling, SPRT, Glicko, redundancy and claim reclamation all want a *chosen*
outcome and a fast one, and the adversarial paths have no real-client
equivalent at all:

```bash
docker compose --profile fake-worker up            # or, directly:
python worker/fake_worker.py --server-url http://localhost:8080 --tasks 10
```

| Flag | What it exercises |
|---|---|
| `--workers N` | Concurrent claims — seed-tiling races, per-identity slot limits |
| `--p1-win-rate 0.65` | Drives SPRT to a chosen verdict instead of waiting for chance |
| `--mode malformed` | Submissions the server should reject with 400 |
| `--mode stale` | A claim token that was never issued; must be ignored, not accepted |
| `--mode abandon` | Claim and never submit, so the heartbeat timeout has to reclaim |
| `--seed` | Makes any of the above reproducible |

Every mode is deterministic under `--seed`, so a failing CI run reproduces.

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

The fake worker needs only `requests` and runs anywhere.

## Deploying

`infra/` is a complete Terraform description of the AWS side. Two values must
be set out of band before the first deploy — Terraform manages the parameter
*names* but never their values:

```bash
aws ssm put-parameter --name /birdtest/DATABASE_URL --type SecureString --overwrite --value '...'
aws ssm put-parameter --name /birdtest/SESSION_SIGNING_KEY --type SecureString --overwrite \
  --value "$(openssl rand -hex 32)"
```

`alert_email` has no default: `terraform apply` refuses to run without
somewhere to send backup failures, because an unmonitored backup is the failure
mode the whole design exists to avoid. SNS emails a subscription confirmation
that has to be accepted once.

The backend has no MAGPIE dependency at all: leave-generation aggregation
builds its KLV artifact directly (`backend/src/jobs/klv.rs`), and it reads
letter distributions out of the `input_data` row a job pins rather than off
disk, so the image carries nothing but its own compiled binary.

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
