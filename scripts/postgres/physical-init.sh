#!/bin/sh
# Runs once, on the first start of db-postgres-physical (docker-entrypoint-initdb.d).
# Physical backups need a role with REPLICATION (not superuser) and a pg_hba.conf
# line for replication connections: the image's "host all all all" does not match them.
set -eu

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<'SQL'
CREATE ROLE portabase_backup WITH LOGIN REPLICATION PASSWORD 'changeme-physical';
SQL

echo "host replication portabase_backup all scram-sha-256" >> "$PGDATA/pg_hba.conf"
