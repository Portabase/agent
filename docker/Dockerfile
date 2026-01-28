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
    libpq5 \
    libreadline8 \
    libncurses6 \
    zlib1g \
    curl \
    mariadb-client \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

ARG TARGETARCH

# =========================
# PostgreSQL client binaries (versions 12-18)
# =========================

RUN for v in 12 13 14 15 16 17 18; do \
        mkdir -p /usr/lib/postgresql/$v/bin; \
    done

COPY assets/tools/amd64/postgresql/ /tmp/pg-x64/
COPY assets/tools/arm64/postgresql/ /tmp/pg-arm/

RUN if [ "$TARGETARCH" = "amd64" ]; then \
        for v in 12 13 14 15 16 17 18; do \
            cp -r /tmp/pg-x64/postgresql-$v/bin/* /usr/lib/postgresql/$v/bin/; \
        done; \
    elif [ "$TARGETARCH" = "arm64" ]; then \
        for v in 12 13 14 15 16 17 18; do \
            cp -r /tmp/pg-arm/postgresql-$v/bin/* /usr/lib/postgresql/$v/bin/; \
        done; \
    fi && \
    rm -rf /tmp/pg-x64 /tmp/pg-arm && \
    chmod +x /usr/lib/postgresql/*/bin/*

# =========================
# MongoDB client binaries
# =========================
COPY assets/tools/${TARGETARCH}/mongodb/ /usr/local/mongodb/
RUN chmod +x /usr/local/mongodb/bin/*

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
FROM ubuntu:24.04 AS prod

RUN apt-get update && apt-get install -y \
    redis-server \
    ca-certificates \
    tzdata \
    libpq5 \
    libreadline8 \
    libncurses6 \
    zlib1g \
    mariadb-client \
    && rm -rf /var/lib/apt/lists/*


WORKDIR /app

COPY --from=builder /app/target/release/app /usr/local/bin/app
COPY --from=builder /app/version.env /app/version.env
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

COPY --from=base /usr/lib/postgresql/ /usr/lib/postgresql/
COPY --from=base /usr/local/mongodb/bin/ /usr/local/mongodb/bin/


ENV APP_ENV=production

CMD ["/entrypoint.sh"]
