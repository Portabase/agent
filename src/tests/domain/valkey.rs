use tempfile::TempDir;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::valkey::{Valkey};
use url::Host;
use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;

async fn create_config() -> (ContainerAsync<Valkey>, DatabaseConfig) {
    let container = Valkey::default().start().await.unwrap();

    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());

    let port = container
        .get_host_port_ipv4(6379)
        .await
        .unwrap_or(6379);

    let config = DatabaseConfig {
        name: "Test Valkey".to_string(),
        database: "valkey".to_string(),
        username: "".to_string(),
        password: "".to_string(),
        db_type: DbType::Valkey,
        port,
        host: host.to_string(),
        generated_id: "40875485-e3d2-4dfe-a26b-2a347ecc64fd".to_string(),
        path: "".to_string(),
    };

    (container, config)
}

#[tokio::test]
async fn valkey_ping_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or(false);

    assert!(reachable);
}

#[tokio::test]
async fn valkey_backup_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let temp_dir = TempDir::new().unwrap();
    let backup_path = temp_dir.path();

    let db = DatabaseFactory::create_for_backup(config.clone()).await;

    let file_path = db.backup(backup_path, Some(true)).await.unwrap();

    assert!(file_path.is_file());
}