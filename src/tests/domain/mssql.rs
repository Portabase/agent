use crate::domain::factory::DatabaseFactory;
use crate::domain::mssql::connection;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tiberius::{AuthMethod, Client, Config};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers::core::IntoContainerPort;
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::{error, info};

const SA_PASSWORD: &str = "Test!Str0ng1";

async fn start_container() -> ContainerAsync<GenericImage> {
    GenericImage::new("mcr.microsoft.com/azure-sql-edge", "latest")
        .with_exposed_port(1433.tcp())
        .with_env_var("ACCEPT_EULA", "Y")
        .with_env_var("MSSQL_SA_PASSWORD", SA_PASSWORD)
        .start()
        .await
        .expect("azure-sql-edge container started")
}

async fn create_user_database(host: &str, port: u16, db_name: &str) {
    let mut config = Config::new();
    config.host(host);
    config.port(port);
    config.authentication(AuthMethod::sql_server("sa", SA_PASSWORD));
    config.trust_cert();

    let tcp = TcpStream::connect(config.get_addr()).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    let mut client = Client::connect(config, tcp.compat_write()).await.unwrap();

    let sql = format!(
        "IF NOT EXISTS (SELECT name FROM sys.databases WHERE name = N'{}') CREATE DATABASE [{}]",
        db_name, db_name
    );
    client.simple_query(sql.as_str()).await.unwrap();
}

fn make_config(host: String, port: u16, database: &str, generated_id: &str) -> DatabaseConfig {
    DatabaseConfig {
        name: "Test MSSQL".to_string(),
        database: database.to_string(),
        db_type: DbType::Mssql,
        username: "sa".to_string(),
        password: SA_PASSWORD.to_string(),
        port,
        host,
        generated_id: generated_id.to_string(),
        path: "".to_string(),
        max_packet_size: "".to_string(),
        volume_name: "".to_string(),
        container_name: None,
        options: std::collections::HashMap::new(),
    }
}

async fn start() -> (ContainerAsync<GenericImage>, String, u16) {
    let container = start_container().await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(1433).await.unwrap();
    (container, host, port)
}

async fn connect_as(host: &str, port: u16, database: &str, user: &str, password: &str) -> Client<Compat<TcpStream>> {
    let mut config = Config::new();
    config.host(host);
    config.port(port);
    config.database(database);
    config.authentication(AuthMethod::sql_server(user, password));
    config.trust_cert();

    let tcp = TcpStream::connect(config.get_addr()).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    Client::connect(config, tcp.compat_write()).await.unwrap()
}

async fn exec(host: &str, port: u16, database: &str, sql: &str) {
    let mut client = connect_as(host, port, database, "sa", SA_PASSWORD).await;
    client.simple_query(sql).await.unwrap().into_results().await.unwrap();
}

async fn scalar(host: &str, port: u16, database: &str, user: &str, password: &str, sql: &str) -> i32 {
    let mut client = connect_as(host, port, database, user, password).await;
    let row = client.simple_query(sql).await.unwrap().into_row().await.unwrap().unwrap();
    row.get::<i32, _>(0).unwrap()
}

fn with_backup_format(mut config: DatabaseConfig, format: &str) -> DatabaseConfig {
    config.options.insert("backup_format".to_string(), serde_json::json!(format));
    config
}

