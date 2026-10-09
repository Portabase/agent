//! Sync mode (rclone replica). Pure helpers first; integration tests need `rclone` on PATH.

use crate::services::sync::excludes::sync_excludes;
use crate::services::sync::stats::{SyncStats, parse_log};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn sync_excludes_translate_p0_patterns() {
    let got = sync_excludes(&strings(&[
        "*.log", "node_modules/", "/build/", "**/tmp", "/docs/**/*.md", "[!a].txt", "{x,y}.dat", "  ", "/",
    ]));
    assert_eq!(
        got,
        strings(&[
            "*.log", "*.log/**",
            "node_modules", "node_modules/**",
            "/build", "/build/**",
            "tmp", "tmp/**",
            "/docs/**/*.md", "/docs/**/*.md/**", "/docs/*.md", "/docs/*.md/**",
            "[^a].txt", "[^a].txt/**",
            "{x,y}.dat", "{x,y}.dat/**",
        ])
    );
}

#[test]
fn every_double_star_gap_gets_a_zero_directory_variant() {
    assert_eq!(
        sync_excludes(&strings(&["a/**/b/**/c"])),
        strings(&[
            "a/**/b/**/c", "a/**/b/**/c/**",
            "a/b/**/c", "a/b/**/c/**",
            "a/**/b/c", "a/**/b/c/**",
            "a/b/c", "a/b/c/**",
        ])
    );
}

#[test]
fn anchored_leading_double_star_also_matches_the_root() {
    assert_eq!(sync_excludes(&strings(&["/**/x"])), strings(&["/**/x", "/**/x/**", "/x", "/x/**"]));
}

#[test]
fn escaped_bracket_is_left_alone() {
    assert_eq!(sync_excludes(&strings(&[r"\[!a]"])), strings(&[r"\[!a]", r"\[!a]/**"]));
}

#[test]
fn parse_log_reads_the_final_stats_and_error_lines() {
    let stderr = concat!(
        r#"{"level":"error","msg":"Failed to copy: couldn't copy from /src/secret: errno -1","source":"operations/copy.go:1","time":"2026-10-03T19:44:59+02:00"}"#, "\n",
        r#"{"level":"error","msg":"not deleting files as there were IO errors","source":"sync/sync.go:1","time":"2026-10-03T19:44:59+02:00"}"#, "\n",
        r#"{"level":"notice","msg":"\nTransferred: 0 B\n","source":"accounting/stats.go:1","stats":{"bytes":0,"checks":12,"deletes":0,"errors":1,"fatalError":false,"lastError":"couldn't copy from /src/secret: errno -1","transfers":0},"time":"2026-10-03T19:44:59+02:00"}"#, "\n",
    );
    let log = parse_log(stderr);
    assert_eq!(
        log.stats,
        Some(SyncStats { errors: 1, last_error: Some("couldn't copy from /src/secret: errno -1".into()), ..Default::default() })
    );
    assert_eq!(log.errors.len(), 2);
    assert_eq!(log.errors[1], "not deleting files as there were IO errors");
}

#[test]
fn parse_log_reads_a_successful_run() {
    let stderr = r#"{"level":"notice","msg":"\nTransferred: 2 B\n","source":"accounting/stats.go:1","stats":{"bytes":2,"checks":5,"deletes":1,"errors":0,"fatalError":false,"transfers":1},"time":"2026-10-03T19:45:10+02:00"}"#;
    let log = parse_log(stderr);
    assert_eq!(log.stats, Some(SyncStats { bytes: 2, transfers: 1, deletes: 1, ..Default::default() }));
    assert!(log.errors.is_empty());
}

#[test]
fn parse_log_keeps_plain_text_lines_as_errors() {
    let log = parse_log("Error: unknown flag: --bogus\n");
    assert!(log.stats.is_none());
    assert_eq!(log.errors, strings(&["Error: unknown flag: --bogus"]));
}

