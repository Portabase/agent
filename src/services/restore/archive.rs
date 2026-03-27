use super::service::RestoreService;

use crate::utils::compress::decompress_large_tar_gz;
use crate::utils::file::decrypt_file_stream_gcm;

use anyhow::Result;
use std::path::{Path, PathBuf};

impl RestoreService {
    pub async fn prepare_archive(
        &self,
        downloaded_file: PathBuf,
        tmp_path: &Path,
    ) -> Result<PathBuf> {
        let filename = downloaded_file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let is_legacy = filename.ends_with(".sql") || filename.ends_with(".dump");

        if is_legacy {
            return Ok(downloaded_file);
        }

        let encrypted = filename.ends_with(".tar.gz.enc");

        let mut archive = downloaded_file.clone();

        if encrypted {
            let new_name = filename.strip_suffix(".enc").unwrap();

            let decrypted = tmp_path.join(new_name);

            decrypt_file_stream_gcm(
                downloaded_file,
                decrypted.clone(),
                self.ctx.edge_key.master_key_b64.clone(),
            )
            .await?;

            archive = decrypted;
        }

        let files = decompress_large_tar_gz(archive.as_path(), tmp_path).await?;

        if files.is_empty() {
            anyhow::bail!("archive empty");
        }

        if files.len() == 1 {
            Ok(files[0].clone())
        } else {
            Ok(archive)
        }
    }
}
