use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, error, info};

use super::connection::{select_pg_path, server_version};
use super::format::PostgresDumpFormat;
use crate::services::config::DatabaseConfig;

pub async fn run(
    cfg: DatabaseConfig,
    format: PostgresDumpFormat,
    backup_dir: PathBuf,
) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        debug!("Starting backup for database {}", cfg.name);

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

        let pg_dump = select_pg_path(&version).join("pg_dump");
        debug!("Using pg_dump at {:?}", pg_dump);

        match format {
            PostgresDumpFormat::Fc => {
                info!("Running FC backup for {}", cfg.name);
                let file_path = backup_dir.join(format!("{}.dump", cfg.generated_id));
                let url = format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    cfg.username, cfg.password, cfg.host, cfg.port, cfg.database
                );

                let status = Command::new(&pg_dump)
                    .arg("--dbname")
                    .arg(&url)
                    .arg("-Fc")
                    .arg("-f")
                    .arg(&file_path)
                    .arg("-v")
                    .arg("--compress=3")
                    .status();

                match status {
                    Ok(s) if s.success() => info!(
                        "FC backup completed successfully for {} at {:?}",
                        cfg.name, file_path
                    ),
                    Ok(s) => {
                        error!("FC backup failed with status {:?} for {}", s, cfg.name);
                        anyhow::bail!("Postgres backup failed for {}", cfg.name);
                    }
                    Err(e) => {
                        error!("Error executing pg_dump for {}: {:?}", cfg.name, e);
                        return Err(e.into());
                    }
                }
                info!("Backup finished for database {}", cfg.name);
                Ok(file_path)
            }

            PostgresDumpFormat::Fd => {
                info!("Running FD backup for {}", cfg.name);
                let dump_dir = backup_dir.join(format!("{}_dir", cfg.generated_id));
                let tar_file = backup_dir.join(format!("{}.tar.gz", cfg.generated_id));

                if let Err(e) = std::fs::create_dir_all(&dump_dir) {
                    error!(
                        "Failed to create dump directory {:?} for {}: {:?}",
                        dump_dir, cfg.name, e
                    );
                    return Err(e.into());
                }

                let url = format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    cfg.username, cfg.password, cfg.host, cfg.port, cfg.database
                );

                let status = Command::new(&pg_dump)
                    .arg("--dbname")
                    .arg(&url)
                    .arg("-Fd")
                    .arg("-j")
                    .arg("4")
                    .arg("-f")
                    .arg(&dump_dir)
                    .arg("-v")
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        info!("FD backup pg_dump completed successfully for {}", cfg.name)
                    }
                    Ok(s) => {
                        error!(
                            "FD backup pg_dump failed with status {:?} for {}",
                            s, cfg.name
                        );
                        anyhow::bail!("Postgres FD backup failed for {}", cfg.name);
                    }
                    Err(e) => {
                        error!("Error executing pg_dump for {}: {:?}", cfg.name, e);
                        return Err(e.into());
                    }
                }

                match std::fs::File::create(&tar_file) {
                    Ok(tar_gz) => {
                        let enc =
                            flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
                        let mut tar = tar::Builder::new(enc);
                        if let Err(e) = tar.append_dir_all(".", &dump_dir) {
                            error!("Failed to append dump_dir to tar for {}: {:?}", cfg.name, e);
                            return Err(e.into());
                        }
                        if let Err(e) = tar.finish() {
                            error!("Failed to finish tar archive for {}: {:?}", cfg.name, e);
                            return Err(e.into());
                        }
                        info!("FD backup archive created at {:?}", tar_file);
                    }
                    Err(e) => {
                        error!(
                            "Failed to create tar.gz file {:?} for {}: {:?}",
                            tar_file, cfg.name, e
                        );
                        return Err(e.into());
                    }
                }
                info!("Backup finished for database {}", cfg.name);
                Ok(tar_file)
            }
        }
    })
    .await?
}
