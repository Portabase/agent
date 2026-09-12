use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::storage::providers::rclone::helpers::{remote_target, validate_config};
use crate::services::storage::providers::rclone::models::RcloneProviderConfig;
use crate::tests::init_tracing_for_test;
use crate::utils::file::full_file_path;

const OVH_CONFIG: &str = "[ovhcloud-rbx]\n\
                          type = s3\n\
                          provider = OVHcloud\n\
                          access_key_id = my_access\n\
                          secret_access_key = my_secret\n\
                          region = rbx\n\
                          endpoint = s3.rbx.io.cloud.ovh.net\n\
                          acl = private\n";

#[test]
fn config_deserializes_from_dashboard_camel_case() {
    init_tracing_for_test();

    let storage: DatabaseStorage = serde_json::from_value(serde_json::json!({
        "id": "storage-1",
        "provider": "rclone",
        "folderName": "backups",
        "config": {
            "configText": OVH_CONFIG,
            "remoteName": "ovhcloud-rbx",
            "remotePath": "my-bucket",
        }
    }))
    .unwrap();

    let config: RcloneProviderConfig = storage.config.try_into().unwrap();

    assert_eq!(config.remote_name, "ovhcloud-rbx");
    assert_eq!(config.remote_path, "my-bucket");
    assert!(config.config_text.contains("type = s3"));
}

#[test]
fn build_rclone_config_skips_empty_and_rejects_line_breaks() {
    use crate::services::storage::providers::rclone::helpers::build_rclone_config;

    let text = build_rclone_config(
        "sftp",
        &[
            ("type", "sftp".to_string()),
            ("host", "h".to_string()),
            ("port", "".to_string()),      // empty -> skipped
            ("user", "  u  ".to_string()), // trimmed
        ],
    )
    .unwrap();
    assert_eq!(text, "[sftp]\ntype = sftp\nhost = h\nuser = u\n");

    let err = build_rclone_config("sftp", &[("host", "a\nkey = injected".to_string())])
        .unwrap_err()
        .to_string();
    assert!(err.contains("line breaks"), "unexpected error: {err}");
}

#[test]
fn validate_config_accepts_the_target_remote() {
    assert!(validate_config(OVH_CONFIG, "ovhcloud-rbx").is_ok());
}

#[test]
fn validate_config_rejects_an_unknown_remote_name() {
    let err = validate_config(OVH_CONFIG, "typo").unwrap_err().to_string();
    assert!(err.contains("typo"), "unexpected error: {err}");
    assert!(err.contains("ovhcloud-rbx"), "error should list the available remotes: {err}");
}

#[test]
fn validate_config_rejects_local_backend() {
    let cfg = "[disk]\ntype = local\n";
    let err = validate_config(cfg, "disk").unwrap_err().to_string();
    assert!(err.contains("local"), "unexpected error: {err}");
}

#[test]
fn validate_config_rejects_alias_backend() {
    let cfg = "[shortcut]\ntype = alias\nremote = other:path\n";
    let err = validate_config(cfg, "shortcut").unwrap_err().to_string();
    assert!(err.contains("alias"), "unexpected error: {err}");
}

#[test]
fn validate_config_rejects_a_blocked_backend_in_a_chained_section() {
    let cfg = "[secret]\ntype = crypt\nremote = disk:vault\n\n[disk]\ntype = local\n";
    let err = validate_config(cfg, "secret").unwrap_err().to_string();
    assert!(err.contains("crypt"), "unexpected error: {err}");
    assert!(err.contains("secret"), "error should name the offending remote: {err}");

    let cfg = "[outer]\ntype = s3\nprovider = Minio\n\n[disk]\ntype = local\n";
    let err = validate_config(cfg, "outer").unwrap_err().to_string();
    assert!(err.contains("local"), "unexpected error: {err}");
    assert!(err.contains("disk"), "error should name the offending remote: {err}");
}

#[test]
fn validate_config_rejects_crypt_even_over_an_allowed_remote() {
    let cfg = format!("[secret]\ntype = crypt\nremote = ovhcloud-rbx:bucket\n\n{OVH_CONFIG}");
    let err = validate_config(&cfg, "secret").unwrap_err().to_string();
    assert!(err.contains("crypt"), "unexpected error: {err}");
}

#[test]
fn validate_config_rejects_a_wrapping_backend_pointing_at_a_bare_local_path() {
    for backend in ["crypt", "chunker", "compress", "union", "combine", "hasher"] {
        let cfg = format!("[sneaky]\ntype = {backend}\nremote = /etc\n");
        let err = validate_config(&cfg, "sneaky")
            .unwrap_err()
            .to_string();
        assert!(err.contains(backend), "{backend} must be rejected: {err}");
    }
}

#[test]
fn validate_config_rejects_backends_that_cannot_hold_a_backup() {
    for backend in ["memory", "http", "googlephotos"] {
        let cfg = format!("[nope]\ntype = {backend}\n");
        let err = validate_config(&cfg, "nope").unwrap_err().to_string();
        assert!(err.contains(backend), "{backend} must be rejected: {err}");
    }
}

