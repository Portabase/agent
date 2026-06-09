use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use tiberius::{AuthMethod, Client, Config};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers::core::IntoContainerPort;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;
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
    }
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
        file_path.extension().and_then(|e| e.to_str()) == Some("bacpac"),
        "backup file should have .bacpac extension"
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
