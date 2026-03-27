use anyhow::{Result, bail};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::domain::factory::Database;
use crate::domain::redis::{backup, ping};
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};

pub struct RedisDatabase {
    cfg: DatabaseConfig,
}

impl RedisDatabase {
    pub fn new(cfg: DatabaseConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Database for RedisDatabase {
    fn file_extension(&self) -> &'static str {
        ".rdb"
    }

    async fn ping(&self) -> Result<bool> {
        ping::run(self.cfg.clone()).await
    }

    async fn backup(&self, dir: &Path) -> Result<PathBuf> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        let res = backup::run(self.cfg.clone(), dir.to_path_buf(), self.file_extension()).await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }

    async fn restore(&self, _file: &Path) -> Result<()> {
        bail!("Restore not supported for Redis databases")
    }
}
