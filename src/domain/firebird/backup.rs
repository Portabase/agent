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
        debug!("Starting backup for database {}", cfg.name);

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));

        let db_path = format!(
            "{}/{}:{}",
            cfg.host,
            cfg.port,
            cfg.database
        );

        info!("Firebird database target: {}", db_path);
        info!("Backup file: {}", file_path.display());

        let output = Command::new("gbak")
            .arg("-b")
            .arg("-v")
            .arg("-user").arg(&cfg.username)
            .arg("-password").arg(&cfg.password)
            .arg(db_path)
            .arg(&file_path)
            .output()
            .with_context(|| format!("Failed to run gbak for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Firebird backup failed: {}", stderr);
            anyhow::bail!("Firebird backup failed for {}: {}", cfg.name, stderr);
        }

        info!("Firebird backup completed: {}", file_path.display());

        Ok(file_path)
    })
    .await?
}
