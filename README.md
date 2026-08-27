# birdtest

Crowdsourced word game analysis, modelled after Fishnet. Admins define jobs;
contributors run a worker that claims tasks, executes them locally with
[MAGPIE](https://github.com/jvc56/MAGPIE), and submits results. The site
aggregates everything onto a live dashboard.

[PLAN.md](PLAN.md) is the design document — architecture, schema, API surface
and rationale all live there. This file is how to run it.
[MAGPIE-CLIENT.md](MAGPIE-CLIENT.md) specifies moving the worker client into
MAGPIE itself, so contributing needs MAGPIE and nothing else.

## Layout

| Path | What it is |
|---|---|
| `backend/` | Axum + SQLx server. Owns scheduling, validation, SPRT, Glicko and aggregation. |
| `frontend/` | SvelteKit SPA (dark mode only), built statically and served by Nginx in production. |
| `worker/` | Single-file Python worker client. Shells out to MAGPIE; contains no business logic. |
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

### Running a worker

MAGPIE is compiled from source into the worker image, along with the lexical
data it needs. There is no host checkout, no `--magpie-dir`, and nothing to
install:

```bash
docker compose --profile worker up --build
```

This is the *same image a real contributor runs* — the only difference is that
compose points it at `http://backend:8080` instead of a deployed URL. There is
no separate mock client, so the local worker exercises the real MAGPIE path.

A contributor's whole setup is one command:

```bash
docker run ghcr.io/jvc56/birdtest-worker \
  --server-url https://birdtest.example --api-key bt_...
```

Set `BIRDTEST_API_KEY` in `.env` to attribute the local worker's results to
your account rather than an anonymous UUID.

**Wordmaps** (`.wmp`) make game play dramatically faster, so a client that runs
games always wants one — but they are roughly ten times the size of everything
else MAGPIE ships. They are never transmitted. The image carries each lexicon's
`.kwg`, and the worker derives the word list and the wordmap from it on first
use, in about 1.3 seconds per lexicon, into a volume that survives restarts.

The backend image carries MAGPIE too, but no wordmaps: it never plays games, and
only shells out to `magpie convert csv2klv` once per completed leave generation.

### Frontend hot reload

```bash
docker compose --profile dev up
```

That adds a Vite dev server with HMR on **http://localhost:5174**, source
bind-mounted from `frontend/`, alongside the production-style Nginx build on
5173. `node_modules` lives in a named volume so the container's install never
collides with a host one.

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

The worker is Docker-only by design: shipping MAGPIE inside the image is what
lets a contributor start with one command, and it pins every contributor to the
same MAGPIE build, which the per-worker anomaly detection depends on.

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
