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
        max_packet_size: "".to_string(),
        volume_name: "".to_string(),
        container_name: None,
        options: std::collections::HashMap::new(),
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
    let file_path = db.backup(backup_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await.unwrap();
    info!("Backup path: {:?}", file_path);
    assert!(file_path.is_file());

    let db = DatabaseFactory::create_for_restore(config.clone(), &file_path).await;
    let reachable = db.ping().await.unwrap_or(false);

    info!("Reachable: {}", reachable);
    assert!(reachable);

    match db.restore(&file_path, std::sync::Arc::new(crate::services::backup::logger::JobLogger::new())).await {
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

use crate::domain::mongodb::connection::build_mongo_uri;

fn uri_cfg(host: &str, port: u16, user: &str, pass: &str) -> DatabaseConfig {
    DatabaseConfig {
        name: "t".into(),
        database: "mydb".into(),
        db_type: DbType::MongoDB,
        username: user.into(),
        password: pass.into(),
        port,
        host: host.into(),
        generated_id: "id".into(),
        path: String::new(),
        max_packet_size: String::new(),
        volume_name: String::new(),
        container_name: None,
        options: std::collections::HashMap::new(),
    }
}

#[test]
fn uri_standard_with_auth() {
    let c = uri_cfg("localhost", 27017, "user", "pass");
    assert_eq!(
        build_mongo_uri(&c, true),
        "mongodb://user:pass@localhost:27017/mydb?authSource=admin"
    );
}

#[test]
fn uri_standard_no_auth() {
    let c = uri_cfg("localhost", 27017, "", "");
    assert_eq!(build_mongo_uri(&c, true), "mongodb://localhost:27017/mydb");
}

#[test]
fn uri_srv_with_auth() {
    let c = uri_cfg("cluster.example.mongodb.net", 0, "user", "pass");
    assert_eq!(
        build_mongo_uri(&c, true),
        "mongodb+srv://user:pass@cluster.example.mongodb.net/mydb?authSource=admin"
    );
}

#[test]
fn uri_srv_no_db_for_dryrun() {
    let c = uri_cfg("cluster.example.mongodb.net", 0, "user", "pass");
    assert_eq!(
        build_mongo_uri(&c, false),
        "mongodb+srv://user:pass@cluster.example.mongodb.net/?authSource=admin"
    );
}

#[test]
fn uri_options_authsource_replicaset_tls() {
    let mut c = uri_cfg("localhost", 27017, "user", "pass");
    c.options.insert("auth_source".into(), "myauthdb".into());
    c.options.insert("replica_set".into(), "rs0".into());
    c.options.insert("tls".into(), serde_json::Value::Bool(true));
    assert_eq!(
        build_mongo_uri(&c, true),
        "mongodb://user:pass@localhost:27017/mydb?authSource=myauthdb&replicaSet=rs0&tls=true"
    );
}

#[test]
fn uri_multi_host_replica_set() {
    let mut c = uri_cfg(
        "mongodb0.example.internal:27017,mongodb1.example.internal:27017,mongodb2.example.internal:27017",
        0,
        "myDatabaseUser",
        "D1fficultP@ssw0rd",
    );
    c.database = "myDB".into();
    c.options.insert("replica_set".into(), "myRepl".into());
    assert_eq!(
        build_mongo_uri(&c, true),
        "mongodb://myDatabaseUser:D1fficultP%40ssw0rd@mongodb0.example.internal:27017,mongodb1.example.internal:27017,mongodb2.example.internal:27017/myDB?authSource=admin&replicaSet=myRepl"
    );
}

#[test]
fn uri_default_authsource_when_auth() {
    let c = uri_cfg("localhost", 27017, "user", "pass");
    assert_eq!(
        build_mongo_uri(&c, true),
        "mongodb://user:pass@localhost:27017/mydb?authSource=admin"
    );
}

#[test]
fn uri_encodes_special_chars_in_credentials() {
    let c = uri_cfg("cluster.example.mongodb.net", 0, "user", "p@ss:w/rd?");
    assert_eq!(
        build_mongo_uri(&c, true),
        "mongodb+srv://user:p%40ss%3Aw%2Frd%3F@cluster.example.mongodb.net/mydb?authSource=admin"
    );
}
