use crate::services::api::models::agent;
use crate::services::api::{ApiClient, ApiError};
use agent::status::PingResult;
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;

#[derive(Serialize)]
pub struct InitUploadRequest {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub path: String,
    pub storage_channel_id: String,
}

impl ApiClient {
    pub async fn backup_upload_init(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        path: impl Into<String>,
        storage_channel_id: impl Into<String>,
    ) -> Result<Option<PingResult>, ApiError> {
        let body = InitUploadRequest {
            generated_id: generated_id.into(),
            path: path.into(),
            storage_channel_id: storage_channel_id.into(),
        };

        let agent_id = agent_id.into();
        let path = format!("/agent/{}/backup/upload/init", agent_id);

        self.request_with_body(Method::POST, path.as_str(), &body)
            .await
    }
}
