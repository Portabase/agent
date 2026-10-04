use crate::services::api::models::agent::backup::BackupUploadResponse;
use crate::services::api::{ApiClient, ApiError};
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;

#[derive(Serialize)]
pub struct InitUploadRequest {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    #[serde(rename = "storageChannelId")]
    pub storage_channel_id: String,
    #[serde(rename = "backupId")]
    pub backup_id: String,
    /// "restic" for snapshots; omitted for archives (older dashboards ignore it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

impl ApiClient {
    pub async fn backup_upload_init(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        storage_channel_id: impl Into<String>,
        backup_id: impl Into<String>,
        engine: Option<&str>,
    ) -> Result<Option<BackupUploadResponse>, ApiError> {
        let body = InitUploadRequest {
            generated_id: generated_id.into(),
            storage_channel_id: storage_channel_id.into(),
            backup_id: backup_id.into(),
            engine: engine.map(str::to_string),
        };

        let agent_id = agent_id.into();
        let path = format!("/agent/{}/backup/upload/init", agent_id);

        self.request_with_body(Method::POST, path.as_str(), &body)
            .await
    }
}
