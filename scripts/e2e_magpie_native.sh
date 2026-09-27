#!/usr/bin/env bash
# Tier 6 (TESTING.md, "6. MAGPIE smoke") on a developer machine, without
# building a single Docker image.
#
# scripts/e2e_magpie.py expects the compose stack. This brings up the same
# thing natively instead -- stock postgres and minio containers, and the
# backend and derived-file builder run straight from `cargo build` -- then runs
# the script against it with a `docker` shim first on PATH that answers the
# three compose calls it makes:
#
#   docker compose exec -T postgres psql ...   -> docker exec -i <postgres> psql ...
#   docker compose logs ... backend            -> the backend's log file
#   docker compose run ... derived-builder     -> the build-derived binary, same env
#
# Everything else goes to the real docker. The repository's scripts are not
# modified, and everything this creates -- two containers, a scratch
# directory holding the binaries, logs, the shim and the worker directories --
# is removed on exit, pass or fail.
#
#   MAGPIE_ROOT=../MAGPIE scripts/e2e_magpie_native.sh                 # every case
#   scripts/e2e_magpie_native.sh --cases M-5,M-11                      # some cases
#   scripts/e2e_magpie_native.sh --cases capture --capture-out contract-fixtures
#
# Arguments are passed through to scripts/e2e_magpie.py (see its --help).
#
# Needs: docker (postgres:16 and Chainguard's MinIO server and client images --
# MinIO no longer publishes its own), cargo,
# python3 with `requests`, and a MAGPIE checkout with a `portable_release`
# bin/magpie (`make magpie BUILD=portable_release`) and a download_data.sh
# install in its data/.
#
# Environment (all optional):
#   MAGPIE_ROOT         the MAGPIE checkout (default: ../MAGPIE)
#   TIER6_PREFIX        container name prefix (default: bt-tier6)
#   TIER6_PG_PORT       host port for postgres (default: 5745)
#   TIER6_MINIO_PORT    host port for minio (default: 9402)
#   TIER6_BACKEND_PORT  host port for the backend (default: 8480)
#   TIER6_GITHUB_PORT   host port e2e_magpie.py serves its GitHub stand-in on
#                       (default: 8481); the backend's GITHUB_API_URL and
#                       GITHUB_RAW_URL point at it
#   TIER6_THREADS       threads per contributor (default: 2)
#   TIER6_SKIP_BUILD=1  use the binaries already in the cargo target directory
#   TIER6_KEEP=1        leave the stack and scratch directory up afterwards
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAGPIE_ROOT="$(cd "${MAGPIE_ROOT:-$REPO_ROOT/../MAGPIE}" && pwd)"
PREFIX="${TIER6_PREFIX:-bt-tier6}"
PG_PORT="${TIER6_PG_PORT:-5745}"
MINIO_PORT="${TIER6_MINIO_PORT:-9402}"
BACKEND_PORT="${TIER6_BACKEND_PORT:-8480}"
GITHUB_PORT="${TIER6_GITHUB_PORT:-8481}"
THREADS="${TIER6_THREADS:-2}"
PG="$PREFIX-pg"
MINIO="$PREFIX-minio"
REAL_DOCKER="$(command -v docker)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/backend/target}"

log() { printf '[tier6] %s\n' "$*"; }
die() { printf '[tier6] %s\n' "$*" >&2; exit 1; }

[ -x "$MAGPIE_ROOT/bin/magpie" ] || die "no $MAGPIE_ROOT/bin/magpie: make magpie BUILD=portable_release"
[ -d "$MAGPIE_ROOT/data/lexica" ] || die "no $MAGPIE_ROOT/data: run its download_data.sh"
python3 -c 'import requests' 2>/dev/null || die "python3 needs requests (pip install requests)"
for name in "$PG" "$MINIO"; do
    if "$REAL_DOCKER" inspect "$name" >/dev/null 2>&1; then
        die "a container named $name already exists; remove it or set TIER6_PREFIX"
    fi
done

