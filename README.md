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

Everything runs on a laptop with no AWS access. SES is replaced by logging to
stdout, S3 by MinIO, and SSM by plain environment variables.

### Prerequisites

Rust (stable), Node 18+, Docker Compose, Python 3.11+, and — only if you want a
worker doing real computation — a compiled MAGPIE checkout.

### 1. Database and object storage

```bash
docker compose up -d
```

Postgres comes up on 5432 with `birdtest`/`birdtest`/`birdtest`, and MinIO
(standing in for S3) on 9000. If either port is taken, override it:

```bash
POSTGRES_PORT=5433 MINIO_PORT=9002 MINIO_CONSOLE_PORT=9003 docker compose up -d
```

### 2. Backend

```bash
cd backend
cp .env.example .env     # adjust DATABASE_URL / S3_ENDPOINT if you overrode ports
cargo run                # http://localhost:8080
```

Migrations run automatically at startup, before the server binds — there is no
separate migration step.

### 3. Frontend

```bash
cd frontend
npm install
npm run dev              # http://localhost:5173, proxying /api/* to :8080
```

### 4. An admin and a first job

The first registered user is deliberately *not* an admin, so promotion is a
manual step against the local database:

```bash
# Register at http://localhost:5173/register, then confirm the email —
# MAIL_BACKEND=console puts the confirmation link in the backend's stdout.
psql "$DATABASE_URL" -c "UPDATE users SET is_admin = true WHERE username = 'you';"
```

Then, signed in as that account: create a player config at
`/admin/player-configs/new`, create a job at `/admin/jobs/new`, and activate it
with an allocation from the admin job page. Nothing dispatches until a job is
active.

Opening-rack and leave-generation jobs enumerate their whole rack space at
creation time. For a real English bag that is millions of rows, so
`data/letterdistributions/TESTDIST.csv` exists as a deliberately tiny bag —
use lexicon `TESTDIST` while you are poking at the UI.

### 5. Worker

```bash
cd worker
python3 -m venv .venv && source .venv/bin/activate
pip install -e .
python worker.py --server-url http://localhost:8080 --magpie-dir /path/to/MAGPIE
```

With no active job the worker gets 204s and sleeps in its retry loop; that is
expected, not an error. Pass `--api-key` (generated at `/account`) to attribute
the work to your account rather than an anonymous UUID.

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
