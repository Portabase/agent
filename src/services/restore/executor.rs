use super::models::RestoreResult;
use super::service::RestoreService;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::domain::files::restic::repo::ResticRepo;
use crate::domain::files::restic::restore::restore;
use crate::services::config::{DatabaseConfig, DbType};
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

impl RestoreService {
    pub async fn execute_restore(
        &self,
        cfg: DatabaseConfig,
        file_url: String,
        expected_size: Option<String>,
    ) -> Result<()> {
        let logger = Arc::new(JobLogger::new());
        let start = Instant::now();
        let generated_id = cfg.generated_id.clone();

        logger.log("info", "Database restoration job started".to_string());

        let outcome = async {
            let temp_dir = TempDir::new()?;
            let tmp_path = temp_dir.path();

            logger.log("info", format!("Created temp directory {}", tmp_path.display()));

            let downloaded = self
                .download_backup(&file_url, tmp_path, Arc::clone(&logger), expected_size)
                .await?;

            let backup_file = self
                .prepare_archive(downloaded, tmp_path, &cfg.db_type, Arc::clone(&logger))
                .await?;

            self.run_restore(cfg, backup_file, Arc::clone(&logger)).await
        }
        .await;

        let result = outcome.unwrap_or_else(|error| {
            logger.log("error", format!("Database restore failed: {error:#}"));
            RestoreResult { generated_id, status: "failed".into() }
        });

        logger.log("info", "Database restore job finished".to_string());

        let duration_ms = start.elapsed().as_millis() as f64;
        let logs = Arc::try_unwrap(logger)
            .unwrap_or_else(|_| JobLogger::new())
            .into_entries();
        self.send_result(result, logs, duration_ms).await?;

        Ok(())
    }

    pub async fn execute_restic_restore(
        &self,
        cfg: DatabaseConfig,
        snapshot_id: Option<String>,
        storage: Option<DatabaseStorage>,
    ) -> Result<()> {
        let logger = Arc::new(JobLogger::new());
        let start = Instant::now();
        logger.log("info", "Snapshot restore job started".to_string());

        let outcome = match (snapshot_id, storage) {
            (Some(id), Some(storage)) => self.restic_restore(&cfg, &id, &storage, &logger).await,
            _ => Err(anyhow::anyhow!(
                "incomplete snapshot restore payload (snapshotId or storage missing)"
            )),
        };
        let status = match outcome {
            Ok(()) => {
                logger.log("info", "Snapshot restore job finished".to_string());
                "success"
            }
            Err(e) => {
                logger.log("error", format!("Snapshot restore failed: {e:#}"));
                "failed"
            }
        };

        let duration_ms = start.elapsed().as_millis() as f64;
        let logs = Arc::try_unwrap(logger).unwrap_or_else(|_| JobLogger::new()).into_entries();
        let result = RestoreResult { generated_id: cfg.generated_id.clone(), status: status.into() };
        self.send_result(result, logs, duration_ms).await?;
        Ok(())
    }

    async fn restic_restore(
        &self,
        cfg: &DatabaseConfig,
        snapshot_id: &str,
        storage: &DatabaseStorage,
        logger: &JobLogger,
    ) -> Result<()> {
        if !matches!(cfg.db_type, DbType::Files) {
            bail!("snapshot restore needs a files source, got {}", cfg.db_type.as_str());
        }
        FileLock::acquire(&cfg.generated_id, DbOpLock::Restore.as_str())
            .await
            .context("another backup or restore of this source is running")?;
        let result = async {
            let repo = ResticRepo::open(storage, &cfg.generated_id, &self.ctx.edge_key)?;
            restore(&repo, cfg, snapshot_id, logger).await
        }
        .await;
        FileLock::release(&cfg.generated_id).await?;
        result
    }
}
