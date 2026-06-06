use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info};

pub async fn run(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    file_extension: &'static str,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        debug!("Starting SQLite backup for database {}", cfg.name);
        logger.log("debug", format!("Starting SQLite backup for database {}", cfg.name));

        let db_path_str = if cfg.path.is_empty() {
            anyhow::bail!("Database path not configured");
        } else {
            cfg.path.as_str().to_string()
        };

        let db_path = PathBuf::from(db_path_str);
        info!("database path: {}", db_path.display());
        logger.log("info", format!("Database path: {}", db_path.display()));

        if !db_path.exists() {
            logger.log("error", format!("SQLite database file not found: {}", db_path.display()));
            anyhow::bail!("SQLite database file not found: {}", db_path.display());
        }

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let cmd_label = format!("sqlite3 {} .backup '{}'", db_path.display(), file_path.display());

        logger.log("info", format!("Running sqlite3 backup for {}", cfg.name));

        let start = Instant::now();
        let output = Command::new("sqlite3")
            .arg(db_path.as_os_str())
            .arg(format!(".backup '{}'", file_path.display()))
            .output()
            .context("SQLite backup command failed to start")?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            error!("SQLite backup failed for {}: {}", cfg.name, stderr);
            logger.log_command(cmd_label, Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            anyhow::bail!("SQLite backup failed for {}: {}", cfg.name, stderr);
        }

        info!("SQLite backup completed for {}", cfg.name);
        logger.log_command(cmd_label, None, Some(0), Some(duration_ms));
        logger.log("info", format!("SQLite backup completed for {}", cfg.name));
        Ok(file_path)
    })
    .await?
}
