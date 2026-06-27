use super::{env_for, start_cluster};
use crate::domain::postgres::{cluster, connection};
use crate::services::backup::logger::JobLogger;
use crate::tests::init_tracing_for_test;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn backup_restore_round_trip_preserves_ownership() {
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
    let sql = cluster::backup::run(src.clone(), dir.path().to_path_buf(), env_for(&src), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    // Target cluster B: fresh, same bootstrap user.
    let (_b, mut dst) = start_cluster("testuser").await;

    cluster::restore::run(dst.clone(), sql.clone(), env_for(&dst), Arc::new(JobLogger::new()))
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

#[tokio::test]
async fn requires_superuser() {
    init_tracing_for_test();
    let (_c, super_cfg) = start_cluster("testuser").await;

    // A non-superuser login role must be rejected before psql runs.
    let client = connection::connect(&super_cfg).await.unwrap();
    client
        .batch_execute("CREATE ROLE appuser LOGIN PASSWORD 'changeme' NOSUPERUSER;")
        .await
        .unwrap();

    let mut weak = super_cfg.clone();
    weak.username = "appuser".to_string();

    // The superuser pre-check happens before the dump file is read, so a
    // non-existent restore path is fine — it must never be touched.
    let missing = std::path::PathBuf::from("/nonexistent/cluster.sql");
    let err = cluster::restore::run(weak.clone(), missing, env_for(&weak), Arc::new(JobLogger::new()))
        .await
        .unwrap_err();

    assert!(
        err.to_string().contains("superuser"),
        "expected a superuser error, got: {err}"
    );
}
