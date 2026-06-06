use crate::domain::mysql::connection::server_version;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info};

pub async fn run(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    env: HashMap<String, String>,
    file_extension: &'static str,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        debug!("Starting backup for database {}", cfg.name);
        logger.log("debug", format!("Starting backup for database {}", cfg.name));

        let version = match futures::executor::block_on(server_version(&cfg)) {
            Ok(v) => {
                debug!("Mysql version detected: {}", v);
                logger.log("debug", format!("MySQL version detected: {}", v));
                v
            }
            Err(e) => {
                error!("Failed to get server version for {}: {:?}", cfg.name, e);
                logger.log("error", format!("Failed to get server version: {}", e));
                return Err(e.into());
            }
        };

        info!("Mysql version found: {}", version);

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let cmd_label = format!("mysqldump --host {} --port {} --user {} {}", cfg.host, cfg.port, cfg.username, cfg.database);

        logger.log("info", format!("Running mysqldump for {}", cfg.name));

        let start = Instant::now();
        let output = Command::new("mysqldump")
            .arg("--host").arg(&cfg.host)
            .arg("--port").arg(cfg.port.to_string())
            .arg("--user").arg(&cfg.username)
            .arg("--routines")
            .arg("--events")
            .arg("--triggers")
            .arg("--verbose")
            .arg("--single-transaction")
            .arg("--quick")
            .arg("--skip-lock-tables")
            .arg("--skip-add-drop-table")
            .arg("--no-create-db")
            .arg("--default-character-set=utf8mb4")
            .arg(&cfg.database)
            .arg("-r").arg(&file_path)
            .envs(env)
            .output()
            .with_context(|| format!("Failed to run mysqldump for {}", cfg.name))?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            info!("mysqldump stderr: {}", stderr);
            logger.log_command(cmd_label, Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            anyhow::bail!("MySQL backup failed for {}: {}", cfg.name, stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        info!("Output {}", stdout);
        logger.log_command(cmd_label, if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("mysqldump completed for {}", cfg.name));

        Ok(file_path)
    })
    .await?
}
