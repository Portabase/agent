use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use super::{backup, ping, restore};
use crate::domain::factory::Database;
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};

pub struct MongoDatabase {
    cfg: DatabaseConfig,
}

impl MongoDatabase {
    pub fn new(cfg: DatabaseConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Database for MongoDatabase {
    fn file_extension(&self) -> &'static str {
        ".archive.gz"
    }

    async fn ping(&self) -> Result<bool> {
        ping::run(self.cfg.clone()).await
    }

    async fn backup(&self, dir: &Path) -> Result<PathBuf> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        let res = backup::run(
            self.cfg.clone(),
            dir.to_path_buf(),
            self.file_extension(),
        )
            .await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }

    async fn restore(&self, file: &Path) -> Result<()> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Restore.as_str()).await?;
        let res = restore::run(self.cfg.clone(), file.to_path_buf()).await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }
}
