#!/bin/sh
set -e

echo "     ____             __        __                       ___                    __  "
echo "    / __ \\____  _____/ /_____ _/ /_  ____ _________     /   | ____ ____  ____  / /_ "
echo "   / /_/ / __ \\/ ___/ __/ __  / __ \\/ __  / ___/ _ \\   / /| |/ __  / _ \\/ __ \\/ __/ "
echo "  / ____/ /_/ / /  / /_/ /_/ / /_/ / /_/ (__  )  __/  / ___ / /_/ /  __/ / / / /_   "
echo " /_/    \\____/_/   \\__/\\__,_/_.___/\\__,_/____/\\___/  /_/  |_|\\__, /\\___/_/ /_/\\__/   "
echo "                                                           /____/                   "


if [ "$APP_ENV" = "production" ]; then
    if [ -f /app/version.env ]; then
        . /app/version.env
        PROJECT_NAME_VERSION=${APP_VERSION:-production}
    else
        PROJECT_NAME_VERSION="development"
    fi
else
    PKG_ID=$(cargo pkgid 2>/dev/null || echo "unknown#0.0.0")
    PROJECT_NAME_VERSION=${PKG_ID##*#}
fi

echo "[INFO] Project: ${PROJECT_NAME_VERSION}"


if [ -n "$TZ" ]; then
    if [ -f "/usr/share/zoneinfo/$TZ" ]; then
        ln -sf /usr/share/zoneinfo/$TZ /etc/localtime
        echo "$TZ" > /etc/timezone
        echo "[INFO] Timezone set to $TZ"
    else
        echo "[WARN] Timezone '$TZ' not found. Using default."
    fi
fi

echo "[entrypoint] APP_ENV=$APP_ENV"
echo "[entrypoint] Starting Redis..."
redis-server --port 84569 --daemonize yes

echo "[entrypoint] Waiting for Redis to be ready..."
until redis-cli ping >/dev/null 2>&1; do
    echo "[entrypoint] Redis not ready, sleeping 1s..."
    sleep 1
done

echo "[entrypoint] Redis is ready"


if [ "$APP_ENV" = "production" ]; then
    echo "[entrypoint] Production mode"
    exec /usr/local/bin/app
else
    echo "[entrypoint] Development mode (live reload)"
    exec cargo watch -x run
fi

