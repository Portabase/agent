//! Snapshots mode (restic). Pure helpers first; the integration tests further
//! down need `restic` and `rclone` on PATH.

use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::services::restic::command::is_snapshot_id;
use crate::services::restic::excludes::{backup_excludes, glob_escape, restore_excludes};
use crate::services::restic::json::{self, BackupSummary, Line};
use crate::services::restic::password::{derive_password, hkdf_sha256_32};
use crate::tests::domain::files::files_config;
use serde_json::json;
use std::path::Path;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn snapshot_ids_are_full_lowercase_hex() {
    assert!(is_snapshot_id(&"a1".repeat(32)));
    for bad in ["--password-command=touch /tmp/pwned", &"A".repeat(64), &"a".repeat(63), &"a".repeat(65), ""] {
        assert!(!is_snapshot_id(bad), "{bad}");
    }
}

#[test]
fn hkdf_matches_rfc5869_case_3_first_block() {
    let okm = hkdf_sha256_32(&[0x0b; 22], b"");
    assert_eq!(
        hex::encode(okm),
        "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d"
    );
}

#[test]
fn password_matches_the_dashboard_vector() {
    // Same vector as src/lib/restic/repo.ts in the dashboard (spike S4).
    let password = derive_password(
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=",
        "test-generated-id",
    )
    .unwrap();
    assert_eq!(
        password,
        "577e00efe6c953e8583d90e14d78109d805b616164279b8182282db76033b090"
    );
}

#[test]
fn password_rejects_a_non_base64_master_key() {
    assert!(derive_password("not base64!", "x").is_err());
}

#[test]
fn backup_excludes_are_rooted_at_the_source() {
    let got = backup_excludes(
        Path::new("/data/files"),
        &strings(&["*.log", "node_modules/", "/build/", "/docs/**/*.md", "  ", "/"]),
    )
    .unwrap();
    assert_eq!(
        got,
        strings(&[
            "/data/files/**/*.log",
            "/data/files/**/node_modules",
            "/data/files/build",
            "/data/files/docs/**/*.md",
        ])
    );
}

#[test]
fn backup_excludes_escape_glob_characters_in_the_root() {
    let got = backup_excludes(Path::new("/data/src[1]"), &strings(&["tmp"])).unwrap();
    assert_eq!(got, strings(&["/data/src\\[1\\]/**/tmp"]));
}

#[test]
fn restore_excludes_are_relative_to_the_snapshot_subfolder() {
    let got = restore_excludes(&strings(&["*.log", "/build/", "a/b"])).unwrap();
    assert_eq!(got, strings(&["/**/*.log", "/build", "/**/a/b"]));
}

#[test]
fn bracket_negation_is_rewritten_for_restic() {
    // globset reads `[!a]`; restic (Go filepath.Match) only knows `[^a]`.
    assert_eq!(
        backup_excludes(Path::new("/data"), &strings(&["[!a]*.log", "/x/[!0-9]", r"\[!a]", "[a[!]"])).unwrap(),
        strings(&["/data/**/[^a]*.log", "/data/x/[^0-9]", r"/data/**/\[!a]", "/data/**/[a[!]"])
    );
    assert_eq!(restore_excludes(&strings(&["/[!a]*", "[!a]"])).unwrap(), strings(&["/[^a]*", "/**/[^a]"]));
}

#[test]
fn brace_patterns_are_rejected_on_both_sides() {
    let patterns = strings(&["{x,y}.dat"]);
    let err = backup_excludes(Path::new("/data"), &patterns).unwrap_err();
    assert!(err.to_string().contains("brace patterns are not supported in Snapshots mode"));
    assert!(restore_excludes(&patterns).is_err());
}

#[test]
fn glob_escape_escapes_every_metacharacter() {
    assert_eq!(glob_escape(r"a*b?c[d]e\f"), r"a\*b\?c\[d\]e\\f");
}

