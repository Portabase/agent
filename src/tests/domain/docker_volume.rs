use crate::domain::docker_volume::docker::parse_container_id;

static ENV_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn parse_container_id_from_mountinfo_line() {
    let id = "a".repeat(64);
    let mountinfo = format!(
        "1234 1000 0:50 / /etc/hostname rw shared:1 - ext4 /var/lib/docker/containers/{id}/hostname rw"
    );
    assert_eq!(parse_container_id(&mountinfo, ""), Some(id.clone()));
    let rootless = format!(
        "1234 1000 0:50 /home/u/.local/share/docker/containers/{id}/hostname /etc/hostname rw - ext4 /dev/sda1 rw"
    );
    assert_eq!(parse_container_id(&rootless, ""), Some(id));
}

#[test]
fn parse_container_id_from_podman_mountinfo_line() {
    let id = "c".repeat(64);
    for driver in ["overlay", "vfs"] {
        let mountinfo = format!(
            "17182 17570 0:411 /containers/{driver}-containers/{id}/userdata/hostname /etc/hostname rw,relatime - tmpfs tmpfs rw"
        );
        assert_eq!(
            parse_container_id(&mountinfo, ""),
            Some(id.clone()),
            "{driver}"
        );
    }
}

#[test]
fn parse_container_id_ignores_foreign_container_mounts() {
    let own = "d".repeat(64);
    let foreign = "e".repeat(64);
    let podman = format!(
        "1 0 0:1 /containers/overlay-containers/{foreign}/userdata/hosts /etc/hosts rw - tmpfs tmpfs rw\n\
         2 0 0:1 /containers/overlay-containers/{foreign}/userdata/hostname /etc/hostname rw - tmpfs tmpfs rw\n\
         3 0 0:1 /containers/overlay-containers/{own}/userdata/.containerenv /run/.containerenv rw - tmpfs tmpfs rw"
    );
    assert_eq!(parse_container_id(&podman, ""), Some(own.clone()));
    let docker = format!(
        "1 0 8:1 /var/lib/docker/containers/{foreign} /logs rw - ext4 /dev/sda1 rw\n\
         2 0 8:1 /var/lib/docker/containers/{own}/hostname /etc/hostname rw - ext4 /dev/sda1 rw"
    );
    assert_eq!(parse_container_id(&docker, ""), Some(own));
}

#[test]
fn parse_container_id_from_cgroup_v1() {
    let id = "b".repeat(64);
    let cgroup = format!("12:memory:/docker/{id}\n11:cpu:/docker/{id}\n");
    assert_eq!(parse_container_id("", &cgroup), Some(id));
}

#[test]
fn parse_container_id_none_on_cgroup_v2() {
    assert_eq!(parse_container_id("", "0::/\n"), None);
}

fn write_tar(dir: &tempfile::TempDir, file_name: &str, entry_names: &[&str]) -> std::path::PathBuf {
    let mut builder = tar::Builder::new(Vec::new());
    for entry_name in entry_names {
        let mut header = tar::Header::new_ustar();
        header.as_mut_bytes()[..entry_name.len()].copy_from_slice(entry_name.as_bytes());
        header.set_size(0);
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    }
    let path = dir.path().join(file_name);
    std::fs::write(&path, builder.into_inner().unwrap()).unwrap();
    path
}

#[test]
fn restore_upload_path_follows_archive_layout() {
    use crate::domain::docker_volume::restore::upload_path;

    let dir = tempfile::TempDir::new().unwrap();
    let docker_archive = write_tar(&dir, "docker.tar", &["vol/", "vol/keeper.txt"]);
    let podman_archive = write_tar(&dir, "podman.tar", &["/", "/keeper.txt"]);
    let empty_archive = write_tar(&dir, "empty.tar", &[]);

    assert_eq!(upload_path(&docker_archive).unwrap(), "/");
    assert_eq!(upload_path(&podman_archive).unwrap(), "/vol");
    assert_eq!(upload_path(&empty_archive).unwrap(), "/");
}

