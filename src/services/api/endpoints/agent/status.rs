use crate::services::api::models::agent;
use crate::services::api::{ApiClient, ApiError};
use agent::status::PingResult;
use anyhow::Result;
use reqwest::Method;
use serde::Serialize;

#[derive(Serialize)]
pub struct DatabasePayload<'a> {
    pub name: &'a str,
    pub dbms: &'a str,
    #[serde(rename = "generatedId")]
    pub generated_id: &'a str,
}

#[derive(Serialize)]
pub struct StatusRequest<'a> {
    pub version: &'a str,
    pub databases: Vec<DatabasePayload<'a>>,
}

// impl ApiClient {
//     // pub async fn agent_status(&self, agent_id: impl Into<String>) -> Result<PingResult, ApiError> {
//     //     let agent_id = agent_id.into();
//     //     let path = format!("/agent/{}/status", agent_id);
//     //     self.request(Method::GET, path.as_str()).await
//     // }
// 
//     pub async fn agent_status<'a>(
//         &self,
//         agent_id: impl Into<String>,
//         version: &'static str,
//         databases: Vec<DatabasePayload<'a>>,
//     ) -> Result<PingResult, ApiError> {
// 
//         let body = StatusRequest {
//             version,
//             databases,
//         };
// 
//         let agent_id = agent_id.into();
//         let path = format!("/agent/{}/status", agent_id);
// 
//         self.request_with_body(Method::POST, path.as_str(), &body).await
//     }
// }

impl ApiClient {
    pub async fn agent_status<'a>(
        &self,
        agent_id: impl Into<String>,
        version: &'a str,
        databases: Vec<DatabasePayload<'a>>,
    ) -> Result<Option<PingResult>, ApiError> {
        let body = StatusRequest { version, databases };

        let agent_id = agent_id.into();
        let path = format!("/agent/{}/status", agent_id);

        self.request_with_body(Method::POST, path.as_str(), &body).await
    }
}