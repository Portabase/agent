use crate::domain::files::matcher::ExcludeMatcher;
use std::path::Path;

fn matcher(patterns: &[&str]) -> ExcludeMatcher {
    let owned: Vec<String> = patterns.iter().map(|p| p.to_string()).collect();
    ExcludeMatcher::new(&owned).unwrap()
}

#[test]
fn matcher_unanchored_matches_any_depth() {
    let m = matcher(&["node_modules", "*.log"]);
    assert!(m.is_excluded(Path::new("node_modules")));
    assert!(m.is_excluded(Path::new("app/node_modules")));
    assert!(m.is_excluded(Path::new("x.log")));
    assert!(m.is_excluded(Path::new("a/b/x.log")));
    assert!(!m.is_excluded(Path::new("src/main.rs")));
}

#[test]
fn matcher_anchored_matches_root_only() {
    let m = matcher(&["/cache", "/build/"]);
    assert!(m.is_excluded(Path::new("cache")));
    assert!(!m.is_excluded(Path::new("app/cache")));
    assert!(m.is_excluded(Path::new("build")));
}

#[test]
fn matcher_star_does_not_cross_separator_but_double_star_does() {
    let m = matcher(&["/logs/*.txt", "/data/**/tmp"]);
    assert!(m.is_excluded(Path::new("logs/a.txt")));
    assert!(!m.is_excluded(Path::new("logs/sub/a.txt")));
    assert!(m.is_excluded(Path::new("data/tmp")));
    assert!(m.is_excluded(Path::new("data/a/b/tmp")));
}

#[test]
fn matcher_ignores_blank_patterns_and_rejects_invalid_ones() {
    let m = matcher(&["", "   "]);
    assert!(!m.is_excluded(Path::new("anything")));
    assert!(ExcludeMatcher::new(&["[".to_string()]).is_err());
}

use crate::domain::files::backup;
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, DbType};
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::sync::Arc;

pub(crate) fn files_config(root: &Path, exclude: &[&str]) -> DatabaseConfig {
    let mut options = HashMap::new();
    options.insert("exclude".to_string(), serde_json::json!(exclude));
    DatabaseConfig {
        name: "files-test".to_string(),
        database: String::new(),
        db_type: DbType::Files,
        username: String::new(),
        password: String::new(),
        port: 0,
        host: String::new(),
        generated_id: uuid::Uuid::new_v4().to_string(),
        path: root.to_string_lossy().into_owned(),
        max_packet_size: String::new(),
        volume_name: String::new(),
        container_name: None,
        options,
    }
}

fn sample_tree(root: &Path) {
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/a.txt"), "v1").unwrap();
    fs::create_dir_all(root.join("app/node_modules")).unwrap();
    fs::write(root.join("app/node_modules/dep.js"), "dep").unwrap();
    fs::write(root.join("app/index.js"), "app").unwrap();
    std::os::unix::fs::symlink("docs/a.txt", root.join("link")).unwrap();
}

#[tokio::test]
async fn backup_writes_tar_gz_without_excluded_paths() {
    let src = tempfile::TempDir::new().unwrap();
    sample_tree(src.path());
    let cfg = files_config(src.path(), &["node_modules"]);
    let out = tempfile::TempDir::new().unwrap();

    let archive = backup::run(
        cfg.clone(),
        out.path().to_path_buf(),
        Arc::new(JobLogger::new()),
    )
    .await
    .unwrap();

    assert_eq!(
        archive.file_name().unwrap().to_string_lossy(),
        format!("{}.tar.gz", cfg.generated_id)
    );

    let mut ar = tar::Archive::new(GzDecoder::new(fs::File::open(&archive).unwrap()));
    let mut files = Vec::new();
    let mut link_is_symlink = false;
    let mut a_txt = String::new();
    for entry in ar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry
            .path()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches('/')
            .to_string();
        if name == "link" {
            link_is_symlink = entry.header().entry_type().is_symlink();
        }
        if name == "docs/a.txt" {
            entry.read_to_string(&mut a_txt).unwrap();
        }
        files.push(name);
    }
    assert!(files.contains(&"docs/a.txt".to_string()));
    assert!(files.contains(&"app/index.js".to_string()));
    assert!(
        !files.iter().any(|n| n.contains("node_modules")),
        "excluded path archived: {files:?}"
    );
    assert!(link_is_symlink, "symlink must be stored as a link");
    assert_eq!(a_txt, "v1", "file content must be archived");
}

