pub mod helpers;
mod models;

use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::models::{BackupResult, UploadResult};
use crate::services::storage::StorageProvider;
use crate::services::storage::providers::azure_blob::helpers::{BLOCK_SIZE, upload_stream_to_azure};
use crate::services::storage::providers::azure_blob::models::AzureBlobProviderConfig;
use crate::utils::common::BackupMethod;
use crate::utils::file::{full_file_name, full_file_path};
use crate::utils::stream::build_stream;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::fs;
use tracing::{error, info};

pub struct AzureBlobProvider {}

#[async_trait]
impl StorageProvider for AzureBlobProvider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        _method: BackupMethod,
        storage: &DatabaseStorage,
        encrypt: Option<bool>,
    ) -> UploadResult {
        let Some(file_path) = result.backup_file else {
            return UploadResult {
                storage_id: storage.id.clone(),
                success: false,
                error: Some("Missing backup file path".to_string()),
                remote_file_path: None,
                total_size: None,
            };
        };

        let total_size = match fs::metadata(&file_path).await {
            Ok(meta) => meta.len(),
            Err(e) => {
                error!("Failed to get file size: {}", e);
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let encrypt = encrypt.unwrap_or(false);

        let upload = match build_stream(&file_path, encrypt, &ctx.edge_key.master_key_b64).await {
            Ok(u) => u,
            Err(e) => {
                error!("Stream build failed: {}", e);
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let config: AzureBlobProviderConfig = match storage.clone().config.try_into() {
            Ok(c) => c,
            Err(e) => {
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let resolved = match config.resolve() {
            Ok(r) => r,
            Err(e) => {
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let file_name = full_file_name(encrypt);
        let remote_file_path = full_file_path(&file_name);
        info!(
            "Starting block upload to azure blob {}/{}",
            config.container_name, remote_file_path
        );

        match upload_stream_to_azure(
            &resolved,
            &config.container_name,
            &remote_file_path,
            upload.stream,
            BLOCK_SIZE,
        )
        .await
        {
            Ok(_) => {
                info!("Azure blob upload successful: {}", remote_file_path);
                UploadResult {
                    storage_id: storage.id.clone(),
                    success: true,
                    error: None,
                    remote_file_path: Some(remote_file_path),
                    total_size: Some(total_size),
                }
            }
            Err(e) => {
                error!("Azure blob upload failed: {:?}", e);
                UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                }
            }
        }
    }
}
