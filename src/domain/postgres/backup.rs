use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use super::connection::{select_pg_path, server_version};
use super::format::PostgresDumpFormat;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

pub async fn run(
    cfg: DatabaseConfig,
    format: PostgresDumpFormat,
    backup_dir: PathBuf,
    logger: Arc<JobLogger>,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        logger.log("info", format!("Starting backup for database {}", cfg.name));

        let version = match futures::executor::block_on(server_version(&cfg)) {
            Ok(v) => {
                logger.log("debug", format!("Postgres version detected: {}", v));
                v
            }
            Err(e) => {
                logger.log("error", format!("Failed to get server version: {}", e));
                return Err(e.into());
            }
        };

        let pg_dump = select_pg_path(&version).join("pg_dump");
        logger.log("debug", format!("Using pg_dump at {:?}", pg_dump));

        match format {
            PostgresDumpFormat::Fc => {
                logger.log("info", format!("Running FC backup for {}", cfg.name));

                let file_path = backup_dir.join(format!("{}.dump", cfg.generated_id));
                let url = format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    cfg.username, cfg.password, cfg.host, cfg.port, cfg.database
                );

                let start = Instant::now();
                let output = Command::new(&pg_dump)
                    .arg("--dbname").arg(&url)
                    .arg("-Fc")
                    .arg("-f").arg(&file_path)
                    .arg("-v")
                    .arg("--compress=3")
                    .output();
                let duration_ms = start.elapsed().as_millis() as f64;

                match output {
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                        let exit_code = o.status.code().unwrap_or(-1);

                        if o.status.success() {
                            logger.log("info", format!("FC backup completed successfully for {} at {:?}", cfg.name, file_path));
                            logger.log_command("pg_dump", if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
                        } else {
                            logger.log("error", format!("FC backup failed with status {:?} for {}", o.status, cfg.name));
                            logger.log_command("pg_dump", Some(stderr), Some(exit_code), Some(duration_ms));
                            anyhow::bail!("Postgres backup failed for {}", cfg.name);
                        }
                    }
                    Err(e) => {
                        logger.log("error", format!("Error executing pg_dump for {}: {:?}", cfg.name, e));
                        logger.log_command("pg_dump", Some(e.to_string()), Some(-1), Some(duration_ms));
                        return Err(e.into());
                    }
                }
                logger.log("info", format!("Backup finished for database {}", cfg.name));
                Ok(file_path)
            }

            PostgresDumpFormat::Fd => {
                logger.log("info", format!("Running FD backup for {}", cfg.name));

                let dump_dir = backup_dir.join(format!("{}_dir", cfg.generated_id));
                let tar_file = backup_dir.join(format!("{}.tar.gz", cfg.generated_id));

                if let Err(e) = std::fs::create_dir_all(&dump_dir) {
                    logger.log("error", format!("Failed to create dump directory {:?} for {}: {:?}", dump_dir, cfg.name, e));
                    return Err(e.into());
                }

                let url = format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    cfg.username, cfg.password, cfg.host, cfg.port, cfg.database
                );
                let cmd_label = format!("pg_dump -Fd {}", url);

                let start = Instant::now();
                let output = Command::new(&pg_dump)
                    .arg("--dbname").arg(&url)
                    .arg("-Fd")
                    .arg("-j").arg("4")
                    .arg("-f").arg(&dump_dir)
                    .arg("-v")
                    .output();
                let duration_ms = start.elapsed().as_millis() as f64;

                match output {
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                        let exit_code = o.status.code().unwrap_or(-1);
                        if o.status.success() {
                            logger.log("info", format!("FD backup pg_dump completed successfully for {}", cfg.name));
                            logger.log_command(cmd_label, if stderr.is_empty() { None } else { Some(stderr) }, Some(0), Some(duration_ms));
                        } else {
                            logger.log("error", format!("FD backup pg_dump failed with status {:?} for {}", o.status, cfg.name));
                            logger.log_command(cmd_label, Some(stderr), Some(exit_code), Some(duration_ms));
                            anyhow::bail!("Postgres FD backup failed for {}", cfg.name);
                        }
                    }
                    Err(e) => {
                        logger.log("error", format!("Error executing pg_dump for {}: {:?}", cfg.name, e));
                        logger.log_command(cmd_label, Some(e.to_string()), Some(-1), Some(duration_ms));
                        return Err(e.into());
                    }
                }

                match std::fs::File::create(&tar_file) {
                    Ok(tar_gz) => {
                        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
                        let mut tar = tar::Builder::new(enc);
                        if let Err(e) = tar.append_dir_all(".", &dump_dir) {
                            logger.log("error", format!("Failed to append dump_dir to tar for {}: {:?}", cfg.name, e));
                            return Err(e.into());
                        }
                        if let Err(e) = tar.finish() {
                            logger.log("error", format!("Failed to finish tar archive for {}: {:?}",  cfg.name, e));
                            return Err(e.into());
                        }
                        logger.log("info", format!("FD backup archive created at {:?}", tar_file));
                    }
                    Err(e) => {
                        logger.log("error", format!("Failed to create tar.gz file {:?} for {}: {:?}", tar_file, cfg.name, e));
                        return Err(e.into());
                    }
                }
                logger.log("info", format!("Backup finished for database {}", cfg.name));
                Ok(tar_file)
            }
        }
    })
    .await?
}
