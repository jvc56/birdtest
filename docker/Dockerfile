# syntax=docker/dockerfile:1.7
#
# Backend and worker in one file, because both consume the same two MAGPIE
# stages and BuildKit then builds each stage once and caches it for both.
#
# The MAGPIE binary and the MAGPIE data are separate stages parameterised by
# separate build args, so bumping one does not rebuild the other:
#
#   docker compose build --build-arg MAGPIE_REF=<sha>            # binary only
#   docker compose build --build-arg MAGPIE_DATA_VERSION=<date>  # data only
#
# Each stage is also independently publishable:
#
#   docker build -f docker/Dockerfile --target magpie-bin  -t ghcr.io/jvc56/magpie-bin:<sha> .
#   docker build -f docker/Dockerfile --target magpie-data -t ghcr.io/jvc56/magpie-data:<ver> .
#
# Those two targets are `FROM scratch` payload images: nothing but the binary
# and the data, so they carry no toolchain and no base OS.
#
# Build with `--target backend` or `--target worker`.

# ---------------------------------------------------------------------------
# MAGPIE executable
# ---------------------------------------------------------------------------

FROM debian:bookworm-slim AS magpie-bin-build
ARG MAGPIE_REF=d513d35020d2e87b83bfa68880f23d7921c660a7
ARG MAGPIE_BOARD_DIM=15
# MAGPIE's release profile hardcodes `-march=native`, which would bake the
# build machine's CPU features into an image other people run and crash with
# SIGILL on older hardware. x86-64-v2 (SSE4.2 + popcnt) is a safe floor.
ARG MAGPIE_MARCH=x86-64-v2

RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential clang git ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
RUN git init -q . \
 && git remote add origin https://github.com/jvc56/MAGPIE.git \
 && git fetch -q --depth 1 origin "${MAGPIE_REF}" \
 && git checkout -q FETCH_HEAD \
 && sed -i "s/-march=native/-march=${MAGPIE_MARCH}/g" Makefile \
 && make magpie BUILD=release BOARD_DIM="${MAGPIE_BOARD_DIM}" -j"$(nproc)" \
 && mkdir -p /out/bin && cp bin/magpie /out/bin/magpie

# The payload alone, so this stage can be published and consumed as an image in
# its own right rather than dragging the toolchain along.
FROM scratch AS magpie-bin
COPY --from=magpie-bin-build /out/bin/magpie /magpie/bin/magpie

# ---------------------------------------------------------------------------
# MAGPIE lexical data
# ---------------------------------------------------------------------------

FROM debian:bookworm-slim AS magpie-data-build
ARG MAGPIE_DATA_VERSION=20251004

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /work
# Mirrors MAGPIE's download_data.sh. Upstream splits the tarball into chunks
# only because GitHub caps raw files at 100MB; a registry has no such limit, so
# this reassembly exists purely to read the published format.
RUN set -eu; \
    base="https://github.com/jvc56/MAGPIE-DATA/raw/main/versioned-tarballs/data-${MAGPIE_DATA_VERSION}.tgz"; \
    if curl -sfI "${base}.aa" >/dev/null 2>&1; then \
        for suffix in a b c d e f g h i j k l m n o p; do \
            curl -sfL "${base}.a${suffix}" -o "chunk.a${suffix}" || break; \
        done; \
        cat chunk.* > data.tgz; \
    else \
        curl -sfL "${base}" -o data.tgz; \
    fi; \
    mkdir -p /out && tar -xzf data.tgz -C /out

# Everything derivable is dropped rather than shipped. Wordmaps are 90% of a
# full MAGPIE data directory and are generated locally by the worker, never
# transmitted; word lists are regenerated from the .kwg in ~0.2s.
#
# Scoped to lexica/ deliberately: board layouts are .txt files too, and MAGPIE
# loads one before it has parsed anything, so deleting those breaks every
# command.
RUN find /out/data/lexica -name '*.wmp' -delete \
 && find /out/data/lexica -name '*.txt' -delete \
 && test -f /out/data/layouts/standard15.txt \
 && du -sh /out/data

FROM scratch AS magpie-data
COPY --from=magpie-data-build /out/data /data

# ---------------------------------------------------------------------------
# Backend
# ---------------------------------------------------------------------------

FROM rust:1-slim-bookworm AS backend-build

# aws-lc-sys (pulled in by rustls via the AWS SDK) compiles C and needs cmake.
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential cmake perl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/migrations ./migrations

# Cache mounts keep the registry and target directory across rebuilds. The
# binary is copied out inside the same RUN because a cache mount is not part of
# the resulting layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release \
 && cp target/release/birdtest /usr/local/bin/birdtest

FROM debian:bookworm-slim AS backend

# ca-certificates for outbound TLS (S3, SES); curl for the compose healthcheck.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 birdtest

WORKDIR /app
COPY --from=backend-build /usr/local/bin/birdtest /usr/local/bin/birdtest
COPY data /app/data

# Leave-generation aggregation shells out to `magpie convert csv2klv` once per
# completed generation. It needs the letter distributions and board layouts, but
# never a wordmap — the server does not play games.
COPY --from=magpie-bin  /magpie/bin/magpie /magpie/bin/magpie
COPY --from=magpie-data /data             /magpie/data

USER birdtest
ENV DATA_PATH=/app/data MAGPIE_DIR=/magpie BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

# Migrations run inside the process, before it binds, so there is no separate
# migration step to sequence here.
CMD ["birdtest"]

# ---------------------------------------------------------------------------
# Worker
# ---------------------------------------------------------------------------

FROM python:3.11-slim AS worker

RUN useradd --create-home --uid 10001 worker

COPY --from=magpie-bin  /magpie/bin/magpie /magpie/bin/magpie
COPY --from=magpie-data /data             /magpie/data

# Wordmaps make game play dramatically faster, so the client always wants them —
# but they are ~10x the size of everything else combined and must never cross a
# network. The worker generates the ones it needs from the shipped .kwg on first
# use and writes them here, which is why this directory is the worker's own and
# is writable.
RUN mkdir -p /magpie/generated/lexica && chown -R worker:worker /magpie/generated
VOLUME /magpie/generated

WORKDIR /app
COPY worker/pyproject.toml worker/worker.py ./
RUN pip install --no-cache-dir .

RUN mkdir -p /home/worker/.birdtest && chown -R worker:worker /home/worker/.birdtest

USER worker
ENV BIRDTEST_MAGPIE_DIR=/magpie
ENTRYPOINT ["python", "worker.py"]
