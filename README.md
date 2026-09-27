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

The command blocks here and in RUNBOOK.md are bash: in zsh, run `bash` first.
Stock zsh treats a `#` as a word, so a commented line fails or passes its
comment on as arguments, and an apostrophe in a comment opens a quote that
swallows the rest of the paste.

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
creating such a job, and it builds what is queued — up to eight files a run —
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
| A built MAGPIE binary | `../MAGPIE/bin/magpie` | `--magpie`, or `$MAGPIE_BIN` |
| A real MAGPIE-DATA install | `../MAGPIE/data` | `--magpie-data`, or `$MAGPIE_DATA_PATH` |

### Choosing how it runs

Everything worth varying is a flag; `./scripts/dev.py --help` is the full list.

```bash
./scripts/dev.py --workers 6                  # six contributors instead of two
./scripts/dev.py --workers 1 --threads 12     # one contributor, more threads each
./scripts/dev.py --job-type games             # seed a plain games job
./scripts/dev.py --no-browser                 # SSH sessions and CI (prints the sign-in link)
./scripts/dev.py --login-as alice             # open the site signed in as another account
./scripts/dev.py --hot-reload                 # add the Vite dev server on :5174
./scripts/dev.py --no-up --no-seed            # attach contributors to a stack already running
./scripts/dev.py --rebuild --reset-db         # after a schema change: new images, fresh database
```

After a schema change the backend refuses a database made by the migration's
older edit ("migration 1 was previously applied but has been modified": the one
migration changes in place until release), and `dev.py` says so. `--reset-db`
drops the schema so the backend rebuilds it; the database's data goes, the
MinIO bucket stays, and `scripts/dev-dump.sh` snapshots both first if you want
them. Add `--rebuild` when the images predate the change.

The fresh database is seeded with a full set, all on CSW24 (`--lexicon`):

- the `dev` admin;
- six jobs at equal allocation: games, opening racks, a small leave
  generation, and three game-pairs jobs among three players — `static-equity`,
  `static-score` and `sim-1ply` (a 1-ply sim, 100 iterations) — one for each
  pair of them. Games and pairs jobs stop at 2,000, if their test has not
  decided first, so all three pairs jobs finish, and an admin can then make a
  rating pool of the three players;
- two contributor accounts, `dev-contributor-1` and `-2`, each with a new API
  key that workers 3 and 4 run under and keep in their `contribute.txt` for
  later runs. Workers 1 and 2 contribute anonymously.

| Flag | Default | What it changes |
|---|---|---|
| `-w`, `--workers` | 4 | How many `magpie contribute` processes run. Workers 1 and 2 are anonymous; 3 and 4 run as `dev-contributor-1` and `-2` under the API keys `--reset-db` makes (anonymous until a reset has made them) |
| `--threads` | 2 | Threads inside each contributor |
| `--max-tasks` | 0 | Tasks each contributor runs before exiting; 0 runs until stopped |
| `--idle-wait` | 5 | Seconds a contributor waits when there is no work |
| `--api-key` | anonymous | Contribute under an account instead of anonymously |
| `--job-type` | `game_pairs` | `game_pairs`, `games` or `opening_rack` |
| `--lexicon`, `--variant` | NWL23, classic | What the seeded job plays |
| `--tarball-date` | your `DATA_VERSION` | Which MAGPIE-DATA version to import |
| `--min-magpie-version` | your build's version | The version floor, on the server and on the job |
| `--web-port`, `--backend-port` | 5173, 8080 | Host ports |
| `--workdir` | `.dev-workers` | Where per-worker directories live |
| `--reset-workers` | off | Delete them first, so each starts as a brand-new anonymous worker (the keyed workers lose their keys until the next `--reset-db`) |
| `--rebuild` | off | Rebuild images before starting |
| `--reset-db` | off | Drop the database's schema before starting, so the backend rebuilds it (after a schema change), and seed the fresh database with six jobs (three of them game pairs among three players) and the two contributor accounts: see below |
| `--down` | off | Stop the stack on exit instead of leaving it up |
| `--no-seed` | off | Skip seeding (the stack already has an active job) |
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

Leave-generation jobs build a zeroed KLV at creation, and each generation writes
one progress row per full 7-tile rack — 3,199,724 rows for a real English bag —
when its first claim finds the universe missing (seeded off the claim path, so
that claim is answered at once). Worth knowing before you create one by hand.
Opening-rack jobs only *count* their rack space (3,199,724 racks for English)
and address it by range, so they are cheap to create.

### Contributing with MAGPIE

A contributor needs only MAGPIE — no Python, no Docker, nothing else to
install. Put a `contribute.txt` in the directory you run it from, the one
holding its `data/` (MAGPIE reads both from its working directory):

```text
server   http://localhost:5173
threads  7
maxtasks 0
```

then run `./bin/magpie contribute` there. A second process in the same
directory needs a file of its own — a copy of the one above, without the
`uuid` line MAGPIE appends on a first run, named on the command line
(`./bin/magpie contribute second.txt`): MAGPIE appends the identity it is issued
to that file. (A directory of its own does not work unless it also holds
MAGPIE's `data/`, or a link to it: MAGPIE loads its default board from
`./data` before it reads anything else.) Settings never
go on the command line, so an API
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
`cargo run` in `backend/` (see `.env.example`) and `npm run dev` in `frontend/`.
You need a Postgres to point `DATABASE_URL` at — `docker compose up -d postgres
minio minio-init` gives you one without the rest of the stack.

`scripts/dev.py` and `scripts/seed.py` need only `requests`.

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
7. Point DNS at the load balancer, run the alert-path checks, and make the
   first admin. Until production access is granted SES sends only to verified
   identities, so the first admin's confirmation mail arrives only if their
   address is in `ses_domain` or verified on its own (below, "SES starts in the
   sandbox").

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

**Terraform's state is local** — `infra/terraform.tfstate`, ignored by git, on
the machine that applied. RUNBOOK.md's recovery steps and both ops scripts read
it (`terraform output`), so keep it somewhere that survives that machine and
the stack's region: copy it off after every apply, or configure a remote
backend (an S3 bucket in another region, versioned, with locking) before the
first one. The repository does not choose one for you. Keep `infra/prod.tfvars`
(and RUNBOOK §5's `infra/dr.tfvars`, when there is one) with it: the state
records no input variables, and every later apply and the region-loss rebuild
read them. Neither holds a secret. (A stack applied before the thirty-second
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

To rotate the password later, run the same `modify-db-instance` and
`put-parameter` pair, then force a new ECS deployment so tasks re-read SSM
(RUNBOOK.md, "Rotating the database password").

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

The three images are built from this repository and pushed to a registry of
your choice (Terraform creates none), at one tag per release:

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

**Check that the alarms reach you** after the first apply (once the SNS
subscription is confirmed), and after any change to the alerts topic: nothing
else will say an alert was dropped. With `REGION` set as above:

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
