use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use crate::utils::compress::{compress_to_tar_gz_large, decompress_large_tar_gz};
use oauth2::url;
use std::path::PathBuf;
use tempfile::TempDir;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tracing::{error, info};
use url::Host;

async fn create_config() -> (ContainerAsync<Postgres>, DatabaseConfig) {
    let container = Postgres::default()
        .with_env_var("POSTGRES_DB", "testdb")
        .with_env_var("POSTGRES_USER", "testuser")
        .with_env_var("POSTGRES_PASSWORD", "changeme")
        .with_tag("17")
        .start()
        .await
        .unwrap();

    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());

    let port = container.get_host_port_ipv4(5432).await.unwrap_or(5432);

    let config = DatabaseConfig {
        name: "My test Postgres Database".to_string(),
        database: "testdb".to_string(),
        db_type: DbType::Postgresql,
        username: "testuser".to_string(),
        password: "changeme".to_string(),
        port,
        host: host.to_string(),
        generated_id: "40875631-e3d2-4dfe-a26b-2a347ecc64fd".to_string(),
        path: "".to_string(),
    };

    (container, config)
}

#[tokio::test]
async fn postgres_ping_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or_else(|_| false);

    assert_eq!(reachable, true);
}

#[tokio::test]
async fn postgres_backup_restore_test() {
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

    let backup_file: PathBuf;

    if files.len() == 1 {
        backup_file = files[0].clone()
    } else {
        backup_file = "".into()
    }

    let db = DatabaseFactory::create_for_restore(config.clone(), &backup_file).await;

    let reachable = db.ping().await.unwrap_or(false);

    info!("Reachable: {}", reachable);

    assert_eq!(reachable, true);

    info!("Running pg_restore: {:?}", backup_file);

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
