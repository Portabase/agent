use crate::core::context::Context;
use crate::services::api::ApiClient;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::models::BackupResult;
use crate::services::config::DbType;
use crate::services::storage::StorageProvider;
use crate::services::storage::providers::s3::S3Provider;
use crate::tests::init_tracing_for_test;
use crate::tests::storage::rclone::start_minio;
use crate::utils::common::BackupMethod;
use crate::utils::edge_key::EdgeKey;

use aws_sdk_s3 as s3;
use std::io::Write as _;
use std::sync::Arc;
use tempfile::NamedTempFile;

const BUCKET: &str = "portabase-s3";

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

fn storage_for(endpoint: &str, bucket: &str) -> DatabaseStorage {
    let url = url::Url::parse(endpoint).unwrap();
    serde_json::from_value(serde_json::json!({
        "id": "storage-s3",
        "provider": "s3",
        "folderName": "backups",
        "config": {
            "accessKey": "minioadmin",
            "secretKey": "minioadmin",
            "bucketName": bucket,
            "endPointUrl": url.host_str().unwrap(),
            "port": url.port().unwrap(),
            "ssl": false,
            "region": "us-east-1",
        }
    }))
    .unwrap()
}

fn sdk_client(endpoint: &str) -> s3::Client {
    let conf = s3::config::Builder::new()
        .credentials_provider(s3::config::Credentials::new(
            "minioadmin",
            "minioadmin",
            None,
            None,
            "test",
        ))
        .region(s3::config::Region::new("us-east-1"))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .behavior_version(s3::config::BehaviorVersion::latest())
        .build();
    s3::Client::from_conf(conf)
}

fn backup_result(file: &NamedTempFile) -> BackupResult {
    BackupResult {
        generated_id: "db-1".to_string(),
        db_type: DbType::Postgresql,
        status: "success".to_string(),
        backup_file: Some(file.path().to_path_buf()),
        code: None,
    }
}

#[tokio::test]
async fn provider_uploads_a_backup_that_reads_back_identically() {
    init_tracing_for_test();
    let (_container, endpoint) = start_minio().await;
    let client = sdk_client(&endpoint);
    client.create_bucket().bucket(BUCKET).send().await.unwrap();

    let payload: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&payload).unwrap();
    file.flush().unwrap();

    let result = S3Provider {}
        .upload(
            test_context(),
            backup_result(&file),
            BackupMethod::Automatic,
            &storage_for(&endpoint, BUCKET),
            Some(false),
            "backup-storage-1",
        )
        .await;

    assert!(result.success, "upload failed: {:?}", result.error);
    let key = result.remote_file_path.expect("remote path");
    assert!(key.starts_with("backups/"), "unexpected key {key}");

    let object = client.get_object().bucket(BUCKET).key(&key).send().await.unwrap();
    let body = object.body.collect().await.unwrap().into_bytes();
    assert_eq!(body.as_ref(), payload.as_slice());
}

#[tokio::test]
async fn provider_reports_failure_for_a_missing_bucket() {
    init_tracing_for_test();
    let (_container, endpoint) = start_minio().await;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(b"dump").unwrap();
    file.flush().unwrap();

    let result = S3Provider {}
        .upload(
            test_context(),
            backup_result(&file),
            BackupMethod::Automatic,
            &storage_for(&endpoint, "does-not-exist"),
            Some(false),
            "backup-storage-1",
        )
        .await;

    assert!(!result.success);
    assert!(result.remote_file_path.is_none());
    assert!(
        result.error.as_deref().unwrap_or_default().contains("NoSuchBucket"),
        "unexpected error: {:?}",
        result.error
    );
}
