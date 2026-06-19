use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;
use super::{backup, format::PostgresDumpFormat, ping, restore};
use crate::domain::factory::Database;
use crate::services::backup::logger::JobLogger;
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

    fn build_env(&self) -> HashMap<String, String> {
        let mut envs = std::env::vars().collect::<HashMap<_, _>>();
        envs.insert("PGPASSWORD".to_string(), self.cfg.password.to_string());
        info!("envs: {:?}", envs);
        envs
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

    async fn backup(&self, dir: &Path, logger: Arc<JobLogger>) -> Result<PathBuf> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Backup.as_str()).await?;
        let res = backup::run(
            self.cfg.clone(),
            self.format,
            dir.to_path_buf(),
            self.build_env(),
            logger,
        )
        .await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }

    async fn restore(&self, file: &Path, logger: Arc<JobLogger>) -> Result<()> {
        FileLock::acquire(&self.cfg.generated_id, DbOpLock::Restore.as_str()).await?;
        let res = restore::run(
            self.cfg.clone(),
            self.format,
            file.to_path_buf(),
            self.build_env(),
            logger,
        )
        .await;
        FileLock::release(&self.cfg.generated_id).await?;
        res
    }
}
