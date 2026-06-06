use super::logger::JobLogger;
use super::service::BackupService;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::DatabaseConfig;
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
    ) -> Result<()> {
        let logger = Arc::new(JobLogger::new());

        if FileLock::is_locked(&generated_id).await? {
            anyhow::bail!("backup already running");
        }
        let start = Instant::now();
        logger.log("info", "Database backup job started".to_string());

        let backup = self.create_backup_record(&generated_id, &method).await?;
        let backup_id = backup.backup.id;

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
