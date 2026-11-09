use super::start_physical_cluster;
use crate::domain::postgres::cluster::physical::commands::{
    missing_config_files, output_dir, start_commands,
};
use crate::domain::postgres::cluster::physical::decision::{decide, Decision};
use crate::domain::postgres::cluster::physical::{backup, checks, manifest};
use crate::domain::postgres::connection;
use crate::services::api::models::agent::backup::ParentOffer;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use crate::tests::init_tracing_for_test;
use crate::utils::compress::decompress_large_tar_gz;
use chrono::{Duration, TimeZone, Utc};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

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
    assert!(matches!(
        decide(Some(&parent), Utc::now(), 7, None),
        Decision::Full { .. }
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
    let commands = start_commands(Path::new(&format!("{ts}/data")), 17);
    assert!(commands.contains(&format!("chown -R 999:999 '{ts}/data'")));
    assert!(commands.contains(&format!("chmod 700 '{ts}/data'")));
    assert!(commands.contains(&format!(
        "docker run -d --name portabase-restore-40875631 -v '{ts}/data':/var/lib/postgresql/data postgres:17"
    )));
    assert!(commands.contains(&format!(
        "/usr/lib/postgresql/17/bin/pg_ctl -D '{ts}/data' start"
    )));
}

#[test]
fn start_commands_for_18_mounts_parent() {
    let ts = timestamp_dir();
    let commands = start_commands(Path::new(&format!("{ts}/18/docker")), 18);
    assert!(commands.contains(&format!(
        "docker run -d --name portabase-restore-40875631 -v '{ts}':/var/lib/postgresql postgres:18"
    )));
}

#[test]
fn start_commands_quote_paths_with_spaces() {
    let commands = start_commands(Path::new("/my restore/it's/data"), 17);
    assert!(commands
        .iter()
        .any(|command| command.contains(r#"'/my restore/it'\''s/data'"#)));
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
    assert!(
        log_messages(logger)
            .iter()
            .any(|message| message.starts_with("Executed: pg_verifybackup"))
    );

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

#[test]
fn is_compression_unsupported_matches_build_errors() {
    for (stderr, expected) in [
        (
            "pg_basebackup: error: could not initiate base backup: ERROR:  zstd compression is not supported by this build",
            true,
        ),
        (
            "pg_basebackup: error: could not initiate base backup: ERROR:  gzip compression is not supported by this build",
            true,
        ),
        (
            "pg_basebackup: error: invalid compression specification: this build does not support compression with zstd",
            true,
        ),
        (
            "pg_basebackup: error: could not upload manifest: ERROR:  system identifier in backup manifest is 7706206809223479334, but database system identifier is 7706206809830854694",
            false,
        ),
        (
            "pg_basebackup: initiating base backup, waiting for checkpoint to complete",
            false,
        ),
        ("", false),
    ] {
        assert_eq!(backup::is_compression_unsupported(stderr), expected, "{stderr}");
    }
}
