use crate::services::backup::BackupService;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::tests::init_tracing_for_test;

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn sqlite_config(path: &str) -> DatabaseConfig {
    DatabaseConfig {
        name: "retry-test".to_string(),
        database: String::new(),
        db_type: DbType::Sqlite,
        username: String::new(),
        password: String::new(),
        port: 0,
        host: String::new(),
        generated_id: "retry-test-gen".to_string(),
        path: path.to_string(),
        max_packet_size: String::new(),
        volume_name: String::new(),
        container_name: None,
        options: HashMap::new(),
    }
}

#[tokio::test]
async fn a_failing_backup_is_retried_and_leaves_no_attempt_directory() {
    init_tracing_for_test();

    let temp_dir = TempDir::new().unwrap();
    let tmp_path = temp_dir.path();
    let logger = Arc::new(JobLogger::new());

    let cfg = sqlite_config("/nonexistent/definitely-not-here.sqlite");

    let result = BackupService::run(cfg, tmp_path, Arc::clone(&logger))
        .await
        .unwrap();

    assert_eq!(result.status, "failed");
    assert!(result.backup_file.is_none());

    let entries = Arc::try_unwrap(logger).unwrap().into_entries();
    assert_eq!(
        entries.iter().filter(|e| e.level == "warn").count(),
        2,
        "expected one warn per non-final failed attempt"
    );
    assert!(
        entries
            .iter()
            .any(|e| e.level == "error" && e.message.starts_with("Backup failed:")),
        "expected a single terminal error from the runner"
    );

    let leftovers: Vec<_> = std::fs::read_dir(tmp_path)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("attempt-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "failed attempt directories must be cleaned up, found {:?}",
        leftovers.iter().map(|e| e.file_name()).collect::<Vec<_>>()
    );
}

fn cluster_config(engine_options: &[(&str, &str)]) -> DatabaseConfig {
    let mut cfg = sqlite_config("");
    cfg.db_type = DbType::PostgresqlCluster;
    cfg.host = "127.0.0.1".to_string();
    cfg.port = 1;
    cfg.options = engine_options
        .iter()
        .map(|(key, value)| (key.to_string(), serde_json::Value::String(value.to_string())))
        .collect();
    cfg
}

async fn update_body_for_engine(engine: &str, cfg: DatabaseConfig) -> (Result<()>, Vec<wiremock::Request>) {
    use crate::services::backup::BackupService;
    use crate::tests::services::backup_uploader_tests::ctx_pointing_at;
    use crate::utils::common::BackupMethod;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent/agent-1/backup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "ok",
            "backup": { "id": "backup-1" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/agent/agent-1/backup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "ok",
            "backup": { "id": "backup-1" }
        })))
        .mount(&server)
        .await;

    let service = BackupService::new(Arc::new(ctx_pointing_at(server.uri())));
    let outcome = service
        .execute_backup(
            "routing-test-gen".to_string(),
            cfg,
            BackupMethod::Manual,
            vec![],
            false,
            engine.to_string(),
        )
        .await;

    let requests = server.received_requests().await.unwrap();
    (outcome, requests)
}

#[tokio::test]
async fn physical_source_routes_to_physical_branch() {
    init_tracing_for_test();

    let (_, requests) = update_body_for_engine("physical", cluster_config(&[])).await;
    let update = requests
        .iter()
        .find(|request| request.method.as_str() == "PATCH")
        .expect("an update request");
    let physical: serde_json::Value = serde_json::from_slice(&update.body).unwrap();
    assert_eq!(physical["status"], "failed");
    assert!(physical.get("manifest").is_none());
    assert!(
        physical["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["message"].as_str().unwrap().starts_with("Physical backup failed:")),
        "{physical}"
    );
}

#[tokio::test]
async fn archive_cluster_source_does_not_take_the_physical_branch() {
    init_tracing_for_test();

    let (outcome, requests) = update_body_for_engine("archive", cluster_config(&[("method", "physical")])).await;
    assert!(outcome.is_ok());
    assert!(requests.iter().any(|request| request.method.as_str() == "POST"));
    let update = requests
        .iter()
        .find(|request| request.method.as_str() == "PATCH")
        .expect("the archive path reports its failure");
    let body: serde_json::Value = serde_json::from_slice(&update.body).unwrap();
    assert_eq!(body["status"], "failed");
    assert!(body.get("manifest").is_none());
    assert!(body.get("parentBackupId").is_none());
    let messages: Vec<&str> = body["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["message"].as_str().unwrap())
        .collect();
    assert!(messages.contains(&"Database unreachable, backup aborted"), "{messages:?}");
    assert!(!messages.iter().any(|message| message.starts_with("Physical backup failed:")), "{messages:?}");
}
