pub mod helpers;
mod models;

use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::models::{BackupResult, UploadResult};
use crate::services::storage::StorageProvider;
use crate::services::storage::providers::google_cloud_storage::helpers::{
    StreamSource, build_client, upload_with_client,
};
use crate::services::storage::providers::google_cloud_storage::models::GoogleCloudStorageProviderConfig;
use crate::utils::common::BackupMethod;
use crate::utils::file::{full_file_name, full_file_path};
use crate::utils::stream::build_stream;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::fs;
use tracing::{error, info};

pub struct GoogleCloudStorageProvider {}

#[async_trait]
impl StorageProvider for GoogleCloudStorageProvider {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        _method: BackupMethod,
        storage: &DatabaseStorage,
        encrypt: Option<bool>,
        _backup_storage_id: &str,
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

        let config: GoogleCloudStorageProviderConfig = match storage.clone().config.try_into() {
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
        let remote_file_path = full_file_path(&file_name, storage.folder_name.as_deref());

        let client = match build_client(&config).await {
            Ok(c) => c,
            Err(e) => {
                error!("GCS client build failed: {:?}", e);
                return UploadResult {
                    storage_id: storage.id.clone(),
                    success: false,
                    error: Some(e.to_string()),
                    remote_file_path: None,
                    total_size: None,
                };
            }
        };

        let source = StreamSource::from_stream(upload.stream, total_size);

        // A custom apiEndpoint (self-hosted / emulator) on a non-443 port trips an
        // upstream SDK bug in the resumable-upload path; force single-shot for it.
        let force_single_shot = config
            .api_endpoint
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());

        info!(
            "Starting GCS upload to {}/{} (single_shot={})",
            config.bucket_name, remote_file_path, force_single_shot
        );

        match upload_with_client(
            &client,
            &config.bucket_name,
            &remote_file_path,
            source,
            force_single_shot,
        )
        .await
        {
            Ok(_) => {
                info!("GCS upload successful: {}", remote_file_path);
                UploadResult {
                    storage_id: storage.id.clone(),
                    success: true,
                    error: None,
                    remote_file_path: Some(remote_file_path),
                    total_size: Some(total_size),
                }
            }
            Err(e) => {
                error!("GCS upload failed: {:?}", e);
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
