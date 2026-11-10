use super::start_physical_cluster;
use crate::core::context::Context;
use crate::domain::postgres::cluster::physical::commands::{
    missing_config_files, output_dir, start_commands,
};
use crate::domain::postgres::cluster::physical::decision::{decide, Decision};
use crate::domain::postgres::cluster::physical::restore::RestoredCluster;
use crate::domain::postgres::cluster::physical::{backup, checks, manifest, restore};
use crate::domain::postgres::connection;
use crate::services::api::ApiClient;
use crate::services::api::models::agent::backup::ParentOffer;
use crate::services::api::models::agent::status::RestoreFile;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use crate::services::restore::RestoreService;
use crate::settings::CONFIG;
use crate::tests::init_tracing_for_test;
use crate::utils::compress::decompress_large_tar_gz;
use crate::utils::edge_key::EdgeKey;
use crate::utils::file::encrypt_file_stream_gcm;
use base64::Engine;
use chrono::{Duration, TimeZone, Utc};
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use testcontainers::core::{IntoContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const GENERATED_ID: &str = "40875631-e3d2-4dfe-a26b-2a347ecc64fd";

fn manifest_fixture(system_identifier: u64) -> Vec<u8> {
    format!(
        r#"{{"PostgreSQL-Backup-Manifest-Version": 2, "System-Identifier": {system_identifier}, "Files": []}}"#
    )
    .into_bytes()
}

fn offer(chain_age_days: i64, encoded_manifest: String) -> ParentOffer {
    ParentOffer {
        backup_id: "parent-1".to_string(),
        chain_started_at: Utc::now() - Duration::days(chain_age_days),
        manifest: encoded_manifest,
    }
}

#[test]
fn manifest_round_trips() {
    let raw = manifest_fixture(1);
    let encoded = manifest::encode(&raw).unwrap();
    assert_eq!(manifest::decode(&encoded).unwrap(), raw);
}

#[test]
fn system_identifier_reads_pg17_manifest() {
    let raw = manifest_fixture(7311234567890123456);
    assert_eq!(manifest::system_identifier(&raw), Some(7311234567890123456));
    assert_eq!(manifest::system_identifier(b"not json"), None);
}

#[test]
fn decide_full_without_offer() {
    assert_eq!(
        decide(None, Utc::now(), 7, None),
        Decision::Full {
            reason: "no parent offered (first backup or chain reset by the dashboard)".to_string()
        }
    );
}

#[test]
fn decide_full_when_chain_too_old() {
    let parent = offer(7, manifest::encode(&manifest_fixture(1)).unwrap());
    match decide(Some(&parent), Utc::now(), 7, None) {
        Decision::Full { reason } => assert!(reason.contains("older than 7 day(s)")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn decide_full_when_manifest_undecodable() {
    let parent = offer(1, "%%%".to_string());
    match decide(Some(&parent), Utc::now(), 7, None) {
        Decision::Full { reason } => {
            assert!(reason.contains("could not be decoded: "), "{reason}");
            assert!(reason.len() > "parent manifest could not be decoded: ".len());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn decide_incremental_just_below_chain_age_boundary() {
    let parent = offer(6, manifest::encode(&manifest_fixture(1)).unwrap());
    assert!(matches!(
        decide(Some(&parent), Utc::now(), 7, None),
        Decision::Incremental { .. }
    ));
}

#[test]
fn decide_full_when_system_identifier_differs() {
    let parent = offer(1, manifest::encode(&manifest_fixture(1)).unwrap());
    match decide(Some(&parent), Utc::now(), 7, Some(2)) {
        Decision::Full { reason } => assert!(reason.contains("system identifier changed")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn decide_incremental_when_identifier_unknown() {
    let raw = manifest_fixture(1);
    let parent = offer(1, manifest::encode(&raw).unwrap());
    assert_eq!(
        decide(Some(&parent), Utc::now(), 7, None),
        Decision::Incremental {
            parent_backup_id: "parent-1".to_string(),
            parent_manifest: raw
        }
    );
}

#[test]
fn output_dir_layout_for_17_and_18() {
    let now = Utc.with_ymd_and_hms(2026, 11, 9, 10, 15, 0).unwrap();
    let root = Path::new("/restore");
    assert_eq!(
        output_dir(root, "abc", now, 17),
        Path::new("/restore/abc/20261109T101500Z/data")
    );
    assert_eq!(
        output_dir(root, "abc", now, 18),
        Path::new("/restore/abc/20261109T101500Z/18/docker")
    );
}

fn timestamp_dir() -> String {
    format!("/restore/{GENERATED_ID}/20261109T101500Z")
}

#[test]
fn start_commands_for_17() {
    let ts = timestamp_dir();
    assert_eq!(
        start_commands(Path::new(&format!("{ts}/data")), 17, GENERATED_ID),
        [
            format!("chown -R 999:999 '{ts}/data'"),
            format!("chmod 700 '{ts}/data'"),
            format!(
                "docker run -d --name portabase-restore-40875631 -v '{ts}/data':/var/lib/postgresql/data postgres:17"
            ),
            format!(
                "sudo chown -R postgres:postgres '{ts}/data' && sudo chmod 700 '{ts}/data' && sudo -u postgres /usr/lib/postgresql/17/bin/pg_ctl -D '{ts}/data' start"
            ),
        ]
    );
}

#[test]
fn start_commands_for_18_mounts_parent() {
    let ts = timestamp_dir();
    assert_eq!(
        start_commands(Path::new(&format!("{ts}/18/docker")), 18, GENERATED_ID),
        [
            format!("chown -R 999:999 '{ts}'"),
            format!("chmod 700 '{ts}/18/docker'"),
            format!(
                "docker run -d --name portabase-restore-40875631 -v '{ts}':/var/lib/postgresql postgres:18"
            ),
            format!(
                "sudo chown -R postgres:postgres '{ts}/18/docker' && sudo chmod 700 '{ts}/18/docker' && sudo -u postgres /usr/lib/postgresql/18/bin/pg_ctl -D '{ts}/18/docker' start"
            ),
        ]
    );
}

#[test]
fn start_commands_quote_paths_with_spaces() {
    let commands = start_commands(Path::new("/my restore/it's/data"), 17, GENERATED_ID);
    assert!(
        commands
            .iter()
            .all(|command| command.contains(r#"'/my restore/it'\''s/data'"#))
    );
}

#[test]
fn missing_config_files_lists_absent_files() {
    let data_dir = tempfile::tempdir().unwrap();
    assert_eq!(
        missing_config_files(data_dir.path()),
        vec!["postgresql.conf", "pg_hba.conf"]
    );
    std::fs::write(data_dir.path().join("postgresql.conf"), "").unwrap();
    assert_eq!(missing_config_files(data_dir.path()), vec!["pg_hba.conf"]);
}

async fn preflight_error(cfg: &DatabaseConfig) -> String {
    checks::preflight(cfg).await.unwrap_err().to_string()
}

async fn execute(cfg: &DatabaseConfig, sql: &str) {
    connection::connect(cfg)
        .await
        .unwrap()
        .batch_execute(sql)
        .await
        .unwrap();
}

fn full_decision() -> Decision {
    Decision::Full {
        reason: "test".to_string(),
    }
}

fn incremental_decision(parent_manifest: &[u8]) -> Decision {
    Decision::Incremental {
        parent_backup_id: "parent-1".to_string(),
        parent_manifest: parent_manifest.to_vec(),
    }
}

fn log_messages(logger: Arc<JobLogger>) -> Vec<String> {
    Arc::try_unwrap(logger)
        .unwrap()
        .into_entries()
        .into_iter()
        .map(|entry| entry.message)
        .collect()
}

async fn unpack_bundle(bundle: &Path, target: &Path) -> Vec<String> {
    let mut names: Vec<String> = decompress_large_tar_gz(bundle, target)
        .await
        .unwrap()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const ADD_ROWS: &str =
    "CREATE TABLE items AS SELECT generate_series(1, 10000) AS id, md5(random()::text) AS payload";

#[tokio::test]
async fn preflight_rejects_pg16() {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster("16", false).await;
    let message = preflight_error(&cfg).await;
    let version = message
        .strip_prefix("Physical backups need PostgreSQL 17 or later (server is ")
        .and_then(|rest| rest.strip_suffix("). Use the Archive method for this source."))
        .unwrap_or_else(|| panic!("{message}"));
    assert!(
        version.starts_with("16.") && !version.contains(' '),
        "{message}"
    );
}

#[tokio::test]
async fn preflight_rejects_role_without_replication() {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster("17", true).await;
    execute(&cfg, "CREATE ROLE appuser LOGIN PASSWORD 'changeme' NOSUPERUSER").await;
    let app_cfg = DatabaseConfig {
        username: "appuser".to_string(),
        ..cfg
    };
    assert_eq!(
        preflight_error(&app_cfg).await,
        "The role appuser lacks the REPLICATION privilege. Run: ALTER ROLE appuser WITH REPLICATION;"
    );
}

#[tokio::test]
async fn preflight_rejects_summarize_off() {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster("17", false).await;
    assert_eq!(
        preflight_error(&cfg).await,
        "WAL summarization is off. Run: ALTER SYSTEM SET summarize_wal = on; SELECT pg_reload_conf(); (managed PostgreSQL: set the summarize_wal parameter)"
    );
}

#[tokio::test]
async fn preflight_rejects_custom_tablespace() {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster("17", true).await;
    let client = connection::connect(&cfg).await.unwrap();
    client
        .batch_execute("SET allow_in_place_tablespaces = on")
        .await
        .unwrap();
    client
        .batch_execute("CREATE TABLESPACE extra LOCATION ''")
        .await
        .unwrap();
    assert_eq!(
        preflight_error(&cfg).await,
        "Custom tablespaces are not supported by physical backups (extra). Drop them or use the Archive method."
    );
}

#[tokio::test]
async fn full_then_incremental_produce_verified_bundles() {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster("17", true).await;
    let facts = checks::preflight(&cfg).await.unwrap();
    assert_eq!(facts.major, 17);
    assert!(facts.system_identifier.is_some());

    let full_dir = TempDir::new().unwrap();
    let full = backup::run(&cfg, &facts, full_decision(), full_dir.path(), Arc::new(JobLogger::new()))
        .await
        .unwrap();
    assert_eq!(full.parent_backup_id, None);
    assert_eq!(manifest::system_identifier(&full.manifest), facts.system_identifier);
    assert_eq!(
        manifest::decode(&manifest::encode(&full.manifest).unwrap()).unwrap(),
        full.manifest
    );
    assert!(!full_dir.path().join("backup").exists());

    execute(&cfg, ADD_ROWS).await;

    let incremental_dir = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let incremental = backup::run(
        &cfg,
        &facts,
        incremental_decision(&full.manifest),
        incremental_dir.path(),
        logger.clone(),
    )
    .await
    .unwrap();
    assert_eq!(incremental.parent_backup_id.as_deref(), Some("parent-1"));
    assert!(!incremental_dir.path().join("backup").exists());
    assert!(!incremental_dir.path().join("verify").exists());
    let messages = log_messages(logger);
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("Executed: pg_verifybackup"))
    );
    assert!(messages.iter().any(|message| message
        == "Waiting for the server's checkpoint before copying (spread checkpoint; can take up to checkpoint_timeout)"));

    let full_size = std::fs::metadata(&full.bundle).unwrap().len();
    let incremental_size = std::fs::metadata(&incremental.bundle).unwrap().len();
    assert!(incremental_size < full_size);

    let unpacked = TempDir::new().unwrap();
    let names = unpack_bundle(&incremental.bundle, unpacked.path()).await;
    assert_eq!(names, ["backup_manifest", "base.tar.zst", "pg_wal.tar.zst"]);
    assert_eq!(
        std::fs::read(unpacked.path().join("backup_manifest")).unwrap(),
        incremental.manifest
    );
}

#[tokio::test]
async fn incremental_after_server_reinit_falls_back_to_full() {
    init_tracing_for_test();
    let ((_old_container, old_cfg), (_new_container, new_cfg)) = tokio::join!(
        start_physical_cluster("17", true),
        start_physical_cluster("17", true)
    );
    let old_facts = checks::preflight(&old_cfg).await.unwrap();
    let old_dir = TempDir::new().unwrap();
    let old_full = backup::run(&old_cfg, &old_facts, full_decision(), old_dir.path(), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    let new_facts = checks::preflight(&new_cfg).await.unwrap();
    let new_dir = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let result = backup::run(
        &new_cfg,
        &new_facts,
        incremental_decision(&old_full.manifest),
        new_dir.path(),
        logger.clone(),
    )
    .await
    .unwrap();

    assert_eq!(result.parent_backup_id, None);
    assert!(result.bundle.is_file());
    assert_eq!(manifest::system_identifier(&result.manifest), new_facts.system_identifier);
    let messages = log_messages(logger);
    let fallback = messages
        .iter()
        .find(|message| message.ends_with("); taking a full backup instead"))
        .unwrap_or_else(|| panic!("{messages:#?}"));
    assert!(
        fallback.starts_with(
            "Incremental backup failed (pg_basebackup: error: could not upload manifest: ERROR:  system identifier in backup manifest is"
        ),
        "{fallback}"
    );
}

#[tokio::test]
async fn corrupted_incremental_fails_verification() {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster("17", true).await;
    let facts = checks::preflight(&cfg).await.unwrap();
    let full_dir = TempDir::new().unwrap();
    let full = backup::run(&cfg, &facts, full_decision(), full_dir.path(), Arc::new(JobLogger::new()))
        .await
        .unwrap();
    execute(&cfg, ADD_ROWS).await;
    let incremental_dir = TempDir::new().unwrap();
    let incremental = backup::run(
        &cfg,
        &facts,
        incremental_decision(&full.manifest),
        incremental_dir.path(),
        Arc::new(JobLogger::new()),
    )
    .await
    .unwrap();

    let backup_dir = TempDir::new().unwrap();
    unpack_bundle(&incremental.bundle, backup_dir.path()).await;
    let extracted = backup_dir.path().join("extracted");
    backup::extract_backup(backup_dir.path(), &extracted).unwrap();
    let logger = JobLogger::new();
    backup::verify(backup_dir.path(), &extracted, facts.major, &logger).unwrap();

    let tampered: PathBuf = walkdir::WalkDir::new(&extracted)
        .into_iter()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.file_name().to_string_lossy().starts_with("INCREMENTAL."))
        .expect("an INCREMENTAL file in the extracted backup")
        .into_path();
    let mut contents = std::fs::read(&tampered).unwrap();
    let last = contents.len() - 1;
    contents[last] ^= 0xff;
    std::fs::write(&tampered, contents).unwrap();

    let message = backup::verify(backup_dir.path(), &extracted, facts.major, &logger)
        .unwrap_err()
        .to_string();
    assert!(message.starts_with("pg_verifybackup failed"), "{message}");
    assert!(message.contains("checksum mismatch"), "{message}");
}

fn offline_cfg() -> DatabaseConfig {
    DatabaseConfig {
        name: "cluster".to_string(),
        database: "postgres".to_string(),
        db_type: DbType::PostgresqlCluster,
        username: String::new(),
        password: String::new(),
        port: 5432,
        host: "localhost".to_string(),
        generated_id: GENERATED_ID.to_string(),
        path: String::new(),
        max_packet_size: String::new(),
        volume_name: String::new(),
        container_name: None,
        options: HashMap::new(),
    }
}

fn fake_pg_basebackup(bin_dir: &Path, succeeding_compression: &str) -> PathBuf {
    let calls = bin_dir.join("calls");
    let script = bin_dir.join("pg_basebackup");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in --compress=*) echo \"$arg\" >> '{}'; [ \"$arg\" = '--compress={succeeding_compression}' ] && exit 0;; esac; done\necho 'pg_basebackup: error: refused in the server language' >&2\nexit 1\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    calls
}

#[test]
fn compression_falls_back_on_any_failure() {
    for (succeeding_compression, expected_calls, succeeded) in [
        ("server-gzip", "--compress=server-zstd\n--compress=server-gzip\n", true),
        (
            "never",
            "--compress=server-zstd\n--compress=server-gzip\n--compress=none\n",
            false,
        ),
    ] {
        let bin_dir = TempDir::new().unwrap();
        let calls = fake_pg_basebackup(bin_dir.path(), succeeding_compression);
        let work_dir = TempDir::new().unwrap();
        let logger = Arc::new(JobLogger::new());
        let outcome = backup::base_backup(
            &offline_cfg(),
            bin_dir.path(),
            &work_dir.path().join("backup"),
            None,
            &logger,
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(&calls).unwrap(), expected_calls);
        assert_eq!(outcome.is_ok(), succeeded);
        if let Err(stderr) = outcome {
            assert!(stderr.contains("refused in the server language"), "{stderr}");
        }
        let fallbacks = log_messages(logger)
            .into_iter()
            .filter(|message| message.ends_with("; trying the next compression method"))
            .count();
        assert_eq!(fallbacks, expected_calls.lines().count() - 1);
    }
}

const RESTORE_PATH_REFUSAL: &str = "RESTORE_PATH is not set or not writable; a physical restore needs a folder shared with the host:
volumes:
  - ./restores:/restores
environment:
  RESTORE_PATH: /restores";

fn api_context(server: &MockServer, master_key_b64: String) -> Arc<Context> {
    Arc::new(Context {
        edge_key: EdgeKey {
            server_url: server.uri(),
            agent_id: "agent-1".to_string(),
            master_key_b64,
        },
        api: ApiClient::new(server.uri()),
    })
}

#[tokio::test]
async fn restore_refuses_without_restore_path() {
    init_tracing_for_test();
    let parent = TempDir::new().unwrap();
    let missing = parent.path().join("missing");
    let read_only = parent.path().join("read-only");
    std::fs::create_dir(&read_only).unwrap();
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555)).unwrap();
    let regular_file = parent.path().join("file");
    std::fs::write(&regular_file, b"").unwrap();

    let mut refused_paths = vec![None, Some(""), missing.to_str(), regular_file.to_str()];
    if std::fs::write(read_only.join("probe"), b"").is_err() {
        refused_paths.push(read_only.to_str());
    }
    for restore_path in refused_paths {
        let message = restore::ensure_restore_root(restore_path)
            .unwrap_err()
            .to_string();
        assert_eq!(message, RESTORE_PATH_REFUSAL, "{restore_path:?}");
    }

    let writable = TempDir::new().unwrap();
    assert_eq!(
        restore::ensure_restore_root(writable.path().to_str()).unwrap(),
        writable.path()
    );
    assert_eq!(std::fs::read_dir(writable.path()).unwrap().count(), 0);

    assert_eq!(
        CONFIG.restore_path, None,
        "run this test without RESTORE_PATH"
    );
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agent/agent-1/restore"))
        .and(body_partial_json(
            json!({ "generatedId": GENERATED_ID, "status": "failed" }),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let service = RestoreService::new(api_context(&server, String::new()));
    let cfg = offline_cfg();
    let files = vec![RestoreFile {
        url: format!("{}/never/{GENERATED_ID}.tar.gz", server.uri()),
        size: None,
    }];
    service.execute_physical_restore(cfg, files).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let report: Value = requests
        .iter()
        .find(|request| request.url.path() == "/agent/agent-1/restore")
        .unwrap()
        .body_json()
        .unwrap();
    assert!(
        report["logs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["message"].as_str())
            .any(|message| message.ends_with(RESTORE_PATH_REFUSAL))
    );
}

fn fake_bundle(dir: &Path) -> PathBuf {
    let data_dir = dir.join("data");
    let backup_dir = dir.join("backup");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::write(data_dir.join("PG_VERSION"), "17\n").unwrap();
    let mut base = tar::Builder::new(std::fs::File::create(backup_dir.join("base.tar")).unwrap());
    base.append_dir_all("", &data_dir).unwrap();
    base.finish().unwrap();
    tar::Builder::new(std::fs::File::create(backup_dir.join("pg_wal.tar")).unwrap())
        .finish()
        .unwrap();
    std::fs::write(backup_dir.join("backup_manifest"), "{}").unwrap();
    let bundle = dir.join(format!("{GENERATED_ID}.tar.gz"));
    let mut bundle_builder = tar::Builder::new(flate2::write::GzEncoder::new(
        std::fs::File::create(&bundle).unwrap(),
        flate2::Compression::none(),
    ));
    bundle_builder.append_dir_all("", &backup_dir).unwrap();
    bundle_builder.into_inner().unwrap().finish().unwrap();
    bundle
}

#[tokio::test]
async fn rebuild_refuses_non_empty_output() {
    init_tracing_for_test();
    let work_dir = TempDir::new().unwrap();
    let bundle = fake_bundle(work_dir.path());
    let restore_root = TempDir::new().unwrap();
    let now = Utc::now();
    let occupied: Vec<PathBuf> = (-2..=30)
        .map(|offset| {
            let output = output_dir(
                restore_root.path(),
                GENERATED_ID,
                now + Duration::seconds(offset),
                17,
            );
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(output.join("keep"), b"previous restore").unwrap();
            output
        })
        .collect();

    let logger = Arc::new(JobLogger::new());
    let message = restore::rebuild(&[bundle], restore_root.path(), GENERATED_ID, logger.clone())
        .await
        .unwrap_err()
        .to_string();

    assert!(message.starts_with("Output directory "), "{message}");
    assert!(
        message.ends_with(" already exists and is not empty; nothing was overwritten"),
        "{message}"
    );
    for output in occupied {
        assert_eq!(
            std::fs::read(output.join("keep")).unwrap(),
            b"previous restore"
        );
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
    }
    assert_eq!(std::fs::read_dir(restore_root.path()).unwrap().count(), 1);
    assert!(
        !log_messages(logger)
            .iter()
            .any(|message| message.contains("pg_combinebackup"))
    );
}

async fn backup_chain(cfg: &DatabaseConfig) -> (Vec<PathBuf>, Vec<TempDir>) {
    let facts = checks::preflight(cfg).await.unwrap();
    let mut bundles = Vec::new();
    let mut work_dirs = Vec::new();
    let mut parent_manifest: Option<Vec<u8>> = None;
    for table in ["rows_a", "rows_b", "rows_c"] {
        let work_dir = TempDir::new().unwrap();
        let decision = parent_manifest
            .as_deref()
            .map_or_else(full_decision, incremental_decision);
        let taken = backup::run(
            cfg,
            &facts,
            decision,
            work_dir.path(),
            Arc::new(JobLogger::new()),
        )
        .await
        .unwrap();
        assert_eq!(taken.parent_backup_id.is_some(), parent_manifest.is_some());
        execute(
            cfg,
            &format!("CREATE TABLE {table} AS SELECT generate_series(1, 1000) AS id"),
        )
        .await;
        parent_manifest = Some(taken.manifest);
        bundles.push(taken.bundle);
        work_dirs.push(work_dir);
    }
    (bundles, work_dirs)
}

fn logged_mount(messages: &[String]) -> (String, String) {
    let docker_run = messages
        .iter()
        .flat_map(|message| message.lines())
        .find(|line| line.starts_with("docker run "))
        .unwrap_or_else(|| panic!("{messages:#?}"));
    let (source, target) = docker_run
        .split_once(" -v '")
        .and_then(|(_, mount)| mount.split_once("':"))
        .unwrap();
    (
        source.to_string(),
        target.split_whitespace().next().unwrap().to_string(),
    )
}

async fn assert_restored_tables(
    restored: &RestoredCluster,
    messages: &[String],
    source_cfg: &DatabaseConfig,
    expected_tables: &[&str],
) {
    assert!(messages.iter().any(|message| message
        == &format!(
            "Restored data directory: {}. In a container, this is the host folder mounted at RESTORE_PATH.",
            restored.data_dir.display()
        )));
    let start_entry = format!(
        "To start it, run the three Docker commands below, or the last command on a host install:\n{}",
        start_commands(&restored.data_dir, restored.major, GENERATED_ID).join("\n")
    );
    assert!(messages.contains(&start_entry), "{messages:#?}");
    assert!(restored.size > 0);

    let (source, target) = logged_mount(messages);
    let container = GenericImage::new("postgres", &restored.major.to_string())
        .with_exposed_port(5432.tcp())
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_mount(Mount::bind_mount(source, target))
        .start()
        .await
        .unwrap();
    let restored_cfg = DatabaseConfig {
        host: container.get_host().await.unwrap().to_string(),
        port: container.get_host_port_ipv4(5432).await.unwrap(),
        ..source_cfg.clone()
    };
    let client = connection::connect(&restored_cfg).await.unwrap();
    let tables: Vec<String> = client
        .query(
            "SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(tables, expected_tables);
    for table in expected_tables {
        let count: i64 = client
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 1000, "{table}");
    }
}

async fn chain_restore_round_trip(tag: &str) {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster(tag, true).await;
    let (bundles, _work_dirs) = backup_chain(&cfg).await;
    let restore_root = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let restored = restore::rebuild(&bundles, restore_root.path(), GENERATED_ID, logger.clone())
        .await
        .unwrap();
    assert_eq!(restored.major.to_string(), tag);
    assert!(bundles.iter().all(|bundle| !bundle.exists()));
    assert_eq!(std::fs::read_dir(restore_root.path()).unwrap().count(), 1);
    assert!(
        restored
            .data_dir
            .starts_with(restore_root.path().join(GENERATED_ID))
    );
    let messages = log_messages(logger);
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("Executed: pg_combinebackup"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("Executed: pg_verifybackup"))
    );
    assert_restored_tables(&restored, &messages, &cfg, &["rows_a", "rows_b"]).await;
}

#[tokio::test]
async fn chain_restore_round_trip_pg17() {
    chain_restore_round_trip("17").await;
}

#[tokio::test]
async fn chain_restore_round_trip_pg18() {
    chain_restore_round_trip("18").await;
}

#[tokio::test]
async fn encrypted_chain_restores() {
    init_tracing_for_test();
    let (_container, cfg) = start_physical_cluster("17", true).await;
    let (bundles, _work_dirs) = backup_chain(&cfg).await;
    let master_key_b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
    let server = MockServer::start().await;
    let mut files = Vec::new();
    for (index, bundle) in bundles[..2].iter().enumerate() {
        let encrypted = encrypt_file_stream_gcm(bundle.clone(), master_key_b64.clone())
            .await
            .unwrap()
            .map(|chunk| chunk.unwrap().to_vec())
            .collect::<Vec<_>>()
            .await
            .concat();
        let file_path = format!("/{index}/{GENERATED_ID}.tar.gz.enc");
        Mock::given(method("GET"))
            .and(path(file_path.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(encrypted.clone()))
            .expect(1)
            .mount(&server)
            .await;
        files.push(RestoreFile {
            url: format!("{}{file_path}", server.uri()),
            size: Some(encrypted.len().to_string()),
        });
    }

    let service = RestoreService::new(api_context(&server, master_key_b64));
    let restore_root = TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());
    let restored = service
        .rebuild_chain(files, restore_root.path(), GENERATED_ID, logger.clone())
        .await
        .unwrap();
    assert_eq!(std::fs::read_dir(restore_root.path()).unwrap().count(), 1);
    let messages = log_messages(logger);
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.starts_with("Decrypting "))
            .count(),
        2
    );
    assert_restored_tables(&restored, &messages, &cfg, &["rows_a"]).await;
}
