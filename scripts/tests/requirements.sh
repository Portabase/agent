#!/usr/bin/env bash

set -e

echo "Detecting OS..."

OS_TYPE="$(uname -s)"

if [[ "$OS_TYPE" == "Linux" ]]; then
    if command -v apt >/dev/null 2>&1; then
        echo "Linux detected with apt. Installing tools..."
        sudo apt update
        echo "Installing redis-tools"
        sudo apt install -y redis-tools
        echo "Installing postgresql-client"
        sudo apt install postgresql-client
    else
        echo "Unsupported Linux distribution. Only apt-based distros are supported."
        exit 1
    fi
elif [[ "$OS_TYPE" == "Darwin" ]]; then
    if command -v brew >/dev/null 2>&1; then
        echo "macOS detected. Installing redis via Homebrew..."
        echo "Installing redis-tools"
        brew install redis
        echo "Installing postgresql-client"
        brew install postgresql
    else
        echo "Homebrew not found. Please install Homebrew first: https://brew.sh/"
        exit 1
    fi
else
    echo "Unsupported OS: $OS_TYPE"
    exit 1
fi

echo "test tools installation completed."