#[tokio::test]
async fn mssql_ping_test() {
    init_tracing_for_test();

    let container = start_container().await;
    tokio::time::sleep(Duration::from_secs(30)).await;

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(1433).await.unwrap();
    let config = make_config(host, port, "master", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d3");

    let db = DatabaseFactory::create_for_backup(config).await;
    let reachable = db.ping().await.unwrap_or(false);

    assert!(reachable, "MSSQL ping should succeed");
}

#[tokio::test]
async fn mssql_backup_test() {
    init_tracing_for_test();

    let container = start_container().await;
    tokio::time::sleep(Duration::from_secs(30)).await;

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(1433).await.unwrap();

    create_user_database(&host, port, "backupdb").await;

    let config = make_config(host, port, "backupdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d4");
    let temp_dir = TempDir::new().unwrap();

    let db = DatabaseFactory::create_for_backup(config).await;
    let file_path = db.backup(temp_dir.path(), std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await.unwrap();

    assert!(file_path.is_file(), "backup file should exist");
    assert!(
        file_path.metadata().unwrap().len() > 0,
        "backup file should be non-empty"
    );
    assert!(
        file_path.extension().and_then(|e| e.to_str()) == Some("dacpac"),
        "backup file should have .dacpac extension"
    );
}

#[tokio::test]
async fn mssql_backup_restore_test() {
    init_tracing_for_test();

    let container = start_container().await;
    tokio::time::sleep(Duration::from_secs(30)).await;

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(1433).await.unwrap();

    create_user_database(&host, port, "sourcedb").await;

    let backup_config = make_config(host.clone(), port, "sourcedb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d5");
    let temp_dir = TempDir::new().unwrap();

    let db = DatabaseFactory::create_for_backup(backup_config).await;
    let file_path = db.backup(temp_dir.path(), std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await.unwrap();
    assert!(file_path.is_file());

    let compression = compress_to_tar_gz_large(&file_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await.unwrap();
    assert!(compression.compressed_path.is_file());

    let files = decompress_large_tar_gz(
        compression.compressed_path.as_path(),
        temp_dir.path(),
    )
    .await
    .unwrap();

    let backup_file: PathBuf = if files.len() == 1 {
        files[0].clone()
    } else {
        panic!("Unexpected number of files after decompression: {}", files.len());
    };

    let restore_config = make_config(host, port, "restoreddb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d5");
    let db_restore = DatabaseFactory::create_for_restore(restore_config, &backup_file).await;

    match db_restore.restore(&backup_file, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await {
        Ok(_) => info!("MSSQL restore succeeded"),
        Err(e) => {
            error!("MSSQL restore failed: {:?}", e);
            panic!("Restore failed: {:?}", e);
        }
    }
}

#[tokio::test]
async fn mssql_backup_bacpac_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "bacpacdb").await;

    let config = with_backup_format(
        make_config(host, port, "bacpacdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d6"),
        "bacpac",
    );
    let temp_dir = TempDir::new().unwrap();
    let file_path = DatabaseFactory::create_for_backup(config)
        .await
        .backup(temp_dir.path(), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    assert!(file_path.metadata().unwrap().len() > 0);
    assert_eq!(file_path.extension().and_then(|e| e.to_str()), Some("bacpac"));
}

#[tokio::test]
async fn mssql_backup_windows_principal_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "windowsdb").await;
    exec(
        &host,
        port,
        "windowsdb",
        "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1); \
         CREATE USER [NT AUTHORITY\\SYSTEM] FOR LOGIN [NT AUTHORITY\\SYSTEM];",
    )
    .await;

    let config = make_config(host, port, "windowsdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d7");
    let temp_dir = TempDir::new().unwrap();

    let bacpac = DatabaseFactory::create_for_backup(with_backup_format(config.clone(), "bacpac"))
        .await
        .backup(temp_dir.path(), Arc::new(JobLogger::new()))
        .await;
    assert!(bacpac.unwrap_err().to_string().contains("SQL71627"));

    let file_path = DatabaseFactory::create_for_backup(config)
        .await
        .backup(temp_dir.path(), Arc::new(JobLogger::new()))
        .await
        .unwrap();
    assert_eq!(file_path.extension().and_then(|e| e.to_str()), Some("dacpac"));
}

async fn restore_over_existing(format: &str, generated_id: &str) {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "appdb").await;
    exec(&host, port, "master", "CREATE LOGIN app_login WITH PASSWORD = 'App!Str0ng1'").await;
    exec(
        &host,
        port,
        "appdb",
        "CREATE TABLE dbo.users (id INT PRIMARY KEY, email NVARCHAR(100) NOT NULL); \
         INSERT dbo.users VALUES (1, 'a@x'), (2, 'b@x'), (3, 'c@x'); \
         CREATE USER app_user FOR LOGIN app_login; \
         GRANT SELECT ON dbo.users TO app_user;",
    )
    .await;

    let config = with_backup_format(make_config(host.clone(), port, "appdb", generated_id), format);
    let temp_dir = TempDir::new().unwrap();
    let file_path = DatabaseFactory::create_for_backup(config.clone())
        .await
        .backup(temp_dir.path(), Arc::new(JobLogger::new()))
        .await
        .unwrap();
    assert_eq!(file_path.extension().and_then(|e| e.to_str()), Some(format));

    exec(
        &host,
        port,
        "appdb",
        "DELETE FROM dbo.users WHERE id = 1; INSERT dbo.users VALUES (4, 'd@x'); CREATE TABLE dbo.extra (id INT);",
    )
    .await;

    DatabaseFactory::create_for_restore(config, &file_path)
        .await
        .restore(&file_path, Arc::new(JobLogger::new()))
        .await
        .unwrap();

    assert_eq!(scalar(&host, port, "appdb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.users").await, 3);
    assert_eq!(
        scalar(&host, port, "appdb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.users WHERE id IN (1, 2, 3)").await,
        3
    );
    assert_eq!(
        scalar(&host, port, "appdb", "sa", SA_PASSWORD, "SELECT CASE WHEN OBJECT_ID('dbo.extra') IS NULL THEN 1 ELSE 0 END").await,
        1
    );
    assert_eq!(
        scalar(&host, port, "appdb", "app_login", "App!Str0ng1", "SELECT COUNT(*) FROM dbo.users").await,
        3
    );
}

#[tokio::test]
async fn mssql_restore_over_existing_dacpac_test() {
    restore_over_existing("dacpac", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d8").await;
}

#[tokio::test]
async fn mssql_restore_over_existing_bacpac_test() {
    restore_over_existing("bacpac", "5a445eb4-c2c6-4bde-a423-ee1385dcf6d9").await;
}

#[tokio::test]
async fn mssql_restore_rejects_unknown_extension_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "keepdb").await;

    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("backup.dump");
    std::fs::write(&file_path, b"not a package").unwrap();

    let config = make_config(host.clone(), port, "keepdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6da");
    let result = DatabaseFactory::create_for_restore(config, &file_path)
        .await
        .restore(&file_path, Arc::new(JobLogger::new()))
        .await;

    assert!(result.unwrap_err().to_string().contains("expected .bacpac or .dacpac"));
    assert_eq!(
        scalar(&host, port, "master", "sa", SA_PASSWORD, "SELECT CASE WHEN DB_ID('keepdb') IS NULL THEN 0 ELSE 1 END").await,
        1
    );
}

