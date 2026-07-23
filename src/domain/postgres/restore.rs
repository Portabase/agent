use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use super::connection::{select_pg_path, server_version, terminate_connections};
use super::format::PostgresDumpFormat;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;

pub(crate) struct PreparedArchive {
    path: PathBuf,
    _tmp: Option<tempfile::TempDir>,
    toc: String,
}

impl PreparedArchive {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn toc(&self) -> &str {
        &self.toc
    }
}

pub(crate) fn prepare_archive(
    format: PostgresDumpFormat,
    restore_file: &Path,
    pg_restore: &Path,
    logger: &JobLogger,
) -> Result<PreparedArchive> {
    let (path, tmp) = match format {
        PostgresDumpFormat::Fc => (restore_file.to_path_buf(), None),
        PostgresDumpFormat::Fd => {
            let tar_gz = std::fs::File::open(restore_file)?;
            let dec = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(dec);
            let tmp_dir = tempfile::TempDir::new()?;
            archive.unpack(tmp_dir.path())?;

            let dump_dir = if tmp_dir.path().join("toc.dat").exists() {
                tmp_dir.path().to_path_buf()
            } else {
                std::fs::read_dir(tmp_dir.path())?
                    .filter_map(|e| e.ok())
                    .find(|entry| entry.path().join("toc.dat").exists())
                    .map(|e| e.path())
                    .ok_or_else(|| anyhow::anyhow!("Invalid FD archive: toc.dat not found"))?
            };
            (dump_dir, Some(tmp_dir))
        }
    };

    let toc_out = Command::new(pg_restore).arg("-l").arg(&path).output()?;
    if !toc_out.status.success() {
        let stderr = String::from_utf8_lossy(&toc_out.stderr).to_string();
        logger.log("error", format!("pg_restore -l failed: {}", stderr));
        anyhow::bail!("Archive validation failed (pg_restore -l): {}", stderr);
    }
    let toc = String::from_utf8_lossy(&toc_out.stdout).to_string();

    Ok(PreparedArchive { path, _tmp: tmp, toc })
}

