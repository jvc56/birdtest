# Build context is the repository root: the backend ships `data/` alongside the
# binary because rack and leave enumeration read the letter distributions at
# job-creation time.

FROM rust:1-slim-bookworm AS builder

# aws-lc-sys (pulled in by rustls via the AWS SDK) compiles C and needs cmake.
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential cmake perl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/migrations ./migrations

# Cache mounts keep the registry and target directory across rebuilds, which is
# the difference between a ~10s incremental build and a multi-minute cold one.
# The binary has to be copied out inside the same RUN — a cache mount is not
# part of the resulting layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release \
 && cp target/release/birdtest /usr/local/bin/birdtest

FROM debian:bookworm-slim

# ca-certificates for outbound TLS (S3, SES); curl for the compose healthcheck.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 birdtest

WORKDIR /app
COPY --from=builder /usr/local/bin/birdtest /usr/local/bin/birdtest
COPY data /app/data

USER birdtest
ENV DATA_PATH=/app/data BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

# Migrations run inside the process, before it binds, so there is no separate
# migration step to sequence here.
CMD ["birdtest"]