#[test]
fn parse_log_accepts_critical_error_level() {
    let stderr = r#"{"level":"critical","msg":"Failed to create file system for \":sftp,host=127.0.0.1,port=1,user=x:/tmp/dst\": NewFs: couldn't connect SSH: dial tcp 127.0.0.1:1: connect: connection refused","source":"cmd/cmd.go:148"}"#;
    let log = parse_log(stderr);
    assert!(log.stats.is_none());
    assert_eq!(log.errors.len(), 1);
    assert!(log.errors[0].starts_with("Failed to create file system"));
}

#[test]
fn parse_log_deduplicates_repeated_error_lines() {
    let stderr = concat!(
        r#"{"level":"error","msg":"connection timeout","source":"operations/copy.go:1","time":"2026-10-03T19:44:59+02:00"}"#, "\n",
        r#"{"level":"error","msg":"connection timeout","source":"operations/copy.go:1","time":"2026-10-03T19:44:59+02:00"}"#, "\n",
        r#"{"level":"error","msg":"connection timeout","source":"operations/copy.go:1","time":"2026-10-03T19:44:59+02:00"}"#, "\n",
    );
    let log = parse_log(stderr);
    assert_eq!(log.errors.len(), 1);
    assert_eq!(log.errors[0], "connection timeout");
}

#[test]
fn sync_excludes_collapses_repeated_double_stars() {
    assert_eq!(
        sync_excludes(&strings(&["a/**/**/b"])),
        strings(&["a/**/b", "a/**/b/**", "a/b", "a/b/**"])
    );
}

// ---------- integration: rclone on PATH, `alias` remote under a temp dir ----------

use crate::domain::files::platform::is_root;
use crate::services::backup::logger::JobLogger;
use crate::services::sync::rclone::{replica_target, sync_dir};
use crate::tests::domain::files::files_config;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn alias(dir: &Path) -> String {
    fs::create_dir_all(dir).unwrap();
    format!("[pb]\ntype = alias\nremote = {}\n", dir.display())
}

fn replica(store: &Path, generated_id: &str) -> PathBuf {
    store.join("backups/sync").join(generated_id).join("current")
}

/// `relative/path=content` for every regular file, sorted.
fn files(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| format!("{}={}", e.path().strip_prefix(root).unwrap().display(), fs::read_to_string(e.path()).unwrap()))
        .collect();
    out.sort();
    out
}

fn tree(root: &Path) {
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/a.txt"), "v1").unwrap();
    fs::write(root.join("docs/b.txt"), "b").unwrap();
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join("build/out.bin"), "out").unwrap();
    fs::write(root.join("app.log"), "log").unwrap();
}

#[test]
fn replica_target_follows_the_path_convention() {
    assert_eq!(replica_target("s3", "bucket", "backups", "g1"), "s3:bucket/backups/sync/g1/current");
    assert_eq!(replica_target("pb", "", "backups", "g1"), "pb:backups/sync/g1/current");
}

#[tokio::test]
async fn sync_mirrors_the_source_and_propagates_changes() {
    let store = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &["*.log", "/build"]);
    let config = alias(store.path());
    let logger = JobLogger::new();

    let first = sync_dir(&config, &replica_target("pb", "", "backups", &cfg.generated_id), &cfg, &logger).await.unwrap();
    assert_eq!(first.transfers, 2);
    assert_eq!(first.replica_bytes, Some(3), "excluded files are not counted");
    assert_eq!(files(&replica(store.path(), &cfg.generated_id)), vec!["docs/a.txt=v1", "docs/b.txt=b"]);

    fs::write(src.path().join("docs/a.txt"), "v2").unwrap();
    fs::remove_file(src.path().join("docs/b.txt")).unwrap();
    fs::write(src.path().join("new.txt"), "new").unwrap();

    let second = sync_dir(&config, &replica_target("pb", "", "backups", &cfg.generated_id), &cfg, &logger).await.unwrap();
    assert_eq!(second.transfers, 2);
    assert_eq!(second.deletes, 1);
    assert_eq!(second.replica_bytes, Some(5));
    assert_eq!(files(&replica(store.path(), &cfg.generated_id)), vec!["docs/a.txt=v2", "new.txt=new"]);
}

