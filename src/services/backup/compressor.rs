use super::logger::JobLogger;
use super::service::BackupService;
use crate::utils::compress::compress_to_tar_gz_large;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

impl BackupService {
    pub async fn compress_backup(&self, backup_file: Option<PathBuf>, logger: Arc<JobLogger>) -> Result<PathBuf> {
        let file = backup_file.ok_or_else(|| anyhow::anyhow!("No backup file generated"))?;

        logger.log("info", "Start compressing archive".to_string());
        let compression = compress_to_tar_gz_large(&file, logger).await?;
        
        Ok(compression.compressed_path)
    }
}
