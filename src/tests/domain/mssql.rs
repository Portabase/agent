use crate::domain::docker_volume::docker::{client, parse_container_id};
use crate::domain::factory::DatabaseFactory;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use bollard::query_parameters::InspectContainerOptions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tiberius::{AuthMethod, Client, Config};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers::core::{IntoContainerPort, Mount};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::{error, info};

const SA_PASSWORD: &str = "Test!Str0ng1";
const BAK_SERVER_PATH: &str = "/var/opt/mssql/backup";

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

    assert!(result.unwrap_err().to_string().contains("expected .bacpac, .dacpac or .bak"));
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
async fn mssql_restore_failed_drop_keeps_multi_user_test() {
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

    assert!(!format!("{:#}", result.unwrap_err()).contains("nothing was dropped"));
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

async fn shared_folder() -> (String, TempDir) {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let container_id = parse_container_id(&mountinfo, &cgroup).expect("tests run inside a container");
    let inspected = client()
        .unwrap()
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await
        .unwrap();
    let app_mount = inspected
        .mounts
        .unwrap()
        .into_iter()
        .find(|mount| mount.destination.as_deref() == Some("/app"))
        .expect("/app is bind-mounted from the host");

    std::fs::create_dir_all("/app/target").unwrap();
    let agent_folder = tempfile::Builder::new().prefix("mssql-bak-").tempdir_in("/app/target").unwrap();
    std::fs::set_permissions(agent_folder.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    let relative_path = agent_folder.path().strip_prefix("/app").unwrap().display().to_string();
    (format!("{}/{}", app_mount.source.unwrap(), relative_path), agent_folder)
}

async fn start_with_shared_folder() -> (ContainerAsync<GenericImage>, String, u16, TempDir) {
    let (host_path, agent_folder) = shared_folder().await;
    let container = GenericImage::new("mcr.microsoft.com/azure-sql-edge", "latest")
        .with_exposed_port(1433.tcp())
        .with_env_var("ACCEPT_EULA", "Y")
        .with_env_var("MSSQL_SA_PASSWORD", SA_PASSWORD)
        .with_mount(Mount::bind_mount(host_path, BAK_SERVER_PATH))
        .start()
        .await
        .expect("azure-sql-edge container started");
    tokio::time::sleep(Duration::from_secs(30)).await;
    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(1433).await.unwrap();
    (container, host, port, agent_folder)
}

fn with_bak(mut config: DatabaseConfig, agent_path: &Path) -> DatabaseConfig {
    config.options.insert("backup_format".to_string(), serde_json::json!("bak"));
    config.options.insert("bak_server_path".to_string(), serde_json::json!(BAK_SERVER_PATH));
    config.options.insert("bak_agent_path".to_string(), serde_json::json!(agent_path.display().to_string()));
    config
}

async fn bak_backup(config: &DatabaseConfig, work_dir: &Path) -> anyhow::Result<PathBuf> {
    DatabaseFactory::create_for_backup(config.clone())
        .await
        .backup(work_dir, Arc::new(JobLogger::new()))
        .await
}

async fn bak_restore(config: &DatabaseConfig, restore_file: &Path) -> anyhow::Result<()> {
    DatabaseFactory::create_for_restore(config.clone(), restore_file)
        .await
        .restore(restore_file, Arc::new(JobLogger::new()))
        .await
}

async fn extract_bak(archive: &Path, work_dir: &Path) -> PathBuf {
    assert!(archive.to_string_lossy().ends_with(".tar.gz"), "got {}", archive.display());
    let extracted = decompress_large_tar_gz(archive, work_dir).await.unwrap();
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].extension().and_then(|extension| extension.to_str()), Some("bak"));
    extracted[0].clone()
}

fn assert_folder_empty(agent_path: &Path) {
    assert_eq!(std::fs::read_dir(agent_path).unwrap().count(), 0, "shared folder must be left empty");
}

