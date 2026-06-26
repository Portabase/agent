mod backup;
mod database;
mod restore;

use crate::services::config::{DatabaseConfig, DbType};
use std::collections::HashMap;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use url::Host;

/// Starts a fresh Postgres 17 cluster whose bootstrap superuser is `user`, and
/// returns the container guard plus a `postgresql-cluster` config pointing at it.
/// Shared by the `backup` and `restore` integration tests.
async fn start_cluster(user: &str) -> (ContainerAsync<Postgres>, DatabaseConfig) {
    let container = Postgres::default()
        .with_env_var("POSTGRES_DB", "postgres")
        .with_env_var("POSTGRES_USER", user)
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
        name: format!("cluster-{}", user),
        database: "postgres".to_string(),
        db_type: DbType::PostgresqlCluster,
        username: user.to_string(),
        password: "changeme".to_string(),
        port,
        host: host.to_string(),
        generated_id: "40875631-e3d2-4dfe-a26b-2a347ecc64fd".to_string(),
        path: "".to_string(),
        max_packet_size: "".to_string(),
    };
    (container, config)
}

fn env_for(cfg: &DatabaseConfig) -> HashMap<String, String> {
    let mut env = std::env::vars().collect::<HashMap<_, _>>();
    env.insert("PGPASSWORD".to_string(), cfg.password.clone());
    env
}
