-- MariaDB seed file for database "mariadb"

-- Drop tables if they exist
DROP TABLE IF EXISTS users;
DROP TABLE IF EXISTS products;

-- Create a users table
CREATE TABLE users (
                       id INT AUTO_INCREMENT PRIMARY KEY,
                       username VARCHAR(50) NOT NULL UNIQUE,
                       email VARCHAR(100) NOT NULL UNIQUE,
                       password VARCHAR(255) NOT NULL,
                       created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Insert sample users
INSERT INTO users (username, email, password) VALUES
                                                  ('alice', 'alice@example.com', 'changeme'),
                                                  ('bob', 'bob@example.com', 'changeme');

-- Create a products table
CREATE TABLE products (
                          id INT AUTO_INCREMENT PRIMARY KEY,
                          name VARCHAR(100) NOT NULL,
                          description TEXT,
                          price DECIMAL(10,2) NOT NULL,
                          created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Insert sample products
INSERT INTO products (name, description, price) VALUES
                                                    ('Laptop', 'High performance laptop', 1299.99),
                                                    ('Phone', 'Smartphone with OLED display', 799.99),
                                                    ('Headphones', 'Noise-cancelling headphones', 199.99);

-- Done
