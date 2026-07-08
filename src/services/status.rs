#![allow(dead_code)]

use crate::core::context::Context;
use crate::services::api::endpoints::status::DatabasePayload;
use crate::services::api::models::agent::status::PingResult;
use crate::services::api::models::agent::status::DatabaseStorage;
use crate::services::config::DatabaseConfig;
use crate::settings::CONFIG;
use crate::utils::file::decrypt_json_gcm;
use reqwest::Client;
use std::error::Error;
use std::sync::Arc;
use futures_util::future::try_join_all;
use tracing::info;
use crate::domain::factory::DatabaseFactory;

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

        let databases_payload: Vec<DatabasePayload> = try_join_all(
            databases.into_iter().map(|db| async move {
                let db_engine = DatabaseFactory::create_for_backup(db.clone()).await;

                let reachable = db_engine.ping().await?;
                info!("Ping {} => {:?}",db.name, reachable);

                Ok::<DatabasePayload, anyhow::Error>(DatabasePayload {
                    name: &db.name,
                    dbms: &db.db_type.as_str(),
                    generated_id: &db.generated_id,
                    ping_status: reachable,
                })
            })
        ).await?;

        let version_str = CONFIG.app_version.as_str();
        let mut result = self
            .ctx
            .api
            .agent_status(&edge_key.agent_id, &version_str, databases_payload)
            .await?
            .unwrap();

        for db in result.databases.iter_mut() {
            if db.storages_encrypted == Some(true) {
                let ciphertext = db
                    .storages_ciphertext
                    .as_deref()
                    .ok_or("storages_encrypted set but storages_ciphertext missing")?;

                let plaintext =
                    decrypt_json_gcm(ciphertext, &edge_key.master_key_b64)
                        .map_err(|e| format!("Failed to decrypt storages: {e}"))?;

                db.storages = serde_json::from_slice::<Vec<DatabaseStorage>>(&plaintext)
                    .map_err(|e| format!("Failed to parse decrypted storages: {e}"))?;
            }
        }

        Ok(result)
    }
}
