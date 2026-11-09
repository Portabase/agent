use crate::domain::postgres::cluster::physical::commands::{
    missing_config_files, output_dir, start_commands,
};
use crate::domain::postgres::cluster::physical::decision::{decide, Decision};
use crate::domain::postgres::cluster::physical::manifest;
use crate::services::api::models::agent::backup::ParentOffer;
use chrono::{Duration, TimeZone, Utc};
use std::path::Path;

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
