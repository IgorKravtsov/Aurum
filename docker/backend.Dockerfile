# Aurum backend — the Rust server (crates/aurum-server).
#
# Build context is the REPOSITORY ROOT, not backend/: the server is a member
# of the Cargo workspace and needs Cargo.toml/Cargo.lock and the sibling
# crates (aurum-domain, ...). docker-compose.yml sets this up:
#   build: { context: ., dockerfile: docker/backend.Dockerfile }
#
# Stages: chef-plan (dependency recipe) -> builder (cached deps layer, then
# the real build) -> runtime (slim Debian with just the binary).

# Base images are pinned to exact tags so a rebuild is reproducible: cargo-chef
# 0.1.78 on Rust 1.99.1 (workspace MSRV is 1.88) and a dated Debian snapshot.
# Bump them deliberately, together with a check that the stack still builds.
ARG RUST_IMAGE=lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-20261104-slim

# ---- 1. recipe: the dependency graph without any of our own code ----------
FROM ${RUST_IMAGE} AS planner
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

# ---- 2. build -------------------------------------------------------------
FROM ${RUST_IMAGE} AS builder
WORKDIR /src

# Dependencies only: this layer is rebuilt when Cargo.toml/Cargo.lock change,
# not on every source edit, which is what makes repeat builds fast.
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p aurum-server --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# --locked: fail instead of silently updating Cargo.lock, so the image is
# built from exactly the dependency versions that were reviewed and tested.
RUN cargo build --release --locked -p aurum-server \
    && strip target/release/aurum-server

# ---- 3. runtime -----------------------------------------------------------
FROM ${RUNTIME_IMAGE}

# ca-certificates: the server calls NBP and CoinGecko over HTTPS (rustls with
# the system trust store). curl: the HEALTHCHECK below, same as the Python
# image had.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# Drop root. The server never writes to disk (no uploads, no local cache), so
# the binary only needs to be readable and executable — it stays root-owned.
# Anything that manages to execute code in this container lands as an
# unprivileged user instead of root. Compatible with `no-new-privileges`:
# nothing here relies on setuid or capabilities.
RUN useradd --create-home --uid 10001 aurum

COPY --from=builder /src/target/release/aurum-server /usr/local/bin/aurum-server

# The server reads ./.env from its working directory if present; in this
# image configuration comes from AURUM_* environment variables instead.
WORKDIR /app
USER aurum

EXPOSE 8000

HEALTHCHECK --interval=10s --timeout=5s --start-period=10s --retries=5 \
    CMD curl -f http://localhost:8000/api/health || exit 1

# `serve` runs migrations and seeding first (what the Python image did with
# `alembic upgrade head && uvicorn ...`), then listens on AURUM_HOST:AURUM_PORT
# (default 0.0.0.0:8000).
CMD ["aurum-server", "serve"]
