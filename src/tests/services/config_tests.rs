use crate::core::context::Context;
use crate::services::api::ApiClient;
use crate::services::config::ConfigService;
use crate::utils::edge_key::EdgeKey;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;

// `ConfigService::load` never touches `self.ctx` on the `Some(file_path)` path,
// so the values here don't matter — but `Context::new()` panics without an
// `EDGE_KEY` env var, so build the struct directly (mirrors
// backup_uploader_tests.rs's `ctx_pointing_at`).
fn test_context() -> Arc<Context> {
    Arc::new(Context {
        edge_key: EdgeKey {
            server_url: String::new(),
            agent_id: "agent-1".to_string(),
            master_key_b64: String::new(),
        },
        api: ApiClient::new(String::new()),
    })
}

fn write_json(contents: &str) -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".json").unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file
}

#[test]
fn parses_postgresql_cluster_type() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "cluster1",
                    "type": "postgresql-cluster",
                    "username": "postgres",
                    "password": "p",
                    "port": 5432,
                    "host": "localhost",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert_eq!(cfg.databases[0].db_type.as_str(), "postgresql-cluster");
    // `database` is optional for cluster entries and defaults to "postgres".
    assert_eq!(cfg.databases[0].database, "postgres");
}

#[test]
fn postgresql_cluster_respects_explicit_database() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "cluster1",
                    "type": "postgresql-cluster",
                    "database": "maintenance",
                    "username": "postgres",
                    "password": "p",
                    "port": 5432,
                    "host": "localhost",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert_eq!(cfg.databases[0].database, "maintenance");
}
