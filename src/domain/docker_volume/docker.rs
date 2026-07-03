#![allow(dead_code)]

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, InspectContainerOptions, ListContainersOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use std::collections::HashMap;
use tracing::{info, warn};
use uuid::Uuid;

pub const EPHEMERAL_LABEL: &str = "io.portabase.ephemeral";
const HELPER_MOUNT: &str = "/vol";

pub fn client() -> Result<Docker> {
    Docker::connect_with_unix_defaults().context("Failed to connect to Docker daemon socket")
}

pub fn parse_container_id(mountinfo: &str, cgroup: &str) -> Option<String> {
    for src in [mountinfo, cgroup] {
        for line in src.lines() {
            for marker in ["/containers/", "/docker/"] {
                if let Some(idx) = line.find(marker) {
                    let rest = &line[idx + marker.len()..];
                    let id: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
                    if id.len() >= 64 {
                        return Some(id[..64].to_string());
                    }
                }
            }
        }
    }
    None
}

pub async fn resolve_helper_image(docker: &Docker) -> Result<String> {
    if let Ok(img) = std::env::var("PORTABASE_HELPER_IMAGE") {
        if !img.trim().is_empty() {
            return Ok(img);
        }
    }
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let id = parse_container_id(&mountinfo, &cgroup).context(
        "Could not determine own container id; set PORTABASE_HELPER_IMAGE to a locally-present image",
    )?;
    let info = docker
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await
        .with_context(|| format!("Failed to inspect self container {id}"))?;
    info.image
        .context("Self container inspection returned no image reference")
}

pub struct Helper {
    pub id: String,
}

pub async fn create_helper(
    docker: &Docker,
    image: &str,
    volume_name: &str,
    generated_id: &str,
    read_only: bool,
    cmd: Option<Vec<String>>,
) -> Result<Helper> {
    let bind = format!(
        "{volume_name}:{HELPER_MOUNT}{}",
        if read_only { ":ro" } else { "" }
    );
    let mut labels = HashMap::new();
    labels.insert(EPHEMERAL_LABEL.to_string(), "true".to_string());
    labels.insert("com.docker.compose.project".to_string(), String::new());
    labels.insert("com.docker.compose.service".to_string(), String::new());
    labels.insert("com.docker.compose.oneoff".to_string(), String::new());

    let name = format!(
        "portabase-vol-{generated_id}-{}",
        &Uuid::new_v4().to_string()[..8]
    );

    let body = ContainerCreateBody {
        image: Some(image.to_string()),
        cmd,
        labels: Some(labels),
        host_config: Some(HostConfig {
            binds: Some(vec![bind]),
            auto_remove: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };

    let opts = CreateContainerOptions {
        name: Some(name),
        ..Default::default()
    };

    let res = docker
        .create_container(Some(opts), body)
        .await
        .with_context(|| format!("Failed to create helper container for volume {volume_name}"))?;

    Ok(Helper { id: res.id })
}


pub async fn remove_helper(docker: &Docker, id: &str) {
    let stop_opts = StopContainerOptions {
        t: Some(2),
        ..Default::default()
    };
    let _ = docker.stop_container(id, Some(stop_opts)).await;

    if let Ok(info) = docker
        .inspect_container(id, None::<InspectContainerOptions>)
        .await
    {
        let name = info.name.unwrap_or_default();
        let name = name.trim_start_matches('/');
        let code = info.state.and_then(|s| s.exit_code).unwrap_or_default();
        info!("Helper container {name} exited with code {code}");
    }

    let opts = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    if let Err(e) = docker.remove_container(id, Some(opts)).await {
        warn!("Failed to remove helper container {id}: {e}");
    }
}

pub async fn stop_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .stop_container(name, None::<StopContainerOptions>)
        .await
        .with_context(|| format!("Failed to stop container {name}"))
}

pub async fn start_container(docker: &Docker, name: &str) -> Result<()> {
    docker
        .start_container(name, None::<StartContainerOptions>)
        .await
        .with_context(|| format!("Failed to start container {name}"))
}

pub async fn sweep_ephemeral(docker: &Docker) -> Result<usize> {
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("{EPHEMERAL_LABEL}=true")]);

    let opts = ListContainersOptions {
        all: true,
        filters: Some(filters),
        ..Default::default()
    };

    let list = docker.list_containers(Some(opts)).await?;
    let mut removed = 0;
    for c in list {
        if let Some(id) = c.id {
            remove_helper(docker, &id).await;
            removed += 1;
        }
    }
    Ok(removed)
}