#[test]
fn remote_path_is_a_prefix_ahead_of_the_backup_folder() {
    assert_eq!(
        remote_target("ovhcloud-rbx", "my-bucket", "backups/2026-09-09/x.tar.gz"),
        "ovhcloud-rbx:my-bucket/backups/2026-09-09/x.tar.gz"
    );

    // Deeper prefixes nest the same way.
    assert_eq!(
        remote_target("ovhcloud-rbx", "my-bucket/portabase", "backups/2026-09-09/x.tar.gz"),
        "ovhcloud-rbx:my-bucket/portabase/backups/2026-09-09/x.tar.gz"
    );
}

#[test]
fn remote_target_trims_surrounding_slashes_and_whitespace() {
    assert_eq!(
        remote_target("r", "  /my-bucket/  ", "a/b.bin"),
        "r:my-bucket/a/b.bin"
    );
}

#[test]
fn remote_target_handles_an_empty_remote_path() {
    assert_eq!(remote_target("r", "", "a/b.bin"), "r:a/b.bin");
    assert_eq!(remote_target("r", "   ", "a/b.bin"), "r:a/b.bin");
}

#[test]
fn an_empty_remote_path_falls_back_to_the_global_backup_folder() {
    let remote_file_path = full_file_path(&"x.tar.gz".to_string(), None);
    assert!(remote_file_path.starts_with("backups/"));

    assert_eq!(
        remote_target("ovhcloud-rbx", "", &remote_file_path),
        format!("ovhcloud-rbx:{remote_file_path}")
    );

    assert_eq!(
        remote_target("ovhcloud-rbx", "my-bucket", &remote_file_path),
        format!("ovhcloud-rbx:my-bucket/{remote_file_path}")
    );
}

use crate::services::storage::providers::rclone::helpers::{rcat, write_config};

use bytes::Bytes;
use futures::stream;
use std::process::Command;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

const BUCKET: &str = "portabase";

async fn start_minio() -> (testcontainers::ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("minio/minio", "latest")
        .with_exposed_port(9000.tcp())
        .with_wait_for(WaitFor::message_on_stderr("API:"))
        .with_env_var("MINIO_ROOT_USER", "minioadmin")
        .with_env_var("MINIO_ROOT_PASSWORD", "minioadmin")
        .with_cmd(["server", "/data"])
        .start()
        .await
        .unwrap();

    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(9000).await.unwrap();
    (container, format!("http://{host}:{port}"))
}

fn minio_config(endpoint: &str) -> String {
    format!(
        "[minio]\n\
         type = s3\n\
         provider = Minio\n\
         access_key_id = minioadmin\n\
         secret_access_key = minioadmin\n\
         endpoint = {endpoint}\n\
         region = us-east-1\n\
         force_path_style = true\n"
    )
}

fn rclone_ok(config_path: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("rclone")
        .arg("--config")
        .arg(config_path)
        .args(args)
        .output()
        .expect("rclone binary not found — is it installed in this image?");

    assert!(
        out.status.success(),
        "rclone {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    out.stdout
}

#[test]
fn write_config_creates_an_owner_only_file_with_the_exact_text() {
    use std::os::unix::fs::PermissionsExt;

    let file = write_config(OVH_CONFIG).unwrap();

    let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "config file must not be group/world readable");

    assert_eq!(std::fs::read_to_string(file.path()).unwrap(), OVH_CONFIG);
}

#[tokio::test]
async fn rcat_streams_a_multi_chunk_body_to_minio() {
    init_tracing_for_test();

    let (_container, endpoint) = start_minio().await;
    let config = write_config(&minio_config(&endpoint)).unwrap();

    rclone_ok(config.path(), &["mkdir", &format!("minio:{BUCKET}")]);

    let data = vec![7u8; 10 * 1024];
    let chunks: Vec<Result<Bytes, std::io::Error>> = data
        .chunks(1024)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();

    let target = remote_target("minio", BUCKET, "backups/2026-09-09/test.bin");

    rcat(config.path(), &target, Box::pin(stream::iter(chunks)))
        .await
        .unwrap();

    let got = rclone_ok(config.path(), &["cat", &target]);
    assert_eq!(got, data);
}

#[tokio::test]
async fn rcat_reports_rclone_stderr_when_the_remote_is_unreachable() {
    init_tracing_for_test();

    let config = write_config(&minio_config("http://127.0.0.1:1")).unwrap();

    let chunks: Vec<Result<Bytes, std::io::Error>> =
        vec![Ok(Bytes::from_static(&[0u8; 4096]))];

    let err = rcat(
        config.path(),
        "minio:portabase/x.bin",
        Box::pin(stream::iter(chunks)),
    )
    .await
    .expect_err("upload to an unreachable endpoint must fail");

    let msg = err.to_string();
    assert!(
        msg.contains("rclone rcat failed"),
        "the broken stdin pipe must not mask rclone's own error: {msg}"
    );
    let (_, stderr_part) = msg
        .rsplit_once(": ")
        .expect("bail message must carry rclone stderr after the exit status");
    assert!(
        !stderr_part.trim().is_empty(),
        "rclone stderr must be included: {msg}"
    );
}

