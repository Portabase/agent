use serde_json::json;

use crate::services::api::models::agent::backup::{BackupResponse, BackupUploadResponse};
use crate::services::api::models::agent::restore::ResultRestoreResponse;
use crate::services::api::models::agent::status::{DatabaseStatus, PingResult};

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

#[test]
fn database_status_legacy_plaintext_storages() {
    let status: DatabaseStatus = serde_json::from_value(json!({
        "dbms": "postgres",
        "generatedId": "gen-1",
        "storages": [ { "id": "s1", "config": { "bucket": "b" }, "provider": "s3" } ],
        "encrypt": true,
        "data": {
            "backup": { "action": false, "cron": null },
            "restore": { "action": false, "file": null, "metaFile": null, "size": null }
        }
    })).unwrap();

    assert_eq!(status.storages.len(), 1);
    assert_eq!(status.storages_encrypted, None);
    assert!(status.storages_ciphertext.is_none());
}

#[test]
fn database_status_encrypted_envelope() {
    let status: DatabaseStatus = serde_json::from_value(json!({
        "dbms": "postgres",
        "generatedId": "gen-1",
        "storages": [],
        "storages_encrypted": true,
        "storages_ciphertext": "AQIDBA==",
        "encrypt": true,
        "data": {
            "backup": { "action": true, "cron": null },
            "restore": { "action": false, "file": null, "metaFile": null, "size": null }
        }
    })).unwrap();

    assert!(status.storages.is_empty());
    assert_eq!(status.storages_encrypted, Some(true));
    assert_eq!(status.storages_ciphertext.as_deref(), Some("AQIDBA=="));
}

#[test]
fn database_status_defaults_config_fields_absent() {
    let json = r#"{
        "dbms": "postgresql",
        "generatedId": "16678159-ff7e-4c97-8c83-0adeff214681",
        "encrypt": false,
        "data": { "backup": { "action": false, "cron": null },
                  "restore": { "action": false, "file": null, "metaFile": null, "size": null } }
    }"#;
    let status: crate::services::api::models::agent::status::DatabaseStatus =
        serde_json::from_str(json).unwrap();
    assert_eq!(status.config_encrypted, None);
    assert!(status.config_ciphertext.is_none());
    assert!(status.resolved_config.is_none());
}

#[test]
fn resolve_dashboard_config_decrypts_full_entry() {
    use crate::services::status::resolve_dashboard_config;
    use base64::{engine::general_purpose, Engine};

    // 32-byte master key, base64 STANDARD (matches decrypt_json_gcm).
    let master_key_b64 = general_purpose::STANDARD.encode([7u8; 32]);

    // Full agent-entry shape the dashboard encrypts.
    let entry = r#"{
        "name": "Dashboard PG",
        "type": "postgresql",
        "database": "app",
        "username": "postgres",
        "password": "s3cret",
        "port": 5432,
        "host": "10.0.0.10",
        "generated_id": "16678159-ff7e-4c97-8c83-0adeff214681"
    }"#;
    let ciphertext = encrypt_json_gcm(entry.as_bytes(), &master_key_b64);

    let mut status: crate::services::api::models::agent::status::DatabaseStatus =
        serde_json::from_str(
            r#"{
                "dbms": "postgresql",
                "generatedId": "16678159-ff7e-4c97-8c83-0adeff214681",
                "encrypt": false,
                "config_encrypted": true,
                "config_ciphertext": "PLACEHOLDER",
                "data": { "backup": { "action": false, "cron": null },
                          "restore": { "action": false, "file": null, "metaFile": null, "size": null } }
            }"#,
        )
        .unwrap();
    status.config_ciphertext = Some(ciphertext);

    resolve_dashboard_config(&mut status, &master_key_b64).unwrap();

    let cfg = status.resolved_config.expect("resolved");
    assert_eq!(cfg.name, "Dashboard PG");
    assert_eq!(cfg.password, "s3cret");
    assert_eq!(cfg.host, "10.0.0.10");
    assert_eq!(cfg.db_type.as_str(), "postgresql");
}

#[test]
fn resolve_dashboard_config_noop_when_not_encrypted() {
    use crate::services::status::resolve_dashboard_config;
    let mut status: crate::services::api::models::agent::status::DatabaseStatus =
        serde_json::from_str(
            r#"{
                "dbms": "postgresql",
                "generatedId": "16678159-ff7e-4c97-8c83-0adeff214681",
                "encrypt": false,
                "data": { "backup": { "action": false, "cron": null },
                          "restore": { "action": false, "file": null, "metaFile": null, "size": null } }
            }"#,
        )
        .unwrap();
    resolve_dashboard_config(&mut status, "unused").unwrap();
    assert!(status.resolved_config.is_none());
}

