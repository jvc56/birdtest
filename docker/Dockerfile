# syntax=docker/dockerfile:1.7
#
# Backend and fake-worker.
#
# The backend image carries a pinned MAGPIE. The server builds the reference
# copy of every wordmap, rack info table and leave-generation KLV with it, and
# publishes the hash for workers to reproduce — see MAGPIE_DEPENDENCY.md. It is
# built here from a pinned commit rather than downloaded, so the image records
# exactly which MAGPIE produced the hashes it hands out.
#
# It is built `portable_release`, not the usual `-march=native`: a binary built
# on a CI runner for that runner's CPU is not the binary a Fargate task or a
# contributor's laptop runs, and the whole point is that two builds of one
# commit agree byte for byte. See MAGPIE's Makefile.
#
# The fake worker still needs no MAGPIE: it submits synthetic results. The real
# contributor client is MAGPIE itself, via `magpie contribute` — see PLAN.md's
# "Worker Client".
#
# Build with `--target backend`, `--target derived-builder` or
# `--target fake-worker`.

# ---------------------------------------------------------------------------
# MAGPIE
# ---------------------------------------------------------------------------

FROM debian:bookworm-slim AS magpie-build

# The commit this image's hashes are produced by. Passed as a build argument
# and recorded on the image, so "which MAGPIE built this wordmap" is a `docker
# inspect` rather than a guess. CI's magpie-contract and nightly jobs test the
# head of `birdtest-contribute` rather than this commit, on purpose: they are
# what notices a MAGPIE-side change before the pin is moved to it. Moving the
# pin is a deliberate step, and MIN_MAGPIE_VERSION must never exceed the
# version this commit reports.
ARG MAGPIE_COMMIT=47b57aadffbb1e720e1251bb81b4a37ad31b87cd
ARG MAGPIE_REPO=https://github.com/jvc56/MAGPIE.git

RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential git ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
# A shallow fetch of the one commit, rather than a clone of the history: the
# repository is large and none of it is wanted except this tree.
RUN git init -q . \
 && git remote add origin "$MAGPIE_REPO" \
 && git fetch -q --depth 1 origin "$MAGPIE_COMMIT" \
 && git checkout -q FETCH_HEAD

# MAGPIE loads a default board layout before it parses any argument, and its
# `data/` is gitignored, so without this even `magpie builders` fails. The
# server writes the same file into every scratch directory for the same reason
# (backend/src/magpie.rs); this is a copy of it, here only so the smoke check
# below can run.
COPY backend/src/magpie_standard15.txt /src/data/layouts/standard15.txt

# portable_release: -O3 -flto with a fixed instruction-set target. Not
# `make release`, which is the PGO build and needs clang plus a training run,
# and not no_pgo_release, which is -march=native.
#
# The smoke check greps rather than just running: MAGPIE reports a failed
# command on stderr and **exits 0**, so `&& ./bin/magpie builders` alone passes
# whatever happens. The server parses this exact output to record which builder
# produced every hash it publishes, so an image whose MAGPIE cannot print it is
# an image that cannot dispatch a job needing a wordmap.
RUN make magpie BUILD=portable_release -j"$(nproc)" \
 && ./bin/magpie builders | tee /dev/stderr | grep -q '"wmp_builder_version"'

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

# Unset, cargo uses every core. `--build-arg CARGO_BUILD_JOBS=2` caps it on a
# machine that has other work to do: a release build of the AWS SDK at full
# parallelism takes several GB.
ARG CARGO_BUILD_JOBS

# Cache mounts keep the registry and target directory across rebuilds. The
# binary is copied out inside the same RUN because a cache mount is not part of
# the resulting layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release \
 && cp target/release/birdtest /usr/local/bin/birdtest \
 && cp target/release/build-derived /usr/local/bin/build-derived

FROM debian:bookworm-slim AS backend

# ca-certificates for outbound TLS (S3, SES); curl for the compose healthcheck.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 birdtest

WORKDIR /app
COPY --from=backend-build /usr/local/bin/birdtest /usr/local/bin/birdtest
COPY --from=magpie-build /src/bin/magpie /usr/local/bin/magpie
ARG MAGPIE_COMMIT=47b57aadffbb1e720e1251bb81b4a37ad31b87cd
LABEL org.birdtest.magpie-commit="$MAGPIE_COMMIT"
# Still no data directory. MAGPIE is run against a throwaway directory built
# per conversion from the bytes the job pins — the object store for lexica and
# leaves, `input_data.content` for distributions — so nothing server-side reads
# a data file off its own disk. See backend/src/magpie.rs.

USER birdtest
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

# Migrations run inside the process, before it binds, so there is no separate
# migration step to sequence here.
CMD ["birdtest"]

# ---------------------------------------------------------------------------
# Derived-file builder
# ---------------------------------------------------------------------------
# The same image with a different entrypoint, run as a scheduled task. It
# drains the `derived_data` queue and exits: a rack info table build peaks at
# about 2.4 GB and writes a 1.9 GB file, neither of which fits the web task's
# 1 vCPU and 2 GB. Sharing the image is what keeps the builder's MAGPIE and the
# server's the same binary — two images would be two builders, and a hash
# recorded under the wrong one is precisely what this design exists to prevent.
#
# It does not migrate: the web task does that before it binds, and two
# processes racing to migrate is worse than a builder waiting a deployment.

FROM backend AS derived-builder
COPY --from=backend-build /usr/local/bin/build-derived /usr/local/bin/build-derived
# Ephemeral storage for the scratch directories; the task definition mounts it.
ENV MAGPIE_SCRATCH_DIR=/scratch
USER root
RUN mkdir -p /scratch && chown birdtest:birdtest /scratch
USER birdtest
CMD ["build-derived"]

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