#[tokio::test]
async fn rcat_aborts_the_upload_when_the_stream_fails() {
    init_tracing_for_test();

    let (_container, endpoint) = start_minio().await;
    let config = write_config(&minio_config(&endpoint)).unwrap();

    rclone_ok(config.path(), &["mkdir", &format!("minio:{BUCKET}")]);

    let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
        Ok(Bytes::from_static(&[1u8; 1024])),
        Err(std::io::Error::other("injected stream failure")),
    ];

    let target = remote_target("minio", BUCKET, "backups/2026-09-09/aborted.bin");

    let err = rcat(config.path(), &target, Box::pin(stream::iter(chunks)))
        .await
        .expect_err("a stream error must fail the upload");
    assert!(
        err.to_string().contains("backup stream failed"),
        "unexpected error: {err}"
    );

    let stat_out = rclone_ok(config.path(), &["lsjson", "--stat", &target]);
    let stat: serde_json::Value = serde_json::from_slice(&stat_out).unwrap();
    assert_eq!(
        stat["Name"], "",
        "rclone must not have finalized the truncated object: {stat}"
    );
    assert_eq!(stat["IsDir"], true, "a miss reports IsDir: true: {stat}");
}

use crate::core::context::Context;
use crate::services::api::ApiClient;
use crate::services::backup::models::BackupResult;
use crate::services::config::DbType;
use crate::services::storage::providers::rclone::RcloneProvider;
use crate::services::storage::{StorageProvider, get_provider};
use crate::utils::common::BackupMethod;
use crate::utils::edge_key::EdgeKey;

use std::io::Write as _;
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

fn storage_for(config_text: &str, remote_path: &str) -> DatabaseStorage {
    serde_json::from_value(serde_json::json!({
        "id": "storage-1",
        "provider": "rclone",
        "folderName": "backups",
        "config": {
            "configText": config_text,
            "remoteName": "minio",
            "remotePath": remote_path,
        }
    }))
    .unwrap()
}

#[test]
fn factory_resolves_the_rclone_provider_key() {
    let storage = storage_for(OVH_CONFIG, "bucket");
    assert!(
        get_provider(&storage).is_some(),
        "get_provider must recognise the \"rclone\" key"
    );
}

#[tokio::test]
async fn provider_uploads_an_unencrypted_backup_to_minio() {
    init_tracing_for_test();

    let (_container, endpoint) = start_minio().await;
    let config_text = minio_config(&endpoint);

    let bootstrap = write_config(&config_text).unwrap();
    rclone_ok(bootstrap.path(), &["mkdir", &format!("minio:{BUCKET}")]);

    let payload = vec![42u8; 64 * 1024];
    let mut backup_file = NamedTempFile::new().unwrap();
    backup_file.write_all(&payload).unwrap();
    backup_file.flush().unwrap();

    let storage = storage_for(&config_text, BUCKET);

    let result = RcloneProvider {}
        .upload(
            test_context(),
            BackupResult {
                generated_id: "db-1".to_string(),
                db_type: DbType::Postgresql,
                status: "success".to_string(),
                backup_file: Some(backup_file.path().to_path_buf()),
                code: None,
            },
            BackupMethod::Automatic,
            &storage,
            Some(false),
            "backup-storage-1",
        )
        .await;

    assert!(result.success, "upload failed: {:?}", result.error);
    assert_eq!(result.total_size, Some(payload.len() as u64));

    let remote_file_path = result.remote_file_path.expect("remote path must be reported");
    assert!(
        remote_file_path.starts_with("backups/"),
        "folder_name must prefix the path: {remote_file_path}"
    );

    let target = remote_target("minio", BUCKET, &remote_file_path);
    assert_eq!(rclone_ok(bootstrap.path(), &["cat", &target]), payload);
}

#[tokio::test]
async fn provider_refuses_a_blocked_backend_without_spawning_rclone() {
    init_tracing_for_test();

    let mut backup_file = NamedTempFile::new().unwrap();
    backup_file.write_all(b"payload").unwrap();
    backup_file.flush().unwrap();

    let storage = storage_for("[minio]\ntype = local\n", "bucket");

    let result = RcloneProvider {}
        .upload(
            test_context(),
            BackupResult {
                generated_id: "db-1".to_string(),
                db_type: DbType::Postgresql,
                status: "success".to_string(),
                backup_file: Some(backup_file.path().to_path_buf()),
                code: None,
            },
            BackupMethod::Automatic,
            &storage,
            Some(false),
            "backup-storage-1",
        )
        .await;

    assert!(!result.success);
    assert!(
        result.error.unwrap_or_default().contains("local"),
        "the error must name the rejected backend type"
    );
}
