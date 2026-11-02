use super::files_engine::{self, FilesEngine};
use super::hooks::{HookEnv, JobHooks};
use super::logger::JobLogger;
use super::models::{BackupResult, UploadResult};
use super::result::{JobStatus, job_status};
use super::service::BackupService;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::{DatabaseConfig, DbType};
use crate::utils::common::BackupMethod;
use crate::utils::locks::FileLock;

use anyhow::{Result, anyhow};
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

fn failed_result(db_cfg: &DatabaseConfig) -> BackupResult {
    BackupResult {
        generated_id: db_cfg.generated_id.clone(),
        db_type: db_cfg.db_type.clone(),
        status: "failed".into(),
        backup_file: None,
        code: None,
    }
}

impl BackupService {
    pub async fn execute_backup(
        &self,
        generated_id: String,
        db_cfg: DatabaseConfig,
        method: BackupMethod,
        storages: Vec<DatabaseStorage>,
        encrypt: bool,
        engine: String,
    ) -> Result<()> {
        let logger = Arc::new(JobLogger::new());

        if FileLock::is_locked(&generated_id).await? {
            anyhow::bail!("backup already running");
        }
        let start = Instant::now();
        logger.log("info", "Database backup job started".to_string());

        let backup = self.create_backup_record(&generated_id, &method).await?;
        let backup_id = backup.backup.id;

        let job_hooks = JobHooks::new(
            db_cfg.hooks.clone(),
            HookEnv {
                db_name: db_cfg.name.clone(),
                generated_id: generated_id.clone(),
                backup_id: backup_id.clone(),
            },
        );

        let outcome = match job_hooks.pre(&logger).await {
            Ok(()) => {
                self.backup_and_upload(db_cfg, method, storages, encrypt, &engine, &backup_id, &job_hooks, &logger)
                    .await
            }
            Err(e) => {
                logger.log("error", format!("Pre-backup hook failed, backup skipped: {e:#}"));
                job_hooks.post(JobStatus::Failed, &logger).await;
                Ok((failed_result(&db_cfg), vec![]))
            }
        };

        let (result, uploads) = match outcome {
            Ok(done) => done,
            Err(e) => {
                job_hooks.finish(JobStatus::Failed, &logger).await;
                return Err(e);
            }
        };

        job_hooks.finish(job_status(&uploads), &logger).await;
        logger.log("info", "Database backup job finished".to_string());

        let duration_ms = start.elapsed().as_millis() as f64;
        let logs = Arc::try_unwrap(logger).unwrap_or_else(|_| JobLogger::new()).into_entries();
        self.send_result(result, uploads, &backup_id, logs, duration_ms).await?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn backup_and_upload(
        &self,
        db_cfg: DatabaseConfig,
        method: BackupMethod,
        storages: Vec<DatabaseStorage>,
        encrypt: bool,
        engine: &str,
        backup_id: &String,
        job_hooks: &JobHooks,
        logger: &Arc<JobLogger>,
    ) -> Result<(BackupResult, Vec<UploadResult>)> {
        if let Some(files_engine) = FilesEngine::parse(engine) {
            if matches!(db_cfg.db_type, DbType::Files) {
                let uploads = files_engine::run(&self.ctx, &db_cfg, files_engine, &storages, backup_id, logger).await;
                job_hooks.post(job_status(&uploads), logger).await;
                let result = BackupResult { status: "success".into(), ..failed_result(&db_cfg) };
                return Ok((result, uploads));
            }
            logger.log("warn", format!("The {engine} method only applies to files sources; making an archive"));
        }

        let temp_dir = TempDir::new();
        let dump = match &temp_dir {
            Ok(dir) => Self::run(db_cfg, dir.path(), Arc::clone(logger)).await,
            Err(e) => Err(anyhow!("Failed to create temp dir: {e}")),
        };
        let dump_status = match &dump {
            Ok(r) if r.status == "success" => JobStatus::Success,
            _ => JobStatus::Failed,
        };
        job_hooks.post(dump_status, logger).await;

        let mut result = dump?;
        let _temp_dir = temp_dir?; // keep the dump on disk until the upload is done
        if result.status == "failed" {
            return Ok((result, vec![]));
        }

        let compressed = self.compress_backup(result.backup_file.take(), Arc::clone(logger)).await?;
        result.backup_file = Some(compressed);

        let uploads = self
            .upload(result.clone(), method, storages, encrypt, backup_id, Arc::clone(logger))
            .await?;

        Ok((result, uploads))
    }
}
