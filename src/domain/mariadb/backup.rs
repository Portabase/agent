use crate::domain::mariadb::connection::{select_mariadb_path, server_version};
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
                debug!("Mariadb version detected: {}", v);
                v
            }
            Err(e) => {
                error!("Failed to get server version for {}: {:?}", cfg.name, e);
                return Err(e.into());
            }
        };

        info!("Mariadb version found: {}", version);

        let file_path = backup_dir.join(format!("{}{}", cfg.generated_id, file_extension));

        let mariadb_dump = select_mariadb_path(&version).join("mariadb-dump");
        info!("Mariadb dump found: {}", mariadb_dump.display());


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
            .arg("--max-allowed-packet=512M")
            .arg("--net-buffer-length=16K")
            .arg("--default-character-set=utf8mb4")
            .arg(&cfg.database)
            .arg("-r").arg(&file_path)
            .envs(env)
            .output()
            .with_context(|| format!("Failed to run mariadb-dump for {}", cfg.name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Mariadb backup failed for {}: {}", cfg.name, stderr);
        }

        Ok(file_path)
    })
    .await?
}
