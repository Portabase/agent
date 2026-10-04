use crate::core::context::Context;
use crate::services::api::ApiClient;
use crate::services::config::ConfigService;
use crate::services::config::{DatabasesConfig, InputDatabaseConfig, build_config, files_method};
use crate::utils::edge_key::EdgeKey;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;

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
    let keep = opts
        .get("keep_ownership")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(keep, "should keep ownership when flag is true");

    // false → strip
    let mut opts2: HashMap<String, Value> = HashMap::new();
    opts2.insert("keep_ownership".to_string(), Value::Bool(false));
    let keep2 = opts2
        .get("keep_ownership")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(!keep2, "should strip when flag is false");

    // missing → strip
    let opts3: HashMap<String, Value> = HashMap::new();
    let keep3 = opts3
        .get("keep_ownership")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(!keep3, "should strip when key absent");

    // wrong type → strip
    let mut opts4: HashMap<String, Value> = HashMap::new();
    opts4.insert(
        "keep_ownership".to_string(),
        Value::String("yes".to_string()),
    );
    let keep4 = opts4
        .get("keep_ownership")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(!keep4, "should strip when value is not bool");
}

#[test]
fn parses_docker_volume_type() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "uploads",
                    "type": "docker-volume",
                    "volume_name": "myapp_uploads",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681",
                    "container_name": "myapp"
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert_eq!(cfg.databases[0].db_type.as_str(), "docker-volume");
    assert_eq!(cfg.databases[0].volume_name, "myapp_uploads");
    assert_eq!(cfg.databases[0].container_name.as_deref(), Some("myapp"));
}

#[test]
fn docker_volume_container_name_optional() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "uploads",
                    "type": "docker-volume",
                    "volume_name": "myapp_uploads",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert_eq!(cfg.databases[0].volume_name, "myapp_uploads");
    assert!(cfg.databases[0].container_name.is_none());
}

#[test]
fn docker_volume_requires_volume_name() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "uploads",
                    "type": "docker-volume",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let err = service
        .load(Some(file.path().to_str().unwrap()))
        .unwrap_err();
    assert!(err.contains("volume_name"), "error was: {err}");
}

#[test]
fn build_config_applies_type_defaults() {
    let input: InputDatabaseConfig = serde_json::from_str(
        r#"{
            "name": "cluster1",
            "type": "postgresql-cluster",
            "username": "postgres",
            "password": "p",
            "port": 5432,
            "host": "localhost",
            "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
        }"#,
    )
    .unwrap();

    let cfg = build_config(input).unwrap();
    assert_eq!(cfg.db_type.as_str(), "postgresql-cluster");
    assert_eq!(cfg.database, "postgres"); // cluster default
}

#[test]
fn build_config_rejects_missing_required_field() {
    let input: InputDatabaseConfig = serde_json::from_str(
        r#"{
            "name": "pg",
            "type": "postgresql",
            "username": "postgres",
            "port": 5432,
            "host": "localhost",
            "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
        }"#,
    )
    .unwrap();

    let err = build_config(input).unwrap_err();
    assert!(err.contains("password"), "unexpected error: {err}");
}

#[test]
fn load_optional_returns_empty_when_file_missing() {
    let service = ConfigService::new(test_context());
    let cfg = service.load_optional(Some("/nonexistent/path/does-not-exist.json"));
    assert!(cfg.databases.is_empty());
}

#[test]
fn databases_config_roundtrips_through_serde() {
    let input: InputDatabaseConfig = serde_json::from_str(
        r#"{
            "name": "pg",
            "type": "postgresql",
            "database": "app",
            "username": "postgres",
            "password": "secret",
            "port": 5432,
            "host": "localhost",
            "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
        }"#,
    )
    .unwrap();
    let cfg = build_config(input).unwrap();
    let wrapped = DatabasesConfig {
        databases: vec![cfg],
    };

    let json = serde_json::to_string(&wrapped).unwrap();
    let back: DatabasesConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.databases[0].name, "pg");
    assert_eq!(back.databases[0].db_type.as_str(), "postgresql");
    assert_eq!(back.databases[0].password, "secret");
}

#[test]
fn files_type_parses_with_options() {
    let input: InputDatabaseConfig = serde_json::from_str(
        r#"{
            "name": "shared",
            "type": "files",
            "path": "/data/files",
            "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681",
            "options": { "exclude": ["node_modules", "/cache"], "one_file_system": true }
        }"#,
    )
    .unwrap();

    let cfg = build_config(input).unwrap();
    assert_eq!(cfg.db_type.as_str(), "files");
    assert_eq!(cfg.path, "/data/files");
    assert_eq!(
        cfg.options["exclude"],
        serde_json::json!(["node_modules", "/cache"])
    );
    assert_eq!(cfg.options["one_file_system"], serde_json::json!(true));
}

#[test]
fn files_type_requires_path() {
    let input: InputDatabaseConfig = serde_json::from_str(
        r#"{ "name": "shared", "type": "files", "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681" }"#,
    )
    .unwrap();
    let err = build_config(input).unwrap_err();
    assert!(err.contains("path"), "unexpected error: {err}");
}

#[test]
fn files_type_rejects_relative_and_root_paths() {
    for bad in ["data/files", "/", "//"] {
        let json = format!(
            r#"{{ "name": "shared", "type": "files", "path": "{bad}", "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681" }}"#
        );
        let input: InputDatabaseConfig = serde_json::from_str(&json).unwrap();
        let err = build_config(input).unwrap_err();
        assert!(err.contains("absolute"), "path {bad:?} gave: {err}");
    }
}

fn files_input(options: &str) -> InputDatabaseConfig {
    serde_json::from_str(&format!(
        r#"{{ "name": "docs", "type": "files", "path": "/data/files",
               "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"{options} }}"#
    ))
    .unwrap()
}

#[test]
fn files_method_defaults_to_archive() {
    let cfg = build_config(files_input("")).unwrap();
    assert_eq!(files_method(&cfg), "archive");
}

#[test]
fn files_method_reads_snapshot_and_sync() {
    let snap = build_config(files_input(r#", "options": { "method": "snapshot" }"#)).unwrap();
    assert_eq!(files_method(&snap), "snapshot");
    let sync = build_config(files_input(r#", "options": { "method": "sync" }"#)).unwrap();
    assert_eq!(files_method(&sync), "sync");
}

#[test]
fn unknown_files_method_refuses_the_source() {
    let err = build_config(files_input(r#", "options": { "method": "mirror" }"#)).unwrap_err();
    assert!(err.contains("Unknown method"), "{err}");
    assert!(err.contains("docs"), "{err}");
}
