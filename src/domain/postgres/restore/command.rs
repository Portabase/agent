use anyhow::Result;
use std::process::Command;
use std::time::Instant;

use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

pub(crate) fn run_pg_restore(
    mut cmd: Command,
    logger: &JobLogger,
    cfg: &DatabaseConfig,
) -> Result<()> {
    let start = Instant::now();
    let output = cmd.output();
    let duration_ms = start.elapsed().as_millis() as f64;

    match output {
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let combined = format!("{}{}", stdout, stderr);
            let exit_code = o.status.code().unwrap_or(-1);
            let payload = if combined.is_empty() { None } else { Some(combined) };

            if o.status.success() {
                logger.log_command("pg_restore", payload, Some(0), Some(duration_ms));
                logger.log("info", format!("Restore completed successfully for {}", cfg.name));
                Ok(())
            } else {
                logger.log_command("pg_restore", payload, Some(exit_code), Some(duration_ms));
                logger.log("error", format!("Restore failed with status {:?} for {}", o.status, cfg.name));
                anyhow::bail!("Postgres restore failed for {}", cfg.name);
            }
        }
        Err(e) => {
            logger.log_command("pg_restore", Some(e.to_string()), Some(-1), Some(duration_ms));
            logger.log("error", format!("Error executing pg_restore for {}: {:?}", cfg.name, e));
            Err(e.into())
        }
    }
}
