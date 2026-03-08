use super::service::BackupService;
use crate::services::config::DatabaseConfig;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::utils::common::BackupMethod;
use crate::utils::locks::FileLock;

use tempfile::TempDir;
use anyhow::Result;

impl BackupService {

    pub async fn execute_backup(
        &self,
        generated_id: String,
        db_cfg: DatabaseConfig,
        method: BackupMethod,
        storages: Vec<DatabaseStorage>,
        encrypt: bool,
    ) -> Result<()> {

        if FileLock::is_locked(&generated_id).await? {
            anyhow::bail!("backup already running");
        }
        
        let backup = self.create_backup_record(&generated_id, &method).await?;
        let backup_id = backup.backup.id;

        let temp_dir = TempDir::new()?;
        let tmp_path = temp_dir.path();

        let mut result = Self::run(db_cfg, tmp_path).await?;

        if result.status == "failed" {
            self.send_result(result, vec![], &backup_id).await?;
            return Ok(());
        }

        let compressed = self.compress_backup(result.backup_file.take()).await?;
        result.backup_file = Some(compressed);

        let uploads = self
            .upload(result.clone(), method, storages, encrypt, &backup_id)
            .await?;

        self.send_result(result, uploads, &backup_id).await?;

        Ok(())
    }
}