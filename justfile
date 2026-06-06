set dotenv-load := true
set shell := ["bash", "-cu"]

CLUSTER_SCRIPT := "docker/entrypoints/app-dev-entrypoint.sh"

up:
    bash {{CLUSTER_SCRIPT}}

seed-mongo:
    echo "Seeding MongoDB..."
    bash ./scripts/mongo/seed-mongo.sh

seed-mongo-auth:
    echo "Seeding MongoDB with auth..."
    bash ./scripts/mongo/seed-mongo.sh auth

seed-mysql:
    echo "Seeding MySQL..."
    mysql -h 127.0.0.1 -P "$MYSQL_PORT" -u "$MYSQL_USER" -p"$MYSQL_PASSWORD" "$MYSQL_DB" < ./scripts/mysql/seed-mysql.sql

seed-mysql-1gb:
    echo "Seeding MySQL (1GB)..."
    mysql -h 127.0.0.1 -P "$MYSQL_PORT" -u "$MYSQL_USER" -p"$MYSQL_PASSWORD" "$MYSQL_DB" < ./scripts/mysql/seed-1gb.sql

seed-postgres:
    echo "Seeding Postgres..."
    docker exec -i -e PGPASSWORD="$PG_PASSWORD" "$PG_CONTAINER" \
        psql -U "$PG_USER" -d "$PG_DB" < ./scripts/postgres/seed.sql

seed-postgres-1gb:
    echo "Seeding Postgres (1GB)..."
    docker exec -i -e PGPASSWORD="$PG_PASSWORD" "$PG_CONTAINER" \
        psql -U "$PG_USER" -d "$PG_DB" < ./scripts/postgres/seed-1gb.sql

seed-sqlite:
    echo "Seeding SQLite..."
    bash ./scripts/sqlite/seed.sh
    echo "Done"

seed-firebird:
    echo "Seeding Firebird..."
    docker exec -i db-firebird isql -user alice -password fake_password /var/lib/firebird/data/mirror.fdb < ./scripts/firebird/seed.sql

    echo "Verifying Firebird tables..."
    echo "SELECT RDB\$RELATION_NAME FROM RDB\$RELATIONS WHERE RDB\$SYSTEM_FLAG = 0 AND RDB\$VIEW_BLR IS NULL;" \
    | docker exec -i db-firebird isql -user alice -password fake_password /var/lib/firebird/data/mirror.fdb

seed-mssql:
    echo "Seeding MSSQL..."
    docker exec -i rust-dev sqlcmd -S "db-mssql,1433" -U sa -P "$MSSQL_SA_PASSWORD" -N disable -i /app/scripts/mssql/seed.sql
    echo "Done"

seed-all:
    just seed-mongo
    just seed-mysql
    just seed-postgres
    just seed-postgres-1gb
    just seed-sqlite
    just seed-mongo
    just seed-firebird
    just seed-mssql

test:
    echo "Build test image (with grcov included)"
    docker compose -f docker-compose.test.yml build agent-test
    echo "Start agent-test container"
    docker compose -f docker-compose.test.yml up -d agent-test
    echo "Run tests inside container"
    docker compose -f docker-compose.test.yml exec -e CARGO_INCREMENTAL -e RUSTFLAGS -e LLVM_PROFILE_FILE agent-test bash -c "cargo test --verbose && sync"