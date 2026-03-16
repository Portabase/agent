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

REDIS_PORT=65515
echo "[entrypoint] APP_ENV=$APP_ENV"
echo "[entrypoint] Starting Redis..."
redis-server --port $REDIS_PORT --daemonize yes

echo "[entrypoint] Waiting for Redis to be ready..."
MAX_RETRIES=20
COUNT=0
until redis-cli -h localhost -p "$REDIS_PORT" ping >/dev/null 2>&1 ; do
    COUNT=$((COUNT+1))
    if [ $COUNT -ge $MAX_RETRIES ]; then
        echo "[ERROR] Redis did not start after $MAX_RETRIES attempts"
        exit 1
    fi
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

