use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        debug!("Starting Firebird restore for database {}", cfg.name);

        let db_path = format!("{}/{}:{}", cfg.host, cfg.port, cfg.database);

        info!("Restore source: {}", restore_file.display());
        info!("Restore target: {}", db_path);

        let output = Command::new("gbak")
            .arg("-c")
            .arg("-v")
            .arg("-replace_database")
            .arg("-user")
            .arg(&cfg.username)
            .arg("-password")
            .arg(&cfg.password)
            .arg(&restore_file)
            .arg(&db_path)
            .output()
            .with_context(|| format!("Failed to run gbak restore for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Firebird restore failed for {}: {}", cfg.name, stderr);
            anyhow::bail!("Firebird restore failed for {}: {}", cfg.name, stderr);
        }

        info!("Firebird restore completed for {}", cfg.name);

        Ok(())
    })
    .await?
}
