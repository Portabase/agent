#!/usr/bin/env bash
set -euo pipefail

SEED_VALUE="${SEED:-}"

if [ "$SEED_VALUE" = "big" ]; then
  SQLITE_SEED_FILE="./scripts/sqlite/seed-big.sql"
else
  SQLITE_SEED_FILE="./scripts/sqlite/seed.sql"
fi

docker exec -u 0 db-sqlite sh -c "chmod -R 777 /workspace/data"
docker exec -u 0 db-sqlite sh -c "touch /workspace/data/app.db"
docker exec -i db-sqlite sh -c "sqlite3 /workspace/data/app.db" < "$SQLITE_SEED_FILE"