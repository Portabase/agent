-- ============================================================
-- HARD RESET
-- ============================================================

DROP TABLE IF EXISTS posts;
DROP TABLE IF EXISTS users;

-- ============================================================
-- SCHEMA
-- ============================================================

CREATE TABLE users (
                       id BIGSERIAL PRIMARY KEY,
                       username VARCHAR(50) NOT NULL,
                       email VARCHAR(100) NOT NULL,
                       password_hash VARCHAR(255) NOT NULL,
                       created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE posts (
                       id BIGSERIAL PRIMARY KEY,
                       user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                       title VARCHAR(255) NOT NULL,
                       content TEXT NOT NULL,
                       created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_posts_user_id ON posts(user_id);

-- ============================================================
-- USERS (~1 million rows)
-- ============================================================

INSERT INTO users (username, email, password_hash)
SELECT
    'user_' || gs,
    'user_' || gs || '@example.com',
    md5(random()::text)
FROM generate_series(1, 1000000) AS gs;

-- ============================================================
-- POSTS
-- ============================================================
-- Each post content ≈ 4 KB
-- 300k users × 100 posts = 30 million rows
-- 30M × 4 KB ≈ 120 GB logical
-- After TOAST + compression ≈ 1–2 GB on disk
--
-- Adjust numbers if needed
-- ============================================================

INSERT INTO posts (user_id, title, content)
SELECT
    u.id,
    'Post #' || p.post_no || ' by user ' || u.id,
    repeat(
            'Lorem ipsum dolor sit amet, consectetur adipiscing elit. '
                || 'Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. ',
            40
    )
FROM users u
         JOIN generate_series(1, 30) AS p(post_no)
              ON u.id <= 300000;

-- ============================================================
-- OPTIONAL: FORCE DISK MATERIALIZATION
-- ============================================================

VACUUM ANALYZE;
