use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        debug!("Starting MSSQL restore for database {}", cfg.name);

        let server = format!("tcp:{},{}", cfg.host, cfg.port);

        info!(
            "MSSQL restore: {} → {}:{}/{}",
            restore_file.display(),
            cfg.host,
            cfg.port,
            cfg.database
        );

        let output = Command::new("sqlpackage")
            .arg("/a:Import")
            .arg(format!("/tsn:{}", server))
            .arg(format!("/tu:{}", cfg.username))
            .arg(format!("/tp:{}", cfg.password))
            .arg(format!("/tdn:{}", cfg.database))
            .arg(format!("/sf:{}", restore_file.display()))
            .output()
            .with_context(|| format!("Failed to run sqlpackage restore for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            error!(
                "MSSQL restore failed for {} — stderr: {} stdout: {}",
                cfg.name, stderr, stdout
            );
            anyhow::bail!("MSSQL restore failed for {}: {}", cfg.name, stderr);
        }

        info!("MSSQL restore completed for {}", cfg.name);
        Ok(())
    })
    .await?
}
