use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use super::connection::{pg_dumpall_binary_name, psql_binary_name, select_pg_path};
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

pub fn dump(
    cfg: &DatabaseConfig,
    pg_version: &str,
    out_dir: &Path,
    env: &HashMap<String, String>,
    logger: &Arc<JobLogger>,
) -> Result<PathBuf> {
    let pg_dumpall = select_pg_path(pg_version).join(pg_dumpall_binary_name());
    let globals_path = out_dir.join("globals.sql");

    logger.log("info", format!("Dumping cluster globals via {:?}", pg_dumpall));

    let start = Instant::now();
    let output = Command::new(&pg_dumpall)
        .arg("--host").arg(&cfg.host)
        .arg("--port").arg(cfg.port.to_string())
        .arg("--username").arg(&cfg.username)
        .arg("--globals-only")
        .arg("-f").arg(&globals_path)
        .envs(env.clone())
        .output();
    let duration_ms = start.elapsed().as_millis() as f64;

    match output {
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let exit_code = o.status.code().unwrap_or(-1);

            if o.status.success() {
                logger.log_command(
                    "pg_dumpall --globals-only",
                    if stderr.is_empty() { None } else { Some(stderr) },
                    Some(0),
                    Some(duration_ms),
                );
                logger.log("info", "Globals dump completed".to_string());
                Ok(globals_path)
            } else {
                logger.log_command("pg_dumpall --globals-only", Some(stderr), Some(exit_code), Some(duration_ms));
                anyhow::bail!("pg_dumpall --globals-only failed for cluster {}:{}", cfg.host, cfg.port);
            }
        }
        Err(e) => {
            logger.log_command("pg_dumpall --globals-only", Some(e.to_string()), Some(-1), Some(duration_ms));
            Err(e.into())
        }
    }
}

/// Replays a previously captured `globals.sql` against the cluster's
/// `postgres` maintenance database. Globals are best-effort enrichment
/// (roles/tablespaces commonly already exist on a shared target cluster) —
/// this function logs failures but never returns an error, so it can never
/// block the real database restore that follows it.
pub fn apply(
    cfg: &DatabaseConfig,
    pg_version: &str,
    globals_sql: &Path,
    env: &HashMap<String, String>,
    logger: &Arc<JobLogger>,
) {
    let psql = select_pg_path(pg_version).join(psql_binary_name());

    logger.log("info", format!("Applying cluster globals from {:?}", globals_sql));

    let start = Instant::now();
    let output = Command::new(&psql)
        .arg("--host").arg(&cfg.host)
        .arg("--port").arg(cfg.port.to_string())
        .arg("--username").arg(&cfg.username)
        .arg("--dbname").arg("postgres")
        .arg("-v").arg("ON_ERROR_STOP=0")
        .arg("-f").arg(globals_sql)
        .envs(env.clone())
        .output();
    let duration_ms = start.elapsed().as_millis() as f64;

    match output {
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let combined = format!("{}{}", stdout, stderr);
            let exit_code = o.status.code();

            logger.log_command(
                "psql -f globals.sql",
                if combined.is_empty() { None } else { Some(combined) },
                exit_code,
                Some(duration_ms),
            );

            if exit_code == Some(0) {
                logger.log("info", "Globals applied successfully".to_string());
            } else {
                logger.log(
                    "warn",
                    "Globals apply reported errors (often pre-existing roles/tablespaces); continuing with database restore".to_string(),
                );
            }
        }
        Err(e) => {
            logger.log("warn", format!("Could not run psql for globals apply, continuing without globals: {:?}", e));
        }
    }
}
