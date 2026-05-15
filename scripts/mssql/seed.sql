IF NOT EXISTS (SELECT name FROM sys.databases WHERE name = N'myappdb')
    CREATE DATABASE [myappdb];
GO

USE [myappdb];
GO

IF OBJECT_ID('users', 'U') IS NULL
    CREATE TABLE users (
        id       INT IDENTITY(1,1) PRIMARY KEY,
        email    NVARCHAR(255) NOT NULL UNIQUE,
        name     NVARCHAR(255),
        created_at DATETIME DEFAULT GETDATE()
    );
GO

INSERT INTO users (email, name) VALUES ('alice@example.com', 'Alice');
INSERT INTO users (email, name) VALUES ('bob@example.com',   'Bob');
INSERT INTO users (email, name) VALUES ('charlie@example.com', 'Charlie');
GO
