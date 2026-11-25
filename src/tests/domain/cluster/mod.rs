mod backup;
mod database;
mod physical;
mod restore;

use crate::services::config::{DatabaseConfig, DbType};
use std::collections::HashMap;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use url::Host;

async fn start_cluster(user: &str) -> (ContainerAsync<Postgres>, DatabaseConfig) {
    let container = Postgres::default()
        .with_env_var("POSTGRES_DB", "postgres")
        .with_env_var("POSTGRES_USER", user)
        .with_env_var("POSTGRES_PASSWORD", "changeme")
        .with_tag("17")
        .start()
        .await
        .unwrap();
    let config = cluster_config(&container, user).await;
    (container, config)
}

async fn start_physical_cluster(
    tag: &str,
    summarize: bool,
) -> (ContainerAsync<Postgres>, DatabaseConfig) {
    let mut server_settings = vec!["-c", "fsync=off"];
    if summarize && tag.parse::<u32>().is_ok_and(|major| major >= 17) {
        server_settings.extend(["-c", "summarize_wal=on"]);
    }
    let container = Postgres::default()
        .with_env_var("POSTGRES_USER", "testuser")
        .with_env_var("POSTGRES_PASSWORD", "changeme")
        .with_copy_to(
            "/docker-entrypoint-initdb.d/replication.sh",
            b"echo 'host replication all all scram-sha-256' >> \"$PGDATA/pg_hba.conf\"\n".to_vec(),
        )
        .with_tag(tag)
        .with_cmd(server_settings)
        .start()
        .await
        .unwrap();
    let config = cluster_config(&container, "testuser").await;
    (container, config)
}

async fn cluster_config(container: &ContainerAsync<Postgres>, user: &str) -> DatabaseConfig {
    let host = container
        .get_host()
        .await
        .unwrap_or(Host::parse("127.0.0.1").unwrap());
    let port = container.get_host_port_ipv4(5432).await.unwrap_or(5432);

    DatabaseConfig {
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
        volume_name: "".to_string(),
        container_name: None,
        options: std::collections::HashMap::new(),
    }
}

fn env_for(cfg: &DatabaseConfig) -> HashMap<String, String> {
    let mut env = std::env::vars().collect::<HashMap<_, _>>();
    env.insert("PGPASSWORD".to_string(), cfg.password.clone());
    env
}
