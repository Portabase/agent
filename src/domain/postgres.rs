#![allow(dead_code)]

use crate::domain::factory::Database;
use crate::services::config::DatabaseConfig;
use anyhow::{Context as AnyhowContext, Result};
use async_trait::async_trait;
use flate2::Compression;
use flate2::write::GzEncoder;
use log::info;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy)]
pub enum PostgresDumpFormat {
    Fc, // legacy
    Fd, // directory format
}

pub struct PostgresDatabase {
    cfg: DatabaseConfig,
    format: PostgresDumpFormat,
}

impl PostgresDatabase {
    pub fn new(cfg: DatabaseConfig, format: PostgresDumpFormat) -> Self {
        Self { cfg, format }
    }

    pub fn detect_format_from_file(restore_file: &Path) -> PostgresDumpFormat {
        match restore_file.extension().and_then(|e| e.to_str()) {
            Some("dump") => PostgresDumpFormat::Fc,
            Some("gz") => PostgresDumpFormat::Fd,
            // Some("tar.gz") => PostgresDumpFormat::Fd,
            _ => PostgresDumpFormat::Fc,
        }
    }

    pub async fn detect_format_from_size(cfg: &DatabaseConfig) -> PostgresDumpFormat {
        let url = format!(
            "host={} port={} user={} password={} dbname={}",
            cfg.host, cfg.port, cfg.username, cfg.password, cfg.database
        );

        let output = std::process::Command::new("psql")
            .arg(&url)
            .arg("-t")
            .arg("-c")
            .arg("SELECT pg_database_size(current_database());")
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let size_bytes: i64 = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .parse()
                    .unwrap_or(0);

                // > 1 Go
                if size_bytes > 1_000_000_000 {
                    PostgresDumpFormat::Fd
                } else {
                    PostgresDumpFormat::Fc
                }
            }
            _ => PostgresDumpFormat::Fc, // fallback legacy
        }
    }
}

#[async_trait]
impl Database for PostgresDatabase {
    fn file_extension(&self) -> &'static str {
        match self.format {
            PostgresDumpFormat::Fc => ".dump",
            PostgresDumpFormat::Fd => ".tar.gz",
        }
    }

    async fn ping(&self) -> Result<bool> {
        let url = format!(
            "postgresql://{}:{}@{}:{}/{}",
            self.cfg.username, self.cfg.password, self.cfg.host, self.cfg.port, self.cfg.database
        );
        let status = Command::new("pg_isready")
            .arg("--dbname")
            .arg(url)
            .status()
            .context("Failed to ping Postgres")?;
        Ok(status.success())
    }

    async fn backup(&self, backup_dir: &Path) -> Result<PathBuf> {
        match self.format {
            PostgresDumpFormat::Fc => {
                let file_path = backup_dir.join(format!(
                    "{}{}",
                    self.cfg.generated_id,
                    self.file_extension()
                ));
                let url = format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    self.cfg.username,
                    self.cfg.password,
                    self.cfg.host,
                    self.cfg.port,
                    self.cfg.database
                );
                let status = Command::new("pg_dump")
                    .arg("--dbname")
                    .arg(url)
                    .arg("-Fc")
                    .arg("-f")
                    .arg(&file_path)
                    .arg("-v")
                    .arg("--compress=3")
                    .status()?;
                if !status.success() {
                    anyhow::bail!("Postgres backup failed for {}", self.cfg.name);
                }
                Ok(file_path)
            }
            PostgresDumpFormat::Fd => {
                // directory dump -> tar.gz
                let dump_dir = backup_dir.join(format!("{}_dir", self.cfg.generated_id));
                let tar_file = backup_dir.join(format!("{}.tar.gz", self.cfg.generated_id));
                std::fs::create_dir_all(&dump_dir)?;
                let url = format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    self.cfg.username,
                    self.cfg.password,
                    self.cfg.host,
                    self.cfg.port,
                    self.cfg.database
                );
                let status = Command::new("pg_dump")
                    .arg("--dbname")
                    .arg(url)
                    .arg("-Fd")
                    .arg("-j")
                    .arg("4")
                    .arg("-f")
                    .arg(&dump_dir)
                    .arg("-v")
                    .status()?;
                if !status.success() {
                    anyhow::bail!("Postgres Fd backup failed for {}", self.cfg.name);
                }

                // Compression tar.gz
                let tar_gz = std::fs::File::create(&tar_file)?;
                let enc = GzEncoder::new(tar_gz, Compression::default());
                let mut tar = tar::Builder::new(enc);
                tar.append_dir_all(".", &dump_dir)?;
                tar.finish()?;
                Ok(tar_file)
            }
        }
    }

    async fn restore(&self, restore_file: &Path) -> Result<()> {
        let url = format!(
            "postgresql://{}:{}@{}:{}/{}",
            self.cfg.username, self.cfg.password, self.cfg.host, self.cfg.port, "postgres"
        );

        // Terminate connections
        let terminate_cmd = format!(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname='{}' AND pid<>pg_backend_pid();",
            self.cfg.database
        );
        Command::new("psql")
            .arg("-U")
            .arg(&self.cfg.username)
            .arg("-d")
            .arg("postgres")
            .arg("-h")
            .arg(&self.cfg.host)
            .arg("-p")
            .arg(self.cfg.port.to_string())
            .arg("-c")
            .arg(&terminate_cmd)
            .env("PGPASSWORD", &self.cfg.password)
            .status()?;

        match self.format {
            PostgresDumpFormat::Fc => {
                let status = Command::new("pg_restore")
                    .arg("--no-owner")
                    .arg("--no-privileges")
                    .arg("--clean")
                    .arg("--if-exists")
                    .arg("--create")
                    .arg("--dbname")
                    .arg(url)
                    .arg("-v")
                    .arg(restore_file)
                    .env("PGPASSWORD", &self.cfg.password)
                    .status()?;
                if !status.success() {
                    anyhow::bail!("Postgres restore failed for {}", self.cfg.name);
                }
            }
            PostgresDumpFormat::Fd => {
                let tar_gz = std::fs::File::open(restore_file)?;
                let dec = flate2::read::GzDecoder::new(tar_gz);
                let mut archive = tar::Archive::new(dec);
                let tmp_dir = tempfile::TempDir::new()?;
                archive.unpack(tmp_dir.path())?;

                let dump_dir = tmp_dir.path();

                info!("Restoring dump from {}", dump_dir.display());
                let status = Command::new("pg_restore")
                    .arg("--no-owner")
                    .arg("--no-privileges")
                    .arg("--clean")
                    .arg("--if-exists")
                    .arg("--create")
                    .arg("--dbname")
                    .arg(url)
                    .arg("-v")
                    .arg(dump_dir)
                    .env("PGPASSWORD", &self.cfg.password)
                    .status()?;
                if !status.success() {
                    anyhow::bail!("Postgres Fd restore failed for {}", self.cfg.name);
                }
            }
        }
        Ok(())
    }
}