#[tokio::test]
async fn mssql_restore_refuses_without_create_database_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "ownerdb").await;
    exec(&host, port, "master", "CREATE LOGIN owner_login WITH PASSWORD = 'Owner!Str0ng1'").await;
    exec(
        &host,
        port,
        "ownerdb",
        "CREATE TABLE dbo.users (id INT PRIMARY KEY, email NVARCHAR(100) NOT NULL); \
         INSERT dbo.users VALUES (1, 'a@x'), (2, 'b@x'), (3, 'c@x'); \
         CREATE USER owner_user FOR LOGIN owner_login; \
         ALTER ROLE db_owner ADD MEMBER owner_user;",
    )
    .await;

    let config = make_config(host.clone(), port, "ownerdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6db");
    let temp_dir = TempDir::new().unwrap();
    let file_path = DatabaseFactory::create_for_backup(config.clone())
        .await
        .backup(temp_dir.path(), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    let mut owner_config = config;
    owner_config.username = "owner_login".to_string();
    owner_config.password = "Owner!Str0ng1".to_string();
    let result = DatabaseFactory::create_for_restore(owner_config, &file_path)
        .await
        .restore(&file_path, Arc::new(JobLogger::new()))
        .await;

    assert!(format!("{:#}", result.unwrap_err()).contains("CREATE DATABASE"));
    assert_eq!(
        scalar(&host, port, "master", "sa", SA_PASSWORD, "SELECT CASE WHEN DB_ID('ownerdb') IS NULL THEN 0 ELSE 1 END").await,
        1
    );
    assert_eq!(scalar(&host, port, "ownerdb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.users").await, 3);
}

