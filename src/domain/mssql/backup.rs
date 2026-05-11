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
        debug!("Starting MSSQL backup for database {}", cfg.name);

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let connection_string = format!(
            "Server=tcp:{},{};Database={};User Id={};Password={};TrustServerCertificate=True;Encrypt=False",
            cfg.host, cfg.port, cfg.database, cfg.username, cfg.password
        );

        info!(
            "MSSQL backup: {}:{}/{} → {}",
            cfg.host,
            cfg.port,
            cfg.database,
            file_path.display()
        );

        let output = Command::new("sqlpackage")
            .arg("/a:Export")
            .arg(format!("/scs:{}", connection_string))
            .arg(format!("/tf:{}", file_path.display()))
            .output()
            .with_context(|| format!("Failed to run sqlpackage for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            error!("MSSQL backup failed — stderr: {} stdout: {}", stderr, stdout);
            anyhow::bail!("MSSQL backup failed for {}: {}", cfg.name, stderr);
        }

        info!("MSSQL backup completed: {}", file_path.display());
        Ok(file_path)
    })
    .await?
}