#[test]
fn tar_succeeded_tolerates_only_files_changed_while_read() {
    use crate::domain::docker_volume::backup::tar_succeeded;

    let changed = "tar: vol/busy.log: file changed as we read it\ntar: vol/other.log: file changed as we read it\n";
    assert!(tar_succeeded(Some(0), ""));
    assert!(tar_succeeded(Some(0), "tar: vol/app.sock: socket ignored\n"));
    assert!(tar_succeeded(Some(1), changed));
    assert!(!tar_succeeded(Some(1), ""));
    assert!(!tar_succeeded(Some(1), "tar: short read\n"));
    assert!(!tar_succeeded(Some(1), &format!("{changed}tar: vol/x: Cannot open: Permission denied\n")));
    assert!(!tar_succeeded(Some(2), changed));
    assert!(!tar_succeeded(None, ""));
}

#[tokio::test]
async fn docker_volume_ping_true_for_existing_volume() {
    use crate::domain::docker_volume::docker::client;
    use bollard::models::VolumeCreateRequest;
    use bollard::query_parameters::RemoveVolumeOptions;

    let docker = client().expect("docker daemon required for this test");
    let vol = format!("portabase-test-{}", uuid::Uuid::new_v4());
    docker
        .create_volume(VolumeCreateRequest { name: Some(vol.clone()), ..Default::default() })
        .await
        .unwrap();

    let cfg = volume_config(&vol);
    let reachable = crate::domain::docker_volume::ping::run(cfg).await.unwrap();
    assert!(reachable);

    let missing = volume_config("portabase-does-not-exist-xyz");
    assert!(!crate::domain::docker_volume::ping::run(missing).await.unwrap());

    docker.remove_volume(&vol, None::<RemoveVolumeOptions>).await.ok();
}

fn volume_config(volume_name: &str) -> crate::services::config::DatabaseConfig {
    use crate::services::config::{DatabaseConfig, DbType};
    DatabaseConfig {
        name: "vol-test".to_string(),
        database: "".to_string(),
        db_type: DbType::DockerVolume,
        username: "".to_string(),
        password: "".to_string(),
        port: 0,
        host: "".to_string(),
        generated_id: uuid::Uuid::new_v4().to_string(),
        path: "".to_string(),
        max_packet_size: "".to_string(),
        volume_name: volume_name.to_string(),
        container_name: None,
        options: std::collections::HashMap::new(),
    }
}

async fn ensure_image(docker: &bollard::Docker, image: &str) {
    use bollard::query_parameters::CreateImageOptionsBuilder;
    use futures::StreamExt;

    let (name, tag) = image.split_once(':').unwrap_or((image, "latest"));
    let opts = CreateImageOptionsBuilder::default()
        .from_image(name)
        .tag(tag)
        .build();
    let mut stream = docker.create_image(Some(opts), None, None);
    while let Some(item) = stream.next().await {
        item.unwrap();
    }
}

async fn seed_volume(docker: &bollard::Docker, volume: &str, filename: &str, content: &str) {
    run_in_volume(docker, volume, &format!("printf '%s' '{content}' > /vol/{filename}")).await;
}

