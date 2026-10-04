use crate::services::api::models::agent::status::DatabaseStorage;
use super::google_cloud_storage::start_fake_gcs;
use super::rclone::{rclone_ok, start_minio};
use crate::services::storage::providers::rclone::helpers::{obscure_password, write_config};
use crate::services::storage::providers::rclone::target::rclone_target;
use serde_json::{Value, json};

const INPUTS: &str = include_str!("fixtures/rclone-target/inputs.json");

pub(super) fn expected_ini(name: &str) -> &'static str {
    match name {
        "s3" => include_str!("fixtures/rclone-target/s3.ini"),
        "s3-port-no-ssl" => include_str!("fixtures/rclone-target/s3-port-no-ssl.ini"),
        "blob-account-key" => include_str!("fixtures/rclone-target/blob-account-key.ini"),
        "blob-endpoint-url" => include_str!("fixtures/rclone-target/blob-endpoint-url.ini"),
        "blob-connection-string" => include_str!("fixtures/rclone-target/blob-connection-string.ini"),
        "google-cloud-storage" => include_str!("fixtures/rclone-target/google-cloud-storage.ini"),
        "google-cloud-storage-emulator" => {
            include_str!("fixtures/rclone-target/google-cloud-storage-emulator.ini")
        }
        "google-drive" => include_str!("fixtures/rclone-target/google-drive.ini"),
        "rclone" => include_str!("fixtures/rclone-target/rclone.ini"),
        "sftp-key" => include_str!("fixtures/rclone-target/sftp-key.ini"),
        other => panic!("no expected fixture for {other}"),
    }
}

pub(super) fn storage(provider: &str, config: Value) -> DatabaseStorage {
    serde_json::from_value(json!({
        "id": "storage-1",
        "provider": provider,
        "folderName": "backups",
        "config": config,
    }))
    .unwrap()
}

