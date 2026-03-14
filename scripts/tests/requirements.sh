#!/usr/bin/env bash
set -e

POSTGRES_BASE="/usr/local/postgresql"
echo "Detecting OS and architecture..."
OS_TYPE="$(uname -s)"
ARCH="$(uname -m)"

install_pg_binaries() {
    echo "Installing PostgreSQL binaries for versions 12-18..."

    for v in 12 13 14 15 16 17 18; do
        TARGET_DIR="$POSTGRES_BASE/$v/bin"
        sudo mkdir -p "$TARGET_DIR"

        if [[ "$OS_TYPE" == "Linux" ]]; then
            if [[ "$ARCH" == "x86_64" ]]; then
                SRC_DIR="../../assets/tools/amd64/postgresql/postgresql-$v/bin"
            elif [[ "$ARCH" == "aarch64" ]]; then
                SRC_DIR="../../assets/tools/arm64/postgresql/postgresql-$v/bin"
            else
                echo "Unsupported architecture: $ARCH"
                continue
            fi

            if [[ -d "$SRC_DIR" ]]; then
                echo "Copying PostgreSQL $v binaries from $SRC_DIR to $TARGET_DIR"
                sudo cp -r "$SRC_DIR"/* "$TARGET_DIR/"
            else
                echo "Binaries for PostgreSQL $v not found for Linux, skipping..."
                continue
            fi

        elif [[ "$OS_TYPE" == "Darwin" ]]; then
            PG_SRC="$(brew --prefix postgresql@$v)/bin" 2>/dev/null || true

            if [[ ! -d "$PG_SRC" ]]; then
                echo "PostgreSQL $v not installed via Homebrew. Trying to install..."
                if ! brew install postgresql@$v; then
                    echo "PostgreSQL $v not available, skipping..."
                    continue
                fi
                PG_SRC="$(brew --prefix postgresql@$v)/bin"
            fi

            echo "Copying PostgreSQL $v binaries from $PG_SRC to $TARGET_DIR"
            sudo cp -r "$PG_SRC"/* "$TARGET_DIR/"
        fi

        sudo chown -R "$(whoami)" "$TARGET_DIR"
        chmod +x "$TARGET_DIR"/*
    done

    echo "PostgreSQL binaries installed under $POSTGRES_BASE"
}

if [[ "$OS_TYPE" == "Linux" ]]; then
    if command -v apt >/dev/null 2>&1; then
        echo "Linux detected with apt. Installing prerequisites..."
        sudo apt update
        sudo apt install -y wget gnupg lsb-release redis-tools
        install_pg_binaries
    else
        echo "Unsupported Linux distribution. Only apt-based distros are supported."
        exit 1
    fi

elif [[ "$OS_TYPE" == "Darwin" ]]; then
    if command -v brew >/dev/null 2>&1; then
        echo "macOS detected. Installing prerequisites..."
        brew install redis

        sudo mkdir -p "$POSTGRES_BASE"
        sudo chown -R "$(whoami)" "$POSTGRES_BASE"

        install_pg_binaries
    else
        echo "Homebrew not found. Please install Homebrew first: https://brew.sh/"
        exit 1
    fi

else
    echo "Unsupported OS: $OS_TYPE"
    exit 1
fi

echo "Tools installation completed successfully."