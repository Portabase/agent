#!/bin/bash
set -euo pipefail

check_docker() {
    if ! docker info > /dev/null 2>&1; then
        echo "Docker is not running. Attempting to start Docker..."
        if [[ "$OSTYPE" == "darwin"* ]]; then
            open -a Docker
            echo "Waiting for Docker to start..."
            until docker info > /dev/null 2>&1; do
                sleep 2
            done
        elif command -v systemctl >/dev/null 2>&1; then
            sudo systemctl start docker
        else
            echo "Cannot start Docker automatically. Please start Docker manually."
            exit 1
        fi
    else
        echo "Docker is running."
    fi
}

check_network() {
    local network_name="portabase_network"
    if ! docker network ls --format '{{.Name}}' | grep -q "^${network_name}$"; then
        echo "Docker network '${network_name}' not found. Creating..."
        docker network create "${network_name}"
    else
        echo "Docker network '${network_name}' already exists."
    fi
}

check_docker
check_network

echo "Starting docker-compose..."
docker compose -f ./docker-compose.yml up
echo "Docker-compose started successfully."