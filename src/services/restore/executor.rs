use super::service::RestoreService;
use crate::services::config::DatabaseConfig;
use tempfile::TempDir;
use anyhow::Result;
use tracing::info;

impl RestoreService {

    pub async fn execute_restore(
        &self,
        cfg: DatabaseConfig,
        file_url: String,
    ) -> Result<()> {

        let temp_dir = TempDir::new()?;
        let tmp_path = temp_dir.path();

        info!("Created temp directory {}", tmp_path.display());

        let downloaded = self.download_backup(&file_url, tmp_path).await?;

        let backup_file = self.prepare_archive(downloaded, tmp_path).await?;

        let result = self.run_restore(cfg, backup_file).await?;

        self.send_result(result).await;

        Ok(())
    }
}