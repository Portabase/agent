#!/bin/bash
set -euo pipefail

# Container engine + compose command, resolved by detect_engine/detect_compose.
ENGINE=""
COMPOSE=""

# True if the given engine binary exists and its daemon/service answers.
# Bounded with `timeout` (when available) so a wedged/slow daemon fails the
# check quickly instead of hanging the whole script on `<engine> info`.
engine_ready() {
    command -v "$1" >/dev/null 2>&1 || return 1
    if command -v timeout >/dev/null 2>&1; then
        timeout 15 "$1" info >/dev/null 2>&1
    else
        "$1" info >/dev/null 2>&1
    fi
}

# Best-effort start of the Docker daemon (macOS app or systemd service).
try_start_docker() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        open -a Docker >/dev/null 2>&1 || return 1
        echo "Waiting for Docker to start..."
        local count=0
        until docker info >/dev/null 2>&1; do
            sleep 2
            count=$((count + 1))
            [ "$count" -ge 30 ] && return 1
        done
        return 0
    elif command -v systemctl >/dev/null 2>&1; then
        sudo systemctl start docker >/dev/null 2>&1 || return 1
        docker info >/dev/null 2>&1
    else
        return 1
    fi
}

# Pick a container engine, preferring one that is already running.
detect_engine() {
    if engine_ready docker; then
        ENGINE="docker"
    elif engine_ready podman; then
        ENGINE="podman"
    elif command -v docker >/dev/null 2>&1; then
        echo "Docker is installed but not running. Attempting to start it..."
        if try_start_docker; then
            ENGINE="docker"
        elif command -v podman >/dev/null 2>&1; then
            echo "Could not start Docker; falling back to Podman."
            ENGINE="podman"
        fi
    elif command -v podman >/dev/null 2>&1; then
        # Podman CLI works without a running daemon for most commands.
        ENGINE="podman"
    fi

    if [ -z "$ENGINE" ]; then
        echo "No working container engine found (need docker or podman)." >&2
        exit 1
    fi
    echo "Using container engine: ${ENGINE}"
}

# Resolve a compose implementation compatible with the chosen engine.
detect_compose() {
    if $ENGINE compose version >/dev/null 2>&1; then
        COMPOSE="$ENGINE compose"
    elif command -v docker-compose >/dev/null 2>&1; then
        COMPOSE="docker-compose"
    elif command -v podman-compose >/dev/null 2>&1; then
        COMPOSE="podman-compose"
    else
        echo "No compose command found (${ENGINE} compose / docker-compose / podman-compose)." >&2
        exit 1
    fi
    echo "Using compose command: ${COMPOSE}"
}

# Ensure the external network referenced by the compose files exists.
# `inspect` is a more reliable existence test than parsing `ls`, and the create
# is made idempotent so a concurrent/pre-existing network doesn't abort the run
# (podman's `network create` errors on an existing name; docker's does not).
check_network() {
    local network_name="portabase_network"
    if $ENGINE network inspect "${network_name}" >/dev/null 2>&1; then
        echo "Network '${network_name}' already exists."
        return 0
    fi
    echo "Network '${network_name}' not found. Creating..."
    $ENGINE network create "${network_name}" >/dev/null 2>&1 \
        || echo "Network '${network_name}' already exists (created concurrently)."
}

# Ensure the external volume declared in docker-compose.yml exists.
check_volume() {
    local volume_name="databases_sqlite-data"
    if $ENGINE volume inspect "${volume_name}" >/dev/null 2>&1; then
        echo "Volume '${volume_name}' already exists."
        return 0
    fi
    echo "Volume '${volume_name}' not found. Creating..."
    $ENGINE volume create "${volume_name}" >/dev/null 2>&1 \
        || echo "Volume '${volume_name}' already exists (created concurrently)."
}

# Resolve the Podman socket and expose it to compose via DOCKER_SOCK.
# The compose files bind-mount ${DOCKER_SOCK:-/var/run/docker.sock} into the
# agent so it can drive containers (testcontainers). Under rootless Podman the
# real socket lives under $XDG_RUNTIME_DIR, not /var/run/docker.sock, so we point
# the mount at it here. Without a socket, backup/restore can't connect.
check_podman_socket() {
    [ "$ENGINE" = "podman" ] || return 0

    local rootless_sock="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/podman/podman.sock"
    local sock=""
    if [ -S "$rootless_sock" ]; then
        sock="$rootless_sock"
    elif [ -S /var/run/docker.sock ]; then
        sock="/var/run/docker.sock"
    fi

    if [ -n "$sock" ]; then
        export DOCKER_SOCK="$sock"
        echo "Using Podman socket: ${sock}"
        return 0
    fi

    echo "[WARN] Running in Podman mode but no Podman socket was found." >&2
    echo "[WARN]   Checked: ${rootless_sock} and /var/run/docker.sock" >&2
    echo "[WARN]   The agent mounts this socket to manage containers; without it," >&2
    echo "[WARN]   backup/restore operations will fail to connect." >&2
    echo "[WARN]   Enable it with: systemctl --user enable --now podman.socket" >&2
    echo "[WARN]   (then re-run 'just up'). Continuing anyway..." >&2
}

detect_engine
detect_compose
check_network
check_volume
check_podman_socket

echo "Stopping old database containers..."
$COMPOSE -f ./docker-compose.databases.yml down

echo "Starting database containers..."
$COMPOSE -f ./docker-compose.databases.yml up -d

echo "Starting main services..."
$COMPOSE -f ./docker-compose.yml up