async fn run_in_volume(docker: &bollard::Docker, volume: &str, script: &str) {
    use bollard::models::{ContainerCreateBody, HostConfig};
    use bollard::query_parameters::{
        CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
        WaitContainerOptions,
    };
    use futures::StreamExt;

    ensure_image(docker, "busybox").await;

    let body = ContainerCreateBody {
        image: Some("busybox".to_string()),
        cmd: Some(vec!["sh".into(), "-c".into(), script.to_string()]),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{volume}:/vol")]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let created = docker
        .create_container(None::<CreateContainerOptions>, body)
        .await
        .unwrap();
    docker.start_container(&created.id, None::<StartContainerOptions>).await.unwrap();
    let mut wait = docker.wait_container(&created.id, None::<WaitContainerOptions>);
    while wait.next().await.is_some() {}
    docker
        .remove_container(&created.id, Some(RemoveContainerOptions { force: true, ..Default::default() }))
        .await
        .ok();
}

#[tokio::test]
async fn docker_volume_backup_keeps_file_ownership() {
    use crate::domain::docker_volume::docker::client;
    use bollard::models::VolumeCreateRequest;
    use bollard::query_parameters::RemoveVolumeOptions;

    let _env_guard = ENV_GUARD.lock().await;
    unsafe { std::env::set_var("PORTABASE_HELPER_IMAGE", "busybox"); }

    let docker = client().expect("docker daemon required");
    let vol = format!("portabase-test-{}", uuid::Uuid::new_v4());
    docker
        .create_volume(VolumeCreateRequest { name: Some(vol.clone()), ..Default::default() })
        .await
        .unwrap();
    run_in_volume(&docker, &vol, "echo owned > /vol/owned.txt && chown 999:998 /vol/owned.txt").await;

    let tmp = tempfile::TempDir::new().unwrap();
    let logger = std::sync::Arc::new(crate::services::backup::logger::JobLogger::new());
    let tar = crate::domain::docker_volume::backup::run(volume_config(&vol), tmp.path().to_path_buf(), logger)
        .await
        .unwrap();

    let mut archive = tar::Archive::new(std::fs::File::open(&tar).unwrap());
    let owner = archive
        .entries()
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.path().unwrap().ends_with("owned.txt"))
        .map(|entry| (entry.header().uid().unwrap(), entry.header().gid().unwrap()));
    assert_eq!(owner, Some((999, 998)));

    docker.remove_volume(&vol, None::<RemoveVolumeOptions>).await.ok();
}

#[tokio::test]
async fn docker_volume_backup_captures_files() {
    use crate::domain::docker_volume::docker::client;
    use bollard::models::VolumeCreateRequest;
    use bollard::query_parameters::RemoveVolumeOptions;

    let _env_guard = ENV_GUARD.lock().await;
    unsafe { std::env::set_var("PORTABASE_HELPER_IMAGE", "busybox"); }

    let docker = client().expect("docker daemon required");
    let vol = format!("portabase-test-{}", uuid::Uuid::new_v4());
    docker
        .create_volume(VolumeCreateRequest { name: Some(vol.clone()), ..Default::default() })
        .await
        .unwrap();
    seed_volume(&docker, &vol, "hello.txt", "backup-me").await;

    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = volume_config(&vol);
    let logger = std::sync::Arc::new(crate::services::backup::logger::JobLogger::new());
    let tar = crate::domain::docker_volume::backup::run(cfg, tmp.path().to_path_buf(), logger)
        .await
        .unwrap();

    assert!(tar.is_file());
    let names = tar_entry_names(&tar).await;
    assert!(names.iter().any(|n| n.ends_with("hello.txt")), "entries: {names:?}");

    docker.remove_volume(&vol, None::<RemoveVolumeOptions>).await.ok();
}

async fn tar_entry_names(tar_path: &std::path::Path) -> Vec<String> {
    let mut archive = tar::Archive::new(std::fs::File::open(tar_path).unwrap());
    archive
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
        .collect()
}

