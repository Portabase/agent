use crate::domain::mongodb::connection::{extract_db_name, get_mongo_uri, select_mongo_path};
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        debug!("Starting MongoDB restore for database {}", cfg.name);

        let mongorestore = select_mongo_path().join("mongorestore");
        let uri = get_mongo_uri(cfg.clone())?;

        let dry_run = Command::new(&mongorestore)
            .arg(format!(
                "--uri={}",
                format!(
                    "mongodb://{}:{}@{}:{}/?authSource=admin",
                    cfg.username, cfg.password, cfg.host, cfg.port
                )
            ))
            .arg(format!("--archive={}", restore_file.display()))
            .arg("--gzip")
            .arg("--dryRun")
            .arg("--verbose")
            .output()?;

        let dry_output = String::from_utf8_lossy(&dry_run.stderr);
        let source_db = extract_db_name(&dry_output)
            .context("Could not detect source database name from archive")?;

        info!("Detected source database in archive: {}", source_db);

        let output = Command::new(&mongorestore)
            .arg(format!("--uri={}", uri))
            .arg(format!("--archive={}", restore_file.display()))
            .arg("--gzip")
            .arg("--drop")
            .arg(format!("--nsInclude={}.*", source_db))
            .arg(format!("--nsFrom={}.*", source_db))
            .arg(format!("--nsTo={}.*", cfg.database))
            .output()
            .with_context(|| format!("Failed to run mongorestore for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("MongoDB restore failed for {}: {}", cfg.name, stderr);
            anyhow::bail!("MongoDB restore failed for: {}", cfg.name);
        }

        info!("MongoDB restore completed for {}", cfg.name);
        Ok(())
    })
    .await?
}
