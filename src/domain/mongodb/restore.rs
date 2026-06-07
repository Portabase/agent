use crate::domain::mongodb::connection::{extract_db_name, get_mongo_uri, select_mongo_path};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(cfg: DatabaseConfig, restore_file: PathBuf, logger: Arc<JobLogger>) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        logger.log("debug", format!("Starting MongoDB restore for database {}", cfg.name));

        let mongorestore = select_mongo_path().join("mongorestore");
        let uri = get_mongo_uri(cfg.clone())?;

        let dry_start = Instant::now();
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

        let dry_duration_ms = dry_start.elapsed().as_millis() as f64;
        let dry_exit_code = dry_run.status.code().unwrap_or(-1);
        let dry_output = String::from_utf8_lossy(&dry_run.stderr);
        logger.log_command(
            "mongorestore --dryRun",
            if dry_output.is_empty() { None } else { Some(dry_output.to_string()) },
            Some(dry_exit_code),
            Some(dry_duration_ms),
        );
        let source_db = extract_db_name(&dry_output).unwrap_or_else(|| {
            logger.log("info", format!("Could not detect source database from archive, falling back to configured database: {}", cfg.database));
            cfg.database.clone()
        });

        logger.log("info", format!("Using source database in archive: {}", source_db));

        let start = Instant::now();
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

        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            logger.log_command("mongorestore", Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            logger.log("error", format!("MongoDB restore failed for {}: {}", cfg.name, stderr));
            anyhow::bail!("MongoDB restore failed for: {}", cfg.name);
        }

        logger.log_command("mongorestore", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("MongoDB restore completed for {}", cfg.name));
        Ok(())
    })
    .await?
}
