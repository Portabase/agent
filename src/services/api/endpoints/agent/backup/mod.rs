pub mod upload;

use crate::services::api::models::agent::backup::BackupResponse;
use crate::services::api::{ApiClient, ApiError};
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;

#[derive(Serialize)]
pub struct BackupCreateRequest {
    pub method: String,
    #[serde(rename = "generatedId")]
    pub generated_id: String,
}

#[derive(Serialize)]
pub struct BackupUpdateRequest {
    #[serde(rename = "backupId")]
    pub backup_id: String,
    pub status: String,
    pub size: u64
}


impl ApiClient {
    pub async fn backup_create(
        &self,
        method: impl Into<String>,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
    ) -> Result<Option<BackupResponse>, ApiError> {
        let body = BackupCreateRequest {
            method: method.into(),
            generated_id: generated_id.into(),
        };

        let agent_id = agent_id.into();
        let path = format!("/agent/{}/backup", agent_id);

        self.request_with_body(Method::POST, path.as_str(), &body)
            .await
    }

    pub async fn backup_update(
        &self,
        agent_id: impl Into<String>,
        backup_id: impl Into<String>,
        status: impl Into<String>,
        file_size: impl Into<u64>
    ) -> Result<Option<BackupResponse>, ApiError> {
        let body = BackupUpdateRequest {
            backup_id: backup_id.into(),
            status: status.into(),
            size: file_size.into(),
        };

        let agent_id = agent_id.into();
        let path = format!("/agent/{}/backup", agent_id);

        self.request_with_body(Method::PATCH, path.as_str(), &body)
            .await
    }
}
