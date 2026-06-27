use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use super::super::connection::{is_superuser, psql_binary_name, select_pg_path, server_version};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

pub async fn run(
    cfg: DatabaseConfig,
    restore_file: PathBuf,
    env: HashMap<String, String>,
    logger: Arc<JobLogger>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        logger.log("info", format!("Starting cluster restore for {}", cfg.name));

        let version = match futures::executor::block_on(server_version(&cfg)) {
            Ok(v) => v,
            Err(e) => {
                logger.log("error", format!("Failed to get server version for {}: {:?}", cfg.name, e));
                return Err(e.into());
            }
        };

        match futures::executor::block_on(is_superuser(&cfg)) {
            Ok(true) => {}
            Ok(false) => {
                logger.log("error", format!("postgresql-cluster restore requires a superuser role for {}", cfg.name));
                anyhow::bail!("postgresql-cluster restore requires a superuser role for {}", cfg.name);
            }
            Err(e) => {
                logger.log("error", format!("Failed to check superuser status for {}: {:?}", cfg.name, e));
                return Err(e.into());
            }
        }

        let psql = select_pg_path(&version).join(psql_binary_name());

        logger.log("info", format!("Replaying cluster dump for {} via {:?}", cfg.name, psql));

        let start = Instant::now();
        let output = Command::new(&psql)
            .arg("--host").arg(&cfg.host)
            .arg("--port").arg(cfg.port.to_string())
            .arg("--username").arg(&cfg.username)
            .arg("--dbname").arg("postgres")
            .arg("-f").arg(&restore_file)
            .envs(env)
            .output();
        let duration_ms = start.elapsed().as_millis() as f64;

        match output {
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let combined = format!("{}{}", stdout, stderr);
                let exit_code = o.status.code().unwrap_or(-1);
                if o.status.success() {
                    logger.log_command("psql", if combined.is_empty() { None } else { Some(combined) }, Some(0), Some(duration_ms));
                    logger.log("info", format!("Cluster restore completed for {}", cfg.name));
                    Ok(())
                } else {
                    logger.log_command("psql", if combined.is_empty() { None } else { Some(combined) }, Some(exit_code), Some(duration_ms));
                    anyhow::bail!("Cluster restore (psql) failed for {}", cfg.name);
                }
            }
            Err(e) => {
                logger.log_command("psql", Some(e.to_string()), Some(-1), Some(duration_ms));
                Err(e.into())
            }
        }
    })
    .await?
}
