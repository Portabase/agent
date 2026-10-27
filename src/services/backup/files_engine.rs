use super::logger::JobLogger;
use super::models::UploadResult;
use crate::core::context::Context as CoreContext;
use crate::domain::files::restic::backup::snapshot;
use crate::domain::files::restic::repo::ResticRepo;
use crate::domain::files::sync::backup::{replica_path, replicate};
use crate::domain::files::sync::rclone::Mode;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::DatabaseConfig;
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::Result;
use serde_json::Value;

#[derive(Clone, Copy)]
pub enum FilesEngine {
    Restic,
    Sync,
    Copy,
}

impl FilesEngine {
    pub fn parse(engine: &str) -> Option<Self> {
        match engine {
            "restic" => Some(Self::Restic),
            "sync" => Some(Self::Sync),
            "copy" => Some(Self::Copy),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Restic => "restic",
            Self::Sync => "sync",
            Self::Copy => "copy",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Restic => "Snapshot",
            Self::Sync => "Sync",
            Self::Copy => "Copy",
        }
    }
}

struct Transfer {
    remote_path: String,
    size: u64,
    report: Value,
    total_size: Option<u64>,
}

pub async fn run(
    ctx: &CoreContext,
    cfg: &DatabaseConfig,
    engine: FilesEngine,
    storages: &[DatabaseStorage],
    backup_id: &str,
    logger: &JobLogger,
) -> Vec<UploadResult> {
    if let Err(e) = FileLock::acquire(&cfg.generated_id, DbOpLock::Backup.as_str()).await {
        logger.log("error", format!("{} aborted: {e}", engine.label()));
        return Vec::new();
    }
    let mut results = Vec::with_capacity(storages.len());
    for storage in storages {
        results.push(one_storage(ctx, cfg, engine, storage, backup_id, logger).await);
    }
    if let Err(e) = FileLock::release(&cfg.generated_id).await {
        logger.log("warn", format!("Failed to release the backup lock: {e}"));
    }
    results
}

async fn transfer(
    ctx: &CoreContext,
    cfg: &DatabaseConfig,
    engine: FilesEngine,
    storage: &DatabaseStorage,
    backup_storage_id: &str,
    logger: &JobLogger,
) -> Result<Transfer> {
    match engine {
        FilesEngine::Restic => {
            let repo = ResticRepo::open(storage, &cfg.generated_id, &ctx.edge_key)?;
            let summary = snapshot(&repo, cfg, backup_storage_id, logger).await?;
            Ok(Transfer {
                size: summary.data_added_packed,
                report: summary.report(),
                total_size: Some(summary.total_bytes_processed),
                remote_path: summary.snapshot_id,
            })
        }
        FilesEngine::Sync | FilesEngine::Copy => {
            let mode = if matches!(engine, FilesEngine::Copy) { Mode::Copy } else { Mode::Sync };
            let stats = replicate(&ctx.edge_key, cfg, storage, mode, logger).await?;
            Ok(Transfer {
                remote_path: replica_path(storage, &cfg.generated_id),
                size: stats.bytes,
                report: stats.report(),
                total_size: stats.replica_bytes,
            })
        }
    }
}

pub(crate) async fn one_storage(
    ctx: &CoreContext,
    cfg: &DatabaseConfig,
    engine: FilesEngine,
    storage: &DatabaseStorage,
    backup_id: &str,
    logger: &JobLogger,
) -> UploadResult {
    let agent_id = ctx.edge_key.agent_id.clone();
    let failed = |error: String| UploadResult {
        storage_id: storage.id.clone(),
        success: false,
        error: Some(error),
        remote_file_path: None,
        total_size: None,
    };

    let backup_storage_id = match ctx
        .api
        .backup_upload_init(agent_id.clone(), cfg.generated_id.clone(), storage.id.clone(), backup_id, Some(engine.as_str()))
        .await
    {
        Ok(Some(response)) => response.backup_storage.id,
        Ok(None) => {
            logger.log("error", "Upload init returned empty response");
            return failed("backup_upload_init returned empty response".into());
        }
        Err(e) => {
            logger.log("error", format!("Upload init failed: {e}"));
            return failed("backup_upload_init failed".into());
        }
    };

    match transfer(ctx, cfg, engine, storage, &backup_storage_id, logger).await {
        Ok(t) => match ctx
            .api
            .backup_upload_success(agent_id, cfg.generated_id.clone(), backup_storage_id, t.remote_path.clone(), t.size, t.report, backup_id)
            .await
        {
            Ok(_) => UploadResult {
                storage_id: storage.id.clone(),
                success: true,
                error: None,
                remote_file_path: Some(t.remote_path),
                total_size: t.total_size,
            },
            Err(e) => {
                logger.log("error", format!("Upload status update failed for {}: {e}", storage.id));
                failed(e.to_string())
            }
        },
        Err(e) => {
            logger.log("error", format!("{} to storage {} failed: {e:#}", engine.label(), storage.id));
            if let Err(err) = ctx
                .api
                .backup_upload_status(agent_id, cfg.generated_id.clone(), backup_storage_id, "failed", String::new(), 0u64, backup_id)
                .await
            {
                logger.log("error", format!("Failed-status update failed for {}: {err}", storage.id));
            }
            failed(format!("{e:#}"))
        }
    }
}
