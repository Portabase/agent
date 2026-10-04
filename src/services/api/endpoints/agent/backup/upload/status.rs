use crate::services::api::models::agent::backup::BackupUploadResponse;
use crate::services::api::{ApiClient, ApiError};
use crate::services::sync::stats::SyncStats;
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
    #[serde(rename = "backupId")]
    pub backup_id: String,
    #[serde(rename = "filesTransferred", skip_serializing_if = "Option::is_none")]
    pub files_transferred: Option<u64>,
    #[serde(rename = "filesDeleted", skip_serializing_if = "Option::is_none")]
    pub files_deleted: Option<u64>,
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
            files_transferred: None,
            files_deleted: None,
        };
        self.send_upload_status(agent_id.into(), &body).await
    }

    /// Success status of one sync replica, with rclone's counters (`size` = bytes transferred).
    pub async fn backup_upload_sync_status(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        backup_storage_id: impl Into<String>,
        path: String,
        stats: &SyncStats,
        backup_id: impl Into<String>,
    ) -> Result<Option<BackupUploadResponse>, ApiError> {
        let body = StatusUploadRequest {
            generated_id: generated_id.into(),
            backup_storage_id: backup_storage_id.into(),
            status: "success".into(),
            path,
            size: stats.bytes,
            backup_id: backup_id.into(),
            files_transferred: Some(stats.transfers),
            files_deleted: Some(stats.deletes),
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
