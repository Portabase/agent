use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use super::{backup, format::PostgresDumpFormat, ping, restore};
use crate::domain::factory::Database;
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};

pub struct PostgresDatabase {
    pub cfg: DatabaseConfig,
    pub format: PostgresDumpFormat,
}

impl PostgresDatabase {
    pub fn new(cfg: DatabaseConfig, format: PostgresDumpFormat) -> Self {
        Self { cfg, format }
    }
}

#[async_trait]
impl Database for PostgresDatabase {
    fn file_extension(&self) -> &'static str {
        match self.format {
            PostgresDumpFormat::Fc => ".dump",
            PostgresDumpFormat::Fd => ".gz",
        }
    }

    async fn ping(&self) -> Result<bool> {
        ping::run(self.cfg.clone()).await
    }

    async fn backup(&self, dir: &Path, is_test: Option<bool>) -> Result<PathBuf> {
        let test_mode = is_test.unwrap_or(false);
        if !test_mode {
            FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        }
        let res = backup::run(self.cfg.clone(), self.format, dir.to_path_buf(), is_test).await;
        if !test_mode {
            FileLock::release(&self.cfg.generated_id).await?;
        }
        res
    }

    async fn restore(&self, file: &Path, is_test: Option<bool>) -> Result<()> {
        let test_mode = is_test.unwrap_or(false);
        if !test_mode {
            FileLock::acquire(&self.cfg.generated_id, DbOpLock::Restore.as_str()).await?;
        }
        let res = restore::run(self.cfg.clone(), self.format, file.to_path_buf(), is_test).await;
        if !test_mode {
            FileLock::release(&self.cfg.generated_id).await?;
        }
        res
    }
}
