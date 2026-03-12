use oauth2::url;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use crate::services::config::{DatabaseConfig, DbType};
use url::Host;
use crate::domain::factory::DatabaseFactory;

#[tokio::test]
async fn postgres_ping_test() {

    let container = Postgres::default()
        .start()
        .await
        .unwrap();

    let host = container.get_host().await.unwrap_or(Host::parse("127.0.0.1").unwrap());
    let port = container.get_host_port_ipv4(5432).await.unwrap_or(5432) ;

    let config = DatabaseConfig {
        name: "My test Postgres Database".to_string(),
        database: "postgres".to_string(),
        db_type: DbType::Postgresql,
        username: "postgres".to_string(),
        password: "postgres".to_string(),
        port,
        host: host.to_string(),
        generated_id: "40875631-e3d2-4dfe-a26b-2a347ecc64fd".to_string(),
        path: "".to_string(),
    };

    let db = DatabaseFactory::create_for_backup(config.clone()).await;
    let reachable = db.ping().await.unwrap_or_else(|_| false);

    assert_eq!(reachable, true);
}