async fn assert_multi_user(host: &str, port: u16, database: &str) {
    let sql = format!("SELECT CASE WHEN DATABASEPROPERTYEX('{database}', 'UserAccess') = 'MULTI_USER' THEN 1 ELSE 0 END");
    assert_eq!(scalar(host, port, "master", "sa", SA_PASSWORD, &sql).await, 1);
}

#[tokio::test]
async fn mssql_bak_cross_database_test() {
    init_tracing_for_test();
    let (_container, host, port, agent_folder) = start_with_shared_folder().await;
    let agent_path = agent_folder.path();
    create_user_database(&host, port, "config_db").await;
    create_user_database(&host, port, "company_db").await;
    exec(&host, port, "config_db", "CREATE TABLE dbo.Settings (id INT PRIMARY KEY); INSERT dbo.Settings VALUES (1);").await;
    exec(&host, port, "company_db", "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;
    exec(&host, port, "company_db", "CREATE VIEW dbo.v_settings AS SELECT id FROM config_db.dbo.Settings;").await;
    exec(&host, port, "company_db", "CREATE VIEW dbo.v_master AS SELECT name FROM master.dbo.spt_values;").await;

    let config = make_config(host.clone(), port, "company_db", "5a445eb4-c2c6-4bde-a423-ee1385dcf6e0");
    let work_dir = TempDir::new().unwrap();

    let bacpac = bak_backup(&with_backup_format(config.clone(), "bacpac"), work_dir.path()).await;
    assert!(bacpac.unwrap_err().to_string().contains("SQL71561"));

    let bak_config = with_bak(config, agent_path);
    let archive = bak_backup(&bak_config, work_dir.path()).await.unwrap();
    let bak_file = extract_bak(&archive, work_dir.path()).await;
    assert_folder_empty(agent_path);

    exec(&host, port, "master", "DROP DATABASE company_db; DROP DATABASE config_db;").await;

    bak_restore(&bak_config, &bak_file).await.unwrap();
    assert_folder_empty(agent_path);
    assert_eq!(scalar(&host, port, "company_db", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.t").await, 2);
    assert_eq!(scalar(&host, port, "company_db", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM sys.views").await, 2);
}

#[tokio::test]
async fn mssql_bak_restore_over_existing_test() {
    init_tracing_for_test();
    let (_container, host, port, agent_folder) = start_with_shared_folder().await;
    let agent_path = agent_folder.path();
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

    let generated_id = "5a445eb4-c2c6-4bde-a423-ee1385dcf6e1";
    let mut config = with_bak(make_config(host.clone(), port, "appdb", generated_id), agent_path);
    config.options.insert("bak_verify".to_string(), serde_json::json!(true));

    let leftover = agent_path.join(format!("{generated_id}.bak"));
    std::fs::write(&leftover, b"not a backup").unwrap();
    std::fs::set_permissions(&leftover, std::fs::Permissions::from_mode(0o666)).unwrap();

    let work_dir = TempDir::new().unwrap();
    let archive = bak_backup(&config, work_dir.path()).await.unwrap();
    let bak_file = extract_bak(&archive, work_dir.path()).await;
    assert_folder_empty(agent_path);

    exec(
        &host,
        port,
        "appdb",
        "DELETE FROM dbo.users WHERE id = 1; INSERT dbo.users VALUES (4, 'd@x'); CREATE TABLE dbo.extra (id INT);",
    )
    .await;

    let held_session = connect_as(&host, port, "appdb", "sa", SA_PASSWORD).await;
    bak_restore(&config, &bak_file).await.unwrap();
    drop(held_session);

    assert_folder_empty(agent_path);
    assert_multi_user(&host, port, "appdb").await;
    assert_eq!(
        scalar(&host, port, "appdb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.users WHERE id IN (1, 2, 3)").await,
        3
    );
    assert_eq!(scalar(&host, port, "appdb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.users").await, 3);
    assert_eq!(
        scalar(&host, port, "appdb", "sa", SA_PASSWORD, "SELECT CASE WHEN OBJECT_ID('dbo.extra') IS NULL THEN 1 ELSE 0 END").await,
        1
    );
    assert_eq!(scalar(&host, port, "appdb", "app_login", "App!Str0ng1", "SELECT COUNT(*) FROM dbo.users").await, 3);
}

#[tokio::test]
async fn mssql_bak_restore_into_other_database_test() {
    init_tracing_for_test();
    let (_container, host, port, agent_folder) = start_with_shared_folder().await;
    let agent_path = agent_folder.path();
    create_user_database(&host, port, "sourcedb").await;
    create_user_database(&host, port, "otherdb").await;
    exec(&host, port, "sourcedb", "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;

    let config = with_bak(make_config(host.clone(), port, "sourcedb", "5a445eb4-c2c6-4bde-a423-ee1385dcf6e2"), agent_path);
    let work_dir = TempDir::new().unwrap();
    let archive = bak_backup(&config, work_dir.path()).await.unwrap();
    let bak_file = extract_bak(&archive, work_dir.path()).await;

    for target in ["copydb", "otherdb"] {
        let mut target_config = config.clone();
        target_config.database = target.to_string();
        bak_restore(&target_config, &bak_file).await.unwrap();
        assert_eq!(scalar(&host, port, target, "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.t").await, 2);
        let moved = format!(
            "SELECT COUNT(*) FROM sys.master_files WHERE database_id = DB_ID('{target}') \
             AND physical_name LIKE '/var/opt/mssql/data/{target}[_]sourcedb%'"
        );
        assert_eq!(scalar(&host, port, "master", "sa", SA_PASSWORD, &moved).await, 2);
        assert_multi_user(&host, port, target).await;
    }

    assert_eq!(scalar(&host, port, "sourcedb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.t").await, 2);
    assert_multi_user(&host, port, "sourcedb").await;
}

#[tokio::test]
async fn mssql_bak_failures_leave_target_intact_test() {
    init_tracing_for_test();
    let (_container, host, port, agent_folder) = start_with_shared_folder().await;
    let agent_path = agent_folder.path();
    create_user_database(&host, port, "keepdb").await;
    exec(&host, port, "keepdb", "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (1), (2);").await;

    let generated_id = "5a445eb4-c2c6-4bde-a423-ee1385dcf6e3";
    let plain_config = make_config(host.clone(), port, "keepdb", generated_id);
    let config = with_bak(plain_config.clone(), agent_path);
    let work_dir = TempDir::new().unwrap();

    let no_paths = bak_backup(&with_backup_format(plain_config.clone(), "bak"), work_dir.path()).await;
    assert!(no_paths.unwrap_err().to_string().contains("bak_server_path"));

    let mut missing_database = config.clone();
    missing_database.database = "nosuchdb".to_string();
    let backup_error = bak_backup(&missing_database, work_dir.path()).await.unwrap_err().to_string();
    assert!(backup_error.contains("nosuchdb"), "SQL Server text must reach the job log: {backup_error}");

    let other_folder = TempDir::new().unwrap();
    let wrong_path_config = with_bak(plain_config.clone(), other_folder.path());
    let wrong_path = bak_backup(&wrong_path_config, work_dir.path()).await;
    assert!(wrong_path.unwrap_err().to_string().contains("bak_agent_path"));
    std::fs::remove_file(agent_path.join(format!("{generated_id}.bak"))).unwrap();

    let archive = bak_backup(&config, work_dir.path()).await.unwrap();
    let bak_file = extract_bak(&archive, work_dir.path()).await;

    let corrupt_file = work_dir.path().join("corrupt.bak");
    std::fs::write(&corrupt_file, b"not a backup").unwrap();
    assert!(bak_restore(&config, &corrupt_file).await.is_err());
    assert_folder_empty(agent_path);

    let mut system_config = config.clone();
    system_config.database = "msdb".to_string();
    let system_error = bak_restore(&system_config, &bak_file).await.unwrap_err();
    assert!(format!("{:#}", system_error).contains("system database"));
    assert_multi_user(&host, port, "msdb").await;

    exec(
        &host,
        port,
        "master",
        "CREATE DATABASE blockerdb ON (NAME = blocker, FILENAME = '/var/opt/mssql/data/occupied_keepdb.mdf') \
         LOG ON (NAME = blocker_log, FILENAME = '/var/opt/mssql/data/blocker_log.ldf');",
    )
    .await;
    create_user_database(&host, port, "occupied").await;
    exec(&host, port, "occupied", "CREATE TABLE dbo.t (id INT PRIMARY KEY); INSERT dbo.t VALUES (9);").await;
    let mut occupied_config = config.clone();
    occupied_config.database = "occupied".to_string();
    assert!(bak_restore(&occupied_config, &bak_file).await.is_err());
    assert_multi_user(&host, port, "occupied").await;
    assert_eq!(scalar(&host, port, "occupied", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.t WHERE id = 9").await, 1);

    assert_folder_empty(agent_path);
    assert_multi_user(&host, port, "keepdb").await;
    assert_eq!(scalar(&host, port, "keepdb", "sa", SA_PASSWORD, "SELECT COUNT(*) FROM dbo.t").await, 2);
}

mod format_tests {
    use super::{make_config, with_backup_format};
    use crate::domain::mssql::format::MssqlFormat;
    use crate::domain::mssql::native::{BackupFile, join_server_path, target_path};
    use std::collections::HashMap;
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
            MssqlFormat::from_config(&with_backup_format(cfg.clone(), "bak")),
            (MssqlFormat::Bak, None)
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
        assert_eq!(MssqlFormat::from_path(Path::new("/tmp/a.BAK")), Some(MssqlFormat::Bak));
        assert_eq!(MssqlFormat::from_path(Path::new("/tmp/backup.dump")), None);
        assert_eq!(MssqlFormat::from_path(Path::new("/tmp/noext")), None);
        assert_eq!(MssqlFormat::Dacpac.extension(), ".dacpac");
        assert_eq!(MssqlFormat::Bacpac.extension(), ".bacpac");
        assert_eq!(MssqlFormat::Bak.extension(), ".bak");
    }

    #[test]
    fn join_server_path_keeps_the_server_separator() {
        assert_eq!(join_server_path(r"D:\Backups", "a.bak"), r"D:\Backups\a.bak");
        assert_eq!(join_server_path(r"D:\Backups\", "a.bak"), r"D:\Backups\a.bak");
        assert_eq!(join_server_path(r"D:\", "a.bak"), r"D:\a.bak");
        assert_eq!(join_server_path(r"\\nas\sql", "a.bak"), r"\\nas\sql\a.bak");
        assert_eq!(join_server_path("/var/opt/mssql/backup", "a.bak"), "/var/opt/mssql/backup/a.bak");
        assert_eq!(join_server_path("/var/opt/mssql/backup/", "a.bak"), "/var/opt/mssql/backup/a.bak");
    }

    #[test]
    fn target_path_keeps_existing_files_and_moves_the_others() {
        let existing = HashMap::from([("app".to_string(), r"E:\Data\app.mdf".to_string())]);
        let backup_file = |logical_name, physical_name, kind| BackupFile { logical_name, physical_name, kind };
        let resolve = |file: &BackupFile| target_path("target", file, &existing, r"D:\Data\", r"L:\Logs\");

        assert_eq!(resolve(&backup_file("app", r"C:\Old\app.mdf", "D")), r"E:\Data\app.mdf");
        assert_eq!(resolve(&backup_file("other", r"C:\Old\other.ndf", "D")), r"D:\Data\target_other.ndf");
        assert_eq!(resolve(&backup_file("app_log", "/var/opt/mssql/data/app_log.ldf", "L")), r"L:\Logs\target_app_log.ldf");
        assert_eq!(resolve(&backup_file("stream", r"C:\Old\stream", "S")), r"D:\Data\target_stream");
    }
}
