use crate::domain::mongodb::connection::{get_mongo_uri, select_mongo_path};
use crate::services::backup::logger::JobLogger;
use crate::services::config::{DatabaseConfig, list_option_args};
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
        logger.log("info", format!("Starting MongoDB backup for database {}", cfg.name));

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let mongodump = select_mongo_path().join("mongodump");
        let uri = get_mongo_uri(cfg.clone())?;

        let (excludes, invalid) = list_option_args(
            &cfg,
            &[
                ("exclude_collections", "--excludeCollection"),
                ("exclude_collections_with_prefix", "--excludeCollectionsWithPrefix"),
            ],
        );
        for key in invalid {
            logger.log("warn", format!("Ignoring options.{} for {}: expected a list of strings", key, cfg.name));
        }
        if !excludes.is_empty() {
            logger.log("info", format!("Excluding from dump of {}: {}", cfg.name, excludes.join(" ")));
        }

        logger.log("info", format!("Running mongodump for {}", cfg.name));

        let start = Instant::now();
        let output = Command::new(mongodump)
            .arg(format!("--uri={}", uri))
            .args(&excludes)
            .arg(format!("--archive={}", file_path.display()))
            .arg("--gzip")
            .arg("--verbose")
            .output()
            .context("MongoDB backup failed")?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            logger.log("error", format!("MongoDB backup failed for {}: {}", cfg.name, stderr));
            logger.log_command("mongodump", Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            anyhow::bail!("MongoDB backup failed for {}: {}", cfg.name, stderr);
        }

        logger.log_command("mongodump", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("MongoDB backup completed for {}", cfg.name));
        Ok(file_path)
    })
    .await?
}
