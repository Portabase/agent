#![allow(dead_code)]

use crate::core::context::Context;
use crate::services::config::DatabaseConfig;
use crate::settings::CONFIG;
use reqwest::Client;
use std::error::Error;
use std::sync::Arc;
use tracing::{error, info};
use crate::services::api::endpoints::status::{DatabasePayload, StatusRequest};
use crate::services::api::models::agent::status::PingResult;

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

        // let body = StatusRequest {
        //     version: &version_str,
        //     databases: databases_payload,
        // };



        let result = self.ctx.api.agent_status(&edge_key.agent_id, &version_str, databases_payload).await?.unwrap();



        // let url = format!(
        //     "{}/api/agent/{}/status",
        //     edge_key.server_url, edge_key.agent_id
        // );
        // info!("Status request | {}", url);
        //
        // let resp = self.client.post(&url).json(&body).send().await?;
        // if !resp.status().is_success() {
        //     let msg = format!("Request failed with status: {}", resp.status());
        //     error!("{}", msg);
        //     return Err(msg.into());
        // }
        //
        // let result: PingResult = resp.json().await?;

        // info!("Status response | {:#?}", result);

        Ok(result)
    }
}