WORK="$(mktemp -d "${TMPDIR:-/tmp}/birdtest-tier6.XXXXXX")"
BACKEND_PID=""
cleanup() {
    local status=$?
    if [ "${TIER6_KEEP:-}" = 1 ]; then
        log "kept: $WORK, containers $PG and $MINIO, backend pid ${BACKEND_PID:-none}"
        exit "$status"
    fi
    if [ -n "$BACKEND_PID" ]; then
        kill "$BACKEND_PID" 2>/dev/null || true
        wait "$BACKEND_PID" 2>/dev/null || true
    fi
    # -v: both images declare a data VOLUME, and after a leave-generation case
    # the database's alone holds 1.6 GB -- without it every run left that behind.
    "$REAL_DOCKER" rm -f -v "$PG" "$MINIO" >/dev/null 2>&1 || true
    rm -rf "$WORK"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

log "scratch directory $WORK"
"$REAL_DOCKER" run -d --name "$PG" --memory 1g --cpus 2 \
    -e POSTGRES_USER=birdtest -e POSTGRES_PASSWORD=birdtest -e POSTGRES_DB=birdtest \
    -p "127.0.0.1:$PG_PORT:5432" postgres:16 >/dev/null
"$REAL_DOCKER" run -d --name "$MINIO" --memory 512m \
    -e MINIO_ROOT_USER=birdtest -e MINIO_ROOT_PASSWORD=birdtestbirdtest \
    -p "127.0.0.1:$MINIO_PORT:9000" cgr.dev/chainguard/minio:latest server /data >/dev/null

if [ "${TIER6_SKIP_BUILD:-}" != 1 ]; then
    log "building the backend"
    (cd "$REPO_ROOT/backend" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" nice cargo build --locked --bins)
fi
# Copied rather than run in place: a build in the same target directory while
# this runs must not swap the binary under a live test.
mkdir -p "$WORK/bin" "$WORK/shim" "$WORK/scratch"
cp "$TARGET_DIR/debug/birdtest" "$TARGET_DIR/debug/build-derived" "$WORK/bin/"

for _ in $(seq 60); do
    "$REAL_DOCKER" exec "$PG" pg_isready -U birdtest -d birdtest >/dev/null 2>&1 && break
    sleep 1
done
"$REAL_DOCKER" exec "$PG" pg_isready -U birdtest -d birdtest >/dev/null || die "postgres did not come up"
"$REAL_DOCKER" run --rm --network host --entrypoint sh \
    cgr.dev/chainguard/minio-client:latest-dev -c "
    for i in \$(seq 60); do
        mc alias set local http://127.0.0.1:$MINIO_PORT birdtest birdtestbirdtest >/dev/null 2>&1 && break
        sleep 1
    done
    mc mb --ignore-existing local/birdtest-artifacts" >/dev/null || die "could not create the bucket"

# The compose file's backend environment, pointed at this stack.
cat >"$WORK/backend.env" <<EOF
DATABASE_URL=postgres://birdtest:birdtest@127.0.0.1:$PG_PORT/birdtest
BIND_ADDR=127.0.0.1:$BACKEND_PORT
SESSION_SIGNING_KEY=00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff
SECURE_COOKIES=false
MAIL_BACKEND=console
MAIL_FROM=no-reply@birdtest.local
PUBLIC_URL=http://localhost:5173
MIN_MAGPIE_VERSION=0.1.1
MAGPIE_BIN=$MAGPIE_ROOT/bin/magpie
MAGPIE_THREADS=$THREADS
MAGPIE_SCRATCH_DIR=$WORK/scratch
MAGPIE_DATA_REPO=${MAGPIE_DATA_REPO:-jvc56/MAGPIE-DATA}
GITHUB_API_URL=http://127.0.0.1:$GITHUB_PORT/api
GITHUB_RAW_URL=http://127.0.0.1:$GITHUB_PORT/raw
TRUSTED_PROXY_HOPS=0
S3_BUCKET=birdtest-artifacts
S3_ENDPOINT=http://127.0.0.1:$MINIO_PORT
AWS_ACCESS_KEY_ID=birdtest
AWS_SECRET_ACCESS_KEY=birdtestbirdtest
AWS_REGION=us-east-1
RUST_LOG=birdtest=info,tower_http=warn
EOF
if [ -n "${GITHUB_TOKEN:-}" ]; then
    printf 'GITHUB_TOKEN=%s\n' "$GITHUB_TOKEN" >>"$WORK/backend.env"
fi

cat >"$WORK/shim/docker" <<EOF
#!/usr/bin/env bash
# The three compose calls scripts/e2e_magpie.py and scripts/seed.py make,
# answered by the native stack; anything else goes to the real docker.
if [ "\$1" = compose ]; then
    shift
    case "\$1" in
        exec)
            shift
            while [ "\${1:-}" = -T ]; do shift; done
            shift  # the service
            exec "$REAL_DOCKER" exec -i "$PG" "\$@" ;;
        logs)
            exec cat "$WORK/backend.log" ;;
        run)
            set -a; . "$WORK/backend.env"; set +a
            exec "$WORK/bin/build-derived" ;;
    esac
    echo "tier-6 docker shim: unexpected compose call: \$*" >&2
    exit 2
fi
exec "$REAL_DOCKER" "\$@"
EOF
chmod +x "$WORK/shim/docker"

log "starting the backend on :$BACKEND_PORT"
(set -a; . "$WORK/backend.env"; set +a; exec "$WORK/bin/birdtest") >"$WORK/backend.log" 2>&1 &
BACKEND_PID=$!
for _ in $(seq 120); do
    curl -fsS "http://127.0.0.1:$BACKEND_PORT/health" >/dev/null 2>&1 && break
    kill -0 "$BACKEND_PID" 2>/dev/null || { tail -n 40 "$WORK/backend.log"; die "the backend exited"; }
    sleep 1
done
curl -fsS "http://127.0.0.1:$BACKEND_PORT/health" >/dev/null || die "the backend did not come up"

PATH="$WORK/shim:$PATH" python3 "$REPO_ROOT/scripts/e2e_magpie.py" \
    --api "http://127.0.0.1:$BACKEND_PORT" \
    --magpie "$MAGPIE_ROOT/bin/magpie" --magpie-root "$MAGPIE_ROOT" \
    --threads "$THREADS" --workdir "$WORK/worker" \
    --github-fixture-port "$GITHUB_PORT" \
    "$@"
