# birdtest

Crowdsourced word game analysis, modelled after Fishnet. Admins define jobs;
contributors run [MAGPIE](https://github.com/jvc56/MAGPIE) itself — `magpie
contribute` claims tasks, executes them locally, and submits results. The site
aggregates everything onto a live dashboard.

[PLAN.md](PLAN.md) is the design document — architecture, schema, API surface
and rationale all live there. This file is how to run it.
[MAGPIE-CLIENT.md](MAGPIE-CLIENT.md) specifies the `contribute` command
contributors run, so contributing needs MAGPIE and nothing else.
[GAME-POSITION-CAPTURE.md](GAME-POSITION-CAPTURE.md) proposes keeping the
position analyses workers already produce while playing games.

## Layout

| Path | What it is |
|---|---|
| `backend/` | Axum + SQLx server. Owns scheduling, validation, SPRT, Glicko and aggregation. |
| `frontend/` | SvelteKit SPA (dark mode only), built statically and served by Nginx in production. |
| `worker/` | `fake_worker.py`, a test client that submits synthetic results with no MAGPIE in the loop — see [Testing without MAGPIE](#testing-without-magpie--the-fake-worker). The real contributor client is MAGPIE itself; see [MAGPIE-CLIENT.md](MAGPIE-CLIENT.md). |
| `data/letterdistributions/` | Tile distributions, mirroring MAGPIE-DATA's layout. Used to enumerate racks and leaves. |
| `infra/` | Terraform: VPC, ALB, ECS Fargate, RDS Postgres, S3, SES, SSM. |

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
transmits either. See [MAGPIE-CLIENT.md](MAGPIE-CLIENT.md) for the full
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

### Updating MAGPIE

The binary and the data are separate build stages with separate build args, so
bumping one leaves the other's layers cached:

```bash
MAGPIE_REF=<git-sha> docker compose build            # binary only (~25s)
MAGPIE_DATA_VERSION=<yyyymmdd> docker compose build   # data only (~10s)
```

Both stages are also `FROM scratch` payload images in their own right, ready to
publish and consume from a registry rather than rebuilt per clone:

```bash
docker build -f docker/Dockerfile --target magpie-bin  -t ghcr.io/jvc56/magpie-bin:<sha> .
docker build -f docker/Dockerfile --target magpie-data -t ghcr.io/jvc56/magpie-data:<ver> .
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

The backend image needs the `magpie` executable and `data/` on board: the
leave-generation aggregation step shells out to `magpie convert csv2klv` once
per generation.