fn entry_names(archive: &Path) -> Vec<String> {
    let mut ar = tar::Archive::new(GzDecoder::new(fs::File::open(archive).unwrap()));
    ar.entries()
        .unwrap()
        .map(|e| {
            e.unwrap()
                .path()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches('/')
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn backup_does_not_archive_its_own_output() {
    let src = tempfile::TempDir::new().unwrap();
    sample_tree(src.path());
    let out_dir = src.path().join("backups");
    fs::create_dir_all(&out_dir).unwrap();
    let cfg = files_config(src.path(), &[]);

    let archive = backup::run(cfg, out_dir.clone(), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    assert!(archive.exists());
    let names = entry_names(&archive);
    assert!(names.contains(&"docs/a.txt".to_string()));
    assert!(
        !names.iter().any(|n| n.starts_with("backups")),
        "own output archived: {names:?}"
    );
}

#[tokio::test]
async fn backup_skips_sockets_with_a_warning() {
    let src = tempfile::TempDir::new().unwrap();
    sample_tree(src.path());
    let _listener = std::os::unix::net::UnixListener::bind(src.path().join("sock")).unwrap();
    let cfg = files_config(src.path(), &[]);
    let out = tempfile::TempDir::new().unwrap();

    let archive = backup::run(cfg, out.path().to_path_buf(), Arc::new(JobLogger::new()))
        .await
        .unwrap();

    let names = entry_names(&archive);
    assert!(names.contains(&"docs/a.txt".to_string()));
    assert!(
        !names.iter().any(|n| n == "sock"),
        "socket archived: {names:?}"
    );
}

#[tokio::test]
async fn backup_fails_when_source_is_not_a_directory() {
    let src = tempfile::TempDir::new().unwrap();
    let file = src.path().join("plain.txt");
    fs::write(&file, "x").unwrap();
    let cfg = files_config(&file, &[]);
    let out = tempfile::TempDir::new().unwrap();

    let result = backup::run(cfg, out.path().to_path_buf(), Arc::new(JobLogger::new())).await;

    assert!(
        result.is_err(),
        "a file path must not produce an empty archive"
    );
}

use crate::domain::files::restore;

#[tokio::test]
async fn restore_is_a_mirror_that_keeps_excluded_paths() {
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    let cfg = files_config(root, &["node_modules"]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();

    fs::write(root.join("docs/a.txt"), "v2").unwrap();
    fs::write(root.join("new.txt"), "new").unwrap();
    fs::create_dir_all(root.join("newdir/sub")).unwrap();
    fs::write(root.join("app/node_modules/dep.js"), "changed").unwrap();

    restore::run(cfg, archive, logger).await.unwrap();

    assert_eq!(fs::read_to_string(root.join("docs/a.txt")).unwrap(), "v1");
    assert_eq!(
        fs::read_to_string(root.join("app/index.js")).unwrap(),
        "app"
    );
    assert!(
        !root.join("new.txt").exists(),
        "file created after backup must be deleted"
    );
    assert!(
        !root.join("newdir").exists(),
        "dir created after backup must be deleted"
    );
    assert_eq!(
        fs::read_to_string(root.join("app/node_modules/dep.js")).unwrap(),
        "changed",
        "excluded path must be left untouched"
    );
    assert!(
        fs::symlink_metadata(root.join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn ensure_restorable_rejects_dangerous_targets() {
    assert!(restore::ensure_restorable(Path::new("/")).is_err());
    assert!(restore::ensure_restorable(Path::new("")).is_err());
    assert!(restore::ensure_restorable(Path::new("relative/dir")).is_err());
    assert!(restore::ensure_restorable(Path::new("/definitely/not/here")).is_err());
    let dir = tempfile::TempDir::new().unwrap();
    assert!(restore::ensure_restorable(dir.path()).is_ok());
    let escapes_to_root = dir.path().join("../../../../../../../../../..");
    assert!(restore::ensure_restorable(&escapes_to_root).is_err());
}

#[tokio::test]
async fn restore_refuses_corrupt_archive_without_touching_data() {
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    let cfg = files_config(root, &[]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();
    let len = fs::metadata(&archive).unwrap().len();
    fs::OpenOptions::new()
        .write(true)
        .open(&archive)
        .unwrap()
        .set_len(len / 2)
        .unwrap();
    fs::write(root.join("keep.txt"), "keep").unwrap();

    let result = restore::run(cfg, archive, logger).await;

    assert!(result.is_err(), "a truncated archive must be refused");
    assert!(
        root.join("keep.txt").exists(),
        "data must survive a refused restore"
    );
    assert!(
        root.join("docs/a.txt").exists(),
        "data must survive a refused restore"
    );
}

#[tokio::test]
async fn ping_reports_directory_reachability() {
    let dir = tempfile::TempDir::new().unwrap();
    let ok = files_config(dir.path(), &[]);
    assert!(crate::domain::files::ping::run(ok).await.unwrap());
    let missing = files_config(Path::new("/definitely/not/here"), &[]);
    assert!(!crate::domain::files::ping::run(missing).await.unwrap());
}

#[tokio::test]
async fn restore_survives_archive_stored_inside_the_source() {
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    let cfg = files_config(root, &[]);
    let tmp_dir = root.join(".tmp-restore");
    fs::create_dir_all(&tmp_dir).unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), tmp_dir.clone(), logger.clone())
        .await
        .unwrap();
    fs::write(root.join("docs/a.txt"), "v2").unwrap();
    fs::write(root.join("new.txt"), "new").unwrap();

    restore::run(cfg, archive.clone(), logger).await.unwrap();

    assert_eq!(fs::read_to_string(root.join("docs/a.txt")).unwrap(), "v1");
    assert_eq!(
        fs::read_to_string(root.join("app/node_modules/dep.js")).unwrap(),
        "dep"
    );
    assert!(!root.join("new.txt").exists());
    assert!(
        archive.exists(),
        "the directory holding the archive must not be wiped"
    );
}

#[tokio::test]
async fn restore_survives_archive_stored_directly_in_the_source() {
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    let cfg = files_config(root, &[]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let outside = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();
    let archive = root.join("snapshot.tar.gz");
    fs::copy(&outside, &archive).unwrap();
    fs::write(root.join("new.txt"), "new").unwrap();

    restore::run(cfg, archive.clone(), logger).await.unwrap();

    assert_eq!(fs::read_to_string(root.join("docs/a.txt")).unwrap(), "v1");
    assert!(!root.join("new.txt").exists());
    assert!(archive.exists(), "the archive itself must not be wiped");
}

#[tokio::test]
async fn restore_never_follows_symlinked_directories() {
    let outside = tempfile::TempDir::new().unwrap();
    fs::write(outside.path().join("secret.txt"), "outside").unwrap();
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    std::os::unix::fs::symlink(outside.path(), root.join("old_link")).unwrap();
    let cfg = files_config(root, &[]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("late_link")).unwrap();

    restore::run(cfg, archive, logger).await.unwrap();

    assert_eq!(
        fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
        "outside"
    );
    assert!(
        fs::symlink_metadata(root.join("late_link")).is_err(),
        "link created after backup must be removed"
    );
    assert!(
        fs::symlink_metadata(root.join("old_link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[tokio::test]
async fn restore_refuses_archive_with_corrupt_gzip_trailer() {
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    let cfg = files_config(root, &[]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();
    // The last 8 bytes are CRC32 + ISIZE; tar itself never reads them.
    let mut bytes = fs::read(&archive).unwrap();
    let idx = bytes.len() - 6;
    bytes[idx] ^= 0xff;
    fs::write(&archive, bytes).unwrap();
    fs::write(root.join("keep.txt"), "keep").unwrap();

    assert!(restore::run(cfg, archive, logger).await.is_err());
    assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "keep");
    assert_eq!(fs::read_to_string(root.join("docs/a.txt")).unwrap(), "v1");
}

#[tokio::test]
async fn restore_anchored_exclude_applies_through_a_symlinked_path() {
    let base = tempfile::TempDir::new().unwrap();
    let real = base.path().join("real");
    fs::create_dir_all(real.join("app/node_modules")).unwrap();
    fs::create_dir_all(real.join("node_modules")).unwrap();
    fs::write(real.join("app/node_modules/x"), "x1").unwrap();
    fs::write(real.join("node_modules/y"), "y1").unwrap();
    let link = base.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let cfg = files_config(&link, &["/app/node_modules"]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();
    fs::write(real.join("app/node_modules/x"), "x2").unwrap();
    fs::write(real.join("node_modules/y"), "y2").unwrap();

    restore::run(cfg, archive, logger).await.unwrap();

    assert_eq!(
        fs::read_to_string(real.join("app/node_modules/x")).unwrap(),
        "x2",
        "anchored exclude must be kept"
    );
    assert_eq!(
        fs::read_to_string(real.join("node_modules/y")).unwrap(),
        "y1",
        "top-level node_modules is not excluded"
    );
}

#[test]
fn wipe_keeps_only_excluded_paths_on_a_single_filesystem() {
    for one_file_system in [false, true] {
        let src = tempfile::TempDir::new().unwrap();
        let root = src.path();
        sample_tree(root);
        let m = matcher(&["node_modules"]);

        let failures =
            restore::wipe_except_excluded(root, &m, one_file_system, &JobLogger::new()).unwrap();

        assert_eq!(failures, 0);

        assert!(root.join("app/node_modules/dep.js").exists());
        assert!(!root.join("app/index.js").exists());
        assert!(!root.join("docs").exists());
        assert!(fs::symlink_metadata(root.join("link")).is_err());
    }
}

use crate::domain::files::platform::is_root;
use std::os::unix::fs::PermissionsExt;

#[test]
fn unpack_retries_without_metadata_when_ownership_cannot_be_set() {
    if is_root() {
        return; // root may chown to uid 0, so the first pass would not fail
    }
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("root-owned.tar.gz");
    let gz = flate2::write::GzEncoder::new(
        fs::File::create(&archive).unwrap(),
        flate2::Compression::default(),
    );
    let mut builder = tar::Builder::new(gz);
    for (name, body) in [("a.txt", "alpha"), ("sub/b.txt", "beta")] {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mode(0o644);
        header.set_size(body.len() as u64);
        header.set_cksum();
        builder.append(&header, body.as_bytes()).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
    let target = tempfile::TempDir::new().unwrap();
    let logger = JobLogger::new();

    restore::unpack(
        &fs::File::open(&archive).unwrap(),
        target.path(),
        true,
        &logger,
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(target.path().join("a.txt")).unwrap(),
        "alpha"
    );
    assert_eq!(
        fs::read_to_string(target.path().join("sub/b.txt")).unwrap(),
        "beta"
    );
    assert!(
        logger
            .into_entries()
            .iter()
            .any(|e| e.level == "warn" && e.message.contains("retrying without metadata")),
        "the chown failure must go through the fallback"
    );
}

#[tokio::test]
async fn restore_extracts_even_when_some_entries_cannot_be_removed() {
    if is_root() {
        return; // root ignores directory permissions
    }
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    let cfg = files_config(root, &[]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();
    fs::write(root.join("docs/a.txt"), "v2").unwrap();
    fs::write(root.join("new.txt"), "new").unwrap();
    let locked = root.join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("stale.txt"), "stale").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

    let result = restore::run(cfg, archive, logger).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    let err = result.expect_err("an entry that could not be removed must fail the restore");
    assert!(
        err.to_string().contains("could not be removed"),
        "unexpected error: {err:#}"
    );
    assert_eq!(
        fs::read_to_string(root.join("docs/a.txt")).unwrap(),
        "v1",
        "the archive must still be extracted"
    );
    assert_eq!(
        fs::read_to_string(root.join("app/index.js")).unwrap(),
        "app"
    );
    assert!(
        !root.join("new.txt").exists(),
        "removable entries must still be wiped"
    );
    assert!(locked.join("stale.txt").exists());
}

#[tokio::test]
async fn backup_fails_when_an_entry_cannot_be_read() {
    if is_root() {
        return; // root reads mode 0o000 files
    }
    let src = tempfile::TempDir::new().unwrap();
    sample_tree(src.path());
    let secret = src.path().join("secret.txt");
    fs::write(&secret, "secret").unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    let cfg = files_config(src.path(), &[]);
    let out = tempfile::TempDir::new().unwrap();

    let result = backup::run(cfg, out.path().to_path_buf(), Arc::new(JobLogger::new())).await;
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();

    let err = result.expect_err("an unreadable entry must fail the backup");
    assert!(
        err.to_string().contains("1 entry could not be read"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn restore_keeps_special_files() {
    let src = tempfile::TempDir::new().unwrap();
    let root = src.path();
    sample_tree(root);
    let cfg = files_config(root, &[]);
    let out = tempfile::TempDir::new().unwrap();
    let logger = Arc::new(JobLogger::new());

    let archive = backup::run(cfg.clone(), out.path().to_path_buf(), logger.clone())
        .await
        .unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(root.join("sock")).unwrap();

    restore::run(cfg, archive, logger).await.unwrap();

    assert!(
        fs::symlink_metadata(root.join("sock")).is_ok(),
        "socket must survive a restore"
    );
    assert_eq!(fs::read_to_string(root.join("docs/a.txt")).unwrap(), "v1");
}
