#!/usr/bin/env bash
# Tier 5, end to end (TESTING.md section 5): bring up an isolated stack, seed a
# confirmed admin and a job, start the fake workers, run the Playwright
# journeys, and tear everything down again -- containers, volumes and the mail
# outbox -- however the run ends.
#
#   e2e/run.sh [--no-build] [--keep] [-- <playwright args>]
#
#   --no-build   use the birdtest-e2e-* images as they are instead of
#                rebuilding them from this checkout first
#   --keep       leave the stack up afterwards, for poking at a failure; tear
#                it down with the command this prints
#
# Needs Docker (compose v2.24 or later), Node 18+, Python 3 with `requests`,
# and a MAGPIE build: the backend refuses to start without one. MAGPIE_ROOT
# (default ../MAGPIE) is the checkout, `make magpie BUILD=portable_release`;
# it needs no data directory.
#
# The stack is its own compose project on its own ports (docker-compose.e2e.yml),
# so it can run beside a development stack.
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
E2E_DIR="$REPO_ROOT/e2e"

build=1
keep=0
playwright_args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) build=0 ;;
    --build) build=1 ;;
    --keep) keep=1 ;;
    --) shift; playwright_args=("$@"); break ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

log() { echo "[e2e] $*"; }

export COMPOSE_PROJECT_NAME=birdtest-e2e
export COMPOSE_FILE="$REPO_ROOT/docker-compose.yml:$REPO_ROOT/docker-compose.e2e.yml"
export MAGPIE_ROOT="${MAGPIE_ROOT:-$REPO_ROOT/../MAGPIE}"
MAGPIE_ROOT=$(cd "$MAGPIE_ROOT" 2>/dev/null && pwd) || {
  echo "MAGPIE_ROOT does not exist; point it at a MAGPIE checkout" >&2; exit 1; }
if [ ! -x "$MAGPIE_ROOT/bin/magpie" ]; then
  echo "no $MAGPIE_ROOT/bin/magpie: run \`make magpie BUILD=portable_release\` there first" >&2
  exit 1
fi

# The accounts and addresses every journey agrees on. The admin is seeded
# here; everything else a journey needs it makes for itself.
export E2E_BASE_URL=http://localhost:5280
export E2E_API_URL=http://localhost:8280
export E2E_ADMIN_USER=e2e-admin
export E2E_ADMIN_EMAIL=e2e-admin@example.invalid
export E2E_ADMIN_PASSWORD='e2e-Admin-passphrase-7431!'

compose() { docker compose "$@"; }

if [ "$build" = 1 ]; then
  # One image at a time, and the backend's Rust build capped: a release build
  # of the AWS SDK at full parallelism takes several GB.
  log "building images"
  docker build -f "$REPO_ROOT/docker/Dockerfile" --target backend \
    --build-arg CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" -t birdtest-e2e-backend "$REPO_ROOT"
  docker build -f "$REPO_ROOT/docker/Dockerfile" --target fake-worker \
    -t birdtest-e2e-fake-worker "$REPO_ROOT"
  docker build -t birdtest-e2e-frontend "$REPO_ROOT/frontend"
fi

# Where the backend writes mail. Its container user is uid 10001, so the
# directory is world-writable; the files it leaves are removed with it.
export E2E_OUTBOX_DIR
E2E_OUTBOX_DIR=$(mktemp -d "${TMPDIR:-/tmp}/birdtest-e2e-outbox.XXXXXX")
chmod 0777 "$E2E_OUTBOX_DIR"

teardown() {
  local status=$?
  if [ "$status" -ne 0 ]; then
    mkdir -p "$E2E_DIR/test-results"
    compose logs --no-color --timestamps > "$E2E_DIR/test-results/stack.log" 2>&1 || true
    log "failed; the stack's logs are in e2e/test-results/stack.log. The backend's last lines:"
    compose logs --no-color --tail 40 backend 2>&1 || true
  fi
  if [ "$keep" = 1 ]; then
    log "left up (--keep). Web: $E2E_BASE_URL, API: $E2E_API_URL, mail: $E2E_OUTBOX_DIR"
    log "tear down: COMPOSE_PROJECT_NAME=$COMPOSE_PROJECT_NAME COMPOSE_FILE=$COMPOSE_FILE" \
      "docker compose --profile fake-worker down -v --remove-orphans && rm -rf $E2E_OUTBOX_DIR"
  else
    log "tearing down"
    compose --profile fake-worker down -v --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$E2E_OUTBOX_DIR"
  fi
  exit "$status"
}
trap teardown EXIT
trap 'exit 130' INT TERM

wait_for() {
  local url=$1 seconds=$2
  for _ in $(seq "$seconds"); do
    curl -fsS -o /dev/null "$url" 2>/dev/null && return 0
    sleep 1
  done
  echo "$url did not answer within ${seconds}s" >&2
  return 1
}

log "starting the stack (project $COMPOSE_PROJECT_NAME)"
compose up -d --no-build postgres minio minio-init fixtures backend frontend
wait_for "$E2E_API_URL/health" 180
wait_for "$E2E_BASE_URL/health" 60

# A confirmed admin, the fixture data, two static player configs and an active
# game-pairs job, through the real API. Small enough to reach its SPRT verdict
# (or its cap) within the first minute or two of fake-worker results, so the
# job-page journeys see a finished pentanomial and the ratings journey has
# fixed evidence.
log "seeding"
python3 "$REPO_ROOT/scripts/seed.py" \
  --api "$E2E_API_URL" \
  --mail-outbox "$E2E_OUTBOX_DIR" \
  --username "$E2E_ADMIN_USER" --email "$E2E_ADMIN_EMAIL" --password "$E2E_ADMIN_PASSWORD" \
  --tarball-date 20260101 --letterdist english_fixture \
  --job-type game_pairs --batch 10 --min-units 100 --max-units 400 --allocation 30

log "starting the fake workers"
compose --profile fake-worker up -d --no-build fake-worker

cd "$E2E_DIR"
if [ ! -d node_modules ]; then
  npm ci --no-audit --no-fund
fi
npx playwright install --only-shell chromium >/dev/null
log "running Playwright"
npx playwright test "${playwright_args[@]}"