#[tokio::test]
async fn a_later_exclude_removes_files_from_the_replica() {
    let store = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &["*.log", "/build"]);
    let config = alias(store.path());
    let dest = replica_target("pb", "", "backups", &cfg.generated_id);
    sync_dir(&config, &dest, &cfg, &JobLogger::new()).await.unwrap();

    let mut later = cfg.clone();
    later.options = files_config(src.path(), &["*.log", "/build", "b.txt"]).options;
    let stats = sync_dir(&config, &dest, &later, &JobLogger::new()).await.unwrap();
    assert_eq!(stats.deletes, 1);
    assert_eq!(stats.replica_bytes, Some(2));
    assert_eq!(files(&replica(store.path(), &cfg.generated_id)), vec!["docs/a.txt=v1"]);
}

#[tokio::test]
async fn failed_sync_keeps_files_the_failed_run_would_delete() {
    if is_root() {
        return; // root reads a 000 file anyway
    }
    use std::os::unix::fs::PermissionsExt;
    let store = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &[]);
    let config = alias(store.path());
    sync_dir(&config, &replica_target("pb", "", "backups", &cfg.generated_id), &cfg, &JobLogger::new()).await.unwrap();

    fs::remove_file(src.path().join("docs/b.txt")).unwrap();
    let locked = src.path().join("docs/a.txt");
    // rclone only opens a file whose size/mtime changed, so the unreadable file must differ from the replica's copy.
    fs::write(&locked, "changed and now unreadable").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let err = sync_dir(&config, &replica_target("pb", "", "backups", &cfg.generated_id), &cfg, &JobLogger::new()).await.unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(err.to_string().contains("rclone sync failed"), "{err:#}");
    assert!(replica(store.path(), &cfg.generated_id).join("docs/b.txt").exists());
}

#[tokio::test]
async fn empty_source_is_refused_and_the_replica_is_untouched() {
    let store = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let cfg = files_config(src.path(), &[]);
    let err = sync_dir(&alias(store.path()), &replica_target("pb", "", "backups", &cfg.generated_id), &cfg, &JobLogger::new()).await.unwrap_err();
    assert!(err.to_string().contains("is empty"), "{err:#}");
    assert!(!store.path().join("backups").exists());
}

// ---------- sync as a backup method ----------

use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::sync::backup::replica_path;
use serde_json::json;

#[test]
fn replica_path_uses_the_channel_folder() {
    let custom: DatabaseStorage =
        serde_json::from_value(json!({ "id": "s", "provider": "s3", "folderName": "/pb/", "config": {} })).unwrap();
    assert_eq!(replica_path(&custom, "g1"), "pb/sync/g1/current");
    let default: DatabaseStorage =
        serde_json::from_value(json!({ "id": "s", "provider": "s3", "config": {} })).unwrap();
    assert_eq!(replica_path(&default, "g1"), "backups/sync/g1/current");
}

#[test]
fn local_sync_config_points_at_the_dashboard_webdav_server() {
    use crate::services::sync::backup::local_sync_config;
    use crate::utils::edge_key::EdgeKey;
    let edge_key = EdgeKey {
        server_url: "http://dash:8887/".into(),
        agent_id: "agent-1".into(),
        master_key_b64: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=".into(),
    };
    let text = local_sync_config(&edge_key).unwrap();
    assert!(
        text.starts_with(
            "[pblocal]\ntype = webdav\nurl = http://dash:8887/storage/sync\nvendor = rclone\nuser = portabase\npass = "
        ),
        "{text}"
    );
    assert!(!text.contains("8abfb788"), "the password must be obscured: {text}");
}