#[test]
fn json_lines_are_classified() {
    // Recorded from restic 0.19.1 (paths shortened).
    let status = r#"{"message_type":"status","percent_done":0.9999997777778271,"total_files":4,"files_done":2,"total_bytes":9000002,"bytes_done":9000000,"error_count":1}"#;
    let error = r#"{"message_type":"error","error":{"message":"open /src/u: permission denied"},"during":"archival","item":"/src/u"}"#;
    let exit = r#"{"message_type":"exit_error","code":3,"message":"Warning: at least one source file could not be read"}"#;

    assert_eq!(json::parse(status), Line::Status { percent_done: 0.9999997777778272 });
    assert_eq!(
        json::parse(error),
        Line::Error { message: "open /src/u: permission denied".into(), item: Some("/src/u".into()) }
    );
    assert_eq!(
        json::parse(exit),
        Line::ExitError { code: 3, message: "Warning: at least one source file could not be read".into() }
    );
    assert_eq!(json::parse("Fatal: wrong password"), Line::Text("Fatal: wrong password".into()));
    assert_eq!(json::parse("[]"), Line::Text("[]".into()));
}

#[test]
fn backup_summary_deserializes_from_a_recorded_line() {
    let line = r#"{"message_type":"summary","files_new":3,"files_changed":0,"files_unmodified":0,"dirs_new":9,"dirs_changed":0,"dirs_unmodified":0,"data_blobs":8,"tree_blobs":10,"data_added":9005859,"data_added_packed":9004808,"total_files_processed":3,"total_bytes_processed":9000000,"total_duration":0.78928,"backup_start":"2026-10-03T16:59:06.569467+02:00","backup_end":"2026-10-03T16:59:07.359124+02:00","snapshot_id":"05ba7db8d08e4e097b869dbc511a8b8936298b2fcb01b221115789491204a470"}"#;
    let Line::Summary(value) = json::parse(line) else { panic!("not a summary") };
    let summary: BackupSummary = serde_json::from_value(value).unwrap();
    assert_eq!(summary.snapshot_id.len(), 64);
    assert_eq!(summary.data_added_packed, 9004808);
    assert_eq!(summary.total_bytes_processed, 9000000);
    assert_eq!(summary.files_new, 3);
}

#[test]
fn removed_files_come_from_the_parent_summary() {
    use crate::services::restic::json::{Snapshot, files_removed};
    let listing: Vec<Snapshot> = serde_json::from_value(serde_json::json!([
        {"id": "p", "paths": ["/src"], "summary": {"total_files_processed": 3, "files_new": 3}},
        {"id": "c", "paths": ["/src"], "parent": "p"},
        {"id": "orphan", "paths": ["/src"], "parent": "forgotten"},
        {"id": "old", "paths": ["/src"]},
        {"id": "child-of-old", "paths": ["/src"], "parent": "old"},
    ]))
    .unwrap();
    assert_eq!(files_removed(&listing, "c", 0, 2), Some(1));
    assert_eq!(files_removed(&listing, "p", 0, 0), Some(0), "no parent");
    assert_eq!(files_removed(&listing, "orphan", 0, 2), None, "parent not listed");
    assert_eq!(files_removed(&listing, "child-of-old", 0, 2), None, "parent without summary");
    assert_eq!(files_removed(&listing, "missing", 0, 2), None);
}

#[test]
fn a_local_channel_opens_the_dashboard_rest_repository() {
    use crate::utils::edge_key::EdgeKey;
    let storage: DatabaseStorage =
        serde_json::from_value(json!({ "id": "storage-1", "provider": "local", "config": {} })).unwrap();
    let edge_key = EdgeKey {
        server_url: "http://dash:8887/".into(),
        agent_id: "agent-1".into(),
        master_key_b64: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=".into(),
    };
    let repo = ResticRepo::open(&storage, "g1", &edge_key).unwrap();
    assert_eq!(repo.repository(), "rest:http://dash:8887/storage/restic/g1/");
    assert!(repo.is_append_only());
}

