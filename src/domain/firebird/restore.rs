use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf, logger: Arc<JobLogger>) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        logger.log("debug", format!("Starting Firebird restore for database {}", cfg.name));

        let db_path = format!("{}/{}:{}", cfg.host, cfg.port, cfg.database);

        logger.log("info", format!("Restore source: {}", restore_file.display()));
        logger.log("info", format!("Restore target: {}", db_path));

        let start = Instant::now();
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

        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            logger.log_command("gbak", Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            logger.log("error", format!("Firebird restore failed for {}: {}", cfg.name, stderr));
            anyhow::bail!("Firebird restore failed for {}: {}", cfg.name, stderr);
        }

        logger.log_command("gbak", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("Firebird restore completed for {}", cfg.name));

        Ok(())
    })
    .await?
}
