ARG DEBIAN_VERSION=trixie

# Stage 1: Build
FROM rust:slim-${DEBIAN_VERSION} AS builder
WORKDIR /app

# Dependencies
RUN rm -f /etc/apt/apt.conf.d/docker-clean; echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-cache
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y \
    pkg-config libssl-dev build-essential perl cmake clang

COPY . .

# Build vecdb-cli (the main binary)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin vecdb

# Stage 2: Runtime
FROM debian:${DEBIAN_VERSION}-slim
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y ca-certificates libssl-dev

WORKDIR /vecdb
# Create standard directories
RUN mkdir -p /vecdb/data /vecdb/config

# Copy binary
COPY --from=builder /app/target/release/vecdb /usr/local/bin/vecdb

# Environment
ENV VECDB_CONFIG="/vecdb/config/config.toml"
ENV VECDB_DATA="/vecdb/data"
ENV XDG_DATA_HOME="/vecdb/data"
ENV XDG_CONFIG_HOME="/vecdb/config"

# Volumes
VOLUME ["/vecdb/data", "/vecdb/config"]

ENTRYPOINT ["vecdb"]
CMD ["start"]