//! Regression test: a per-storage upload failure must be reported to the server via
//! `backup_upload_status("failed", ...)`. Previously the uploader early-returned on failure
//! and skipped the status call, so `backup_upload_init` opened a record that was never closed.

use crate::core::context::Context;
use crate::services::api::ApiClient;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::BackupService;
use crate::services::backup::logger::JobLogger;
use crate::services::backup::models::BackupResult;
use crate::services::config::DbType;
use crate::tests::init_tracing_for_test;
use crate::utils::common::BackupMethod;
use crate::utils::edge_key::EdgeKey;

use serde_json::json;
use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub(crate) fn ctx_pointing_at(base_url: String) -> Context {
    Context {
        edge_key: EdgeKey {
            server_url: String::new(),
            agent_id: "agent-1".to_string(),
            master_key_b64: String::new(),
        },
        api: ApiClient::new(base_url),
    }
}

#[tokio::test]
async fn failed_upload_reports_failed_status_to_server() {
    init_tracing_for_test();
    let server = MockServer::start().await;

    // init opens the per-storage record and returns its id.
    Mock::given(method("POST"))
        .and(path("/agent/agent-1/backup/upload/init"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "ok",
            "backupStorage": { "id": "bs-1" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    // The fix: on failure the uploader must PATCH the status as "failed".
    Mock::given(method("PATCH"))
        .and(path("/agent/agent-1/backup/upload/status"))
        .and(body_partial_json(json!({ "status": "failed" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let service = BackupService::new(Arc::new(ctx_pointing_at(server.uri())));

    // backup_file = None makes the provider fail immediately ("Missing backup file path"),
    // exercising the failure path without any network/Azure dependency.
    let result = BackupResult {
        generated_id: "gen-1".to_string(),
        db_type: DbType::Postgresql,
        status: "success".to_string(),
        backup_file: None,
        code: None,
    };

    let storage: DatabaseStorage = serde_json::from_value(json!({
        "id": "storage-1",
        "provider": "blob",
        "config": {}
    }))
    .unwrap();

    let backup_id = "backup-1".to_string();
    let logger = Arc::new(JobLogger::new());

    let results = service
        .upload(
            result,
            BackupMethod::Manual,
            vec![storage],
            false,
            &backup_id,
            logger,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);

    // MockServer drop verifies both `.expect(1)` mounts were hit — including the "failed" PATCH.
}

#[tokio::test]
async fn a_failing_upload_is_retried_until_it_succeeds() {
    init_tracing_for_test();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/agent/agent-1/backup/upload/init"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "ok",
            "backupStorage": { "id": "bs-1" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/tus/files"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .with_priority(1)
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/tus/files"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("Location", format!("{}/tus/files/upload-1", server.uri()).as_str()),
        )
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("PATCH"))
        .and(path("/tus/files/upload-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    Mock::given(method("PATCH"))
        .and(path("/agent/agent-1/backup/upload/status"))
        .and(body_partial_json(json!({ "status": "success" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": "ok",
            "backupStorage": { "id": "bs-1" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut backup_file = NamedTempFile::new().unwrap();
    backup_file.write_all(b"portabase-retry-test-payload").unwrap();
    backup_file.flush().unwrap();

    let ctx = Context {
        edge_key: EdgeKey {
            server_url: server.uri(),
            agent_id: "agent-1".to_string(),
            master_key_b64: String::new(),
        },
        api: ApiClient::new(server.uri()),
    };

    let service = BackupService::new(Arc::new(ctx));

    let result = BackupResult {
        generated_id: "gen-1".to_string(),
        db_type: DbType::Postgresql,
        status: "success".to_string(),
        backup_file: Some(backup_file.path().to_path_buf()),
        code: None,
    };

    let storage: DatabaseStorage = serde_json::from_value(json!({
        "id": "storage-1",
        "provider": "local",
        "config": {}
    }))
    .unwrap();

    let backup_id = "backup-1".to_string();
    let logger = Arc::new(JobLogger::new());

    let results = service
        .upload(
            result,
            BackupMethod::Manual,
            vec![storage],
            false,
            &backup_id,
            Arc::clone(&logger),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].success);

    let entries = Arc::try_unwrap(logger).unwrap().into_entries();
    assert_eq!(entries.iter().filter(|e| e.level == "warn").count(), 2);
    assert!(
        entries.iter().any(|e| e.message
            == "Upload to storage storage-1 succeeded on attempt 3/3")
    );
}
