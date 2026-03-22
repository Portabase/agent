use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use oauth2::url;
use std::path::PathBuf;
use tempfile::TempDir;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mysql::Mysql;
use tracing::{error, info};
use url::Host;

async fn create_config() -> (ContainerAsync<Mysql>, DatabaseConfig) {
    let container = Mysql::default().with_tag("8.1").start().await.unwrap();

    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());

    let port = container.get_host_port_ipv4(3306).await.unwrap_or(3306);

    let config = DatabaseConfig {
        name: "Test MySQL".to_string(),
        database: "test".to_string(),
        db_type: DbType::Mysql,
        username: "root".to_string(),
        password: "".to_string(),
        port,
        host: host.to_string(),
        generated_id: "0f1bb8f2-35a0-4c91-8098-e36873d3ce31".to_string(),
        path: "".to_string(),
    };

    (container, config)
}

#[tokio::test]
async fn mysql_ping_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or(false);

    assert!(reachable);
}

#[tokio::test]
async fn mysql_backup_restore_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let temp_dir = TempDir::new().unwrap();
    let backup_path = temp_dir.path();

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let file_path = db.backup(backup_path).await.unwrap();

    assert!(file_path.is_file());

    let compression = compress_to_tar_gz_large(&file_path).await.unwrap();
    assert!(compression.compressed_path.is_file());

    let files = decompress_large_tar_gz(compression.compressed_path.as_path(), temp_dir.path())
        .await
        .unwrap();

    let backup_file: PathBuf = if files.len() == 1 {
        files[0].clone()
    } else {
        "".into()
    };

    let db = DatabaseFactory::create_for_restore(config.clone(), &backup_file).await;
    let reachable = db.ping().await.unwrap_or(false);

    info!("Reachable: {}", reachable);
    assert!(reachable);

    match db.restore(&backup_file).await {
        Ok(_) => {
            info!("Restore succeeded for {}", config.generated_id);
            assert!(true)
        }
        Err(e) => {
            error!("Restore failed for {}: {:?}", config.generated_id, e);
            assert!(false)
        }
    }
}