#[tokio::test]
async fn docker_volume_restore_is_clean_replace() {
    use crate::domain::docker_volume::docker::client;
    use bollard::models::VolumeCreateRequest;
    use bollard::query_parameters::RemoveVolumeOptions;
    
    let _env_guard = ENV_GUARD.lock().await;
    unsafe { std::env::set_var("PORTABASE_HELPER_IMAGE", "busybox"); }

    let docker = client().expect("docker daemon required");
    let vol = format!("portabase-test-{}", uuid::Uuid::new_v4());
    docker
        .create_volume(VolumeCreateRequest { name: Some(vol.clone()), ..Default::default() })
        .await
        .unwrap();

    seed_volume(&docker, &vol, "keeper.txt", "original").await;

    let tmp = tempfile::TempDir::new().unwrap();
    let logger = std::sync::Arc::new(crate::services::backup::logger::JobLogger::new());
    let tar = crate::domain::docker_volume::backup::run(
        volume_config(&vol),
        tmp.path().to_path_buf(),
        logger.clone(),
    )
    .await
    .unwrap();
    
    seed_volume(&docker, &vol, "drift.txt", "added-later").await;

    // Restore uploads the raw Docker tar directly.
    crate::domain::docker_volume::restore::run(volume_config(&vol), tar.clone(), logger)
        .await
        .unwrap();

    let listing = list_volume(&docker, &vol).await;
    assert!(listing.contains("keeper.txt"), "listing: {listing}");
    assert!(!listing.contains("drift.txt"), "clean-replace failed, listing: {listing}");

    docker.remove_volume(&vol, None::<RemoveVolumeOptions>).await.ok();
}

async fn list_volume(docker: &bollard::Docker, volume: &str) -> String {
    use bollard::models::{ContainerCreateBody, HostConfig};
    use bollard::query_parameters::{
        CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
        WaitContainerOptions,
    };
    use tokio_stream::StreamExt;

    ensure_image(docker, "busybox").await;

    let body = ContainerCreateBody {
        image: Some("busybox".to_string()),
        cmd: Some(vec!["sh".into(), "-c".into(), "ls -A /vol".into()]),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{volume}:/vol")]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let created = docker.create_container(None::<CreateContainerOptions>, body).await.unwrap();
    docker.start_container(&created.id, None::<StartContainerOptions>).await.unwrap();
    let mut wait = docker.wait_container(&created.id, None::<WaitContainerOptions>);
    while wait.next().await.is_some() {}

    let mut logs = docker.logs(
        &created.id,
        Some(LogsOptions { stdout: true, stderr: false, ..Default::default() }),
    );
    let mut out = String::new();
    while let Some(chunk) = logs.next().await {
        if let Ok(l) = chunk {
            out.push_str(&l.to_string());
        }
    }
    docker
        .remove_container(&created.id, Some(RemoveContainerOptions { force: true, ..Default::default() }))
        .await
        .ok();
    out
}

#[tokio::test]
async fn sweep_removes_labeled_helpers() {
    use crate::domain::docker_volume::docker::{client, create_helper, sweep_ephemeral, EPHEMERAL_LABEL};
    use bollard::models::VolumeCreateRequest;
    use bollard::query_parameters::{ListContainersOptions, RemoveVolumeOptions};
    use std::collections::HashMap;
    
    let _env_guard = ENV_GUARD.lock().await;
    unsafe { std::env::set_var("PORTABASE_HELPER_IMAGE", "busybox"); }
    let docker = client().expect("docker daemon required");

    let vol = format!("portabase-test-{}", uuid::Uuid::new_v4());
    docker
        .create_volume(VolumeCreateRequest { name: Some(vol.clone()), ..Default::default() })
        .await
        .unwrap();

    ensure_image(&docker, "busybox").await;

    let helper = create_helper(&docker, "busybox", &vol, "sweep-test", true, None).await.unwrap();

    let removed = sweep_ephemeral(&docker).await.unwrap();
    assert!(removed >= 1);

    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("{EPHEMERAL_LABEL}=true")]);
    let remaining = docker
        .list_containers(Some(ListContainersOptions { all: true, filters: Some(filters), ..Default::default() }))
        .await
        .unwrap();
    assert!(remaining.iter().all(|c| c.id.as_deref() != Some(helper.id.as_str())));

    docker.remove_volume(&vol, None::<RemoveVolumeOptions>).await.ok();
}
