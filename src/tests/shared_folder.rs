use crate::domain::docker_volume::docker::{client, parse_container_id};
use bollard::query_parameters::InspectContainerOptions;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

pub(crate) fn shared_tempdir(prefix: &str) -> TempDir {
    std::fs::create_dir_all("/app/target").unwrap();
    let dir = tempfile::Builder::new().prefix(prefix).tempdir_in("/app/target").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    dir
}

pub(crate) async fn host_path(path: &Path) -> String {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let container_id = parse_container_id(&mountinfo, &cgroup).expect("tests run inside a container");
    let inspected = client()
        .unwrap()
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await
        .unwrap();
    let app_mount = inspected
        .mounts
        .unwrap()
        .into_iter()
        .find(|mount| mount.destination.as_deref() == Some("/app"))
        .expect("/app is bind-mounted from the host");
    let relative_path = path.strip_prefix("/app").unwrap().display().to_string();
    format!("{}/{}", app_mount.source.unwrap(), relative_path)
}
