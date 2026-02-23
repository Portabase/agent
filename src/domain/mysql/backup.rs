use crate::domain::mysql::connection::{server_version};
use crate::services::config::DatabaseConfig;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

pub async fn run(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    env: HashMap<String, String>,
    file_extension: &'static str,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        debug!("Starting backup for database {}", cfg.name);

        let version = match futures::executor::block_on(server_version(&cfg)) {
            Ok(v) => {
                debug!("Mysql version detected: {}", v);
                v
            }
            Err(e) => {
                error!("Failed to get server version for {}: {:?}", cfg.name, e);
                return Err(e.into());
            }
        };

        info!("Mysql version found: {}", version);

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));

        let output = Command::new("mysqldump")
            .arg("--host")
            .arg(cfg.host)
            .arg("--port")
            .arg(cfg.port.to_string())
            .arg("--user")
            .arg(cfg.username)
            .arg("--routines")
            .arg("--events")
            .arg("--triggers")
            .arg("--verbose")
            .arg("--single-transaction")
            .arg("--quick")
            .arg("--add-drop-database")
            .arg("--databases")
            .arg(cfg.database)
            .arg("-r")
            .arg(&file_path)
            .envs(env)
            .output()
            .with_context(|| format!("Failed to run mysqldump for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("MySQL backup failed for {}: {}", cfg.name, stderr);
        }

        Ok(file_path)
    })
    .await?
}
