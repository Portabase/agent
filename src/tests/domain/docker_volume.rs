use crate::domain::docker_volume::docker::parse_container_id;

static ENV_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn parse_container_id_from_mountinfo_line() {
    let id = "a".repeat(64);
    let mountinfo = format!(
        "1234 1000 0:50 / /etc/hostname rw shared:1 - ext4 /var/lib/docker/containers/{id}/hostname rw"
    );
    assert_eq!(parse_container_id(&mountinfo, ""), Some(id));
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

#[test]
fn parse_container_id_from_podman_rootless_mountinfo() {
    // Rootless Podman: cgroup is "0::/" (no id); the id lives in mountinfo under
    // overlay-containers/. The line also contains a "/containers/" substring
    // (…/share/containers/storage/…) that must not be mistaken for the id.
    let id = "c".repeat(64);
    let mountinfo = format!(
        "1234 1000 0:60 / /vol rw,relatime shared:1 - overlay overlay \
         rw,lowerdir=/home/u/.local/share/containers/storage/overlay/L1/diff,\
         upperdir=/home/u/.local/share/containers/storage/overlay-containers/{id}/userdata/upper"
    );
    assert_eq!(parse_container_id(&mountinfo, "0::/\n"), Some(id));
}

#[test]
fn parse_container_id_from_podman_libpod_cgroup() {
    let id = "d".repeat(64);
    let cgroup = format!(
        "0::/user.slice/user-1000.slice/user@1000.service/user.slice/libpod-{id}.scope/container\n"
    );
    assert_eq!(parse_container_id("", &cgroup), Some(id));
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
    use futures_util::StreamExt;

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
    use bollard::models::{ContainerCreateBody, HostConfig};
    use bollard::query_parameters::{
        CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
        WaitContainerOptions,
    };
    use futures_util::StreamExt;

    ensure_image(docker, "busybox").await;

    let body = ContainerCreateBody {
        image: Some("busybox".to_string()),
        cmd: Some(vec![
            "sh".into(),
            "-c".into(),
            format!("printf '%s' '{content}' > /vol/{filename}"),
        ]),
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
    use tokio_stream::StreamExt;
    let f = tokio::fs::File::open(tar_path).await.unwrap();
    let mut archive = tokio_tar::Archive::new(f);
    let mut names = Vec::new();
    let mut entries = archive.entries().unwrap();
    while let Some(e) = entries.next().await {
        let e = e.unwrap();
        names.push(e.path().unwrap().to_string_lossy().to_string());
    }
    names
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
