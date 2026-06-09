use crate::services::api::models::agent::restore::ResultRestoreResponse;
use crate::services::api::{ApiClient, ApiError};
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;
use crate::services::backup::logger::JobLogEntry;

#[derive(Serialize)]
pub struct ResultRestoreRequest {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub status: String,
    pub logs: Vec<JobLogEntry>,
    #[serde(rename = "durationMs")]
    pub duration_ms: f64,
}

impl ApiClient {
    pub async fn restore_result(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        status: impl Into<String>,
        job_logs: Vec<JobLogEntry>,
        duration_ms: f64,
    ) -> Result<Option<ResultRestoreResponse>, ApiError> {
        let body = ResultRestoreRequest {
            generated_id: generated_id.into(),
            status: status.into(),
            logs: job_logs,
            duration_ms,
        };

        let agent_id = agent_id.into();

        let path = format!("/agent/{}/restore", agent_id);

        self.request_with_body(Method::POST, path.as_str(), &body)
            .await
    }
}