#[tokio::test]
async fn mssql_restore_refuses_without_drop_permission_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "alterdb").await;
    exec(
        &host,
        port,
        "master",
        "CREATE LOGIN alter_login WITH PASSWORD = 'Alter!Str0ng1'; GRANT CREATE ANY DATABASE TO alter_login;",
    )
    .await;
    exec(
        &host,
        port,
        "alterdb",
        "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1); \
         CREATE USER alter_user FOR LOGIN alter_login; GRANT ALTER TO alter_user;",
    )
    .await;

    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("backup.dacpac");
    std::fs::write(&file_path, b"not a package").unwrap();

    let mut config = make_config(host.clone(), port, "alterdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6dc");
    config.username = "alter_login".to_string();
    config.password = "Alter!Str0ng1".to_string();
    let result = DatabaseFactory::create_for_restore(config, &file_path)
        .await
        .restore(&file_path, Arc::new(JobLogger::new()))
        .await;

    assert!(format!("{:#}", result.unwrap_err()).contains("CONTROL"));
    assert_eq!(scalar(&host, port, "alterdb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.t").await, 1);
    assert_eq!(
        scalar(&host, port, "master", "sa", SA_PASSWORD, "SELECT CASE WHEN DB_ID('alterdb') IS NULL THEN 0 ELSE 1 END").await,
        1
    );
    assert_eq!(
        scalar(
            &host,
            port,
            "master",
            "sa",
            SA_PASSWORD,
            "SELECT CASE WHEN DATABASEPROPERTYEX('alterdb', 'UserAccess') = 'MULTI_USER' THEN 1 ELSE 0 END"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn mssql_restore_refuses_system_database_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;

    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("backup.dacpac");
    std::fs::write(&file_path, b"not a package").unwrap();

    let config = make_config(host.clone(), port, "msdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6dd");
    let result = DatabaseFactory::create_for_restore(config, &file_path)
        .await
        .restore(&file_path, Arc::new(JobLogger::new()))
        .await;

    assert!(format!("{:#}", result.unwrap_err()).contains("system database"));
    assert_eq!(
        scalar(&host, port, "master", "sa", SA_PASSWORD, "SELECT CASE WHEN DB_ID('msdb') IS NULL THEN 0 ELSE 1 END").await,
        1
    );
    assert_eq!(
        scalar(
            &host,
            port,
            "master",
            "sa",
            SA_PASSWORD,
            "SELECT CASE WHEN DATABASEPROPERTYEX('msdb', 'UserAccess') = 'MULTI_USER' THEN 1 ELSE 0 END"
        )
        .await,
        1
    );
}

async fn backup_package(config: &DatabaseConfig, work_dir: &Path) -> PathBuf {
    DatabaseFactory::create_for_backup(config.clone())
        .await
        .backup(work_dir, Arc::new(JobLogger::new()))
        .await
        .unwrap()
}

async fn restore_package(config: &DatabaseConfig, package: &Path) -> anyhow::Result<()> {
    DatabaseFactory::create_for_restore(config.clone(), package)
        .await
        .restore(package, Arc::new(JobLogger::new()))
        .await
}

async fn row_count(host: &str, port: u16, database: &str) -> i32 {
    scalar(host, port, database, "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.t").await
}

async fn database_exists(host: &str, port: u16, database: &str) -> bool {
    let sql = format!("SELECT CASE WHEN DB_ID('{database}') IS NULL THEN 0 ELSE 1 END");
    scalar(host, port, "master", "sa", SA_PASSWORD, &sql).await == 1
}

async fn assert_multi_user_named(host: &str, port: u16, database: &str) {
    let multi_user = format!("SELECT CASE WHEN DATABASEPROPERTYEX('{database}', 'UserAccess') = 'MULTI_USER' THEN 1 ELSE 0 END");
    assert_eq!(scalar(host, port, "master", "sa", SA_PASSWORD, &multi_user).await, 1);
}

async fn assert_target_intact(host: &str, port: u16, database: &str, expected_rows: i32) {
    assert_eq!(row_count(host, port, database).await, expected_rows);
    assert_multi_user_named(host, port, database).await;
    let side_databases = "SELECT COUNT(*) FROM sys.databases WHERE name LIKE '%[_]restore[_]%-%'";
    assert_eq!(scalar(host, port, "master", "sa", SA_PASSWORD, side_databases).await, 0);
}

#[tokio::test]
async fn mssql_restore_failure_keeps_target_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "config_db").await;
    create_user_database(&host, port, "company_db").await;
    exec(&host, port, "config_db", "CREATE TABLE dbo.Settings (id INT PRIMARY KEY); INSERT dbo.Settings VALUES (1);").await;
    exec(&host, port, "company_db", "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;
    exec(&host, port, "company_db", "CREATE VIEW dbo.v_settings AS SELECT id FROM config_db.dbo.Settings;").await;
    exec(&host, port, "company_db", "CREATE PROCEDURE dbo.p_t AS SELECT id FROM dbo.t;").await;

    let config = make_config(host.clone(), port, "company_db", "5a445eb4-c2c6-4bde-a423-ee1385dcf6e4");
    let work_dir = TempDir::new().unwrap();
    let package = backup_package(&config, work_dir.path()).await;

    exec(&host, port, "company_db", "INSERT dbo.t VALUES (3);").await;
    exec(&host, port, "master", "ALTER DATABASE config_db MODIFY NAME = config_hidden;").await;

    assert!(restore_package(&config, &package).await.is_err());
    assert_target_intact(&host, port, "company_db", 3).await;
    assert_eq!(scalar(&host, port, "company_db", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM sys.views").await, 1);
    assert_eq!(scalar(&host, port, "company_db", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM sys.procedures").await, 1);

    let mut fresh_config = config.clone();
    fresh_config.database = "fresh_db".to_string();
    assert!(restore_package(&fresh_config, &package).await.is_err());
    assert!(!database_exists(&host, port, "fresh_db").await, "a failed restore must not leave a partial database");

    for extension in ["dacpac", "bacpac"] {
        let corrupt_file = work_dir.path().join(format!("corrupt.{extension}"));
        std::fs::write(&corrupt_file, b"not a package").unwrap();
        assert!(restore_package(&config, &corrupt_file).await.is_err());
        assert_target_intact(&host, port, "company_db", 3).await;
    }

    exec(&host, port, "master", "ALTER DATABASE config_hidden MODIFY NAME = config_db;").await;
    restore_package(&config, &package).await.unwrap();
    assert_target_intact(&host, port, "company_db", 2).await;
    assert_eq!(scalar(&host, port, "company_db", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.v_settings").await, 1);
}

#[tokio::test]
async fn mssql_restore_repeats_and_sweeps_leftovers_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    let generated_id = "5a445eb4-c2c6-4bde-a423-ee1385dcf6e5";
    let leftover = format!("appdb_restore_{generated_id}_deadbeef");
    create_user_database(&host, port, "appdb").await;
    create_user_database(&host, port, &leftover).await;
    create_user_database(&host, port, "appdb_restore_old").await;
    exec(&host, port, "appdb", "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;

    let config = make_config(host.clone(), port, "appdb", generated_id);
    let work_dir = TempDir::new().unwrap();
    let package = backup_package(&config, work_dir.path()).await;

    for _ in 0..2 {
        exec(&host, port, "appdb", "INSERT dbo.t VALUES (3);").await;
        restore_package(&config, &package).await.unwrap();
        assert_target_intact(&host, port, "appdb", 2).await;
    }

    let side_named_files = "SELECT COUNT(*) FROM sys.master_files WHERE database_id = DB_ID('appdb') AND physical_name LIKE '%appdb[_]restore[_]%'";
    assert_eq!(scalar(&host, port, "master", "sa", SA_PASSWORD, side_named_files).await, 2);
    assert!(!database_exists(&host, port, &leftover).await, "a side database from a crashed run must be removed");
    assert!(database_exists(&host, port, "appdb_restore_old").await, "a user database must not be swept");

    let long_name = "l".repeat(123);
    create_user_database(&host, port, &long_name).await;
    exec(&host, port, &long_name, "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;
    let long_config = make_config(host.clone(), port, &long_name, generated_id);
    let long_package = backup_package(&long_config, work_dir.path()).await;
    exec(&host, port, &long_name, "INSERT dbo.t VALUES (3);").await;
    restore_package(&long_config, &long_package).await.unwrap();
    assert_target_intact(&host, port, &long_name, 2).await;
}

#[tokio::test]
async fn mssql_restore_accepts_owner_and_dbcreator_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    let work_dir = TempDir::new().unwrap();
    exec(
        &host,
        port,
        "master",
        "CREATE LOGIN owner_login WITH PASSWORD = 'Owner!Str0ng1'; GRANT CREATE ANY DATABASE TO owner_login; \
         CREATE LOGIN creator_login WITH PASSWORD = 'Creator!Str0ng1'; ALTER SERVER ROLE dbcreator ADD MEMBER creator_login;",
    )
    .await;

    for (database, login, password) in [("ownerdb", "owner_login", "Owner!Str0ng1"), ("creatordb", "creator_login", "Creator!Str0ng1")] {
        create_user_database(&host, port, database).await;
        exec(&host, port, database, "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;
        let mut config = make_config(host.clone(), port, database, "5a445eb4-c2c6-4bde-a423-ee1385dcf6e6");
        let package = backup_package(&config, work_dir.path()).await;
        exec(&host, port, database, "INSERT dbo.t VALUES (3);").await;
        if login == "owner_login" {
            exec(&host, port, database, "CREATE USER owner_user FOR LOGIN owner_login; ALTER ROLE db_owner ADD MEMBER owner_user;").await;
        }

        config.username = login.to_string();
        config.password = password.to_string();
        restore_package(&config, &package).await.unwrap();
        assert_target_intact(&host, port, database, 2).await;
    }
}

#[tokio::test]
async fn mssql_restore_failed_drop_keeps_target_test() {
    init_tracing_for_test();
    let (_container, host, port) = start().await;
    create_user_database(&host, port, "appdb").await;
    exec(&host, port, "appdb", "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;

    let config = make_config(host.clone(), port, "appdb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6e7");
    let work_dir = TempDir::new().unwrap();
    let package = backup_package(&config, work_dir.path()).await;

    exec(&host, port, "appdb", "INSERT dbo.t VALUES (3);").await;
    exec(
        &host,
        port,
        "master",
        "CREATE TRIGGER block_appdb_drop ON ALL SERVER FOR DROP_DATABASE AS \
         IF EVENTDATA().value('(/EVENT_INSTANCE/DatabaseName)[1]', 'nvarchar(128)') = 'appdb' \
         BEGIN ROLLBACK; THROW 50001, 'blocked by test', 1; END",
    )
    .await;

    assert!(restore_package(&config, &package).await.is_err());
    assert_target_intact(&host, port, "appdb", 3).await;

    exec(&host, port, "master", "DROP TRIGGER block_appdb_drop ON ALL SERVER;").await;
    restore_package(&config, &package).await.unwrap();
    assert_target_intact(&host, port, "appdb", 2).await;

    create_user_database(&host, port, "side_db").await;
    let held_session = connect_as(&host, port, "side_db", "sa", SA_PASSWORD).await;
    connection::rename_database(&config, "side_db", "renamed_db").await.unwrap();
    drop(held_session);
    assert_multi_user_named(&host, port, "renamed_db").await;
}

mod format_tests {
    use super::{make_config, with_backup_format};
    use crate::domain::mssql::format::MssqlFormat;
    use std::path::Path;

    #[test]
    fn from_config_defaults_to_dacpac() {
        let cfg = make_config("localhost".into(), 1433, "db", "00000000-0000-0000-0000-000000000000");
        assert_eq!(MssqlFormat::from_config(&cfg), (MssqlFormat::Dacpac, None));
        assert_eq!(
            MssqlFormat::from_config(&with_backup_format(cfg.clone(), "dacpac")),
            (MssqlFormat::Dacpac, None)
        );
        assert_eq!(
            MssqlFormat::from_config(&with_backup_format(cfg.clone(), "bacpac")),
            (MssqlFormat::Bacpac, None)
        );
        assert_eq!(
            MssqlFormat::from_config(&with_backup_format(cfg, "zip")),
            (MssqlFormat::Dacpac, Some("zip".to_string()))
        );
    }

    #[test]
    fn from_path_reads_extension() {
        assert_eq!(MssqlFormat::from_path(Path::new("/tmp/a.dacpac")), Some(MssqlFormat::Dacpac));
        assert_eq!(MssqlFormat::from_path(Path::new("/tmp/a.BACPAC")), Some(MssqlFormat::Bacpac));
        assert_eq!(MssqlFormat::from_path(Path::new("/tmp/backup.dump")), None);
        assert_eq!(MssqlFormat::from_path(Path::new("/tmp/noext")), None);
        assert_eq!(MssqlFormat::Dacpac.extension(), ".dacpac");
        assert_eq!(MssqlFormat::Bacpac.extension(), ".bacpac");
    }
}
