use crate::services::api::models::agent::backup::BackupUploadResponse;
use crate::services::api::{ApiClient, ApiError};
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub struct StatusUploadRequest {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    #[serde(rename = "backupStorageId")]
    pub backup_storage_id: String,
    pub status: String,
    pub path: String,
    pub size: u64,
    #[serde(rename = "backupId")]
    pub backup_id: String,
    /// Counters of a successful snapshot or sync run, shown by the dashboard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<Value>,
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
        backup_id: impl Into<String>,
    ) -> Result<Option<BackupUploadResponse>, ApiError> {
        let body = StatusUploadRequest {
            generated_id: generated_id.into(),
            backup_storage_id: backup_storage_id.into(),
            status: status.into(),
            path: remote_path.into(),
            size: total_size.into(),
            backup_id: backup_id.into(),
            stats: None,
        };
        self.send_upload_status(agent_id.into(), &body).await
    }

    /// Success status of one snapshot or sync run, with its counters.
    pub async fn backup_upload_success(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        backup_storage_id: impl Into<String>,
        path: String,
        size: u64,
        stats: Value,
        backup_id: impl Into<String>,
    ) -> Result<Option<BackupUploadResponse>, ApiError> {
        let body = StatusUploadRequest {
            generated_id: generated_id.into(),
            backup_storage_id: backup_storage_id.into(),
            status: "success".into(),
            path,
            size,
            backup_id: backup_id.into(),
            stats: Some(stats),
        };
        self.send_upload_status(agent_id.into(), &body).await
    }

    async fn send_upload_status(
        &self,
        agent_id: String,
        body: &StatusUploadRequest,
    ) -> Result<Option<BackupUploadResponse>, ApiError> {
        let path = format!("/agent/{}/backup/upload/status", agent_id);
        self.request_with_body(Method::PATCH, path.as_str(), body).await
    }
}
