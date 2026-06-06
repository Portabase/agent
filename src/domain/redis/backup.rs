use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    file_extension: &'static str,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        logger.log(
            "debug",
            format!("Starting Redis backup for database {}", cfg.name),
        );

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));

        let mut cmd = Command::new("redis-cli");
        cmd.arg("-h")
            .arg(&cfg.host)
            .arg("-p")
            .arg(cfg.port.to_string());

        if !cfg.username.is_empty() {
            cmd.arg("--user").arg(&cfg.username);
        }
        if !cfg.password.is_empty() {
            cmd.arg("-a").arg(&cfg.password);
        }
        cmd.arg("--rdb").arg(&file_path);

        logger.log("info", format!("Running redis-cli --rdb for {}", cfg.name));

        let start = Instant::now();
        let output = cmd.output().context("Redis backup command failed")?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        if !output.status.success() {
            if stderr.contains("NOAUTH") {
                logger.log(
                    "error",
                    format!(
                        "Redis backup failed for {}: Authentication required (NOAUTH)",
                        cfg.name
                    ),
                );
                logger.log_command(
                    "redis-cli",
                    Some("Authentication required (NOAUTH)".into()),
                    Some(exit_code),
                    Some(duration_ms),
                );
                anyhow::bail!(
                    "Redis backup failed for {}: Authentication required",
                    cfg.name
                );
            } else {
                logger.log(
                    "error",
                    format!("Redis backup failed for {}: {}", cfg.name, stderr),
                );
                logger.log_command(
                    "redis-cli",
                    Some(stderr.to_string()),
                    Some(exit_code),
                    Some(duration_ms),
                );
                anyhow::bail!("Redis backup failed for {}: {}", cfg.name, stderr);
            }
        }

        logger.log_command(
            "redis-cli",
            if stdout.is_empty() {
                None
            } else {
                Some(stdout.to_string())
            },
            Some(0),
            Some(duration_ms),
        );
        logger.log("info", format!("Redis backup completed for {}", cfg.name));

        Ok(file_path)
    })
    .await?
}
