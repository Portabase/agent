use serde_json::json;

use crate::services::api::models::agent::backup::{BackupResponse, BackupUploadResponse};
use crate::services::api::models::agent::restore::ResultRestoreResponse;
use crate::services::api::models::agent::status::PingResult;

#[test]
fn backup_response_deserializes_nested_backup_id() {
    let response: BackupResponse = serde_json::from_value(json!({
        "message": "created",
        "backup": {
            "id": "backup-123"
        }
    }))
    .unwrap();

    assert_eq!(response.message, "created");
    assert_eq!(response.backup.id, "backup-123");
}

#[test]
fn backup_upload_response_deserializes_storage_payload() {
    let response: BackupUploadResponse = serde_json::from_value(json!({
        "message": "uploaded",
        "backupStorage": {
            "id": "storage-456"
        }
    }))
    .unwrap();

    assert_eq!(response.message, "uploaded");
    assert_eq!(response.backup_storage.id, "storage-456");
}

#[test]
fn restore_response_deserializes_status() {
    let response: ResultRestoreResponse = serde_json::from_value(json!({
        "message": "ok",
        "status": true
    }))
    .unwrap();

    assert_eq!(response.message, "ok");
    assert!(response.status);
}

#[test]
fn ping_result_deserializes_and_normalizes_storage_config_keys() {
    let payload = json!({
        "agent": {
            "id": "agent-1",
            "lastContact": "2026-03-22T10:00:00Z"
        },
        "databases": [{
            "dbms": "postgres",
            "generatedId": "db-1",
            "storages": [{
                "id": "storage-1",
                "provider": "s3",
                "config": {
                    "bucketName": "agent-backups",
                    "nestedConfig": {
                        "regionName": "eu-west-3"
                    },
                    "allowedRegions": [
                        { "regionCode": "eu-west-3" }
                    ]
                }
            }],
            "encrypt": true,
            "data": {
                "backup": {
                    "action": true,
                    "cron": "*/5 * * * *"
                },
                "restore": {
                    "action": false,
                    "file": null,
                    "metaFile": null
                }
            }
        }]
    });

    let result: PingResult = serde_json::from_value(payload).unwrap();
    let storage = &result.databases[0].storages[0];

    assert_eq!(result.agent.id, "agent-1");
    assert_eq!(result.agent.last_contact, "2026-03-22T10:00:00Z");
    assert_eq!(result.databases[0].generated_id, "db-1");
    assert_eq!(storage.provider, "s3");
    assert_eq!(
        storage.config["bucket_name"].as_str(),
        Some("agent-backups")
    );
    assert_eq!(
        storage.config["nested_config"]["region_name"].as_str(),
        Some("eu-west-3")
    );
    assert_eq!(
        storage.config["allowed_regions"][0]["region_code"].as_str(),
        Some("eu-west-3")
    );
    assert_eq!(
        result.databases[0].data.backup.cron.as_deref(),
        Some("*/5 * * * *")
    );
    assert!(result.databases[0].data.backup.action);
    assert!(!result.databases[0].data.restore.action);
    assert!(result.databases[0].data.restore.file.is_none());
    assert!(result.databases[0].data.restore.meta_file.is_none());
}
