use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use mongodb::{Client, bson::doc};
use tempfile::TempDir;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;
use tracing::{error, info};
use url::Host;

async fn create_config() -> (ContainerAsync<Mongo>, DatabaseConfig) {
    let container = Mongo::default().start().await.unwrap();

    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());

    let port = container.get_host_port_ipv4(27017).await.unwrap_or(27017);

    let config = DatabaseConfig {
        name: "Test MongoDB".to_string(),
        database: "testdb".to_string(),
        db_type: DbType::MongoDB,
        username: "".to_string(),
        password: "".to_string(),
        port,
        host: host.to_string(),
        generated_id: "96d30a9f-ff4b-47c9-aaab-f3147bb34f16".to_string(),
        path: "".to_string(),
    };

    (container, config)
}

async fn seed_database(config: &DatabaseConfig) {
    let client = Client::with_uri_str(format!(
        "mongodb://{}:{}/{}",
        config.host, config.port, config.database
    ))
    .await
    .unwrap();

    let collection = client
        .database(&config.database)
        .collection::<mongodb::bson::Document>("sample");

    collection
        .insert_one(doc! { "name": "hello mongo" })
        .await
        .unwrap();
}

#[tokio::test]
async fn mongodb_ping_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or(false);

    assert!(reachable);
}

#[tokio::test]
async fn mongodb_backup_restore_test() {
    init_tracing_for_test();

    let (_container, config) = create_config().await;
    seed_database(&config).await;

    let temp_dir = TempDir::new().unwrap();
    let backup_path = temp_dir.path();

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let file_path = db.backup(backup_path).await.unwrap();
    info!("Backup path: {:?}", file_path);
    assert!(file_path.is_file());

    let db = DatabaseFactory::create_for_restore(config.clone(), &file_path).await;
    let reachable = db.ping().await.unwrap_or(false);

    info!("Reachable: {}", reachable);
    assert!(reachable);

    match db.restore(&file_path).await {
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
