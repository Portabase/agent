use crate::domain::mongodb::connection::{get_mongo_uri, select_mongo_path};
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
        debug!("Starting MongoDB backup for database {}", cfg.name);

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let mongodump = select_mongo_path().join("mongodump");
        let uri = get_mongo_uri(cfg.clone());

        let output = Command::new(mongodump)
            .arg(format!("--uri={}", uri))
            .arg(format!("--archive={}", file_path.display()))
            .arg("--gzip")
            .output()
            .context("MongoDB backup failed")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("MongoDB backup failed for {}: {}", cfg.name, stderr);
            anyhow::bail!("MongoDB backup failed for {}: {}", cfg.name, stderr);
        }
        info!("MongoDB backup completed for {}", cfg.name);
        Ok(file_path)
    })
    .await?
}
