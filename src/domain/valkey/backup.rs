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
        debug!("Starting Valkey backup for database {}", cfg.name);

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));

        let mut cmd = Command::new("valkey-cli");

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

        let output = cmd.output().context("Valkey backup command failed")?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        if !output.status.success() {
            if stderr.contains("NOAUTH") {
                error!(
                    "Valkey backup failed for {}: Authentication required (NOAUTH)",
                    cfg.name
                );
                anyhow::bail!(
                    "Valkey backup failed for {}: Authentication required",
                    cfg.name
                );
            } else {
                error!("Valkey backup failed for {}: {}", cfg.name, stderr);
                anyhow::bail!("Valkey backup failed for {}: {}", cfg.name, stderr);
            }
        }

        info!(
            "Valkey backup completed for {}. Output: {}",
            cfg.name, stdout
        );

        Ok(file_path)
    })
    .await?
}
