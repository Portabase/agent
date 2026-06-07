use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf, logger: Arc<JobLogger>) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        logger.log("debug", format!("Starting SQLite restore for database {}", cfg.name));

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

        let start = Instant::now();
        let output = Command::new("sqlite3")
            .arg(db_path.as_os_str())
            .arg(format!(".restore '{}'", restore_file.display()))
            .output()
            .with_context(|| format!("Failed to run sqlite3 restore for {}", cfg.name))?;

        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            logger.log_command("sqlite3", Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            logger.log("error", format!("SQLite restore failed for {}: {}", cfg.name, stderr));
            anyhow::bail!("SQLite restore failed for {}", cfg.name);
        }

        logger.log_command("sqlite3", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("SQLite restore completed for {}", cfg.name));
        Ok(())
    })
    .await?
}
