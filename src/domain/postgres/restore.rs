use anyhow::Result;
use tracing::{debug, error, info};
use std::path::PathBuf;
use std::process::Command;

use super::connection::{select_pg_path, server_version, terminate_connections};
use super::format::PostgresDumpFormat;
use crate::services::config::DatabaseConfig;

pub async fn run(
    cfg: DatabaseConfig,
    format: PostgresDumpFormat,
    restore_file: PathBuf,
) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        debug!("Starting restore for database {}", cfg.name);
        let version = match futures::executor::block_on(server_version(&cfg)) {
            Ok(v) => {
                debug!("Postgres version detected: {}", v);
                v
            }
            Err(e) => {
                error!("Failed to get server version for {}: {:?}", cfg.name, e);
                return Err(e.into());
            }
        };

        let pg_restore = select_pg_path(&version).join("pg_restore");
        debug!("Using pg_restore at {:?}", pg_restore);

        if let Err(e) = futures::executor::block_on(terminate_connections(&cfg)) {
            error!("Failed to terminate connections for {}: {:?}", cfg.name, e);
            return Err(e.into());
        }
        info!("Connections terminated for database {}", cfg.name);

        let url = format!(
            "postgresql://{}:{}@{}:{}/postgres",
            cfg.username, cfg.password, cfg.host, cfg.port
        );

        debug!("Restore URL: {}", url);

        match format {
            PostgresDumpFormat::Fc => {
                info!("Running FC restore for {}", cfg.name);
                let status = Command::new(&pg_restore)
                    .arg("--no-owner")
                    .arg("--no-privileges")
                    .arg("--clean")
                    .arg("--if-exists")
                    .arg("--create")
                    .arg("--dbname")
                    .arg(&url)
                    .arg("-v")
                    .arg(&restore_file)
                    .env("PGPASSWORD", &cfg.password)
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        info!("FC restore completed successfully for {}", cfg.name)
                    }
                    Ok(s) => {
                        error!("FC restore failed with status {:?} for {}", s, cfg.name);
                        anyhow::bail!("Postgres restore failed for {}", cfg.name);
                    }
                    Err(e) => {
                        error!("Error executing pg_restore for {}: {:?}", cfg.name, e);
                        return Err(e.into());
                    }
                }
            }

            PostgresDumpFormat::Fd => {
                info!("Running FD restore for {}", cfg.name);

                let tar_gz = match std::fs::File::open(&restore_file) {
                    Ok(f) => f,
                    Err(e) => {
                        error!(
                            "Failed to open restore file {:?} for {}: {:?}",
                            restore_file, cfg.name, e
                        );
                        return Err(e.into());
                    }
                };

                info!("tar_gz {:?}", tar_gz);

                let dec = flate2::read::GzDecoder::new(tar_gz);
                let mut archive = tar::Archive::new(dec);



                let tmp_dir = match tempfile::TempDir::new() {
                    Ok(d) => d,
                    Err(e) => {
                        error!(
                            "Failed to create temporary directory for FD restore of {}: {:?}",
                            cfg.name, e
                        );
                        return Err(e.into());
                    }
                };



                if let Err(e) = archive.unpack(tmp_dir.path()) {
                    error!("Failed to unpack FD archive for {}: {:?}", cfg.name, e);
                    return Err(e.into());
                }

                debug!("Listing contents of temp dir: {}", tmp_dir.path().display());


                for entry in std::fs::read_dir(tmp_dir.path())? {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        let file_type = entry.file_type()?;
                        debug!(
                            " - {} | is_dir: {} | is_file: {}",
                            path.display(),
                            file_type.is_dir(),
                            file_type.is_file()
                        );
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

                let status = Command::new(&pg_restore)
                    .arg("--no-owner")
                    .arg("--no-privileges")
                    .arg("--clean")
                    .arg("--if-exists")
                    .arg("--create")
                    .arg("--dbname")
                    .arg(&url)
                    .arg("-v")
                    .arg("-j")
                    .arg("4")
                    .arg(dump_dir)
                    .env("PGPASSWORD", &cfg.password)
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        info!("FD restore completed successfully for {}", cfg.name)
                    }
                    Ok(s) => {
                        error!("FD restore failed with status {:?} for {}", s, cfg.name);
                        anyhow::bail!("Postgres FD restore failed for {}", cfg.name);
                    }
                    Err(e) => {
                        error!("Error executing pg_restore for {}: {:?}", cfg.name, e);
                        return Err(e.into());
                    }
                }
            }
        }

        info!("Restore finished for database {}", cfg.name);

        Ok(())
    })
    .await?
}