pub async fn run(
    cfg: DatabaseConfig,
    format: PostgresDumpFormat,
    restore_file: PathBuf,
    env: HashMap<String, String>,
    logger: Arc<JobLogger>,
) -> Result<()> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || -> Result<()> {
        logger.log("info", format!("Starting restore for database {}", cfg.name));

        let version = match handle.block_on(server_version(&cfg)) {
            Ok(v) => {
                logger.log("debug", format!("Postgres version detected: {}", v));
                v
            }
            Err(e) => {
                logger.log("error", format!("Failed to get server version for {}: {:?}", cfg.name, e));
                return Err(e.into());
            }
        };

        let pg_restore = select_pg_path(&version).join("pg_restore");

        logger.log("debug", format!("Using pg_restore at {:?}", pg_restore));

        if let Err(e) = handle.block_on(terminate_connections(&cfg)) {
            logger.log("error", format!("Failed to terminate connections for {}: {:?}", cfg.name, e));
            return Err(e.into());
        }
        logger.log("info", format!("Connections terminated for database {}", cfg.name));

        let keep_ownership = cfg.options
            .get("keep_ownership")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if keep_ownership {
            logger.log("info", format!("Restoring ownership and privileges for {}", cfg.name));
        } else {
            logger.log("info", format!("Stripping ownership and privileges for {} (--no-owner --no-privileges)", cfg.name));
        }

        match format {
            PostgresDumpFormat::Fc => {
                logger.log("info", format!("Running FC restore for {}", cfg.name));
                let start = Instant::now();
                let mut cmd = Command::new(&pg_restore);
                if !keep_ownership {
                    cmd.arg("--no-owner").arg("--no-privileges");
                }
                let output = cmd
                    .arg("--clean")
                    .arg("--if-exists")
                    // .arg("--create")
                    .arg("--host").arg(&cfg.host)
                    .arg("--port").arg(cfg.port.to_string())
                    .arg("--username").arg(&cfg.username)
                    .arg("--dbname").arg(&cfg.database)
                    .arg("-v")
                    .arg(&restore_file)
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
                            logger.log_command("pg_restore", if combined.is_empty() { None } else { Some(combined) }, Some(0), Some(duration_ms));
                            logger.log("info", format!("FC restore completed successfully for {}", cfg.name))
                        } else {
                            logger.log_command("pg_restore", if combined.is_empty() { None } else { Some(combined) }, Some(exit_code), Some(duration_ms));
                            logger.log("error", format!("FC restore failed with status {:?} for {}", o.status, cfg.name));
                            anyhow::bail!("Postgres restore failed for {}", cfg.name);
                        }
                    }
                    Err(e) => {
                        logger.log_command("pg_restore", Some(e.to_string()), Some(-1), Some(duration_ms));
                        logger.log("error", format!("Error executing pg_restore for {}: {:?}", cfg.name, e));
                        return Err(e.into());
                    }
                }
            }

            PostgresDumpFormat::Fd => {
                logger.log("info", format!("Running FD restore for {}", cfg.name));

                let tar_gz = match std::fs::File::open(&restore_file) {
                    Ok(f) => f,
                    Err(e) => {
                        logger.log("error", format!(
                            "Failed to open restore file {:?} for {}: {:?}",
                            restore_file, cfg.name, e
                        ));
                        return Err(e.into());
                    }
                };

                logger.log("info", format!("tar_gz {:?}", tar_gz));

                let dec = flate2::read::GzDecoder::new(tar_gz);
                let mut archive = tar::Archive::new(dec);

                let tmp_dir = match tempfile::TempDir::new() {
                    Ok(d) => d,
                    Err(e) => {
                        logger.log("error", format!(
                            "Failed to create temporary directory for FD restore of {}: {:?}",
                            cfg.name, e
                        ));
                        return Err(e.into());
                    }
                };

                if let Err(e) = archive.unpack(tmp_dir.path()) {
                    logger.log("error", format!("Failed to unpack FD archive for {}: {:?}", cfg.name, e));
                    return Err(e.into());
                }

                logger.log("debug", format!("Listing contents of temp dir: {}", tmp_dir.path().display()));

                for entry in std::fs::read_dir(tmp_dir.path())? {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        let file_type = entry.file_type()?;
                        logger.log("debug", format!(
                            " - {} | is_dir: {} | is_file: {}",
                            path.display(),
                            file_type.is_dir(),
                            file_type.is_file()
                        ));
                    }
                }

                let dump_dir = if tmp_dir.path().join("toc.dat").exists() {
                    tmp_dir.path().to_path_buf()
                } else {
                    std::fs::read_dir(tmp_dir.path())?
                        .filter_map(|e| e.ok())
                        .find(|entry| entry.path().join("toc.dat").exists())
                        .map(|e| e.path())
                        .ok_or_else(|| anyhow::anyhow!("Invalid FD archive: toc.dat not found"))?
                };

                let start = Instant::now();
                let mut cmd = Command::new(&pg_restore);
                if !keep_ownership {
                    cmd.arg("--no-owner").arg("--no-privileges");
                }
                let output = cmd
                    .arg("--clean")
                    .arg("--if-exists")
                    // .arg("--create")
                    .arg("--host").arg(&cfg.host)
                    .arg("--port").arg(cfg.port.to_string())
                    .arg("--username").arg(&cfg.username)
                    .arg("--dbname").arg(&cfg.database)
                    .arg("-v")
                    .arg("-j")
                    .arg("4")
                    .arg(dump_dir)
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
                            logger.log_command("pg_restore", if combined.is_empty() { None } else { Some(combined) }, Some(0), Some(duration_ms));
                            logger.log("info", format!("FD restore completed successfully for {}", cfg.name))
                        } else {
                            logger.log_command("pg_restore", if combined.is_empty() { None } else { Some(combined) }, Some(exit_code), Some(duration_ms));
                            logger.log("error", format!("FD restore failed with status {:?} for {}", o.status, cfg.name));
                            anyhow::bail!("Postgres FD restore failed for {}", cfg.name);
                        }
                    }
                    Err(e) => {
                        logger.log_command("pg_restore", Some(e.to_string()), Some(-1), Some(duration_ms));
                        logger.log("error", format!("Error executing pg_restore for {}: {:?}", cfg.name, e));
                        return Err(e.into());
                    }
                }
            }
        }

        logger.log("info", format!("Restore finished for database {}", cfg.name));

        Ok(())
    })
    .await?
}
