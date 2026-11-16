#!/bin/sh
# Keeps writing to db-postgres-physical so that successive physical backups carry real changes.
# Each tick inserts new rows, rewrites a random sample of existing rows and trims the oldest
# inserted rows, so the database changes steadily without growing without bound.
#
# Usage: just load-postgres-physical [interval_seconds] [rows_per_tick]   (Ctrl-C to stop)
set -eu

INTERVAL=${1:-10}
ROWS=${2:-5000}
UPDATES=$((ROWS / 5))
KEEP=$((ROWS * 20))

echo "Writing to db-postgres-physical every ${INTERVAL}s: +${ROWS} rows, ${UPDATES} rewritten, at most ${KEEP} load rows kept (Ctrl-C to stop)"
while :; do
    docker exec -i db-postgres-physical psql -v ON_ERROR_STOP=1 -q -At -U devuser -d devdb_physical <<SQL
INSERT INTO events (kind, payload)
SELECT 'load', (SELECT string_agg(md5(random()::text), '') FROM generate_series(1, 30) WHERE g > 0)
FROM generate_series(1, $ROWS) AS g;
UPDATE events SET payload = md5(random()::text) || substr(payload, 33)
WHERE id IN (SELECT id FROM events TABLESAMPLE SYSTEM (0.2) LIMIT $UPDATES);
DELETE FROM events WHERE id IN (SELECT id FROM events WHERE kind = 'load' ORDER BY id DESC OFFSET $KEEP);
SELECT to_char(now(), 'HH24:MI:SS') || '  rows=' || count(*) || '  database=' || pg_size_pretty(pg_database_size(current_database()))
FROM events;
SQL
    sleep "$INTERVAL"
done
