//! Sync as a backup method: one `rclone sync` per storage channel, each reported
//! as its own `backup_storage` row (`engine = sync`), like restic snapshots.
use super::rclone::{replica_target, sync_dir};
use super::stats::SyncStats;
use crate::core::context::Context as CoreContext;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::logger::JobLogger;
use crate::services::backup::models::UploadResult;
use crate::services::config::DatabaseConfig;
use crate::services::restic::password::{LOCAL_STORAGE_USER, local_storage_password};
use crate::services::storage::providers::rclone::helpers::{build_rclone_config, obscure_password};
use crate::services::storage::providers::rclone::target::rclone_target;
use crate::utils::edge_key::EdgeKey;
use crate::utils::locks::{DbOpLock, FileLock};
use anyhow::Result;

/// Syncs every storage in turn, under the source's backup lock. Never errors:
/// each storage reports its own `backup_storage` row.
pub async fn run(
    ctx: &CoreContext,
    cfg: &DatabaseConfig,
    storages: &[DatabaseStorage],
    backup_id: &str,
    logger: &JobLogger,
) -> Vec<UploadResult> {
    if let Err(e) = FileLock::acquire(&cfg.generated_id, DbOpLock::Backup.as_str()).await {
        logger.log("error", format!("Sync aborted: {e}"));
        return Vec::new();
    }
    let mut results = Vec::with_capacity(storages.len());
    for storage in storages {
        results.push(one_storage(ctx, cfg, storage, backup_id, logger).await);
    }
    if let Err(e) = FileLock::release(&cfg.generated_id).await {
        logger.log("warn", format!("Failed to release the backup lock: {e}"));
    }
    results
}

fn folder(storage: &DatabaseStorage) -> &str {
    storage
        .folder_name
        .as_deref()
        .map(|f| f.trim().trim_matches('/'))
        .filter(|f| !f.is_empty())
        .unwrap_or("backups")
}

/// `<folder>/sync/<generated_id>/current`, relative to the channel's base path.
pub fn replica_path(storage: &DatabaseStorage, generated_id: &str) -> String {
    format!("{}/sync/{generated_id}/current", folder(storage))
}

const LOCAL_REMOTE: &str = "pblocal";

/// rclone config of the dashboard's local storage (`rclone serve webdav` at `/storage/sync`).
pub fn local_sync_config(edge_key: &EdgeKey) -> Result<String> {
    build_rclone_config(
        LOCAL_REMOTE,
        &[
            ("type", "webdav".into()),
            ("url", format!("{}/storage/sync", edge_key.server_url.trim_end_matches('/'))),
            // rclone's own WebDAV extensions keep modification times: unchanged files are skipped.
            ("vendor", "rclone".into()),
            ("user", LOCAL_STORAGE_USER.into()),
            ("pass", obscure_password(&local_storage_password(&edge_key.master_key_b64)?)?),
        ],
    )
}

async fn replicate(ctx: &CoreContext, cfg: &DatabaseConfig, storage: &DatabaseStorage, logger: &JobLogger) -> Result<SyncStats> {
    if storage.provider == "local" {
        let dest = format!("{LOCAL_REMOTE}:{}/current", cfg.generated_id);
        return sync_dir(&local_sync_config(&ctx.edge_key)?, &dest, cfg, logger).await;
    }
    let target = rclone_target(storage)?;
    let dest = replica_target(&target.remote_name, &target.base_path, folder(storage), &cfg.generated_id);
    sync_dir(&target.config_text, &dest, cfg, logger).await
}

pub(crate) async fn one_storage(
    ctx: &CoreContext,
    cfg: &DatabaseConfig,
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
        .backup_upload_init(agent_id.clone(), cfg.generated_id.clone(), storage.id.clone(), backup_id, Some("sync"))
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

    match replicate(ctx, cfg, storage, logger).await {
        Ok(stats) => {
            let path = replica_path(storage, &cfg.generated_id);
            match ctx
                .api
                .backup_upload_sync_status(agent_id, cfg.generated_id.clone(), backup_storage_id, path.clone(), &stats, backup_id)
                .await
            {
                Ok(_) => UploadResult {
                    storage_id: storage.id.clone(),
                    success: true,
                    error: None,
                    remote_file_path: Some(path),
                    // No total size: rclone does not report the replica's size (backups.file_size stays null).
                    total_size: None,
                },
                Err(e) => {
                    logger.log("error", format!("Upload status update failed for {}: {e}", storage.id));
                    failed(e.to_string())
                }
            }
        }
        Err(e) => {
            logger.log("error", format!("Sync to storage {} failed: {e:#}", storage.id));
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
