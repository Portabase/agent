use crate::services::api::models::agent::restore::ResultRestoreResponse;
use crate::services::api::{ApiClient, ApiError};
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;

#[derive(Serialize)]
pub struct ResultRestoreRequest {
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub status: String,
}

impl ApiClient {
    pub async fn restore_result(
        &self,
        agent_id: impl Into<String>,
        generated_id: impl Into<String>,
        status: impl Into<String>,
    ) -> Result<Option<ResultRestoreResponse>, ApiError> {
        let body = ResultRestoreRequest {
            generated_id: generated_id.into(),
            status: status.into(),
        };

        let agent_id = agent_id.into();

        let path = format!("/agent/{}/restore", agent_id);

        self.request_with_body(Method::POST, path.as_str(), &body)
            .await
    }
}
