use super::models::RestoreResult;
use super::service::RestoreService;
use crate::domain::postgres::cluster::physical::restore::{
    RestoredCluster, ensure_restore_root, rebuild,
};
use crate::services::api::models::agent::status::RestoreFile;
use crate::services::backup::logger::JobLogger;
use crate::services::config::DatabaseConfig;
use crate::settings::CONFIG;
use crate::utils::file::decrypt_file_stream_gcm;
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

impl RestoreService {
    pub async fn execute_physical_restore(
        &self,
        cfg: DatabaseConfig,
        files: Vec<RestoreFile>,
    ) -> Result<()> {
        let logger = Arc::new(JobLogger::new());
        let start = Instant::now();
        logger.log("info", "Physical restore job started");

        let outcome = async {
            let restore_root = ensure_restore_root(CONFIG.restore_path.as_deref())?;
            FileLock::acquire(&cfg.generated_id, DbOpLock::Restore.as_str())
                .await
                .context("another backup or restore of this source is running")?;
            let rebuilt = self
                .rebuild_chain(files, &restore_root, &cfg.generated_id, Arc::clone(&logger))
                .await;
            if let Err(error) = FileLock::release(&cfg.generated_id).await {
                tracing::warn!("Failed to release the restore lock: {error}");
            }
            rebuilt
        }
        .await;

        let status = match outcome {
            Ok(restored) => {
                tracing::info!(
                    "Physical restore of {} rebuilt PostgreSQL {} into {} ({} bytes)",
                    cfg.generated_id,
                    restored.major,
                    restored.data_dir.display(),
                    restored.size
                );
                "success"
            }
            Err(error) => {
                logger.log("error", format!("Physical restore failed: {error:#}"));
                "failed"
            }
        };
        logger.log("info", "Physical restore job finished");

        let duration_ms = start.elapsed().as_millis() as f64;
        let logs = Arc::try_unwrap(logger)
            .unwrap_or_else(|_| JobLogger::new())
            .into_entries();
        let result = RestoreResult {
            generated_id: cfg.generated_id,
            status: status.into(),
        };
        self.send_result(result, logs, duration_ms).await?;
        Ok(())
    }

    pub async fn rebuild_chain(
        &self,
        files: Vec<RestoreFile>,
        restore_root: &Path,
        generated_id: &str,
        logger: Arc<JobLogger>,
    ) -> Result<RestoredCluster> {
        let download_dir = TempDir::new_in(restore_root)?;
        let mut bundles = Vec::new();
        for (index, file) in files.into_iter().enumerate() {
            let target = download_dir.path().join(index.to_string());
            tokio::fs::create_dir(&target).await?;
            let downloaded = self
                .download_backup(&file.url, &target, Arc::clone(&logger), file.size)
                .await?;
            if downloaded
                .extension()
                .is_some_and(|extension| extension == "enc")
            {
                let decrypted = downloaded.with_extension("");
                logger.log("info", format!("Decrypting {}", downloaded.display()));
                decrypt_file_stream_gcm(
                    downloaded.clone(),
                    decrypted.clone(),
                    self.ctx.edge_key.master_key_b64.clone(),
                )
                .await
                .with_context(|| format!("could not decrypt {}", downloaded.display()))?;
                tokio::fs::remove_file(&downloaded).await?;
                bundles.push(decrypted);
            } else {
                bundles.push(downloaded);
            }
        }
        rebuild(&bundles, restore_root, generated_id, logger).await
    }
}
