#![allow(dead_code)]

use crate::core::context::Context;
use crate::services::config::DatabaseConfig;
use crate::settings::CONFIG;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;
use tracing::{error, info};

/// Payload for sending database info in the request
#[derive(Serialize)]
struct DatabasePayload<'a> {
    name: &'a str,
    dbms: &'a str,
    #[serde(rename = "generatedId")]
    generated_id: &'a str,
}

/// Body for the status API request
#[derive(Serialize)]
struct StatusRequestBody<'a> {
    version: &'a str,
    databases: Vec<DatabasePayload<'a>>,
}

/// Typed structs for the response
#[derive(Debug, Deserialize)]
pub struct PingResult {
    pub agent: AgentInfo,
    pub databases: Vec<DatabaseStatus>,
}

#[derive(Debug, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    #[serde(rename = "lastContact")]
    pub last_contact: String,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseStatus {
    pub dbms: String,
    #[serde(rename = "generatedId")]
    pub generated_id: String,
    pub data: DatabaseData,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseData {
    pub backup: BackupInfo,
    pub restore: RestoreInfo,
}

#[derive(Debug, Deserialize)]
pub struct BackupInfo {
    pub action: bool,
    pub cron: Option<String>, // can be null
}

#[derive(Debug, Deserialize)]
pub struct RestoreInfo {
    pub action: bool,
    pub file: String,
}

/// Service for contacting the agent API
pub struct StatusService {
    ctx: Arc<Context>,
    client: Client,
}

impl StatusService {
    pub fn new(ctx: Arc<Context>) -> Self {
        StatusService {
            ctx,
            client: Client::new(),
        }
    }

    pub async fn ping(&self, databases: &[DatabaseConfig]) -> Result<PingResult, Box<dyn Error>> {
        let edge_key = &self.ctx.edge_key;

        let databases_payload: Vec<DatabasePayload> = databases
            .iter()
            .map(|db| DatabasePayload {
                name: &db.name,
                dbms: &db.db_type.as_str(),
                generated_id: &db.generated_id,
            })
            .collect();

        let version_str = CONFIG.app_version.as_str();

        let body = StatusRequestBody {
            version: &version_str,
            databases: databases_payload,
        };

        let url = format!(
            "{}/api/agent/{}/status",
            edge_key.server_url, edge_key.agent_id
        );
        info!("Status request | {}", url);

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let msg = format!("Request failed with status: {}", resp.status());
            error!("{}", msg);
            return Err(msg.into());
        }

        let result: PingResult = resp.json().await?;

        Ok(result)
    }
}