#[test]
fn local_storage_password_matches_the_dashboard_vector() {
    use crate::services::restic::password::local_storage_password;
    // Same vector as src/lib/local-storage/credential.ts in the dashboard.
    assert_eq!(
        local_storage_password("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap(),
        "8abfb788701b8c502658154746af6af768ea08f579381ad73b0f1e7dd43caea2"
    );
}

// ---------- integration: restic + rclone on PATH ----------

use crate::domain::files::platform::is_root;
use crate::services::restic::backup::snapshot;
use crate::services::restic::command::ResticRepo;
use crate::services::restic::restore::restore;
use std::fs;
use tempfile::TempDir;

/// A repository on an rclone `alias` remote pointing at a temp dir (test-only:
/// production channels always go through `rclone_target`).
fn local_repo(dir: &Path, generated_id: &str) -> ResticRepo {
    fs::create_dir_all(dir).unwrap();
    let config = format!("[pb]\ntype = alias\nremote = {}\n", dir.display());
    ResticRepo::new(&config, "pb", "", "backups", generated_id, "test-password", dir.join("cache")).unwrap()
}

fn tree(root: &Path) {
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/a.txt"), "v1").unwrap();
    fs::write(root.join("docs/b.txt"), "b").unwrap();
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join("build/out.bin"), "out").unwrap();
    fs::write(root.join("app.log"), "log").unwrap();
}

/// `relative/path=content` for every regular file, sorted.
fn files(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let rel = e.path().strip_prefix(root).unwrap().display().to_string();
            format!("{rel}={}", fs::read_to_string(e.path()).unwrap())
        })
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn first_snapshot_initializes_the_repository_and_the_second_reuses_its_parent() {
    let base = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &[]);
    let repo = local_repo(&base.path().join("repo"), &cfg.generated_id);
    let logger = JobLogger::new();

    let first = snapshot(&repo, &cfg, "bs-1", &logger).await.unwrap();
    assert_eq!(first.snapshot_id.len(), 64);
    assert_eq!(first.files_new, 4);
    assert_eq!(first.files_removed, Some(0));
    assert!(base.path().join("repo/backups/restic").join(&cfg.generated_id).join("config").exists());

    fs::write(src.path().join("docs/a.txt"), "v2").unwrap();
    let second = snapshot(&repo, &cfg, "bs-2", &logger).await.unwrap();
    assert_eq!(second.files_changed, 1);
    assert_eq!(second.files_unmodified, 3);
    assert_eq!(second.files_removed, Some(0));

    let tagged = repo.run(["snapshots", "--json", "--tag", "bs:bs-2"], &logger).await.unwrap();
    // The tag selects only the second snapshot, and its `parent` is the first
    // (a plain `contains(first)` check would match that `parent` field).
    let listed: serde_json::Value = serde_json::from_str(tagged.stdout.trim()).unwrap();
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1, "{}", tagged.stdout);
    assert_eq!(listed[0]["id"], second.snapshot_id.as_str());
    assert_eq!(listed[0]["parent"], first.snapshot_id.as_str());

    fs::remove_file(src.path().join("docs/b.txt")).unwrap();
    let third = snapshot(&repo, &cfg, "bs-3", &logger).await.unwrap();
    assert_eq!((third.files_new, third.files_changed, third.files_unmodified), (0, 0, 3));
    assert_eq!(third.files_removed, Some(1));
    assert_eq!(third.report()["filesRemoved"], 1);
}

#[tokio::test]
async fn restore_is_a_mirror_that_keeps_excluded_paths() {
    let base = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &["*.log", "/build"]);
    let repo = local_repo(&base.path().join("repo"), &cfg.generated_id);
    let logger = JobLogger::new();
    let snap = snapshot(&repo, &cfg, "bs-1", &logger).await.unwrap();

    fs::write(src.path().join("docs/a.txt"), "v2").unwrap();
    fs::remove_file(src.path().join("docs/b.txt")).unwrap();
    fs::write(src.path().join("new.txt"), "new").unwrap();
    fs::write(src.path().join("docs/new.log"), "kept").unwrap();
    fs::write(src.path().join("build/new.bin"), "kept").unwrap();

    restore(&repo, &cfg, &snap.snapshot_id, &logger).await.unwrap();

    assert_eq!(
        files(src.path()),
        vec![
            "app.log=log",
            "build/new.bin=kept",
            "build/out.bin=out",
            "docs/a.txt=v1",
            "docs/b.txt=b",
            "docs/new.log=kept",
        ]
    );
}

