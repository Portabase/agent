use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{cluster, ping};
use crate::domain::factory::Database;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};

pub struct PostgresClusterDatabase {
    pub cfg: DatabaseConfig,
}

impl PostgresClusterDatabase {
    pub fn new(cfg: DatabaseConfig) -> Self {
        Self { cfg }
    }

    fn build_env(&self) -> HashMap<String, String> {
        let mut envs = std::env::vars().collect::<HashMap<_, _>>();
        envs.insert("PGPASSWORD".to_string(), self.cfg.password.to_string());
        envs
    }
}

#[async_trait]
impl Database for PostgresClusterDatabase {
    fn file_extension(&self) -> &'static str {
        ".sql"
    }

    async fn ping(&self) -> Result<bool> {
        ping::run(self.cfg.clone()).await
    }

    async fn backup(&self, dir: &Path, logger: Arc<JobLogger>) -> Result<PathBuf> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        let res = cluster::backup(self.cfg.clone(), dir.to_path_buf(), self.build_env(), logger).await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }

    async fn restore(&self, file: &Path, logger: Arc<JobLogger>) -> Result<()> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Restore.as_str()).await?;
        let res = cluster::restore(self.cfg.clone(), file.to_path_buf(), self.build_env(), logger).await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }
}
