-- ~1 GB on disk for db-postgres-physical (just seed-postgres-physical-1gb).
-- Rows stay under the 2 KB TOAST threshold and the payload is random hex,
-- so the size on disk does not shrink through compression.
DROP TABLE IF EXISTS events;

CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO events (kind, payload)
SELECT
    (ARRAY['login', 'order', 'refund', 'audit'])[1 + (gs % 4)],
    -- WHERE gs > 0 correlates the subquery, so it runs once per row
    (SELECT string_agg(md5(random()::text), '') FROM generate_series(1, 30) WHERE gs > 0)
FROM generate_series(1, 1000000) AS gs;

CREATE INDEX idx_events_kind ON events (kind);

VACUUM ANALYZE events;

SELECT pg_size_pretty(pg_database_size(current_database())) AS database_size;
