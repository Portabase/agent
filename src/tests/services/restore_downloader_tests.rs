use crate::core::context::Context;
use crate::services::api::ApiClient;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::services::restore::RestoreService;
use crate::tests::init_tracing_for_test;
use crate::utils::edge_key::EdgeKey;

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn a_failing_download_is_retried_until_it_succeeds() {
    init_tracing_for_test();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/backups/archive.tar.gz"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/backups/archive.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"portabase-archive".to_vec()))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;

    let ctx = Context {
        edge_key: EdgeKey {
            server_url: server.uri(),
            agent_id: "agent-1".to_string(),
            master_key_b64: String::new(),
        },
        api: ApiClient::new(server.uri()),
    };

    let service = RestoreService::new(Arc::new(ctx));

    let temp_dir = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let url = format!("{}/backups/archive.tar.gz", server.uri());

    let downloaded = service
        .download_backup(&url, temp_dir.path(), Arc::clone(&logger), None)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&downloaded).unwrap(), b"portabase-archive");

    let entries = Arc::try_unwrap(logger).unwrap().into_entries();
    assert_eq!(entries.iter().filter(|e| e.level == "warn").count(), 2);
    assert!(
        entries
            .iter()
            .any(|e| e.message == "Backup download succeeded on attempt 3/3")
    );
}

#[tokio::test]
async fn a_restore_whose_download_fails_is_reported_as_failed_with_its_logs() {
    init_tracing_for_test();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/backups/archive.tar.gz"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/agent/agent-1/restore"))
        .and(body_partial_json(json!({ "generatedId": "database-1", "status": "failed" })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let ctx = Context {
        edge_key: EdgeKey {
            server_url: server.uri(),
            agent_id: "agent-1".to_string(),
            master_key_b64: String::new(),
        },
        api: ApiClient::new(server.uri()),
    };

    let config = DatabaseConfig {
        name: "Restore target".to_string(),
        database: "restore_target".to_string(),
        db_type: DbType::Postgresql,
        username: String::new(),
        password: String::new(),
        port: 5432,
        host: "localhost".to_string(),
        generated_id: "database-1".to_string(),
        path: String::new(),
        max_packet_size: String::new(),
        volume_name: String::new(),
        container_name: None,
        options: HashMap::new(),
    };

    let service = RestoreService::new(Arc::new(ctx));
    let url = format!("{}/backups/archive.tar.gz", server.uri());

    service.execute_restore(config, url, None).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let report: Value = requests
        .iter()
        .find(|request| request.url.path() == "/agent/agent-1/restore")
        .unwrap()
        .body_json()
        .unwrap();
    let messages: Vec<&str> = report["logs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["message"].as_str())
        .collect();

    assert!(messages.iter().any(|message| message.starts_with("Download failed")));
}
