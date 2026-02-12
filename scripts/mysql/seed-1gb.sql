CREATE DATABASE IF NOT EXISTS mariadb;
USE mariadb;

DROP TABLE IF EXISTS users;
DROP TABLE IF EXISTS products;

CREATE TABLE users (
                       id BIGINT AUTO_INCREMENT PRIMARY KEY,
                       username VARCHAR(50) NOT NULL UNIQUE,
                       email VARCHAR(100) NOT NULL UNIQUE,
                       password VARCHAR(255) NOT NULL,
                       created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB;

CREATE TABLE products (
                          id BIGINT AUTO_INCREMENT PRIMARY KEY,
                          name VARCHAR(100) NOT NULL,
                          description MEDIUMTEXT,
                          price DECIMAL(10,2) NOT NULL,
                          created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB;


DELIMITER $$

DROP PROCEDURE IF EXISTS seed_data$$
CREATE PROCEDURE seed_data()
BEGIN
    DECLARE i BIGINT DEFAULT 1;
    DECLARE j BIGINT;
    DECLARE large_text TEXT;

    SET large_text = REPEAT('Lorem ipsum dolor sit amet, consectetur adipiscing elit. ', 300);

    WHILE i <= 200000 DO
        INSERT INTO users (username, email, password)
        VALUES (CONCAT('user', i), CONCAT('user', i, '@example.com'), 'changeme');
        SET i = i + 1;
END WHILE;

    SET i = 1;
    WHILE i <= 200000 DO
        INSERT INTO products (name, description, price)
        VALUES (CONCAT('Product ', i), large_text, ROUND(RAND()*1000,2));
        SET i = i + 1;
END WHILE;
END$$

DELIMITER ;

CALL seed_data();

DROP PROCEDURE IF EXISTS seed_data;
