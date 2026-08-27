# birdtest

Crowdsourced word game analysis, modelled after Fishnet. Admins define jobs;
contributors run a worker that claims tasks, executes them locally with
[MAGPIE](https://github.com/jvc56/MAGPIE), and submits results. The site
aggregates everything onto a live dashboard.

[PLAN.md](PLAN.md) is the design document — architecture, schema, API surface
and rationale all live there. This file is how to run it.

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

The worker needs MAGPIE, which is *not* in the image: birdtest does not build,
fetch or manage it, and its lexical data dwarfs the client. Point the stack at
a MAGPIE checkout on the host and it is bind-mounted into the container at
`/magpie`:

```bash
# Defaults to $HOME/MAGPIE; set MAGPIE_DIR in .env for anywhere else.
docker compose --profile worker up --build
```

Set `BIRDTEST_API_KEY` in `.env` (generate one at `/account`) to attribute the
work to your account rather than an anonymous UUID.

The same mount is given to the backend read-only, because leave-generation
aggregation shells out to `magpie convert csv2klv` once per completed
generation. Everything except leave-generation works fine without it.

### Frontend hot reload

```bash
docker compose --profile dev up
```

That adds a Vite dev server with HMR on **http://localhost:5174**, source
bind-mounted from `frontend/`, alongside the production-style Nginx build on
5173. `node_modules` lives in a named volume so the container's install never
collides with a host one.

### Without Docker

Each component still runs directly on the host if you would rather: `cargo run`
in `backend/` (see `.env.example`), `npm run dev` in `frontend/`, and
`pip install -e . && python worker.py` in `worker/`. You need a Postgres to
point `DATABASE_URL` at — `docker compose up -d postgres minio minio-init`
gives you one without the rest of the stack.

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
