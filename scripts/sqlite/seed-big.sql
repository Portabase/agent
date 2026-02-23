PRAGMA foreign_keys = ON;
BEGIN TRANSACTION;

-- Drop tables
DROP TABLE IF EXISTS users;
DROP TABLE IF EXISTS projects;
DROP TABLE IF EXISTS tasks;

-- Users
CREATE TABLE users (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       email TEXT NOT NULL UNIQUE,
                       full_name TEXT NOT NULL,
                       role TEXT NOT NULL CHECK (role IN ('admin','manager','user')),
                       created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Projects
CREATE TABLE projects (
                          id INTEGER PRIMARY KEY AUTOINCREMENT,
                          name TEXT NOT NULL,
                          description TEXT,
                          owner_id INTEGER NOT NULL,
                          created_at TEXT NOT NULL DEFAULT (datetime('now')),
                          FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Tasks
CREATE TABLE tasks (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       project_id INTEGER NOT NULL,
                       title TEXT NOT NULL,
                       status TEXT NOT NULL CHECK (status IN ('todo','in_progress','done')),
                       priority INTEGER NOT NULL DEFAULT 3,
                       due_date TEXT,
                       created_at TEXT NOT NULL DEFAULT (datetime('now')),
                       FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

-- Seed minimal users
INSERT INTO users (email, full_name, role) VALUES
                                               ('admin@example.com', 'System Admin', 'admin'),
                                               ('manager@example.com', 'Project Manager', 'manager'),
                                               ('user@example.com', 'Standard User', 'user');

-- Generate 10_000 projects
WITH RECURSIVE numbers(x) AS (
    SELECT 1
    UNION ALL
    SELECT x+1 FROM numbers WHERE x<10000
)
INSERT INTO projects (name, description, owner_id)
SELECT
    'Project #' || x,
    'Auto-generated project description for project #' || x,
    (1 + (x % 3)) -- cycle users 1..3
FROM numbers;

-- Generate 100_000 tasks
WITH RECURSIVE numbers(x) AS (
    SELECT 1
    UNION ALL
    SELECT x+1 FROM numbers WHERE x<100000
)
INSERT INTO tasks (project_id, title, status, priority, due_date)
SELECT
    (1 + (x % 10000)), -- project id cycle
    'Task #' || x,
    CASE (x % 3) WHEN 0 THEN 'todo' WHEN 1 THEN 'in_progress' ELSE 'done' END,
    1 + (x % 5),
    date('now', '+' || (x % 30) || ' days')
FROM numbers;

COMMIT;