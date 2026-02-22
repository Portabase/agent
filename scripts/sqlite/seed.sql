PRAGMA foreign_keys = ON;

BEGIN TRANSACTION;

-- Drop existing tables (idempotent reset)
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

-- Seed users
INSERT INTO users (email, full_name, role) VALUES
                                               ('admin@example.com', 'System Admin', 'admin'),
                                               ('manager@example.com', 'Project Manager', 'manager'),
                                               ('user@example.com', 'Standard User', 'user');

-- Seed projects
INSERT INTO projects (name, description, owner_id) VALUES
                                                       ('Internal Tooling', 'Backoffice automation platform', 2),
                                                       ('Client Portal', 'Customer-facing SaaS interface', 2);

-- Seed tasks
INSERT INTO tasks (project_id, title, status, priority, due_date) VALUES
                                                                      (1, 'Define architecture', 'done', 1, date('now', '+3 days')),
                                                                      (1, 'Implement authentication', 'in_progress', 1, date('now', '+7 days')),
                                                                      (2, 'Design landing page', 'todo', 2, date('now', '+5 days')),
                                                                      (2, 'Setup CI/CD', 'todo', 2, date('now', '+10 days'));

COMMIT;