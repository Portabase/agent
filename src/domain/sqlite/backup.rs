use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

pub async fn run(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    file_extension: &'static str,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        debug!("Starting SQLite backup for database {}", cfg.name);

        let db_path_str = if cfg.path.is_empty() {
            anyhow::bail!("Database path not configured");
        } else {
            cfg.path.as_str().to_string()
        };

        let db_path = PathBuf::from(db_path_str);

        if !db_path.exists() {
            anyhow::bail!("SQLite database file not found: {}", db_path.display());
        }

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));

        let output = Command::new("sqlite3")
            .arg(db_path.as_os_str())
            .arg(format!(".backup '{}'", file_path.display()))
            .output()
            .context("SQLite backup command failed to start")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("SQLite backup failed for {}: {}", cfg.name, stderr);
            anyhow::bail!("SQLite backup failed for {}: {}", cfg.name, stderr);
        }

        info!("SQLite backup completed for {}", cfg.name);
        Ok(file_path)
    })
        .await?
}