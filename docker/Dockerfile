# =========================
# Base image (shared)
# =========================
FROM rust:1.92.0 AS base

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    pkg-config \
    libssl-dev \
    tzdata \
    redis-server \
    ca-certificates \
    curl \
    postgresql-client  \
    libpq5  \
    mariadb-client \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# =========================
# Development image
# =========================
FROM base AS dev

RUN cargo install cargo-watch

# Pre-cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build
RUN rm -rf src

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

CMD ["/entrypoint.sh"]

# =========================
# Builder (production)
# =========================
FROM base AS builder

COPY . .
RUN cargo build --release

RUN echo "APP_VERSION=$(cargo pkgid | awk -F# '{print $2}')" > /app/version.env

# =========================
# Runtime (production)
# =========================
#FROM debian:bookworm-slim AS prod
FROM ubuntu:24.04 AS prod


RUN apt-get update && apt-get install -y \
    redis-server \
    ca-certificates \
    tzdata \
    postgresql-client  \
    libpq5  \
    mariadb-client \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/app /usr/local/bin/app
COPY --from=builder /app/version.env /app/version.env
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENV APP_ENV=production

CMD ["/entrypoint.sh"]
