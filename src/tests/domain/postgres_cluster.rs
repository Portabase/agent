use crate::domain::postgres::{cluster, connection};
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
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

#[tokio::test]
async fn cluster_backup_produces_sql_with_roles_and_databases() {
    init_tracing_for_test();
    let (_c, cfg) = start_cluster("testuser").await;

    let dir = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let sql = cluster::backup(cfg.clone(), dir.path().to_path_buf(), env_for(&cfg), logger)
        .await
        .unwrap();

    assert!(sql.is_file());
    let contents = std::fs::read_to_string(&sql).unwrap();
    assert!(contents.contains("CREATE ROLE"), "expected CREATE ROLE in dump");
    assert!(contents.contains("CREATE DATABASE") || contents.contains("\\connect"),
            "expected database statements in dump");
}

#[tokio::test]
async fn cluster_backup_requires_superuser() {
    init_tracing_for_test();
    let (_c, super_cfg) = start_cluster("testuser").await;

    // Create a NON-superuser login role on the cluster.
    let client = connection::connect(&super_cfg).await.unwrap();
    client
        .batch_execute("CREATE ROLE appuser LOGIN PASSWORD 'changeme' NOSUPERUSER;")
        .await
        .unwrap();

    let mut weak = super_cfg.clone();
    weak.username = "appuser".to_string();

    let dir = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let err = cluster::backup(weak.clone(), dir.path().to_path_buf(), env_for(&weak), logger)
        .await
        .unwrap_err();

    assert!(
        err.to_string().contains("superuser"),
        "expected a superuser error, got: {err}"
    );
}

#[tokio::test]
async fn cluster_backup_restore_round_trip() {
    init_tracing_for_test();

    // Source cluster A: seed a role + a table owned by that role.
    let (_a, src) = start_cluster("testuser").await;
    let client = connection::connect(&src).await.unwrap();
    client
        .batch_execute(
            "CREATE ROLE appowner LOGIN PASSWORD 'changeme' NOSUPERUSER;\n\
             CREATE TABLE owned_tbl (id int);\n\
             ALTER TABLE owned_tbl OWNER TO appowner;",
        )
        .await
        .unwrap();

    let dir = TempDir::new().unwrap();
    let sql = cluster::backup(src.clone(), dir.path().to_path_buf(), env_for(&src), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    // Target cluster B: fresh, same bootstrap user.
    let (_b, mut dst) = start_cluster("testuser").await;

    cluster::restore(dst.clone(), sql.clone(), env_for(&dst), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    // Verify the seeded role exists and the table's owner was preserved on B.
    dst.database = "postgres".to_string();
    let bclient = connection::connect(&dst).await.unwrap();
    let role_exists: bool = bclient
        .query_one("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'appowner');", &[])
        .await
        .unwrap()
        .get(0);
    assert!(role_exists, "appowner role must be recreated on the target");

    let owner: String = bclient
        .query_one(
            "SELECT tableowner FROM pg_tables WHERE tablename = 'owned_tbl';",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(owner, "appowner", "table ownership must be preserved");
}
