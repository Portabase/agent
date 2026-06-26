use crate::domain::factory::DatabaseFactory;
use crate::services::config::{DatabaseConfig, DbType};
use std::path::Path;

fn cluster_config() -> DatabaseConfig {
    DatabaseConfig {
        name: "cluster".to_string(),
        database: "postgres".to_string(),
        db_type: DbType::PostgresqlCluster,
        username: "postgres".to_string(),
        password: "changeme".to_string(),
        port: 5432,
        host: "localhost".to_string(),
        generated_id: "40875631-e3d2-4dfe-a26b-2a347ecc64fd".to_string(),
        path: String::new(),
        max_packet_size: String::new(),
    }
}

#[tokio::test]
async fn factory_routes_cluster_for_backup_with_sql_extension() {
    let db = DatabaseFactory::create_for_backup(cluster_config()).await;
    assert_eq!(db.file_extension(), ".sql");
}

#[tokio::test]
async fn factory_routes_cluster_for_restore_with_sql_extension() {
    let db = DatabaseFactory::create_for_restore(cluster_config(), Path::new("dump.sql")).await;
    assert_eq!(db.file_extension(), ".sql");
}
