use super::{env_for, start_cluster};
use crate::domain::postgres::{cluster, connection};
use crate::services::backup::logger::JobLogger;
use crate::tests::init_tracing_for_test;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn produces_sql_with_roles_and_databases() {
    init_tracing_for_test();
    let (_c, cfg) = start_cluster("testuser").await;

    let dir = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let sql = cluster::backup::run(cfg.clone(), dir.path().to_path_buf(), env_for(&cfg), logger)
        .await
        .unwrap();

    assert!(sql.is_file());
    let contents = std::fs::read_to_string(&sql).unwrap();
    assert!(contents.contains("CREATE ROLE"), "expected CREATE ROLE in dump");
    assert!(
        contents.contains("CREATE DATABASE") || contents.contains("\\connect"),
        "expected database statements in dump"
    );
}

#[tokio::test]
async fn requires_superuser() {
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
    let err = cluster::backup::run(weak.clone(), dir.path().to_path_buf(), env_for(&weak), logger)
        .await
        .unwrap_err();

    assert!(
        err.to_string().contains("superuser"),
        "expected a superuser error, got: {err}"
    );
}
