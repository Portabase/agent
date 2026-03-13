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
        debug!("Starting Redis backup for database {}", cfg.name);

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

        debug!("Command Backup: {:?}", cmd);

        let output = cmd.output().context("Redis backup command failed")?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        if !output.status.success() {
            if stderr.contains("NOAUTH") {
                error!(
                    "Redis backup failed for {}: Authentication required (NOAUTH)",
                    cfg.name
                );
                anyhow::bail!(
                    "Redis backup failed for {}: Authentication required",
                    cfg.name
                );
            } else {
                error!("Redis backup failed for {}: {}", cfg.name, stderr);
                anyhow::bail!("Redis backup failed for {}: {}", cfg.name, stderr);
            }
        }

        info!(
            "Redis backup completed for {}. Output: {}",
            cfg.name, stdout
        );

        Ok(file_path)
    })
    .await?
}
