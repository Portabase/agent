mod helpers;
mod models; 

use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::{BackupResult, UploadResult};
use crate::services::storage::StorageProvider;
use crate::utils::common::BackupMethod;
use crate::utils::file::{full_file_name, full_file_path};
use crate::utils::stream::build_stream;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::fs;
use tracing::{error, info};
use crate::services::storage::providers::google_drive::helpers::{upload_stream_to_google_drive};
use crate::services::storage::providers::google_drive::models::GoogleDriveProviderConfig;

pub struct GoogleDriveProvider {}

#[async_trait]
impl StorageProvider for GoogleDriveProvider {
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

        let upload = match build_stream(
            &file_path,
            encrypt,
            encrypt.then(|| ctx.edge_key.public_key.as_bytes().to_vec()),
        )
            .await
        {
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

        let config: GoogleDriveProviderConfig = match storage.clone().config.try_into() {
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


        let file_name = full_file_name(encrypt);

        info!("Uploading file {}", file_name);

        let remote_file_path = full_file_path(&file_name);

        match upload_stream_to_google_drive(
            &config,
            &remote_file_path,
            upload.stream,   
            total_size,
            Some("application/octet-stream"),
        ).await {
            Ok(_file_id) => {
                
                info!("Google Drive upload successful");

                UploadResult {
                    storage_id: storage.id.clone(),
                    success: true,
                    error: None,
                    remote_file_path: Some(remote_file_path),
                    total_size: Some(total_size),
                }
            }
            Err(e) => {
                error!("Google Drive upload failed: {:?}", e);

                UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: Some(total_size),
                }
            }
        }

    }
}
