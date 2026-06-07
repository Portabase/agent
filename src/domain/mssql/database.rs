use super::{backup, ping, restore};
use crate::domain::factory::Database;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct MssqlDatabase {
    cfg: DatabaseConfig,
}

impl MssqlDatabase {
    pub fn new(cfg: DatabaseConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Database for MssqlDatabase {
    fn file_extension(&self) -> &'static str {
        ".bacpac"
    }

    async fn ping(&self) -> Result<bool> {
        ping::run(self.cfg.clone()).await
    }

    async fn backup(&self, dir: &Path, logger: Arc<JobLogger>) -> Result<PathBuf> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        let res = backup::run(
            self.cfg.clone(),
            dir.to_path_buf(),
            self.file_extension(),
            logger,
        )
        .await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }

    async fn restore(&self, file: &Path, logger: Arc<JobLogger>) -> Result<()> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Restore.as_str()).await?;
        let res = restore::run(self.cfg.clone(), file.to_path_buf(), logger).await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }
}
