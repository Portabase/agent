use super::logger::JobLogger;
use super::models::BackupResult;
use super::service::BackupService;
use crate::domain::postgres::cluster::physical::decision::{Decision, decide};
use crate::domain::postgres::cluster::physical::{backup, checks, manifest};
use crate::services::api::endpoints::agent::backup::PhysicalOutcome;
use crate::services::api::models::agent::backup::BackupResponse;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::{DatabaseConfig, full_every_days};
use crate::utils::common::BackupMethod;
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

impl BackupService {
    pub async fn execute_physical_backup(
        &self,
        generated_id: String,
        db_cfg: DatabaseConfig,
        storages: Vec<DatabaseStorage>,
        encrypt: bool,
        backup: BackupResponse,
        method: BackupMethod,
        logger: Arc<JobLogger>,
        start: Instant,
    ) -> Result<()> {
        let backup_id = backup.backup.id.clone();
        let mut result = BackupResult {
            generated_id: generated_id.clone(),
            db_type: db_cfg.db_type.clone(),
            status: "failed".into(),
            backup_file: None,
            code: None,
        };

        let outcome = self
            .take_physical_backup(&db_cfg, &generated_id, &backup, Arc::clone(&logger))
            .await;

        let (uploads, physical) = match outcome {
            Ok((physical_backup, physical, work_dir)) => {
                result.status = "success".into();
                result.backup_file = Some(physical_backup.bundle);
                let uploads = self
                    .upload(
                        result.clone(),
                        method,
                        storages,
                        encrypt,
                        &backup_id,
                        Arc::clone(&logger),
                        Some("physical"),
                    )
                    .await;
                drop(work_dir);
                (uploads?, Some(physical))
            }
            Err(error) => {
                logger.log("error", format!("Physical backup failed: {error:#}"));
                (vec![], None)
            }
        };

        logger.log("info", "Database backup job finished".to_string());
        let duration_ms = start.elapsed().as_millis() as f64;
        let logs = Arc::try_unwrap(logger).unwrap_or_else(|_| JobLogger::new()).into_entries();
        self.send_result(result, uploads, &backup_id, logs, duration_ms, physical.as_ref())
            .await?;
        Ok(())
    }

    async fn take_physical_backup(
        &self,
        db_cfg: &DatabaseConfig,
        generated_id: &str,
        backup: &BackupResponse,
        logger: Arc<JobLogger>,
    ) -> Result<(backup::PhysicalBackup, PhysicalOutcome, TempDir)> {
        let facts = checks::preflight(db_cfg).await?;
        let decision = decide(
            backup.parent.as_ref(),
            Utc::now(),
            full_every_days(db_cfg),
            facts.system_identifier,
        );
        match &decision {
            Decision::Full { reason } => logger.log("info", format!("Taking a full backup: {reason}")),
            Decision::Incremental { parent_backup_id, .. } => {
                logger.log("info", format!("Taking an incremental backup on {parent_backup_id}"))
            }
        }

        let work_dir = TempDir::new()?;
        FileLock::acquire(generated_id, DbOpLock::Backup.as_str()).await?;
        let physical_backup = backup::run(db_cfg, &facts, decision, work_dir.path(), logger).await;
        if let Err(error) = FileLock::release(generated_id).await {
            tracing::warn!("Failed to release the backup lock: {error}");
        }
        let physical_backup = physical_backup?;
        let physical = PhysicalOutcome {
            parent_backup_id: physical_backup.parent_backup_id.clone(),
            manifest: manifest::encode(&physical_backup.manifest)?,
        };
        Ok((physical_backup, physical, work_dir))
    }
}
