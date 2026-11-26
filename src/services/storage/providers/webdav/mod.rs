pub mod helpers;
pub mod models;

use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::models::{BackupResult, UploadResult};
use crate::services::storage::StorageProvider;
use crate::services::storage::providers::rclone::helpers::{rcat, remote_target, write_config};
use crate::services::storage::providers::rclone::target::rclone_target;
use crate::utils::common::BackupMethod;
use crate::utils::file::{full_file_name, full_file_path};
use crate::utils::stream::build_stream;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::fs;
use tracing::{error, info};

pub struct WebDavProvider {}

fn failed(storage_id: &str, error: impl ToString, total_size: Option<u64>) -> UploadResult {
    UploadResult {
        storage_id: storage_id.to_string(),
        success: false,
        error: Some(error.to_string()),
        remote_file_path: None,
        total_size,
    }
}

#[async_trait]
impl StorageProvider for WebDavProvider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        _method: BackupMethod,
        storage: &DatabaseStorage,
        encrypt: Option<bool>,
        _backup_storage_id: &str,
    ) -> UploadResult {
        let storage_id = storage.id.clone();
        let Some(file_path) = result.backup_file else {
            return failed(&storage_id, "Missing backup file path", None);
        };
        let total_size = match fs::metadata(&file_path).await {
            Ok(meta) => meta.len(),
            Err(e) => return failed(&storage_id, e, None),
        };
        let target = match rclone_target(storage) {
            Ok(target) => target,
            Err(e) => {
                error!("WebDAV config build failed: {e:#}");
                return failed(&storage_id, format!("{e:#}"), Some(total_size));
            }
        };
        let encrypted = encrypt.unwrap_or(false);
        let upload = match build_stream(&file_path, encrypted, &ctx.edge_key.master_key_b64).await {
            Ok(upload) => upload,
            Err(e) => return failed(&storage_id, e, Some(total_size)),
        };
        let file_name = full_file_name(encrypted);
        let remote_file_path = full_file_path(&file_name, storage.folder_name.as_deref());
        let config_file = match write_config(&target.config_text) {
            Ok(file) => file,
            Err(e) => return failed(&storage_id, e, Some(total_size)),
        };
        let remote = remote_target(&target.remote_name, &target.base_path, &remote_file_path);
        info!("Starting WebDAV (rclone) upload to {}", remote);
        match rcat(config_file.path(), &remote, upload.stream).await {
            Ok(()) => UploadResult {
                storage_id,
                success: true,
                error: None,
                remote_file_path: Some(remote_file_path),
                total_size: Some(total_size),
            },
            Err(e) => failed(&storage_id, e, Some(total_size)),
        }
    }
}
