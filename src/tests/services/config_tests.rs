use crate::core::context::Context;
use crate::services::api::ApiClient;
use crate::services::config::ConfigService;
use crate::utils::edge_key::EdgeKey;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;

// `ConfigService::load` never touches `self.ctx` on the `Some(file_path)`
// path these tests use, so the values here don't matter — but
// `Context::new()` panics without an `EDGE_KEY` env var, so build the
// struct directly, the same way `backup_uploader_tests.rs`'s
// `ctx_pointing_at` helper does.
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
fn include_globals_defaults_to_false_when_absent() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "db1",
                    "database": "app",
                    "type": "postgresql",
                    "username": "u",
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

    assert_eq!(cfg.databases[0].include_globals, false);
}

#[test]
fn include_globals_true_is_parsed() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "db1",
                    "database": "app",
                    "type": "postgresql",
                    "username": "u",
                    "password": "p",
                    "port": 5432,
                    "host": "localhost",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681",
                    "include_globals": true
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert_eq!(cfg.databases[0].include_globals, true);
}

#[test]
fn include_globals_camel_case_alias_is_accepted() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "db1",
                    "database": "app",
                    "type": "postgresql",
                    "username": "u",
                    "password": "p",
                    "port": 5432,
                    "host": "localhost",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681",
                    "includeGlobals": true
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert_eq!(cfg.databases[0].include_globals, true);
}

#[test]
fn include_globals_ignored_for_non_postgres_types() {
    let file = write_json(
        r#"{
            "databases": [
                {
                    "name": "db1",
                    "type": "redis",
                    "port": 6379,
                    "host": "localhost",
                    "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681",
                    "include_globals": true
                }
            ]
        }"#,
    );

    let service = ConfigService::new(test_context());
    let cfg = service.load(Some(file.path().to_str().unwrap())).unwrap();

    assert_eq!(cfg.databases[0].include_globals, false);
}
