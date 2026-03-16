use anyhow::{Result, bail};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::domain::factory::Database;
use crate::domain::valkey::{backup, ping};
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};

pub struct ValkeyDatabase {
    cfg: DatabaseConfig,
}

impl ValkeyDatabase {
    pub fn new(cfg: DatabaseConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Database for ValkeyDatabase {
    fn file_extension(&self) -> &'static str {
        ".rdb"
    }

    async fn ping(&self) -> Result<bool> {
        ping::run(self.cfg.clone()).await
    }

    async fn backup(&self, dir: &Path, is_test: Option<bool>) -> Result<PathBuf> {
        let test_mode = is_test.unwrap_or(false);
        if !test_mode {
            FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        }
        let res = backup::run(self.cfg.clone(), dir.to_path_buf(), self.file_extension()).await;
        if !test_mode {
            FileLock::release(&self.cfg.generated_id).await?;
        }
        res
    }

    async fn restore(&self, _file: &Path, _is_test: Option<bool>) -> Result<()> {
        bail!("Restore not supported for Valkey databases")
    }
}
