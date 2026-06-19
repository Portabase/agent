use crate::domain::mysql::connection::server_version;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub async fn run(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    env: HashMap<String, String>,
    file_extension: &'static str,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        logger.log("info", format!("Starting backup for database {}", cfg.name));

        let _version = match futures::executor::block_on(server_version(&cfg)) {
            Ok(v) => {
                logger.log("debug", format!("MySQL version detected: {}", v));
                v
            }
            Err(e) => {
                logger.log("error", format!("Failed to get server version: {}", e));
                return Err(e.into());
            }
        };

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));

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
            .arg(format!("--net-read-timeout={}", cfg.net_read_timeout))
            .arg(format!("--net-write-timeout={}", cfg.net_write_timeout))
            .arg(format!("--max-allowed-packet={}", cfg.max_allowed_packet))
            .arg(&cfg.database)
            .arg("-r").arg(&file_path)
            .envs(env)
            .output()
            .with_context(|| format!("Failed to run mysqldump for {}", cfg.name))?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        let _stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            logger.log_command("mysqldump", Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            anyhow::bail!("MySQL backup failed for {}: {}", cfg.name, stderr);
        }

        logger.log_command("mysqldump", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("mysqldump completed for {}", cfg.name));

        Ok(file_path)
    })
    .await?
}
