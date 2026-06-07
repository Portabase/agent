use super::service::RestoreService;

use crate::utils::compress::decompress_large_tar_gz;
use crate::utils::file::decrypt_file_stream_gcm;

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::services::backup::logger::JobLogger;

impl RestoreService {
    pub async fn prepare_archive(
        &self,
        downloaded_file: PathBuf,
        tmp_path: &Path,
        logger: Arc<JobLogger>
    ) -> Result<PathBuf> {
        logger.log("info", "Start preparing backup archive".to_string());

        let filename = downloaded_file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        logger.log("debug", format!("Archive filename: {}", filename));

        let is_legacy = filename.ends_with(".sql") || filename.ends_with(".dump");

        if is_legacy {
            logger.log("info", "Legacy archive detected, skipping extraction".to_string());
            return Ok(downloaded_file);
        }

        let encrypted = filename.ends_with(".tar.gz.enc");

        let mut archive = downloaded_file.clone();

        if encrypted {
            logger.log("info", "Archive is encrypted, decrypting".to_string());

            let new_name = filename.strip_suffix(".enc").unwrap();

            let decrypted = tmp_path.join(new_name);

            if let Err(e) = decrypt_file_stream_gcm(
                downloaded_file,
                decrypted.clone(),
                self.ctx.edge_key.master_key_b64.clone(),
            )
            .await
            {
                logger.log("error", format!("Failed to decrypt archive: {}", e));
                return Err(e);
            }

            logger.log("info", format!("Archive decrypted to {}", decrypted.display()));

            archive = decrypted;
        }

        logger.log("info", format!("Decompressing archive {}", archive.display()));

        let files = match decompress_large_tar_gz(archive.as_path(), tmp_path).await {
            Ok(f) => f,
            Err(e) => {
                logger.log("error", format!("Failed to decompress archive: {}", e));
                return Err(e);
            }
        };

        if files.is_empty() {
            logger.log("error", "Archive is empty after decompression".to_string());
            anyhow::bail!("archive empty");
        }

        logger.log("info", format!("Archive prepared, {} file(s) extracted", files.len()));

        if files.len() == 1 {
            logger.log("debug", format!("Using single extracted file: {}", files[0].display()));
            Ok(files[0].clone())
        } else {
            logger.log("debug", format!("Multiple files extracted, using archive root: {}", archive.display()));
            Ok(archive)
        }
    }
}
