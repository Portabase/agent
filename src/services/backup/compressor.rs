use super::service::BackupService;
use crate::utils::compress::compress_to_tar_gz_large;
use anyhow::Result;
use std::path::PathBuf;

impl BackupService {
    pub async fn compress_backup(&self, backup_file: Option<PathBuf>) -> Result<PathBuf> {
        let file = backup_file.ok_or_else(|| anyhow::anyhow!("No backup file generated"))?;

        let compression = compress_to_tar_gz_large(&file).await?;

        Ok(compression.compressed_path)
    }
}
