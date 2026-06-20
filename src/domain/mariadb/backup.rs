use crate::domain::mariadb::connection::{select_mariadb_path, server_version};
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

        let version = match futures::executor::block_on(server_version(&cfg)) {
            Ok(v) => {
                logger.log("info", format!("MariaDB version detected: {}", v));
                v
            }
            Err(e) => {
                logger.log("error", format!("Failed to get server version: {}", e));
                return Err(e.into());
            }
        };

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));
        let _mariadb_dump = select_mariadb_path(&version).join("mariadb-dump");

        logger.log("debug", format!("Using mariadb-dump at {}", _mariadb_dump.display()));

        if let Ok(out) = Command::new("mariadb-dump").arg("--version").output() {
            logger.log("debug", format!("mariadb-dump client: {}", String::from_utf8_lossy(&out.stdout).trim()));
        }

        logger.log("info", format!("Running mariadb-dump for {}", cfg.name));

        let start = Instant::now();
        let output = Command::new("mariadb-dump")
            .arg("--host").arg(&cfg.host)
            .arg("--port").arg(cfg.port.to_string())
            .arg("--user").arg(&cfg.username)
            .arg("--routines")
            .arg("--events")
            .arg("--triggers")
            .arg("--single-transaction")
            .arg("--quick")
            .arg("--skip-lock-tables")
            .arg("--no-create-db")
            .arg("--skip-add-drop-table")
            .arg("--compress")
            .arg("--verbose")
            .arg(format!("--max-allowed-packet={}", cfg.max_packet_size))
            .arg("--net-buffer-length=16K")
            .arg("--default-character-set=utf8mb4")
            .arg(&cfg.database)
            .arg("-r").arg(&file_path)
            .envs(env)
            .output()
            .with_context(|| format!("Failed to run mariadb-dump for {}", cfg.name))?;
        let duration_ms = start.elapsed().as_millis() as f64;
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            logger.log_command("mariadb-dump", Some(stderr.clone()), Some(exit_code), Some(duration_ms));
            anyhow::bail!("Mariadb backup failed for {}: {}", cfg.name, stderr);
        }

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        logger.log_command("mariadb-dump", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
        logger.log("info", format!("mariadb-dump completed for {}", cfg.name));
        Ok(file_path)
    })
    .await?
}
