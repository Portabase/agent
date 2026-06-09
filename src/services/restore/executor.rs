use super::service::RestoreService;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

impl RestoreService {
    pub async fn execute_restore(&self, cfg: DatabaseConfig, file_url: String) -> Result<()> {
        let logger = Arc::new(JobLogger::new());
        let start = Instant::now();

        logger.log("info", "Database restoration job started".to_string());

        let temp_dir = TempDir::new()?;
        let tmp_path = temp_dir.path();

        logger.log("info", format!("Created temp directory {}", tmp_path.display()));

        let downloaded = self.download_backup(&file_url, tmp_path, Arc::clone(&logger)).await?;

        let backup_file = self.prepare_archive(downloaded, tmp_path, Arc::clone(&logger)).await?;

        let result = self.run_restore(cfg, backup_file, Arc::clone(&logger)).await?;

        logger.log("info", "Database restore job finished".to_string());

        let duration_ms = start.elapsed().as_millis() as f64;
        let logs = Arc::try_unwrap(logger)
            .unwrap_or_else(|_| JobLogger::new())
            .into_entries();
        self.send_result(result, logs, duration_ms).await?;

        Ok(())
    }
}
