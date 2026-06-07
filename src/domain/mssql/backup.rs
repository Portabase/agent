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
        logger.log("info", format!("Starting MSSQL backup for database {}", cfg.name));

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let connection_string = format!(
            "Server=tcp:{},{};Database={};User Id={};Password={};TrustServerCertificate=True;Encrypt=False",
            cfg.host, cfg.port, cfg.database, cfg.username, cfg.password
        );

        logger.log("info", format!("MSSQL backup: {}:{}/{} → {}", cfg.host, cfg.port, cfg.database, file_path.display()));

        let start = Instant::now();
        let output = Command::new("sqlpackage")
            .arg("/a:Export")
            .arg(format!("/scs:{}", connection_string))
            .arg(format!("/tf:{}", file_path.display()))
            .output()
            .with_context(|| format!("Failed to run sqlpackage for {}", cfg.name))?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        if !output.status.success() {
            logger.log("error", format!("MSSQL backup failed — stderr: {} stdout: {}", stderr, stdout));
            let out = format!("stderr: {} stdout: {}", stderr, stdout);
            logger.log_command("sqlpackage", Some(out), Some(exit_code), Some(duration_ms));
            anyhow::bail!("MSSQL backup failed for {}: {}", cfg.name, stderr);
        }

        let combined = if stdout.is_empty() && stderr.is_empty() {
            None
        } else {
            Some(format!("{}{}", stdout, stderr).trim().to_string())
        };
        logger.log_command("sqlpackage", combined, Some(0), Some(duration_ms));
        logger.log("info", format!("MSSQL backup completed for {}", cfg.name));
        Ok(file_path)
    })
    .await?
}
