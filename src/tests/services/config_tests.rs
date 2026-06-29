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

#[test]
fn postgresql_options_keep_ownership_parses() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "db1",
                    "type": "postgresql",
                    "username": "u",
                    "password": "p",
                    "port": 5432,
                    "host": "localhost",
                    "database": "mydb",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681",
                    "options": {
                        "keep_ownership": true
                    }
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    let keep = cfg.databases[0]
        .options
        .get("keep_ownership")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    assert!(keep);
}

#[test]
fn postgresql_options_absent_defaults_to_empty() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "db1",
                    "type": "postgresql",
                    "username": "u",
                    "password": "p",
                    "port": 5432,
                    "host": "localhost",
                    "database": "mydb",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert!(cfg.databases[0].options.is_empty());
}

#[test]
fn postgresql_options_non_bool_keep_ownership_falls_back_to_false() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "db1",
                    "type": "postgresql",
                    "username": "u",
                    "password": "p",
                    "port": 5432,
                    "host": "localhost",
                    "database": "mydb",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681",
                    "options": {
                        "keep_ownership": "yes"
                    }
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    let keep = cfg.databases[0]
        .options
        .get("keep_ownership")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    assert!(!keep);
}

#[test]
fn keep_ownership_extraction_logic() {
    use serde_json::Value;
    use std::collections::HashMap;

    // true → keep ownership
    let mut opts: HashMap<String, Value> = HashMap::new();
    opts.insert("keep_ownership".to_string(), Value::Bool(true));
    let keep = opts.get("keep_ownership").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(keep, "should keep ownership when flag is true");

    // false → strip
    let mut opts2: HashMap<String, Value> = HashMap::new();
    opts2.insert("keep_ownership".to_string(), Value::Bool(false));
    let keep2 = opts2.get("keep_ownership").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(!keep2, "should strip when flag is false");

    // missing → strip
    let opts3: HashMap<String, Value> = HashMap::new();
    let keep3 = opts3.get("keep_ownership").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(!keep3, "should strip when key absent");

    // wrong type → strip
    let mut opts4: HashMap<String, Value> = HashMap::new();
    opts4.insert("keep_ownership".to_string(), Value::String("yes".to_string()));
    let keep4 = opts4.get("keep_ownership").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(!keep4, "should strip when value is not bool");
}
