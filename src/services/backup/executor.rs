use super::logger::JobLogger;
use super::models::BackupResult;
use super::service::BackupService;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::{DatabaseConfig, DbType};
use crate::services::restic;
use crate::services::sync;
use crate::utils::common::BackupMethod;
use crate::utils::locks::FileLock;

use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

impl BackupService {
    pub async fn execute_backup(
        &self,
        generated_id: String,
        db_cfg: DatabaseConfig,
        method: BackupMethod,
        storages: Vec<DatabaseStorage>,
        encrypt: bool,
        engine: String,
    ) -> Result<()> {
        let logger = Arc::new(JobLogger::new());

        if FileLock::is_locked(&generated_id).await? {
            anyhow::bail!("backup already running");
        }
        let start = Instant::now();
        logger.log("info", "Database backup job started".to_string());

        let backup = self.create_backup_record(&generated_id, &method).await?;
        let backup_id = backup.backup.id;

        if engine == "restic" || engine == "sync" {
            if matches!(db_cfg.db_type, DbType::Files) {
                let uploads = if engine == "restic" {
                    restic::backup::run(&self.ctx, &db_cfg, &storages, &backup_id, &logger).await
                } else {
                    sync::backup::run(&self.ctx, &db_cfg, &storages, &backup_id, &logger).await
                };
                logger.log("info", "Database backup job finished".to_string());
                let result = BackupResult {
                    generated_id: generated_id.clone(),
                    db_type: db_cfg.db_type.clone(),
                    status: "success".into(),
                    backup_file: None,
                    code: None,
                };
                let duration_ms = start.elapsed().as_millis() as f64;
                let logs = Arc::try_unwrap(logger).unwrap_or_else(|_| JobLogger::new()).into_entries();
                // fileSize = average logical size across storages (send_result): restic reports
                // the size each snapshot processed; sync reports none, so it stays null.
                self.send_result(result, uploads, &backup_id, logs, duration_ms).await?;
                return Ok(());
            }
            logger.log("warn", format!("The {engine} method only applies to files sources; making an archive"));
        }

        let temp_dir = TempDir::new()?;
        let tmp_path = temp_dir.path();

        let mut result = Self::run(db_cfg, tmp_path, Arc::clone(&logger)).await?;

        if result.status == "failed" {
            let duration_ms = start.elapsed().as_millis() as f64;
            let logs = Arc::try_unwrap(logger).unwrap_or_else(|_| JobLogger::new()).into_entries();
            self.send_result(result, vec![], &backup_id, logs, duration_ms).await?;
            return Ok(());
        }

        let compressed = self.compress_backup(result.backup_file.take(), Arc::clone(&logger)).await?;
        result.backup_file = Some(compressed);

        let uploads = self
            .upload(result.clone(), method, storages, encrypt, &backup_id, Arc::clone(&logger))
            .await?;

        logger.log("info", "Database backup job finished".to_string());

        let duration_ms = start.elapsed().as_millis() as f64;
        let logs = Arc::try_unwrap(logger).unwrap_or_else(|_| JobLogger::new()).into_entries();
        self.send_result(result, uploads, &backup_id, logs, duration_ms).await?;

        Ok(())
    }
}
