-- Drop tables if they exist
DROP TABLE IF EXISTS posts;
DROP TABLE IF EXISTS users;

-- Create users table
CREATE TABLE users (
                       id SERIAL PRIMARY KEY,
                       username VARCHAR(50) UNIQUE NOT NULL,
                       email VARCHAR(100) UNIQUE NOT NULL,
                       password_hash VARCHAR(255) NOT NULL,
                       created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Create posts table
CREATE TABLE posts (
                       id SERIAL PRIMARY KEY,
                       user_id INT REFERENCES users(id) ON DELETE CASCADE,
                       title VARCHAR(255) NOT NULL,
                       content TEXT NOT NULL,
                       created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Insert sample users
INSERT INTO users (username, email, password_hash) VALUES
                                                       ('alice', 'alice@example.com', 'hashedpassword1'),
                                                       ('bob', 'bob@example.com', 'hashedpassword2');

-- Insert sample posts
INSERT INTO posts (user_id, title, content) VALUES
                                                (1, 'Hello World', 'This is the first post by Alice.'),
                                                (2, 'Greetings', 'This is a post by Bob.'),
                                                (1, 'Another Post', 'Alice writes again.');
