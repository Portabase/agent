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

pub struct SmbProvider {}

fn failed(storage_id: &str, error: impl ToString, total_size: Option<u64>) -> UploadResult {
    UploadResult {
        storage_id: storage_id.into(),
        success: false,
        error: Some(error.to_string()),
        remote_file_path: None,
        total_size,
    }
}

#[async_trait]
impl StorageProvider for SmbProvider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        _method: BackupMethod,
        storage: &DatabaseStorage,
        encrypt: Option<bool>,
        _backup_storage_id: &str,
    ) -> UploadResult {
        let id = storage.id.clone();
        let Some(path) = result.backup_file else {
            return failed(&id, "Missing backup file path", None);
        };
        let size = match fs::metadata(&path).await {
            Ok(meta) => meta.len(),
            Err(error) => return failed(&id, error, None),
        };
        let target = match rclone_target(storage) {
            Ok(target) => target,
            Err(error) => return failed(&id, format!("{error:#}"), Some(size)),
        };
        let upload = match build_stream(
            &path,
            encrypt.unwrap_or(false),
            &ctx.edge_key.master_key_b64,
        )
        .await
        {
            Ok(upload) => upload,
            Err(error) => return failed(&id, error, Some(size)),
        };
        let remote_file_path = full_file_path(
            &full_file_name(encrypt.unwrap_or(false)),
            storage.folder_name.as_deref(),
        );
        let config = match write_config(&target.config_text) {
            Ok(config) => config,
            Err(error) => return failed(&id, error, Some(size)),
        };
        let remote = remote_target(&target.remote_name, &target.base_path, &remote_file_path);
        match rcat(config.path(), &remote, upload.stream).await {
            Ok(()) => UploadResult {
                storage_id: id,
                success: true,
                error: None,
                remote_file_path: Some(remote_file_path),
                total_size: Some(size),
            },
            Err(error) => failed(&id, error, Some(size)),
        }
    }
}
