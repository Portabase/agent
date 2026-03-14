use std::collections::HashMap;
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use super::{
    backup,
    ping, restore,
};
use crate::domain::factory::Database;
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};

pub struct MySQLDatabase {
    cfg: DatabaseConfig,
}

impl MySQLDatabase {
    pub fn new(cfg: DatabaseConfig) -> Self {
        Self { cfg }
    }

    fn build_env(&self) -> HashMap<String, String> {
        let mut envs = std::env::vars().collect::<HashMap<_, _>>();
        envs.insert("MYSQL_PWD".to_string(), self.cfg.password.to_string());
        envs
    }
}

#[async_trait]
impl Database for MySQLDatabase {
    fn file_extension(&self) -> &'static str {
        ".sql"
    }

    async fn ping(&self) -> Result<bool> {
        ping::run(self.cfg.clone(), self.build_env().clone()).await
    }


    async fn backup(&self, dir: &Path, is_test: Option<bool>) -> Result<PathBuf> {
        let test_mode = is_test.unwrap_or(false);
        if !test_mode {
            FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        }
        let res = backup::run(self.cfg.clone(), dir.to_path_buf(), self.build_env().clone(), self.file_extension()).await;
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
        let res = restore::run(self.cfg.clone(), file.to_path_buf()).await;
        if !test_mode {
            FileLock::release(&self.cfg.generated_id).await?;
        }
        res
    }
}
