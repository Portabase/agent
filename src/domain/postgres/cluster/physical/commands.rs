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

/// How to start a restored data directory with any PostgreSQL install of the same major: the long path
/// is given once, so each command stays short and can be pasted as is.
pub fn start_commands(data_dir: &Path, major: u32) -> Vec<String> {
    vec![
        format!("To start the restored cluster (PostgreSQL {major}):"),
        format!("  PGDATA={}", quote(data_dir)),
        "  chown -R postgres:postgres \"$PGDATA\" && chmod 700 \"$PGDATA\"".to_string(),
        "  pg_ctl -D \"$PGDATA\" start    # as the postgres OS user".to_string(),
    ]
}

pub fn missing_config_files(data_dir: &Path) -> Vec<&'static str> {
    ["postgresql.conf", "pg_hba.conf", "pg_ident.conf"]
        .into_iter()
        .filter(|name| !data_dir.join(name).exists())
        .collect()
}