/// Temp-file paths differ per run; normalize them before comparing. Keeps the exact
/// trailing newline so a missing final `\n` fails the golden test.
pub(super) fn normalize(config_text: &str) -> String {
    config_text
        .split('\n')
        .map(|l| {
            if l.starts_with("key_file = ") {
                "key_file = <KEY_FILE>".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn rclone_target_matches_golden_fixtures() {
    let inputs: Value = serde_json::from_str(INPUTS).unwrap();
    for (name, case) in inputs.as_object().unwrap() {
        let target = rclone_target(&storage(
            case["provider"].as_str().unwrap(),
            case["config"].clone(),
        ))
        .unwrap_or_else(|e| panic!("{name}: {e:#}"));

        assert_eq!(normalize(&target.config_text), expected_ini(name), "{name}: config_text");
        assert_eq!(target.remote_name, case["expected"]["remoteName"].as_str().unwrap(), "{name}: remote_name");
        assert_eq!(target.base_path, case["expected"]["basePath"].as_str().unwrap(), "{name}: base_path");
    }
}

#[test]
fn rclone_target_rejects_local_and_unknown_providers() {
    let err = rclone_target(&storage("local", json!({}))).err().unwrap();
    assert!(format!("{err:#}").contains("a local storage channel cannot be used as an rclone target"));

    let err = rclone_target(&storage("ftp-of-doom", json!({}))).err().unwrap();
    assert!(format!("{err:#}").contains("ftp-of-doom"));
}

#[test]
fn rclone_target_rejects_azure_without_account_key() {
    let err = rclone_target(&storage(
        "blob",
        json!({
            "authMode": "connectionString",
            "connectionString": "BlobEndpoint=https://acct.blob.core.windows.net;SharedAccessSignature=sv=2022&sig=x",
            "containerName": "c"
        }),
    ))
    .err()
    .unwrap();
    assert!(format!("{err:#}").contains("account key"), "{err:#}");
}

#[test]
fn rclone_target_applies_the_rclone_backend_blocklist() {
    let err = rclone_target(&storage(
        "rclone",
        json!({ "configText": "[disk]\ntype = local\n", "remoteName": "disk", "remotePath": "" }),
    ))
    .err()
    .unwrap();
    assert!(format!("{err:#}").contains("not allowed"), "{err:#}");
}

#[test]
fn sftp_key_file_lives_exactly_as_long_as_the_target() {
    let target = rclone_target(&storage(
        "sftp",
        json!({ "host": "h", "username": "u", "privateKey": "KEY", "remotePath": "" }),
    ))
    .unwrap();
    let key_path = target
        .config_text
        .lines()
        .find_map(|l| l.strip_prefix("key_file = "))
        .unwrap()
        .to_string();
    assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "KEY");
    drop(target);
    assert!(!std::path::Path::new(&key_path).exists(), "key file must be removed with the target");
}

#[test]
fn sftp_config_error_keeps_the_root_cause() {
    let err = rclone_target(&storage("sftp", json!({ "username": "u", "password": "p" })))
        .err()
        .unwrap();
    let text = format!("{err:#}");
    assert!(text.contains("invalid sftp storage config"), "{text}");
    assert!(text.contains("host"), "{text}");
}

#[test]
fn sftp_password_only_has_pass_and_no_key_file() {
    let target = rclone_target(&storage(
        "sftp",
        json!({ "host": "h", "username": "u", "password": "secret", "remotePath": "" }),
    ))
    .unwrap();
    assert!(!target.config_text.contains("key_file"), "{}", target.config_text);
    assert!(target.config_text.lines().any(|l| l.starts_with("pass = ")), "{}", target.config_text);
}

#[test]
fn obscured_password_round_trips_through_rclone_reveal() {
    let obscured = obscure_password("s3cr3t").unwrap();
    assert_ne!(obscured, "s3cr3t");

    let out = std::process::Command::new("rclone").args(["reveal", &obscured]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "s3cr3t");
}

#[tokio::test]
async fn s3_target_reaches_minio_through_rclone() {
    let (_container, endpoint) = start_minio().await; // "http://host:port"
    let host_port = endpoint.trim_start_matches("http://");
    let (host, port) = host_port.rsplit_once(':').unwrap();

    let target = rclone_target(&storage(
        "s3",
        json!({ "endPointUrl": host, "port": port.parse::<u16>().unwrap(), "ssl": false,
                "accessKey": "minioadmin", "secretKey": "minioadmin", "bucketName": "portabase" }),
    ))
    .unwrap();
    let config = write_config(&target.config_text).unwrap();

    // The mapper sets `no_check_bucket = true`, so create the bucket with a per-call override.
    let create = format!("{},no_check_bucket=false:{}", target.remote_name, target.base_path);
    rclone_ok(config.path(), &["mkdir", &create]);
    let listing = String::from_utf8(rclone_ok(config.path(), &["lsd", &format!("{}:", target.remote_name)])).unwrap();
    assert!(listing.contains("portabase"), "lsd output: {listing}");
}

#[tokio::test]
async fn gcs_emulator_target_reaches_fake_gcs_through_rclone() {
    let (_container, endpoint) = start_fake_gcs().await; // "http://host:port"

    let target = rclone_target(&storage(
        "google-cloud-storage",
        json!({ "projectId": "test", "bucketName": "bucket1", "clientEmail": "x@test",
                "privateKey": "", "apiEndpoint": endpoint }),
    ))
    .unwrap();
    let config = write_config(&target.config_text).unwrap();

    // Same per-call override as the s3 test: the mapper never creates buckets on its own.
    let create = format!("{},no_check_bucket=false:{}", target.remote_name, target.base_path);
    rclone_ok(config.path(), &["mkdir", &create]);
    let listing = String::from_utf8(rclone_ok(config.path(), &["lsd", &format!("{}:", target.remote_name)])).unwrap();
    assert!(listing.contains("bucket1"), "lsd output: {listing}");
}

#[tokio::test]
async fn s3_target_hosts_a_restic_repository() {
    use crate::services::backup::logger::JobLogger;
    use crate::services::restic::backup::snapshot;
    use crate::services::restic::command::ResticRepo;
    use base64::{Engine as _, engine::general_purpose};

    let (_container, endpoint) = start_minio().await; // "http://host:port"
    let host_port = endpoint.trim_start_matches("http://");
    let (host, port) = host_port.rsplit_once(':').unwrap();
    let channel = storage(
        "s3",
        json!({ "endPointUrl": host, "port": port.parse::<u16>().unwrap(), "ssl": false,
                "accessKey": "minioadmin", "secretKey": "minioadmin", "bucketName": "portabase" }),
    );
    let target = rclone_target(&channel).unwrap();
    let config = write_config(&target.config_text).unwrap();
    let create = format!("{},no_check_bucket=false:{}", target.remote_name, target.base_path);
    rclone_ok(config.path(), &["mkdir", &create]);

    let src = tempfile::TempDir::new().unwrap();
    std::fs::write(src.path().join("a.txt"), "a").unwrap();
    let cfg = crate::tests::domain::files::files_config(src.path(), &[]);
    let master_key_b64 = general_purpose::STANDARD.encode([7u8; 32]);

    let edge_key = crate::utils::edge_key::EdgeKey {
        server_url: String::new(),
        agent_id: "agent-1".into(),
        master_key_b64: master_key_b64.clone(),
    };
    let repo = ResticRepo::open(&channel, &cfg.generated_id, &edge_key).unwrap();
    assert_eq!(repo.repository(), format!("rclone:s3:portabase/backups/restic/{}", cfg.generated_id));
    let snap = snapshot(&repo, &cfg, "bs-1", &JobLogger::new()).await.unwrap();

    let listed = String::from_utf8(rclone_ok(
        config.path(),
        &["lsf", &format!("s3:portabase/backups/restic/{}/snapshots", cfg.generated_id)],
    ))
    .unwrap();
    assert!(listed.contains(&snap.snapshot_id), "{listed}");
}

#[tokio::test]
async fn s3_targets_host_one_sync_replica_per_channel() {
    use crate::services::backup::logger::JobLogger;
    use crate::services::sync::backup::one_storage;
    use crate::tests::services::backup_uploader_tests::ctx_pointing_at;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let (_container, endpoint) = start_minio().await; // "http://host:port"
    let host_port = endpoint.trim_start_matches("http://");
    let (host, port) = host_port.rsplit_once(':').unwrap();
    let channel = |id: &str, bucket: &str| -> DatabaseStorage {
        serde_json::from_value(json!({
            "id": id, "provider": "s3", "folderName": "backups",
            "config": { "endPointUrl": host, "port": port.parse::<u16>().unwrap(), "ssl": false,
                        "accessKey": "minioadmin", "secretKey": "minioadmin", "bucketName": bucket }
        }))
        .unwrap()
    };
    let channels = [channel("ch-a", "portabase"), channel("ch-b", "portabase2")];
    for c in &channels {
        let target = rclone_target(c).unwrap();
        let config = write_config(&target.config_text).unwrap();
        let create = format!("{},no_check_bucket=false:{}", target.remote_name, target.base_path);
        rclone_ok(config.path(), &["mkdir", &create]);
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent/agent-1/backup/upload/init"))
        .and(body_partial_json(json!({ "engine": "sync" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok", "backupStorage": { "id": "bs" } })))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/agent/agent-1/backup/upload/status"))
        .and(body_partial_json(json!({ "status": "success", "filesTransferred": 2, "filesDeleted": 0 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok", "backupStorage": { "id": "bs" } })))
        .expect(2)
        .mount(&server)
        .await;

    let src = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(src.path().join("docs")).unwrap();
    std::fs::write(src.path().join("a.txt"), "a").unwrap();
    std::fs::write(src.path().join("docs/b.txt"), "b").unwrap();
    let cfg = crate::tests::domain::files::files_config(src.path(), &[]);
    let ctx = ctx_pointing_at(server.uri());

    for c in &channels {
        let result = one_storage(&ctx, &cfg, c, "backup-1", &JobLogger::new()).await;
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.remote_file_path.as_deref(), Some(format!("backups/sync/{}/current", cfg.generated_id).as_str()));
    }
    for (c, bucket) in channels.iter().zip(["portabase", "portabase2"]) {
        let target = rclone_target(c).unwrap();
        let config = write_config(&target.config_text).unwrap();
        let listed = String::from_utf8(rclone_ok(
            config.path(),
            &["lsf", "-R", &format!("s3:{bucket}/backups/sync/{}/current", cfg.generated_id)],
        ))
        .unwrap();
        assert!(listed.contains("a.txt") && listed.contains("docs/b.txt"), "{bucket}: {listed}");
    }
}
