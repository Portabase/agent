pub mod upload;

use crate::services::api::models::agent::backup::BackupResponse;
use crate::services::api::{ApiClient, ApiError};
use crate::services::backup::logger::JobLogEntry;
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
    pub size: Option<u64>,
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub logs: Vec<JobLogEntry>,
    #[serde(rename = "durationMs")]
    pub duration_ms: f64,
    #[serde(rename = "parentBackupId", skip_serializing_if = "Option::is_none")]
    pub parent_backup_id: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PhysicalOutcome {
    pub parent_backup_id: Option<String>,
    pub manifest: String,
}

impl BackupUpdateRequest {
    pub fn new(
        backup_id: String,
        status: String,
        size: Option<u64>,
        generated_id: String,
        logs: Vec<JobLogEntry>,
        duration_ms: f64,
        physical: Option<&PhysicalOutcome>,
    ) -> Self {
        Self {
            backup_id,
            status,
            size,
            generated_id,
            logs,
            duration_ms,
            parent_backup_id: physical.map(|outcome| outcome.parent_backup_id.clone()),
            manifest: physical.map(|outcome| outcome.manifest.clone()),
        }
    }
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
        file_size: impl Into<Option<u64>>,
        generated_id: impl Into<String>,
        job_logs: Vec<JobLogEntry>,
        duration_ms: f64,
        physical: Option<&PhysicalOutcome>,
    ) -> Result<Option<BackupResponse>, ApiError> {
        let body = BackupUpdateRequest::new(
            backup_id.into(),
            status.into(),
            file_size.into(),
            generated_id.into(),
            job_logs,
            duration_ms,
            physical,
        );

        let agent_id = agent_id.into();
        let path = format!("/agent/{}/backup", agent_id);

        self.request_with_body(Method::PATCH, path.as_str(), &body)
            .await
    }
}
