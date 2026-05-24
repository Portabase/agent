# =========================
# Base image (shared)
# =========================
FROM rust:1.94.0 AS base

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
    sqlite3 \
    redis-tools \
    valkey \
    firebird3.0-utils \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

ENV DOTNET_ROOT=/usr/local/dotnet
RUN curl -sSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh \
    && chmod +x /tmp/dotnet-install.sh \
    && /tmp/dotnet-install.sh --channel 8.0 --install-dir /usr/local/dotnet \
    && rm /tmp/dotnet-install.sh \
    && /usr/local/dotnet/dotnet tool install --global microsoft.sqlpackage

ENV PATH="$PATH:/usr/local/dotnet:/root/.dotnet/tools"

RUN ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/') \
    && curl -sSL "https://github.com/microsoft/go-sqlcmd/releases/latest/download/sqlcmd-linux-${ARCH}.tar.bz2" \
       | tar -xjf - -C /usr/local/bin sqlcmd \
    && chmod +x /usr/local/bin/sqlcmd

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
RUN cargo install grcov --locked

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
    curl \
    mariadb-client \
    sqlite3 \
    redis-tools \
    valkey \
    firebird3.0-utils \
    && rm -rf /var/lib/apt/lists/*

ENV DOTNET_ROOT=/usr/local/dotnet
RUN curl -sSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh \
    && chmod +x /tmp/dotnet-install.sh \
    && /tmp/dotnet-install.sh --channel 8.0 --runtime dotnet --install-dir /usr/local/dotnet \
    && rm /tmp/dotnet-install.sh

WORKDIR /app

COPY --from=builder /app/target/release/app /usr/local/bin/app
COPY --from=builder /app/version.env /app/version.env
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

COPY --from=base /usr/lib/postgresql/ /usr/lib/postgresql/
COPY --from=base /usr/local/mongodb/bin/ /usr/local/mongodb/bin/
COPY --from=base /root/.dotnet/tools/ /root/.dotnet/tools/

ENV PATH="$PATH:/usr/local/dotnet:/root/.dotnet/tools"
ENV APP_ENV=production

CMD ["/entrypoint.sh"]
