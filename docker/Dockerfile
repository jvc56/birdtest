# syntax=docker/dockerfile:1.7
#
# Backend and fake-worker. Neither needs MAGPIE: the backend builds its own
# leave-generation KLV artifacts directly (see backend/src/jobs/klv.rs)
# instead of shelling out to `magpie convert csv2klv`, and the fake worker
# submits synthetic results. The real contributor client is MAGPIE itself,
# via `magpie contribute` — see MAGPIE-CLIENT.md. Nothing in this repo builds
# or ships a MAGPIE binary.
#
# Build with `--target backend` or `--target fake-worker`.

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

USER birdtest
ENV DATA_PATH=/app/data BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

# Migrations run inside the process, before it binds, so there is no separate
# migration step to sequence here.
CMD ["birdtest"]

# ---------------------------------------------------------------------------
# Fake worker
# ---------------------------------------------------------------------------
# Speaks the worker API and submits synthetic results. No MAGPIE, so this image
# is a few hundred KB on top of python-slim and starts instantly.

FROM python:3.11-slim AS fake-worker

RUN useradd --create-home --uid 10001 worker
WORKDIR /app
RUN pip install --no-cache-dir requests
COPY worker/fake_worker.py ./
USER worker
ENTRYPOINT ["python", "fake_worker.py"]