#[test]
fn ping_without_engine_fields_defaults_to_none() {
    let status: DatabaseStatus = serde_json::from_value(json!({
        "dbms": "files",
        "generatedId": "16678159-ff7e-4c97-8c83-0adeff214681",
        "encrypt": false,
        "data": { "backup": { "action": false, "cron": null },
                  "restore": { "action": false, "file": null, "metaFile": null, "size": null } }
    }))
    .unwrap();
    assert!(status.data.backup.engine.is_none());
    assert!(status.data.restore.engine.is_none());
    assert!(status.data.restore.snapshot_id.is_none());
    assert!(status.data.restore.storage.is_none());
}

#[test]
fn resolve_restore_storage_decrypts_the_snapshot_channel() {
    use crate::services::status::resolve_restore_storage;
    use base64::{engine::general_purpose, Engine};

    let master_key_b64 = general_purpose::STANDARD.encode([7u8; 32]);
    let channel = r#"[{"id":"ch-1","provider":"s3","folderName":"backups","config":{"endPointUrl":"s3.example.com","bucketName":"b"}}]"#;
    let snapshot_id = "ab".repeat(32);

    let mut status: DatabaseStatus = serde_json::from_value(json!({
        "dbms": "files",
        "generatedId": "16678159-ff7e-4c97-8c83-0adeff214681",
        "encrypt": false,
        "data": {
            "backup": { "action": false, "cron": "0 * * * *", "engine": "restic" },
            "restore": { "action": true, "file": null, "metaFile": null, "size": null,
                         "engine": "restic", "snapshotId": snapshot_id,
                         "storageCiphertext": encrypt_json_gcm(channel.as_bytes(), &master_key_b64) }
        }
    }))
    .unwrap();

    assert_eq!(status.data.backup.engine.as_deref(), Some("restic"));
    assert_eq!(status.data.restore.snapshot_id.as_deref(), Some(snapshot_id.as_str()));

    resolve_restore_storage(&mut status, &master_key_b64).unwrap();

    let storage = status.data.restore.storage.expect("decrypted");
    assert_eq!(storage.id, "ch-1");
    assert_eq!(storage.provider, "s3");
    assert_eq!(storage.folder_name.as_deref(), Some("backups"));
}

#[test]
fn resolve_restore_storage_rejects_a_bad_ciphertext() {
    use crate::services::status::resolve_restore_storage;
    use base64::{engine::general_purpose, Engine};

    let mut status: DatabaseStatus = serde_json::from_value(json!({
        "dbms": "files",
        "generatedId": "16678159-ff7e-4c97-8c83-0adeff214681",
        "encrypt": false,
        "data": { "backup": { "action": false, "cron": null },
                  "restore": { "action": true, "engine": "restic", "storageCiphertext": "bm9wZQ==" } }
    }))
    .unwrap();
    assert!(resolve_restore_storage(&mut status, &general_purpose::STANDARD.encode([7u8; 32])).is_err());
    assert!(status.data.restore.storage.is_none());
}

fn encrypt_json_gcm(plaintext: &[u8], master_key_b64: &str) -> String {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    use base64::{engine::general_purpose, Engine};

    let key_bytes = general_purpose::STANDARD.decode(master_key_b64).unwrap();
    let key = Key::<Aes256Gcm>::try_from(key_bytes.as_slice()).unwrap();
    let cipher = Aes256Gcm::new(&key);
    let nonce_bytes = [0u8; 12];
    let nonce = Nonce::try_from(&nonce_bytes[..]).unwrap();
    let ct = cipher.encrypt(&nonce, plaintext).unwrap();
    let mut data = nonce_bytes.to_vec();
    data.extend_from_slice(&ct);
    general_purpose::STANDARD.encode(data)
}

#[test]
fn ping_payload_sends_method_only_when_set() {
    use crate::services::api::endpoints::status::DatabasePayload;
    let payload = |method: Option<&'static str>| DatabasePayload {
        name: "docs",
        dbms: "files",
        generated_id: "g",
        ping_status: true,
        method,
    };
    let with = serde_json::to_value(payload(Some("sync"))).unwrap();
    assert_eq!(with["method"], "sync");
    let without = serde_json::to_value(payload(None)).unwrap();
    assert!(without.get("method").is_none(), "{without}");
}

#[test]
fn upload_status_sends_sync_counters_only_when_set() {
    use crate::services::api::endpoints::agent::backup::upload::status::StatusUploadRequest;
    let request = |files_transferred: Option<u64>, files_deleted: Option<u64>| StatusUploadRequest {
        generated_id: "g".into(),
        backup_storage_id: "bs".into(),
        status: "success".into(),
        path: "p".into(),
        size: 3,
        backup_id: "b".into(),
        files_transferred,
        files_deleted,
    };
    let with = serde_json::to_value(request(Some(2), Some(1))).unwrap();
    assert_eq!(with["filesTransferred"], 2);
    assert_eq!(with["filesDeleted"], 1);
    let without = serde_json::to_value(request(None, None)).unwrap();
    assert!(without.get("filesTransferred").is_none() && without.get("filesDeleted").is_none(), "{without}");
}
