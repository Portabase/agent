pub mod tus;
// pub mod s3;

use std::sync::Arc;
use crate::services::backup::{BackupResult, UploadResult};
use crate::utils::common::BackupMethod;
use crate::services::status::DatabaseStorage;
use async_trait::async_trait;
use crate::core::context::Context;

#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn upload(
        &self,
        ctx: Arc<Context>,
        result: BackupResult,
        method: BackupMethod,
        config: &DatabaseStorage,
    ) -> UploadResult;
}

/// Factory to create provider instance from storage config
pub fn get_provider(storage: &DatabaseStorage) -> Box<dyn StorageProvider> {
    match storage.provider.as_str() {
        "local" => Box::new(tus::TusProvider {}),
        // "s3" => Box::new(s3::S3Provider {}),
        _ => panic!("Unknown storage provider: {}", storage.provider),
    }
}
