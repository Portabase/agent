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

pub fn start_commands(data_dir: &Path, major: u32, generated_id: &str) -> Vec<String> {
    let source_prefix: String = generated_id.chars().take(8).collect();
    let timestamp_dir = if major >= 18 {
        data_dir.parent().and_then(Path::parent)
    } else {
        data_dir.parent()
    }
    .unwrap_or(data_dir);
    let restored_at = timestamp_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let (mount_source, mount_target) = if major >= 18 {
        (timestamp_dir, "/var/lib/postgresql")
    } else {
        (data_dir, "/var/lib/postgresql/data")
    };
    let data_dir = quote(data_dir);
    vec![
        format!("chown -R 999:999 {}", quote(mount_source)),
        format!("chmod 700 {data_dir}"),
        format!(
            "docker run -d --name portabase-restore-{source_prefix}-{restored_at} -v {}:{mount_target} postgres:{major}",
            quote(mount_source)
        ),
        format!(
            "sudo chown -R postgres:postgres {data_dir} && sudo chmod 700 {data_dir} && sudo -u postgres /usr/lib/postgresql/{major}/bin/pg_ctl -D {data_dir} start"
        ),
    ]
}

pub fn missing_config_files(data_dir: &Path) -> Vec<&'static str> {
    ["postgresql.conf", "pg_hba.conf"]
        .into_iter()
        .filter(|name| !data_dir.join(name).exists())
        .collect()
}
