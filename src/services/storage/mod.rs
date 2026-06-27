pub mod providers;

use crate::core::context::Context;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::backup::models::{BackupResult, UploadResult};
use crate::utils::common::BackupMethod;
use async_trait::async_trait;
use providers::azure_blob;
use providers::google_cloud_storage;
use providers::google_drive;
use providers::local;
use providers::s3;
use std::sync::Arc;
use tracing::{error, info};

#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        method: BackupMethod,
        config: &DatabaseStorage,
        encrypt: Option<bool>,
    ) -> UploadResult;
}

/// Factory to create provider instance from storage config
pub fn get_provider(storage: &DatabaseStorage) -> Option<Box<dyn StorageProvider>> {
    info!("Getting provider");
    info!("{:#?}", storage.provider.as_str());

    match storage.provider.as_str() {
        "local" => Some(Box::new(local::LocalProvider {})),
        "s3" => Some(Box::new(s3::S3Provider {})),
        "blob" => Some(Box::new(azure_blob::AzureBlobProvider {})),
        "google-drive" => Some(Box::new(google_drive::GoogleDriveProvider {})),
        "google-cloud-storage" => Some(Box::new(
            google_cloud_storage::GoogleCloudStorageProvider {},
        )),
        _ => {
            error!("Unknown storage provider: {}", storage.provider);
            None
        }
    }
}
