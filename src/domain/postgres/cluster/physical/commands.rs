use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

pub fn output_dir(
    restore_root: &Path,
    generated_id: &str,
    now: DateTime<Utc>,
    major: u32,
) -> PathBuf {
    let timestamped = restore_root
        .join(generated_id)
        .join(now.format("%Y%m%dT%H%M%SZ").to_string());
    if major >= 18 {
        timestamped.join(major.to_string()).join("docker")
    } else {
        timestamped.join("data")
    }
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

pub fn start_commands(data_dir: &Path, major: u32) -> Vec<String> {
    let container_suffix: String = data_dir
        .components()
        .rev()
        .filter_map(|component| component.as_os_str().to_str())
        .find(|name| name.len() == 36)
        .unwrap_or("restore")
        .chars()
        .take(8)
        .collect();
    let (mount_source, mount_target) = if major >= 18 {
        (
            data_dir.parent().and_then(Path::parent).unwrap_or(data_dir),
            "/var/lib/postgresql",
        )
    } else {
        (data_dir, "/var/lib/postgresql/data")
    };
    vec![
        format!("chown -R 999:999 {}", quote(mount_source)),
        format!("chmod 700 {}", quote(data_dir)),
        format!(
            "docker run -d --name portabase-restore-{container_suffix} -v {}:{mount_target} postgres:{major}",
            quote(mount_source)
        ),
        format!(
            "/usr/lib/postgresql/{major}/bin/pg_ctl -D {} start",
            quote(data_dir)
        ),
    ]
}

pub fn missing_config_files(data_dir: &Path) -> Vec<&'static str> {
    ["postgresql.conf", "pg_hba.conf"]
        .into_iter()
        .filter(|name| !data_dir.join(name).exists())
        .collect()
}
