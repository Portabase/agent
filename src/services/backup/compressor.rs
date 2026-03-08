use super::service::BackupService;
use crate::utils::compress::compress_to_tar_gz_large;
use std::path::PathBuf;
use anyhow::Result;

impl BackupService {

    pub async fn compress_backup(
        &self,
        backup_file: Option<PathBuf>,
    ) -> Result<PathBuf> {

        let file = backup_file
            .ok_or_else(|| anyhow::anyhow!("No backup file generated"))?;

        let compression = compress_to_tar_gz_large(&file).await?;

        Ok(compression.compressed_path)
    }
}