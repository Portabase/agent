use crate::services::api::{ApiClient, ApiError};
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;
use crate::services::api::models::agent::backup::BackupUploadResponse;

#[derive(Serialize)]
pub struct InitUploadRequest {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    #[serde(rename = "storageChannelId")]
    pub storage_channel_id: String,
}

impl ApiClient {
    pub async fn backup_upload_init(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        storage_channel_id: impl Into<String>,
    ) -> Result<Option<BackupUploadResponse>, ApiError> {
        let body = InitUploadRequest {
            generated_id: generated_id.into(),
            storage_channel_id: storage_channel_id.into(),
        };


        let agent_id = agent_id.into();
        let path = format!("/agent/{}/backup/upload/init", agent_id);

        self.request_with_body(Method::POST, path.as_str(), &body)
            .await
    }
}
