use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use super::connection::{
    is_superuser, pg_dumpall_binary_name, psql_binary_name, select_pg_path, server_version,
};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

/// Backs up an entire PostgreSQL cluster (roles + all databases + ownership +
/// privileges) with `pg_dumpall` into a single `.sql`. Requires a superuser.
pub async fn backup(
    cfg: DatabaseConfig,
    backup_dir: PathBuf,
    env: HashMap<String, String>,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        logger.log("info", format!("Starting cluster backup for {}", cfg.name));

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
                logger.log("error", format!("postgresql-cluster backup requires a superuser role for {}", cfg.name));
                anyhow::bail!("postgresql-cluster backup requires a superuser role for {}", cfg.name);
            }
            Err(e) => {
                logger.log("error", format!("Failed to check superuser status for {}: {:?}", cfg.name, e));
                return Err(e.into());
            }
        }

        let pg_dumpall = select_pg_path(&version).join(pg_dumpall_binary_name());
        let file_path = backup_dir.join(format!("{}.sql", cfg.generated_id));

        logger.log("info", format!("Running pg_dumpall for cluster {} via {:?}", cfg.name, pg_dumpall));

        let start = Instant::now();
        let output = Command::new(&pg_dumpall)
            .arg("--host").arg(&cfg.host)
            .arg("--port").arg(cfg.port.to_string())
            .arg("--username").arg(&cfg.username)
            .arg("-f").arg(&file_path)
            .envs(env)
            .output();
        let duration_ms = start.elapsed().as_millis() as f64;

        match output {
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let exit_code = o.status.code().unwrap_or(-1);
                if o.status.success() {
                    logger.log_command("pg_dumpall", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
                    logger.log("info", format!("Cluster backup completed for {} at {:?}", cfg.name, file_path));
                    Ok(file_path)
                } else {
                    logger.log_command("pg_dumpall", Some(stderr), Some(exit_code), Some(duration_ms));
                    anyhow::bail!("Cluster backup (pg_dumpall) failed for {}", cfg.name);
                }
            }
            Err(e) => {
                logger.log_command("pg_dumpall", Some(e.to_string()), Some(-1), Some(duration_ms));
                Err(e.into())
            }
        }
    })
    .await?
}

/// Restores a cluster `.sql` produced by [`backup`] via `psql` against a fresh
/// target cluster. Requires a superuser. psql runs continue-on-error (its
/// default); a non-zero process exit is treated as failure.
pub async fn restore(
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