#[tokio::test]
async fn unreadable_file_fails_the_snapshot_and_forgets_it() {
    if is_root() {
        return; // root reads a 000 file anyway
    }
    use std::os::unix::fs::PermissionsExt;
    let base = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let locked = src.path().join("docs/b.txt");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let cfg = files_config(src.path(), &[]);
    let repo = local_repo(&base.path().join("repo"), &cfg.generated_id);
    let logger = JobLogger::new();

    let err = snapshot(&repo, &cfg, "bs-1", &logger).await.unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(err.to_string().contains("could not be read"), "{err:#}");
    let listed = repo.run(["snapshots", "--json"], &logger).await.unwrap();
    assert_eq!(listed.stdout.trim(), "[]");
}

#[tokio::test]
async fn unanchored_exclude_never_matches_directories_above_the_source() {
    // Regression for spike S5: the source lives under a directory named like the pattern.
    let base = TempDir::new().unwrap();
    let src = base.path().join("tmp/src");
    fs::create_dir_all(src.join("tmp")).unwrap();
    fs::write(src.join("kept.txt"), "k").unwrap();
    fs::write(src.join("tmp/dropped.txt"), "d").unwrap();
    let cfg = files_config(&src, &["tmp"]);
    let repo = local_repo(&base.path().join("repo"), &cfg.generated_id);

    let snap = snapshot(&repo, &cfg, "bs-1", &JobLogger::new()).await.unwrap();
    assert_eq!(snap.files_new, 1);
}

#[tokio::test]
async fn restoring_an_unknown_snapshot_fails_without_touching_the_target() {
    let base = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &[]);
    let repo = local_repo(&base.path().join("repo"), &cfg.generated_id);
    let logger = JobLogger::new();
    snapshot(&repo, &cfg, "bs-1", &logger).await.unwrap();
    fs::write(src.path().join("new.txt"), "new").unwrap();
    let before = files(src.path());

    let err = restore(&repo, &cfg, &"deadbeef".repeat(8), &logger).await.unwrap_err();

    assert!(err.to_string().contains("not found"), "{err:#}");
    assert_eq!(files(src.path()), before);
}

#[tokio::test]
async fn wrong_password_is_reported_explicitly() {
    let base = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &[]);
    let dir = base.path().join("repo");
    snapshot(&local_repo(&dir, &cfg.generated_id), &cfg, "bs-1", &JobLogger::new()).await.unwrap();

    let config = format!("[pb]\ntype = alias\nremote = {}\n", dir.display());
    let other = ResticRepo::new(&config, "pb", "", "backups", &cfg.generated_id, "other", dir.join("cache")).unwrap();
    let err = snapshot(&other, &cfg, "bs-2", &JobLogger::new()).await.unwrap_err();
    assert!(err.to_string().contains("wrong repository password"), "{err:#}");
}

#[tokio::test]
async fn an_unusable_cache_dir_runs_restic_without_cache() {
    let base = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    tree(src.path());
    let cfg = files_config(src.path(), &[]);
    let dir = base.path().join("repo");
    fs::create_dir_all(&dir).unwrap();
    let blocker = tempfile::NamedTempFile::new().unwrap(); // a regular file: nothing can be created below it
    let config = format!("[pb]\ntype = alias\nremote = {}\n", dir.display());
    let repo = ResticRepo::new(&config, "pb", "", "backups", &cfg.generated_id, "test-password", blocker.path().join("cache")).unwrap();
    snapshot(&repo, &cfg, "bs-1", &JobLogger::new()).await.unwrap();
}
