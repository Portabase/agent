use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers::core::{IntoContainerPort};
use tracing::{error, info};


async fn create_config() -> (ContainerAsync<GenericImage>, DatabaseConfig) {

    let container = GenericImage::new("jacobalberty/firebird", "latest")
        .with_exposed_port(3050.tcp())
        .with_env_var("FIREBIRD_ROOT_PASSWORD", "fake_root_password")
        .with_env_var("FIREBIRD_USER", "alice")
        .with_env_var("FIREBIRD_PASSWORD", "fake_password")
        .with_env_var("FIREBIRD_DATABASE", "mirror.fdb")
        .with_env_var("FIREBIRD_DATABASE_DEFAULT_CHARSET", "UTF8")
        .start()
        .await
        .expect("Firebird started");

    tokio::time::sleep(Duration::from_secs(10)).await;

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(3050).await.unwrap();

    let database = "/firebird/data/mirror.fdb";

    let config = DatabaseConfig {
        name: "Test Firebird".to_string(),
        database: database.to_string(),
        db_type: DbType::Firebird,
        username: "alice".to_string(),
        password: "fake_password".to_string(),
        port,
        host,
        generated_id: "3c445eb4-c2c6-4bde-a423-ee1385dcf6d2".to_string(),
        path: "".to_string(),
    };

    (container, config)
}

#[tokio::test]
async fn firebird_ping_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or(false);

    assert!(reachable);
}

#[tokio::test]
async fn firebird_backup_restore_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let temp_dir = TempDir::new().unwrap();
    let backup_path = temp_dir.path();

    let db = DatabaseFactory::create_for_backup(config.clone()).await;

    let file_path = db.backup(backup_path).await.unwrap();
    assert!(file_path.is_file());

    let compression = compress_to_tar_gz_large(&file_path).await.unwrap();
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
        panic!("Unexpected number of files after decompression");
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