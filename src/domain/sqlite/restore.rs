use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        debug!("Starting SQLite restore for database {}", cfg.name);

        let db_path_str = if cfg.path.is_empty() {
            anyhow::bail!("Database path not configured");
        } else {
            cfg.path.as_str().to_string()
        };

        let db_path = PathBuf::from(db_path_str);

        if !restore_file.exists() {
            anyhow::bail!("Restore file not found: {}", restore_file.display());
        }

        if db_path.exists() {
            std::fs::remove_file(&db_path)
                .with_context(|| format!("Failed to remove existing DB {}", db_path.display()))?;
        }

        let output = Command::new("sqlite3")
            .arg(db_path.as_os_str())
            .arg(format!(".restore '{}'", restore_file.display()))
            .output()
            .with_context(|| format!("Failed to run sqlite3 restore for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("SQLite restore failed for {}: {}", cfg.name, stderr);
            anyhow::bail!("SQLite restore failed for {}", cfg.name);
        }

        info!("SQLite restore completed for {}", cfg.name);
        Ok(())
    })
    .await?
}
