#!/usr/bin/env bash

set -e

echo "Detecting OS..."

OS_TYPE="$(uname -s)"

if [[ "$OS_TYPE" == "Linux" ]]; then
    if command -v apt >/dev/null 2>&1; then
        echo "Linux detected with apt. Installing tools..."

        echo "Adding PostgreSQL APT repository..."
        sudo apt update
        sudo apt install -y wget gnupg lsb-release
        wget --quiet -O - https://www.postgresql.org/media/keys/ACCC4CF8.asc | sudo apt-key add -
        echo "deb http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" | sudo tee /etc/apt/sources.list.d/pgdg.list

        sudo apt update

        echo "Installing redis-tools..."
        sudo apt install -y redis-tools

        echo "Installing PostgreSQL client v17..."
        sudo apt install -y postgresql-client-17

    else
        echo "Unsupported Linux distribution. Only apt-based distros are supported."
        exit 1
    fi

elif [[ "$OS_TYPE" == "Darwin" ]]; then
    if command -v brew >/dev/null 2>&1; then
        echo "macOS detected. Installing tools via Homebrew..."

        echo "Installing redis..."
        brew install redis

        echo "Installing PostgreSQL client..."
        brew install postgresql@18
        brew link --force postgresql@18

    else
        echo "Homebrew not found. Please install Homebrew first: https://brew.sh/"
        exit 1
    fi

else
    echo "Unsupported OS: $OS_TYPE"
    exit 1
fi

echo "Tools installation completed successfully."