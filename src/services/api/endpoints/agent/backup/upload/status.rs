use crate::services::api::models::agent::backup::{BackupUploadResponse};
use crate::services::api::{ApiClient, ApiError};
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;

#[derive(Serialize)]
pub struct StatusUploadRequest {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    #[serde(rename = "backupStorageId")]
    pub backup_storage_id: String,
    pub status: String,
    pub path: String,
    pub size: u64,
}

impl ApiClient {
    pub async fn backup_upload_status(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        backup_storage_id: impl Into<String>,
        status: impl Into<String>,
        remote_path: impl Into<String>,
        total_size: impl Into<u64>,
    ) -> Result<Option<BackupUploadResponse>, ApiError> {
        let body = StatusUploadRequest {
            generated_id: generated_id.into(),
            backup_storage_id: backup_storage_id.into(),
            status: status.into(),
            path: remote_path.into(),
            size: total_size.into()
        };

        let agent_id = agent_id.into();
        let path = format!("/agent/{}/backup/upload/status", agent_id);

        self.request_with_body(Method::PATCH, path.as_str(), &body)
            .await
    }
}
