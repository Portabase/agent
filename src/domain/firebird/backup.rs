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
        logger.log("debug", format!("Starting Firebird backup for database: {}", cfg.name));

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let db_path = format!("{}/{}:{}", cfg.host, cfg.port, cfg.database);

        logger.log("info", format!("Firebird target: {} → {}", db_path, file_path.display()));

        let cmd_label = format!("gbak -b -v -user {} {}", cfg.username, db_path);

        let start = Instant::now();
        let output = Command::new("gbak")
            .arg("-b")
            .arg("-v")
            .arg("-user").arg(&cfg.username)
            .arg("-password").arg(&cfg.password)
            .arg(&db_path)
            .arg(&file_path)
            .output()
            .with_context(|| format!("Failed to run gbak for {}", cfg.name))?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let combined_output = if stderr.is_empty() && stdout.is_empty() {
            None
        } else {
            Some(format!("{}{}", stdout, stderr).trim().to_string())
        };

        if !output.status.success() {
            logger.log_command(cmd_label, combined_output, Some(exit_code), Some(duration_ms));
            anyhow::bail!("Firebird backup failed for {}: {}", cfg.name, stderr);
        }

        logger.log_command(cmd_label, combined_output, Some(0), Some(duration_ms));
        logger.log("info", format!("Firebird backup completed for {}", cfg.name));
        Ok(file_path)
    })
    .await?
}
