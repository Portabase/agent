//! Spec B: the dashboard's local storage, served here by `rclone serve restic
//! --append-only` and `rclone serve webdav` on free ports. The restic tests need
//! restic on PATH (test container); the sync test only needs rclone.

use crate::domain::files::platform::is_root;
use crate::services::backup::logger::JobLogger;
use crate::services::restic::backup::snapshot;
use crate::services::restic::command::ResticRepo;
use crate::services::restic::restore::restore;
use crate::tests::domain::files::files_config;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

pub(crate) const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

pub(crate) struct RcloneServer {
    child: Child,
    pub base_url: String,
}

impl Drop for RcloneServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `rclone serve <kind> --baseurl /storage/<prefix> <root>` on a free port, user
/// `portabase`, credentials through the env like the dashboard does.
pub(crate) fn serve(kind: &str, prefix: &str, root: &Path, extra: &[&str], password: &str) -> RcloneServer {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = Command::new("rclone")
        .args(["serve", kind, "--addr", &format!("127.0.0.1:{port}"), "--baseurl", &format!("/storage/{prefix}")])
        .args(extra)
        .arg(root)
        .env("RCLONE_USER", "portabase")
        .env("RCLONE_PASS", password)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("rclone binary not found");
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    RcloneServer { child, base_url: format!("http://127.0.0.1:{port}") }
}

fn tree(root: &Path) {
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("a.txt"), "v1").unwrap();
    fs::write(root.join("docs/b.txt"), "b").unwrap();
}

fn local_repo(server: &RcloneServer, generated_id: &str, cache: &Path) -> ResticRepo {
    ResticRepo::rest(&format!("{}/storage/restic", server.base_url), generated_id, "test-password", "s3cret", cache.to_path_buf())
}

#[tokio::test]
async fn local_snapshot_and_restore_go_through_the_append_only_server() {
    let store = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let server = serve("restic", "restic", store.path(), &["--append-only"], "s3cret");
    let src = work.path().join("src");
    tree(&src);
    let cfg = files_config(&src, &[]);
    let repo = local_repo(&server, &cfg.generated_id, &work.path().join("cache"));
    let logger = JobLogger::new();

    let snap = snapshot(&repo, &cfg, "bs-1", &logger).await.unwrap();

    let repo_dir = store.path().join(&cfg.generated_id);
    assert!(repo_dir.join("config").exists(), "repository written on the served disk");
    assert_eq!(fs::read_dir(repo_dir.join("locks")).unwrap().count(), 0, "lock removed despite --append-only");

    fs::write(src.join("a.txt"), "changed").unwrap();
    fs::write(src.join("extra.txt"), "x").unwrap();
    restore(&repo, &cfg, &snap.snapshot_id, &logger).await.unwrap();
    assert_eq!(fs::read_to_string(src.join("a.txt")).unwrap(), "v1");
    assert!(!src.join("extra.txt").exists());
}

#[tokio::test]
async fn the_agent_cannot_forget_on_the_local_server() {
    let store = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let server = serve("restic", "restic", store.path(), &["--append-only"], "s3cret");
    let src = work.path().join("src");
    tree(&src);
    let cfg = files_config(&src, &[]);
    let repo = local_repo(&server, &cfg.generated_id, &work.path().join("cache"));
    let logger = JobLogger::new();
    let snap = snapshot(&repo, &cfg, "bs-1", &logger).await.unwrap();

    let forget = repo.run(["forget", snap.snapshot_id.as_str()], &logger).await.unwrap();

    assert_ne!(forget.code, 0, "append-only must refuse forget");
    let listed = repo.run(["snapshots", "--json"], &logger).await.unwrap();
    assert!(listed.stdout.contains(&snap.snapshot_id), "{}", listed.stdout);
}

#[tokio::test]
async fn unreadable_file_on_a_local_repo_keeps_the_snapshot_for_the_dashboard() {
    if is_root() {
        return; // root reads a 000 file anyway
    }
    use std::os::unix::fs::PermissionsExt;
    let store = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let server = serve("restic", "restic", store.path(), &["--append-only"], "s3cret");
    let src = work.path().join("src");
    tree(&src);
    let locked = src.join("docs/b.txt");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let cfg = files_config(&src, &[]);
    let repo = local_repo(&server, &cfg.generated_id, &work.path().join("cache"));
    let logger = JobLogger::new();

    let err = snapshot(&repo, &cfg, "bs-1", &logger).await.unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(err.to_string().contains("the dashboard forgets it"), "{err:#}");
    let listed = repo.run(["snapshots", "--json"], &logger).await.unwrap();
    assert!(listed.stdout.contains("bs:bs-1"), "{}", listed.stdout);
}

#[tokio::test]
async fn local_sync_mirrors_through_the_webdav_server() {
    use crate::core::context::Context;
    use crate::services::api::ApiClient;
    use crate::services::api::models::agent::status::DatabaseStorage;
    use crate::services::restic::password::local_storage_password;
    use crate::services::sync::backup::one_storage;
    use crate::utils::edge_key::EdgeKey;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let store = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let password = local_storage_password(MASTER_KEY_B64).unwrap();
    let server = serve("webdav", "sync", store.path(), &[], &password);

    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent/agent-1/backup/upload/init"))
        .and(body_partial_json(json!({ "engine": "sync" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok", "backupStorage": { "id": "bs" } })))
        .expect(2)
        .mount(&api)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/agent/agent-1/backup/upload/status"))
        .and(body_partial_json(json!({ "status": "success" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "message": "ok", "backupStorage": { "id": "bs" } })))
        .expect(2)
        .mount(&api)
        .await;

    let ctx = Context {
        edge_key: EdgeKey {
            server_url: server.base_url.clone(),
            agent_id: "agent-1".into(),
            master_key_b64: MASTER_KEY_B64.into(),
        },
        api: ApiClient::new(api.uri()),
    };
    let storage: DatabaseStorage =
        serde_json::from_value(json!({ "id": "ch-local", "provider": "local", "folderName": "backups", "config": {} })).unwrap();
    let src = work.path().join("src");
    tree(&src);
    let cfg = files_config(&src, &[]);
    let replica = store.path().join(&cfg.generated_id).join("current");

    let first = one_storage(&ctx, &cfg, &storage, "backup-1", &JobLogger::new()).await;
    assert!(first.success, "{:?}", first.error);
    assert_eq!(first.remote_file_path.as_deref(), Some(format!("backups/sync/{}/current", cfg.generated_id).as_str()));
    assert_eq!(fs::read_to_string(replica.join("docs/b.txt")).unwrap(), "b");

    fs::remove_file(src.join("docs/b.txt")).unwrap();
    let second = one_storage(&ctx, &cfg, &storage, "backup-2", &JobLogger::new()).await;
    assert!(second.success, "{:?}", second.error);
    assert!(!replica.join("docs/b.txt").exists(), "deletion propagated");
    assert!(replica.join("a.txt").exists());
}
